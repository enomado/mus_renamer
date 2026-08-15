//! Deleting, without deleting.
//!
//! The cleaning commands never unlink anything. A file that is "removed" is
//! moved to `<root>/.mus_renamer/trash/<session>/<its original path>`, which is
//! a rename on the same filesystem: instant, and reversible with `undo`.
//! Emptying the trash is a separate, explicit act.

use std::path::{Path, PathBuf};

use mus_core::plan::STATE_DIR;
use mus_core::Config;

use crate::engine::{is_empty_dir, rename_noreplace};
use crate::journal::{session_id, Journal, Record};

#[derive(Debug, Clone)]
pub struct CleanOptions {
    pub execute: bool,
    /// Move files whose extension is not in the audio list to the trash.
    pub non_audio: bool,
    /// Remove directories that hold nothing at all (`rmdir`, never recursive).
    pub empty_dirs: bool,
    pub state_dir: Option<PathBuf>,
}

impl Default for CleanOptions {
    fn default() -> Self {
        Self { execute: false, non_audio: false, empty_dirs: true, state_dir: None }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CleanReport {
    pub dry_run: bool,
    pub trashed: Vec<(PathBuf, PathBuf)>,
    pub removed_dirs: Vec<PathBuf>,
    pub errors: Vec<String>,
    pub journal: Option<PathBuf>,
    pub trash_dir: Option<PathBuf>,
}

pub fn clean(root: &Path, cfg: &Config, opts: &CleanOptions) -> Result<CleanReport, crate::FsError> {
    let session = session_id();
    let state_dir = opts.state_dir.clone().unwrap_or_else(|| root.join(STATE_DIR));
    let trash_dir = state_dir.join("trash").join(&session);

    let mut journal = if opts.execute {
        Journal::create(&state_dir, &format!("{session}-clean"))
            .map_err(|source| crate::FsError::Journal { path: state_dir.clone(), source })?
    } else {
        Journal::disabled()
    };

    let mut report = CleanReport {
        dry_run: !opts.execute,
        journal: journal.path().map(|p| p.to_path_buf()),
        trash_dir: Some(trash_dir.clone()),
        ..Default::default()
    };

    if opts.non_audio {
        for entry in walkdir::WalkDir::new(root)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|e| e.file_name() != std::ffi::OsStr::new(STATE_DIR))
        {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    report.errors.push(e.to_string());
                    continue;
                }
            };
            if entry.depth() == 0 || entry.file_type().is_dir() {
                continue;
            }
            let path = entry.path();
            if cfg.is_audio(path) {
                continue;
            }
            match trash_one(root, path, &trash_dir, opts.execute, &mut journal) {
                Ok(dest) => report.trashed.push((path.to_path_buf(), dest)),
                Err(e) => report.errors.push(format!("{}: {e}", path.display())),
            }
        }
    }

    if opts.empty_dirs {
        // Deepest first, repeatedly, so a directory that only held empty
        // directories also goes. `remove_dir` fails on anything non-empty, so
        // this can never take data with it.
        loop {
            let mut removed_this_round = 0usize;
            let mut dirs: Vec<PathBuf> = walkdir::WalkDir::new(root)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| e.file_name() != std::ffi::OsStr::new(STATE_DIR))
                .filter_map(|e| e.ok())
                .filter(|e| e.depth() > 0 && e.file_type().is_dir())
                .map(|e| e.path().to_path_buf())
                .collect();
            dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));

            for dir in dirs {
                if report.removed_dirs.contains(&dir) {
                    continue;
                }
                match is_empty_dir(&dir) {
                    Ok(true) => {
                        if opts.execute {
                            if let Err(e) = std::fs::remove_dir(&dir) {
                                report.errors.push(format!("{}: {e}", dir.display()));
                                continue;
                            }
                        }
                        report.removed_dirs.push(dir);
                        removed_this_round += 1;
                    }
                    Ok(false) => {}
                    Err(e) => report.errors.push(format!("{}: {e}", dir.display())),
                }
            }
            // A dry run cannot converge by actually deleting, so one pass is all
            // it gets; the report then lists the leaves only.
            if removed_this_round == 0 || !opts.execute {
                break;
            }
        }
    }

    journal
        .append(&Record::Footer {
            applied: report.trashed.len(),
            failed: report.errors.len(),
            skipped: report.removed_dirs.len(),
        })
        .map_err(crate::FsError::JournalWrite)?;

    Ok(report)
}

/// Move one path into the trash, preserving its location under the root.
pub fn trash_one(
    root: &Path,
    path: &Path,
    trash_dir: &Path,
    execute: bool,
    journal: &mut Journal,
) -> std::io::Result<PathBuf> {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let mut dest = trash_dir.join(rel);

    if !execute {
        return Ok(dest);
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // The trash is not a place where things overwrite each other either.
    let mut n = 2;
    while dest.symlink_metadata().is_ok() {
        let name = format!("{} ({n})", dest.file_name().unwrap_or_default().to_string_lossy());
        dest = trash_dir.join(rel).with_file_name(name);
        n += 1;
    }

    journal.append(&Record::Trashed { original: path.to_path_buf(), trash: dest.clone() })?;
    rename_noreplace(path, &dest)?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &[u8])]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, body) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        dir
    }

    #[test]
    fn cleaning_moves_junk_to_the_trash_instead_of_deleting_it() {
        let root = tree(&[("album/a.mp3", b"x"), ("album/cover.jpg", b"j"), ("album/notes.txt", b"t")]);
        let cfg = Config::default();
        let opts = CleanOptions { execute: true, non_audio: true, empty_dirs: false, ..Default::default() };

        let report = clean(root.path(), &cfg, &opts).unwrap();

        assert_eq!(report.trashed.len(), 2);
        assert!(root.path().join("album/a.mp3").exists());
        assert!(!root.path().join("album/cover.jpg").exists());
        // Still on disk, just moved aside.
        let trashed: Vec<_> = report.trashed.iter().map(|(_, to)| to.clone()).collect();
        assert!(trashed.iter().all(|p| p.exists()), "{trashed:?}");
        assert_eq!(std::fs::read(&trashed[0]).unwrap(), b"j");
    }

    #[test]
    fn a_dry_run_of_clean_changes_nothing() {
        let root = tree(&[("album/a.mp3", b"x"), ("album/cover.jpg", b"j")]);
        let opts = CleanOptions { execute: false, non_audio: true, empty_dirs: true, ..Default::default() };
        let report = clean(root.path(), &Config::default(), &opts).unwrap();

        assert_eq!(report.trashed.len(), 1);
        assert!(root.path().join("album/cover.jpg").exists());
        assert!(report.journal.is_none());
    }

    #[test]
    fn empty_directories_are_removed_bottom_up_but_full_ones_never_are() {
        let root = tree(&[("keep/a.mp3", b"x")]);
        std::fs::create_dir_all(root.path().join("gone/inner/deeper")).unwrap();

        let opts = CleanOptions { execute: true, non_audio: false, empty_dirs: true, ..Default::default() };
        let report = clean(root.path(), &Config::default(), &opts).unwrap();

        assert!(!root.path().join("gone").exists());
        assert!(root.path().join("keep/a.mp3").exists());
        assert_eq!(report.removed_dirs.len(), 3);
    }
}
