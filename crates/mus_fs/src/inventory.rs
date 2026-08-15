//! A directory's contents, captured as a comparable value.
//!
//! Taken before a move and again after it, the two inventories are the proof
//! that nothing was lost, truncated or silently replaced. Sizes catch the
//! ordinary failures; `--verify hash` catches the rest.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
    /// Fifo, socket, block/char device — never copied across devices.
    Special,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryEntry {
    /// Path relative to the inventoried directory, `/`-separated.
    pub rel: String,
    pub kind: EntryKind,
    pub len: u64,
    /// Hex blake3 of the file contents, when hashing was requested.
    pub hash: Option<String>,
    /// Symlink target, so a link is verified by where it points.
    pub link_target: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    pub entries: Vec<InventoryEntry>,
}

impl Inventory {
    pub fn file_count(&self) -> usize {
        self.entries.iter().filter(|e| e.kind == EntryKind::File).count()
    }

    pub fn total_bytes(&self) -> u64 {
        self.entries.iter().filter(|e| e.kind == EntryKind::File).map(|e| e.len).sum()
    }

    pub fn has_special_files(&self) -> bool {
        self.entries.iter().any(|e| e.kind == EntryKind::Special)
    }

    /// Human-readable list of the differences, empty when the two match.
    ///
    /// Hashes are compared only where both sides have one, so a metadata-level
    /// "before" can still be checked against a hashed "after".
    pub fn diff(&self, other: &Inventory) -> Vec<String> {
        let mut out = Vec::new();
        for a in &self.entries {
            match other.entries.iter().find(|b| b.rel == a.rel) {
                None => out.push(format!("missing after the move: {}", a.rel)),
                Some(b) => {
                    if a.kind != b.kind {
                        out.push(format!("{}: {:?} became {:?}", a.rel, a.kind, b.kind));
                    } else if a.len != b.len {
                        out.push(format!("{}: {} bytes became {} bytes", a.rel, a.len, b.len));
                    } else if let (Some(ha), Some(hb)) = (&a.hash, &b.hash) {
                        if ha != hb {
                            out.push(format!("{}: contents changed ({ha} != {hb})", a.rel));
                        }
                    }
                    if a.link_target != b.link_target {
                        out.push(format!("{}: symlink target changed", a.rel));
                    }
                }
            }
        }
        for b in &other.entries {
            if !self.entries.iter().any(|a| a.rel == b.rel) {
                out.push(format!("appeared out of nowhere: {}", b.rel));
            }
        }
        out
    }
}

/// Walk `dir` and record everything in it. Symlinks are recorded, never followed.
pub fn inventory(dir: &Path, hash: bool) -> std::io::Result<Inventory> {
    let mut entries = Vec::new();

    for entry in WalkDir::new(dir).follow_links(false).sort_by_file_name() {
        let entry =
            entry.map_err(|e| e.into_io_error().unwrap_or_else(|| std::io::Error::other("walk failed")))?;
        if entry.depth() == 0 {
            continue;
        }
        let path = entry.path();
        let rel = relative(dir, path);
        let meta = std::fs::symlink_metadata(path)?;
        let file_type = meta.file_type();

        let (kind, len, link_target) = if file_type.is_symlink() {
            let target = std::fs::read_link(path)?.to_string_lossy().to_string();
            (EntryKind::Symlink, 0, Some(target))
        } else if file_type.is_dir() {
            (EntryKind::Dir, 0, None)
        } else if file_type.is_file() {
            (EntryKind::File, meta.len(), None)
        } else {
            (EntryKind::Special, 0, None)
        };

        let hash = if hash && kind == EntryKind::File { Some(hash_file(path)?) } else { None };

        entries.push(InventoryEntry { rel, kind, len, hash, link_target });
    }

    entries.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(Inventory { entries })
}

pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut file = std::fs::File::open(path)?;
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().to_hex().to_string())
}

fn relative(base: &Path, path: &Path) -> String {
    let rel: PathBuf = path.strip_prefix(base).unwrap_or(path).to_path_buf();
    rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_notices_a_lost_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.mp3"), b"aaa").unwrap();
        std::fs::write(dir.path().join("b.mp3"), b"bbbb").unwrap();

        let before = inventory(dir.path(), true).unwrap();
        assert_eq!(before.file_count(), 2);
        assert_eq!(before.total_bytes(), 7);

        std::fs::remove_file(dir.path().join("b.mp3")).unwrap();
        let after = inventory(dir.path(), true).unwrap();

        let diff = before.diff(&after);
        assert_eq!(diff.len(), 1);
        assert!(diff[0].contains("b.mp3"), "{diff:?}");
    }

    #[test]
    fn inventory_notices_changed_contents_of_the_same_size() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.mp3"), b"aaa").unwrap();
        let before = inventory(dir.path(), true).unwrap();

        std::fs::write(dir.path().join("a.mp3"), b"zzz").unwrap();
        let after = inventory(dir.path(), true).unwrap();

        assert!(before.diff(&after)[0].contains("contents changed"));
        // Without hashing the same-size overwrite is invisible — that is exactly
        // why `--verify hash` exists.
        let before_meta = Inventory {
            entries: before
                .entries
                .iter()
                .cloned()
                .map(|mut e| {
                    e.hash = None;
                    e
                })
                .collect(),
        };
        let after_meta = Inventory {
            entries: after
                .entries
                .iter()
                .cloned()
                .map(|mut e| {
                    e.hash = None;
                    e
                })
                .collect(),
        };
        assert!(before_meta.diff(&after_meta).is_empty());
    }

    #[test]
    fn nested_directories_are_inventoried_by_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("cd1")).unwrap();
        std::fs::write(dir.path().join("cd1/track.mp3"), b"x").unwrap();

        let inv = inventory(dir.path(), false).unwrap();
        let rels: Vec<&str> = inv.entries.iter().map(|e| e.rel.as_str()).collect();
        assert_eq!(rels, vec!["cd1", "cd1/track.mp3"]);
    }
}
