//! Turning verdicts into a validated set of moves.
//!
//! A [`Plan`] is the only thing the core produces, and it is inert: it can be
//! printed, diffed, stored as JSON and re-read. Every property that protects
//! data is established *here*, before a single byte moves:
//!
//! * no two moves share a destination;
//! * no destination collides with a name that already exists in the target
//!   directory — including names that are only there because another folder is
//!   about to vacate them (we never count on a move having happened yet);
//! * a folder already sitting under its correct name is skipped, not "moved onto itself";
//! * no move puts a folder inside itself, outside the root, or onto the root;
//! * every move carries the source's [`DirStamp`], so the executor can refuse a
//!   plan built against a tree that has changed since.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::classify::{classify, FolderKind};
use crate::config::{Config, Layout};
use crate::naming::{build_name, fields_of, NameAllocator, NameError};
use crate::scan::{DirStamp, FolderScan};

/// Directory inside the music root that holds journals and the trash.
/// Never scanned, never moved.
pub const STATE_DIR: &str = ".mus_renamer";

pub const PLAN_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub format_version: u32,
    pub root: PathBuf,
    pub config: Config,
    pub moves: Vec<PlannedMove>,
    pub skipped: Vec<Skip>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedMove {
    /// Position in the plan; also the id used in the journal.
    pub id: usize,
    pub from: PathBuf,
    pub to: PathBuf,
    pub kind: FolderKind,
    /// Identity of `from` when the plan was built.
    pub stamp: DirStamp,
    pub audio_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skip {
    pub dir: PathBuf,
    pub kind: FolderKind,
    pub reason: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error(transparent)]
    Name(#[from] NameError),
    #[error("listing {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.moves.is_empty()
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(text: &str) -> serde_json::Result<Self> {
        serde_json::from_str(text)
    }
}

/// Build the plan for a scanned tree. Pure with respect to the tree: the only
/// filesystem access is listing destination directories to learn which names
/// are already occupied.
pub fn build_plan(root: &Path, scans: &[FolderScan], cfg: &Config) -> Result<Plan, PlanError> {
    let root = root.to_path_buf();
    let mut moves: Vec<PlannedMove> = Vec::new();
    let mut skipped: Vec<Skip> = Vec::new();

    // One allocator per destination directory, seeded with what is on disk there.
    let mut allocators: Vec<(PathBuf, NameAllocator)> = Vec::new();

    // Deepest folders first: a nested album leaves its parent before the parent
    // itself is considered, so no move ever needs a move that has not happened.
    let mut ordered: Vec<&FolderScan> = scans.iter().filter(|s| s.depth > 0).collect();
    ordered.sort_by(|a, b| b.depth.cmp(&a.depth).then_with(|| a.dir.cmp(&b.dir)));

    for scan in ordered {
        let kind = classify(scan, &cfg.classify);
        if !kind.is_renameable() {
            skipped.push(Skip { dir: scan.dir.clone(), kind: kind.clone(), reason: skip_reason(&kind) });
            continue;
        }

        let Some(fields) = fields_of(&kind, &cfg.classify) else {
            continue;
        };

        // Where does this folder belong, and under which name?
        let (dest_dir, desired) = match (cfg.layout.layout, &kind) {
            (Layout::ArtistFolders, FolderKind::Album { .. }) => {
                let artist_dir = build_name("{artist}", &fields, &cfg.naming)?;
                (
                    root.join(artist_dir),
                    build_name(&cfg.layout.album_subfolder_template, &fields, &cfg.naming)?,
                )
            }
            (_, FolderKind::Album { .. }) => {
                (root.clone(), build_name(&cfg.naming.album_template, &fields, &cfg.naming)?)
            }
            (_, FolderKind::Collection { .. }) => {
                (root.clone(), build_name(&cfg.naming.collection_template, &fields, &cfg.naming)?)
            }
            (_, FolderKind::VariousArtists { .. }) => {
                (root.clone(), build_name(&cfg.naming.various_template, &fields, &cfg.naming)?)
            }
            _ => continue,
        };

        let allocator = allocator_for(&mut allocators, &dest_dir, cfg)?;

        // Already where it belongs: reserve the name so nothing else claims it,
        // and emit no move at all.
        if scan.dir.parent() == Some(dest_dir.as_path())
            && scan.dir.file_name().map(|n| n.to_string_lossy().to_string()).as_deref()
                == Some(desired.as_str())
        {
            allocator.reserve(&desired);
            skipped.push(Skip { dir: scan.dir.clone(), kind, reason: "already named correctly".to_string() });
            continue;
        }

        let name = allocator.allocate(&desired);
        let to = dest_dir.join(&name);

        if let Err(reason) = validate_move(&root, &scan.dir, &to) {
            skipped.push(Skip { dir: scan.dir.clone(), kind, reason });
            continue;
        }

        moves.push(PlannedMove {
            id: moves.len(),
            from: scan.dir.clone(),
            to,
            kind,
            stamp: scan.stamp.clone(),
            audio_files: scan.audio_count(),
        });
    }

    debug_assert!(no_duplicate_destinations(&moves));

    Ok(Plan { format_version: PLAN_FORMAT_VERSION, root, config: cfg.clone(), moves, skipped })
}

fn skip_reason(kind: &FolderKind) -> String {
    match kind {
        FolderKind::Container => "contains sub-folders".to_string(),
        FolderKind::TooFewTracks { audio_files } => format!("{audio_files} audio files"),
        FolderKind::Ambiguous { reason } => reason.clone(),
        _ => String::new(),
    }
}

fn allocator_for<'a>(
    allocators: &'a mut Vec<(PathBuf, NameAllocator)>,
    dest_dir: &Path,
    cfg: &Config,
) -> Result<&'a mut NameAllocator, PlanError> {
    if let Some(idx) = allocators.iter().position(|(p, _)| p == dest_dir) {
        return Ok(&mut allocators[idx].1);
    }
    let mut allocator =
        NameAllocator::new(cfg.naming.case_insensitive_collisions, cfg.naming.max_component_bytes);
    // Everything already in the destination is off limits — including entries
    // that are themselves scheduled to move away. Freeing those would make the
    // plan depend on an ordering the filesystem does not guarantee.
    for name in existing_names(dest_dir)? {
        allocator.reserve(&name);
    }
    allocators.push((dest_dir.to_path_buf(), allocator));
    let idx = allocators.len() - 1;
    Ok(&mut allocators[idx].1)
}

