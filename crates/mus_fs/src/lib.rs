//! Filesystem execution layer for `mus_renamer`.
//!
//! The core decides *what* should happen; this crate is the only code allowed to
//! make it happen, and it is built around one promise: **no operation can
//! destroy or overwrite data.**
//!
//! How that promise is kept:
//!
//! * **Nothing is ever written over.** Moves use `renameat2(RENAME_NOREPLACE)`
//!   where the kernel supports it, and an explicit existence check where it does
//!   not. There is no `create`/`truncate` of an existing path anywhere.
//! * **Nothing is deleted.** `clean` moves files into `<root>/.mus_renamer/trash/`
//!   (a rename, not a copy). Only empty directories are actually removed, with
//!   `rmdir`, which fails on anything that still holds data.
//! * **Every move is proved.** The source's file tree — names, sizes, and with
//!   `--verify hash` a blake3 of every byte — is captured before the move and
//!   compared after it. A mismatch fails the move and rolls it back.
//! * **A copy is deleted only after it is verified.** Across filesystems the
//!   data is copied into a private staging directory, hashed against the
//!   original, put in place, and only then is the original removed.
//! * **The tree is re-checked.** A plan carries the identity each source folder
//!   had when it was built; a folder that changed in between is refused.
//! * **Everything is written down.** Each operation is journalled and `fsync`ed
//!   before it is attempted, so [`undo`] can walk the run backwards.

pub mod apply;
pub mod engine;
pub mod inventory;
pub mod journal;
pub mod trash;
pub mod undo;

pub use apply::{apply, apply_with, ApplyOptions, ApplyReport, Event, FsError};
pub use engine::{CrossDevice, MoveMethod, VerifyLevel};
pub use inventory::{inventory, Inventory};
pub use journal::{list_journals, read_journal, Journal, Record};
pub use trash::{clean, CleanOptions, CleanReport};
pub use undo::{undo, UndoOptions, UndoReport};
