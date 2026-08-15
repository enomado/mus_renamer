//! Domain core of `mus_renamer`.
//!
//! The core answers one question: *given a music tree, which folder should be
//! renamed to what*. It reads tags and directory listings, but it never moves,
//! deletes or overwrites anything — producing a [`Plan`] is the only output.
//! Executing the plan is the job of the `mus_fs` crate.
//!
//! Layers:
//! * [`tags`] — reading track metadata (a trait, so tests do not need real mp3s)
//! * [`scan`] — walking the tree into per-folder [`scan::FolderScan`]s
//! * [`classify`] — deciding what a folder *is* (album / collection / VA / ambiguous)
//! * [`naming`] — templates, transliteration, filesystem-safe sanitizing
//! * [`plan`] — turning classifications into a validated, collision-free [`Plan`]

pub mod classify;
pub mod config;
pub mod naming;
pub mod plan;
pub mod scan;
pub mod tags;
#[doc(hidden)]
pub mod testing;

pub use classify::{classify, FolderKind};
pub use config::Config;
pub use plan::{build_plan, Plan, PlannedMove, Skip};
pub use scan::{scan_tree, DirStamp, FolderScan};
pub use tags::{TagReader, TrackTags};
