//! Putting a run back the way it was.
//!
//! Undo reads a journal and replays it backwards: each verified move is moved
//! back, each trashed entry is restored to where it came from, each directory
//! the run created is removed if it is empty. It uses the same engine as the
//! forward run, so restoring cannot overwrite anything either — a destination
//! that has been re-occupied since is reported, not clobbered.
//!
//! The undo writes its own journal, which makes it undoable in turn.

use std::path::{Path, PathBuf};

use crate::engine::{execute_move, move_file, preflight, CrossDevice, VerifyLevel};
use crate::journal::{read_journal, session_id, Journal, Record};
use crate::FsError;

#[derive(Debug, Clone)]
pub struct UndoOptions {
    pub execute: bool,
    pub verify: VerifyLevel,
    pub cross_device: CrossDevice,
    pub journal: bool,
}

impl Default for UndoOptions {
    fn default() -> Self {
        Self {
            execute: false,
            verify: VerifyLevel::Metadata,
            cross_device: CrossDevice::Refuse,
            journal: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct UndoReport {
    pub dry_run: bool,
    pub restored: Vec<(PathBuf, PathBuf)>,
    pub removed_dirs: Vec<PathBuf>,
    pub errors: Vec<String>,
    pub journal: Option<PathBuf>,
}

pub fn undo(journal_path: &Path, opts: &UndoOptions) -> Result<UndoReport, FsError> {
    let records = read_journal(journal_path)?;
    let state_dir = journal_path.parent().unwrap_or(Path::new(".")).to_path_buf();

    let mut journal = if opts.execute && opts.journal {
        Journal::create(&state_dir, &format!("{}-undo", session_id()))
            .map_err(|source| FsError::Journal { path: state_dir.clone(), source })?
    } else {
        Journal::disabled()
    };

    let mut report = UndoReport {
        dry_run: !opts.execute,
        journal: journal.path().map(|p| p.to_path_buf()),
        ..Default::default()
    };

    for record in records.iter().rev() {
        match record {
            // The move succeeded, so `to` holds the data: send it home.
            Record::MoveDone { from, to, .. } => {
                restore(to, from, opts, &mut journal, &mut report, true);
            }
            Record::Trashed { original, trash } => {
                restore(trash, original, opts, &mut journal, &mut report, false);
            }
            Record::DirCreated { path } => {
                match crate::engine::is_empty_dir(path) {
                    Ok(true) => {
                        if opts.execute {
                            if let Err(e) = std::fs::remove_dir(path) {
                                report.errors.push(format!("{}: {e}", path.display()));
                                continue;
                            }
                        }
                        report.removed_dirs.push(path.clone());
                    }
                    // Someone put something there; leave it alone.
                    Ok(false) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => report.errors.push(format!("{}: {e}", path.display())),
                }
            }
            _ => {}
        }
    }

    Ok(report)
}

fn restore(
    from: &Path,
    to: &Path,
    opts: &UndoOptions,
    journal: &mut Journal,
    report: &mut UndoReport,
    expect_dir: bool,
) {
    if !opts.execute {
        // Report exactly what the real run would attempt, including the checks
        // that would stop it.
        match std::fs::symlink_metadata(from) {
            Err(e) => report.errors.push(format!("{}: {e}", from.display())),
            Ok(_) => match std::fs::symlink_metadata(to) {
                Ok(_) => report.errors.push(format!("{} is occupied again; would not restore", to.display())),
                Err(_) => report.restored.push((from.to_path_buf(), to.to_path_buf())),
            },
        }
        return;
    }

    let result = if expect_dir {
        match preflight(from, to, None, opts.verify) {
            Ok(before) => execute_move(from, to, &before, opts.verify, opts.cross_device).map(|_| ()),
            Err(refusal) => {
                report.errors.push(refusal.0);
                return;
            }
        }
    } else {
        move_file(from, to, opts.verify, opts.cross_device).map(|_| ())
    };

    match result {
        Ok(()) => {
            let _ = journal.append(&Record::MoveDone {
                id: report.restored.len(),
                from: from.to_path_buf(),
                to: to.to_path_buf(),
                method: "undo".to_string(),
                verified: opts.verify.as_str().to_string(),
            });
            report.restored.push((from.to_path_buf(), to.to_path_buf()));
        }
        Err(failure) => report.errors.push(failure.error),
    }
}
