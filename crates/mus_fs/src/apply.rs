//! Executing a [`Plan`] move by move, journalling as it goes.

use std::path::{Path, PathBuf};

use mus_core::plan::STATE_DIR;
use mus_core::Plan;

use crate::engine::{execute_move, preflight, CrossDevice, MoveMethod, VerifyLevel};
use crate::journal::{session_id, unix_ms, Journal, Record, JOURNAL_FORMAT_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("opening the journal in {path}: {source}")]
    Journal {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("writing the journal: {0}")]
    JournalWrite(#[source] std::io::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct ApplyOptions {
    /// `false` runs every check and reports what would happen, touching nothing.
    pub execute: bool,
    pub verify: VerifyLevel,
    pub cross_device: CrossDevice,
    /// Write a journal (and thus make the run undoable).
    pub journal: bool,
    /// Where the journal and the trash live. Defaults to `<root>/.mus_renamer`.
    pub state_dir: Option<PathBuf>,
    /// Refuse folders that changed between planning and applying.
    pub check_stamps: bool,
    pub stop_on_first_error: bool,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            execute: false,
            verify: VerifyLevel::Metadata,
            cross_device: CrossDevice::Refuse,
            journal: true,
            state_dir: None,
            check_stamps: true,
            stop_on_first_error: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppliedMove {
    pub id: usize,
    pub from: PathBuf,
    pub to: PathBuf,
    /// `None` in a dry run: the move passed every check but was not performed.
    pub method: Option<MoveMethod>,
    pub files: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct RefusedMove {
    pub id: usize,
    pub from: PathBuf,
    pub to: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct FailedMove {
    pub id: usize,
    pub from: PathBuf,
    pub to: PathBuf,
    pub error: String,
    pub source_intact: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ApplyReport {
    pub dry_run: bool,
    pub session: String,
    pub journal: Option<PathBuf>,
    pub applied: Vec<AppliedMove>,
    pub refused: Vec<RefusedMove>,
    pub failed: Vec<FailedMove>,
}

impl ApplyReport {
    pub fn is_clean(&self) -> bool {
        self.failed.is_empty()
    }

    pub fn files(&self) -> usize {
        self.applied.iter().map(|a| a.files).sum()
    }

    pub fn bytes(&self) -> u64 {
        self.applied.iter().map(|a| a.bytes).sum()
    }
}

/// Progress, for a CLI line or a UI log.
#[derive(Debug, Clone)]
pub enum Event<'a> {
    Started { moves: usize, dry_run: bool },
    Applied(&'a AppliedMove),
    Refused(&'a RefusedMove),
    Failed(&'a FailedMove),
    Finished(&'a ApplyReport),
}

pub fn apply(plan: &Plan, opts: &ApplyOptions) -> Result<ApplyReport, FsError> {
    apply_with(plan, opts, &mut |_| {})
}

pub fn apply_with(
    plan: &Plan,
    opts: &ApplyOptions,
    observer: &mut dyn FnMut(Event<'_>),
) -> Result<ApplyReport, FsError> {
    let session = session_id();
    let state_dir = opts.state_dir.clone().unwrap_or_else(|| plan.root.join(STATE_DIR));

    let mut journal = if opts.execute && opts.journal {
        Journal::create(&state_dir, &session)
            .map_err(|source| FsError::Journal { path: state_dir.clone(), source })?
    } else {
        Journal::disabled()
    };

    journal
        .append(&Record::Header {
            version: JOURNAL_FORMAT_VERSION,
            session: session.clone(),
            unix_ms: unix_ms(),
            root: plan.root.clone(),
            planned_moves: plan.moves.len(),
            verify: opts.verify.as_str().to_string(),
        })
        .map_err(FsError::JournalWrite)?;

    let mut report = ApplyReport {
        dry_run: !opts.execute,
        session,
        journal: journal.path().map(|p| p.to_path_buf()),
        ..Default::default()
    };

    observer(Event::Started { moves: plan.moves.len(), dry_run: !opts.execute });

    for m in &plan.moves {
        let stamp = opts.check_stamps.then_some(&m.stamp);
        let before = match preflight(&m.from, &m.to, stamp, opts.verify) {
            Ok(inv) => inv,
            Err(refusal) => {
                report.refused.push(RefusedMove {
                    id: m.id,
                    from: m.from.clone(),
                    to: m.to.clone(),
                    reason: refusal.0,
                });
                observer(Event::Refused(report.refused.last().unwrap()));
                continue;
            }
        };

        if !opts.execute {
            report.applied.push(AppliedMove {
                id: m.id,
                from: m.from.clone(),
                to: m.to.clone(),
                method: None,
                files: before.file_count(),
                bytes: before.total_bytes(),
            });
            observer(Event::Applied(report.applied.last().unwrap()));
            continue;
        }

        journal
            .append(&Record::MoveBefore {
                id: m.id,
                from: m.from.clone(),
                to: m.to.clone(),
                files: before.file_count(),
                bytes: before.total_bytes(),
            })
            .map_err(FsError::JournalWrite)?;

        match execute_move(&m.from, &m.to, &before, opts.verify, opts.cross_device) {
            Ok(outcome) => {
                for dir in &outcome.created_dirs {
                    journal
                        .append(&Record::DirCreated { path: dir.clone() })
                        .map_err(FsError::JournalWrite)?;
                }
                journal
                    .append(&Record::MoveDone {
                        id: m.id,
                        from: m.from.clone(),
                        to: m.to.clone(),
                        method: outcome.method.as_str().to_string(),
                        verified: outcome.verified.as_str().to_string(),
                    })
                    .map_err(FsError::JournalWrite)?;
                report.applied.push(AppliedMove {
                    id: m.id,
                    from: m.from.clone(),
                    to: m.to.clone(),
                    method: Some(outcome.method),
                    files: outcome.files,
                    bytes: outcome.bytes,
                });
                observer(Event::Applied(report.applied.last().unwrap()));
            }
            Err(failure) => {
                journal
                    .append(&Record::MoveFailed {
                        id: m.id,
                        from: m.from.clone(),
                        to: m.to.clone(),
                        error: failure.error.clone(),
                        source_intact: failure.source_intact,
                    })
                    .map_err(FsError::JournalWrite)?;
                report.failed.push(FailedMove {
                    id: m.id,
                    from: m.from.clone(),
                    to: m.to.clone(),
                    error: failure.error,
                    source_intact: failure.source_intact,
                });
                observer(Event::Failed(report.failed.last().unwrap()));
                if opts.stop_on_first_error {
                    break;
                }
            }
        }
    }

    journal
        .append(&Record::Footer {
            applied: report.applied.len(),
            failed: report.failed.len(),
            skipped: report.refused.len(),
        })
        .map_err(FsError::JournalWrite)?;

    observer(Event::Finished(&report));
    Ok(report)
}

/// Default state directory for a music root.
pub fn state_dir_of(root: &Path) -> PathBuf {
    root.join(STATE_DIR)
}
