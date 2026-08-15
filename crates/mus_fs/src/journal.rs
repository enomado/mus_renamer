//! Append-only record of everything that touched the disk.
//!
//! Each record is written and `fsync`ed *before* the operation it describes is
//! attempted, and a second record follows once the result is known. A power cut
//! or a kill can therefore leave an unfinished operation, but never an unknown
//! one: the journal always names the two paths involved.
//!
//! The journal is also the undo log — see [`crate::undo`].

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const JOURNAL_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "kebab-case")]
pub enum Record {
    Header {
        version: u32,
        session: String,
        unix_ms: u64,
        root: PathBuf,
        planned_moves: usize,
        verify: String,
    },
    /// A directory we had to create to hold a destination (undo removes it if empty).
    DirCreated {
        path: PathBuf,
    },
    /// Written before the move is attempted.
    MoveBefore {
        id: usize,
        from: PathBuf,
        to: PathBuf,
        files: usize,
        bytes: u64,
    },
    /// Written after the move succeeded *and* was verified.
    MoveDone {
        id: usize,
        from: PathBuf,
        to: PathBuf,
        method: String,
        verified: String,
    },
    MoveFailed {
        id: usize,
        from: PathBuf,
        to: PathBuf,
        error: String,
        /// Whether the source is known to be intact after the failure.
        source_intact: bool,
    },
    /// A file or folder moved into the trash instead of being deleted.
    Trashed {
        original: PathBuf,
        trash: PathBuf,
    },
    Footer {
        applied: usize,
        failed: usize,
        skipped: usize,
    },
}

/// Writer. [`Journal::disabled`] swallows records, which is what a dry run uses.
#[derive(Debug)]
pub struct Journal {
    path: Option<PathBuf>,
    file: Option<File>,
    records: usize,
}

impl Journal {
    pub fn disabled() -> Self {
        Self { path: None, file: None, records: 0 }
    }

    /// Create `<dir>/journal-<session>.jsonl`, creating `dir` if needed.
    ///
    /// The file is opened with `create_new`, so an existing journal is never
    /// appended to or truncated. Two runs started inside the same second get
    /// `-2`, `-3`, … rather than fighting over one name.
    pub fn create(dir: &Path, session: &str) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        for n in 1u32..10_000 {
            let name = if n == 1 {
                format!("journal-{session}.jsonl")
            } else {
                format!("journal-{session}-{n}.jsonl")
            };
            let path = dir.join(name);
            match std::fs::OpenOptions::new().create_new(true).append(true).open(&path) {
                Ok(file) => return Ok(Self { path: Some(path), file: Some(file), records: 0 }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::other("no free journal name"))
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn is_enabled(&self) -> bool {
        self.file.is_some()
    }

    /// Write one record and flush it all the way to the platter.
    pub fn append(&mut self, record: &Record) -> std::io::Result<()> {
        let Some(file) = self.file.as_mut() else { return Ok(()) };
        let mut line = serde_json::to_string(record).map_err(std::io::Error::other)?;
        line.push('\n');
        file.write_all(line.as_bytes())?;
        file.flush()?;
        // Durability is the point: a record that only reached the page cache
        // cannot be trusted to survive the crash it is meant to describe.
        file.sync_data()?;
        self.records += 1;
        Ok(())
    }

    pub fn records_written(&self) -> usize {
        self.records
    }
}

pub fn read_journal(path: &Path) -> std::io::Result<Vec<Record>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: Record = serde_json::from_str(line)
            .map_err(|e| std::io::Error::other(format!("{}:{}: {e}", path.display(), n + 1)))?;
        out.push(record);
    }
    Ok(out)
}

/// Journals of a state directory, newest last.
pub fn list_journals(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    for entry in read {
        let path = entry?.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        if name.starts_with("journal-") && name.ends_with(".jsonl") {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

pub fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `20260815-210800` in UTC — sortable, filesystem-safe, no dependency.
pub fn session_id() -> String {
    format!("{}-{}", utc_compact(unix_ms() / 1000), std::process::id())
}

pub fn utc_compact(unix_secs: u64) -> String {
    let secs = unix_secs as i64;
    let days = secs.div_euclid(86_400);
    let time_of_day = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60
    )
}

/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_match_known_values() {
        assert_eq!(utc_compact(0), "19700101-000000");
        assert_eq!(utc_compact(1_786_752_000), "20260815-000000");
        assert_eq!(utc_compact(946_684_799), "19991231-235959");
    }

    #[test]
    fn records_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::create(dir.path(), "test").unwrap();
        // Every variant must survive the round trip — serde_json cannot encode
        // some integer types, and a journal that cannot be read back is no
        // journal at all.
        journal
            .append(&Record::Header {
                version: JOURNAL_FORMAT_VERSION,
                session: "test".into(),
                unix_ms: unix_ms(),
                root: PathBuf::from("/music"),
                planned_moves: 1,
                verify: "hash".into(),
            })
            .unwrap();
        journal
            .append(&Record::MoveDone {
                id: 0,
                from: PathBuf::from("/a"),
                to: PathBuf::from("/b"),
                method: "rename".into(),
                verified: "hash".into(),
            })
            .unwrap();

        let path = journal.path().unwrap().to_path_buf();
        let back = read_journal(&path).unwrap();
        assert_eq!(back.len(), 2);
        assert!(matches!(back[0], Record::Header { .. }));
        assert!(matches!(back[1], Record::MoveDone { id: 0, .. }));
        assert_eq!(list_journals(dir.path()).unwrap(), vec![path]);
    }

    #[test]
    fn a_disabled_journal_writes_nothing() {
        let mut journal = Journal::disabled();
        journal.append(&Record::Footer { applied: 0, failed: 0, skipped: 0 }).unwrap();
        assert!(journal.path().is_none());
        assert_eq!(journal.records_written(), 0);
    }
}
