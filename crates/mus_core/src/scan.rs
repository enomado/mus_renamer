//! Walking a music tree into one [`FolderScan`] per directory.
//!
//! The scan is read-only. Besides the tags it records a [`DirStamp`] per folder:
//! the identity the folder had *at scan time*. `mus_fs` re-checks that stamp
//! immediately before moving anything, so a plan built against a tree that has
//! since changed is refused instead of applied blindly.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::config::Config;
use crate::tags::{TagReader, TrackTags};

/// Identity of a directory as observed during the scan.
///
/// `dev`/`ino` pin the exact directory (a rename of an ancestor does not change
/// them, a swap for a different directory does). `entries` and `mtime_ns` catch
/// content changes made between planning and applying.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirStamp {
    pub dev: u64,
    pub ino: u64,
    pub entries: usize,
    pub mtime_ns: i128,
}

impl DirStamp {
    pub fn of(path: &Path) -> std::io::Result<Self> {
        let meta = std::fs::symlink_metadata(path)?;
        if !meta.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                format!("{} is not a directory", path.display()),
            ));
        }
        let entries = std::fs::read_dir(path)?.count();
        Ok(Self { dev: device_of(&meta), ino: inode_of(&meta), entries, mtime_ns: mtime_ns(&meta) })
    }

    /// Same directory, same contents — safe to act on the plan that recorded this.
    pub fn matches(&self, other: &DirStamp) -> bool {
        self == other
    }
}

#[cfg(unix)]
fn device_of(meta: &std::fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::dev(meta)
}
#[cfg(unix)]
fn inode_of(meta: &std::fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::ino(meta)
}

#[cfg(not(unix))]
fn device_of(_meta: &std::fs::Metadata) -> u64 {
    0
}
#[cfg(not(unix))]
fn inode_of(_meta: &std::fs::Metadata) -> u64 {
    0
}

fn mtime_ns(meta: &std::fs::Metadata) -> i128 {
    match meta.modified().ok().map(|t| t.duration_since(std::time::UNIX_EPOCH)) {
        Some(Ok(d)) => d.as_nanos() as i128,
        Some(Err(e)) => -(e.duration().as_nanos() as i128),
        None => 0,
    }
}

/// One directory of the tree, with everything needed to classify it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderScan {
    pub dir: PathBuf,
    /// Path components between the scan root and this folder (root itself = 0).
    pub depth: usize,
    pub stamp: DirStamp,
    pub tracks: Vec<TrackEntry>,
    /// Files whose tags could not be read at all — a reason to stay careful.
    pub unreadable: Vec<UnreadableTrack>,
    pub non_audio_files: Vec<PathBuf>,
    pub subdirs: Vec<PathBuf>,
}

impl FolderScan {
    pub fn audio_count(&self) -> usize {
        self.tracks.len() + self.unreadable.len()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackEntry {
    pub path: PathBuf,
    pub tags: TrackTags,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnreadableTrack {
    pub path: PathBuf,
    pub error: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("scan root {0} does not exist or is not a directory")]
    BadRoot(PathBuf),
    #[error("walking {path}: {source}")]
    Walk {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Read-only walk of `root`. The root itself is scanned too (depth 0) so callers
/// can report loose files lying in it, but planning never renames the root.
pub fn scan_tree(root: &Path, cfg: &Config, reader: &dyn TagReader) -> Result<Vec<FolderScan>, ScanError> {
    if !root.is_dir() {
        return Err(ScanError::BadRoot(root.to_path_buf()));
    }

    let mut out = Vec::new();
    let walker = WalkDir::new(root)
        .follow_links(cfg.scan.follow_symlinks)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            if !cfg.scan.include_hidden && name.starts_with('.') && e.file_type().is_dir() {
                return false;
            }
            if e.file_type().is_dir() && cfg.scan.excluded_dirs.iter().any(|d| *d == name) {
                return false;
            }
            true
        });

    for entry in walker {
        let entry = entry.map_err(|e| ScanError::Walk {
            path: e.path().unwrap_or(root).to_path_buf(),
            source: std::io::Error::other(e.to_string()),
        })?;
        if !entry.file_type().is_dir() {
            continue;
        }
        out.push(scan_folder(entry.path(), entry.depth(), cfg, reader)?);
    }

    Ok(out)
}

/// Scan a single directory (non-recursive).
pub fn scan_folder(
    dir: &Path,
    depth: usize,
    cfg: &Config,
    reader: &dyn TagReader,
) -> Result<FolderScan, ScanError> {
    let mut tracks = Vec::new();
    let mut unreadable = Vec::new();
    let mut non_audio_files = Vec::new();
    let mut subdirs = Vec::new();

    let read =
        std::fs::read_dir(dir).map_err(|source| ScanError::Walk { path: dir.to_path_buf(), source })?;

    let mut children: Vec<PathBuf> = Vec::new();
    for entry in read {
        let entry = entry.map_err(|source| ScanError::Walk { path: dir.to_path_buf(), source })?;
        children.push(entry.path());
    }
    children.sort();

    for path in children {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let hidden = name.starts_with('.');
        // A symlink is never followed for classification: its target may live
        // outside the tree, and moving the link is not moving the music.
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(source) => return Err(ScanError::Walk { path: path.clone(), source }),
        };

        if meta.is_dir() {
            if cfg.scan.excluded_dirs.contains(&name) {
                continue;
            }
            if hidden && !cfg.scan.include_hidden {
                continue;
            }
            subdirs.push(path);
            continue;
        }

        if hidden && !cfg.scan.include_hidden {
            continue;
        }
        if !cfg.is_audio(&path) {
            non_audio_files.push(path);
            continue;
        }
        if meta.is_symlink() {
            unreadable.push(UnreadableTrack { path, error: "symlink, not followed".to_string() });
            continue;
        }

        match reader.read(&path) {
            Ok(tags) => tracks.push(TrackEntry { path, tags }),
            Err(err) => unreadable.push(UnreadableTrack { path, error: err.to_string() }),
        }
    }

    let stamp = DirStamp::of(dir).map_err(|source| ScanError::Walk { path: dir.to_path_buf(), source })?;

    Ok(FolderScan { dir: dir.to_path_buf(), depth, stamp, tracks, unreadable, non_audio_files, subdirs })
}