fn existing_names(dir: &Path) -> Result<Vec<String>, PlanError> {
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        // A destination directory that does not exist yet (artist-folders layout)
        // simply has no occupied names.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(PlanError::Io { path: dir.to_path_buf(), source }),
    };
    let mut names = Vec::new();
    for entry in read {
        let entry = entry.map_err(|source| PlanError::Io { path: dir.to_path_buf(), source })?;
        names.push(entry.file_name().to_string_lossy().to_string());
    }
    Ok(names)
}

/// Structural checks that do not depend on tags.
fn validate_move(root: &Path, from: &Path, to: &Path) -> Result<(), String> {
    if from == root {
        return Err("refusing to move the root itself".to_string());
    }
    if from == to {
        return Err("source and destination are the same path".to_string());
    }
    if to.starts_with(from) {
        return Err(format!("destination {} is inside the source", to.display()));
    }
    if !to.starts_with(root) {
        return Err(format!("destination {} is outside the root", to.display()));
    }
    if to == root {
        return Err("destination is the root itself".to_string());
    }
    Ok(())
}

fn no_duplicate_destinations(moves: &[PlannedMove]) -> bool {
    let mut seen = HashSet::new();
    moves.iter().all(|m| seen.insert(m.to.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_into_its_own_source_is_rejected() {
        let root = Path::new("/music");
        assert!(validate_move(root, Path::new("/music/Kino"), Path::new("/music/Kino/1988")).is_err());
        assert!(validate_move(root, Path::new("/music"), Path::new("/music/x")).is_err());
        assert!(validate_move(root, Path::new("/music/a/b"), Path::new("/elsewhere/b")).is_err());
        assert!(validate_move(root, Path::new("/music/a/b"), Path::new("/music/b")).is_ok());
    }
}
