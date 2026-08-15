//! End-to-end tests of the promise the crate makes: whatever happens, the music
//! is still there.
//!
//! The oracle used throughout is *data conservation*: the multiset of file
//! contents (blake3 hashes) under the root must be identical before and after a
//! run, no matter how the folders were shuffled. Renames move data around; they
//! never create or destroy any.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mus_core::config::UntaggedPolicy;
use mus_core::tags::{MapReader, TrackTags};
use mus_core::{build_plan, scan_tree, Config, Plan};
use mus_fs::{apply, ApplyOptions, CrossDevice, VerifyLevel};

/// blake3 → how many files have exactly these bytes, anywhere under `root`.
fn contents(root: &Path) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.unwrap();
        if !entry.file_type().is_file() {
            continue;
        }
        // The journal is written by the run itself; it is bookkeeping, not data.
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("journal-") && name.ends_with(".jsonl") {
            continue;
        }
        let hash = mus_fs::inventory::hash_file(entry.path()).unwrap();
        *out.entry(hash).or_insert(0) += 1;
    }
    out
}

/// Every path under `root`, relative and sorted — the shape of the tree.
fn shape(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.depth() > 0)
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            !(name.starts_with("journal-") && name.ends_with(".jsonl"))
        })
        .map(|e| e.path().strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    out.sort();
    out
}

struct Fixture {
    dir: tempfile::TempDir,
    reader: MapReader,
    cfg: Config,
}

impl Fixture {
    fn new() -> Self {
        let mut cfg = Config::default();
        // The fixtures carry the tags that matter and nothing else.
        cfg.classify.untagged_year = UntaggedPolicy::AsUnknown;
        Self { dir: tempfile::tempdir().unwrap(), reader: MapReader::new(), cfg }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Create `<root>/<rel>` holding `body`, tagged with artist/album/year.
    fn track(&mut self, rel: &str, artist: &str, album: &str, year: Option<u32>, body: &[u8]) -> PathBuf {
        let path = self.root().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        self.reader = std::mem::take(&mut self.reader).with(
            path.clone(),
            TrackTags {
                artist: Some(artist.to_string()),
                album: Some(album.to_string()),
                year,
                ..Default::default()
            },
        );
        path
    }

    fn file(&self, rel: &str, body: &[u8]) {
        let path = self.root().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn plan(&self) -> Plan {
        let scans = scan_tree(self.root(), &self.cfg, &self.reader).unwrap();
        build_plan(self.root(), &scans, &self.cfg).unwrap()
    }
}

fn execute() -> ApplyOptions {
    ApplyOptions {
        execute: true,
        verify: VerifyLevel::Hash,
        cross_device: CrossDevice::Refuse,
        journal: true,
        state_dir: None,
        check_stamps: true,
        stop_on_first_error: false,
    }
}

#[test]
fn a_plain_album_folder_is_renamed_and_nothing_is_lost() {
    let mut fx = Fixture::new();
    fx.track("incoming/01.mp3", "Kino", "Gruppa krovi", Some(1988), b"one");
    fx.track("incoming/02.mp3", "Kino", "Gruppa krovi", Some(1988), b"two");
    fx.file("incoming/cover.jpg", b"jpeg");

    let before = contents(fx.root());
    let plan = fx.plan();
    assert_eq!(plan.moves.len(), 1);
    assert_eq!(plan.moves[0].to, fx.root().join("Kino - 1988 (Gruppa krovi)"));

    let report = apply(&plan, &execute()).unwrap();
    assert!(report.is_clean(), "{report:?}");
    assert_eq!(report.applied.len(), 1);

    assert_eq!(contents(fx.root()), before, "data was not conserved");
    assert!(fx.root().join("Kino - 1988 (Gruppa krovi)/cover.jpg").exists());
    assert!(!fx.root().join("incoming").exists());
}

#[test]
fn two_folders_that_want_the_same_name_both_survive() {
    let mut fx = Fixture::new();
    // Two rips of the same album, in different folders.
    fx.track("rip-a/01.mp3", "Kino", "Best", Some(1990), b"a");
    fx.track("rip-b/01.mp3", "Kino", "Best", Some(1990), b"b");

    let before = contents(fx.root());
    let plan = fx.plan();
    assert_eq!(plan.moves.len(), 2);

    let destinations: Vec<_> = plan.moves.iter().map(|m| m.to.clone()).collect();
    assert_ne!(destinations[0], destinations[1], "the plan aimed both at one path");

    let report = apply(&plan, &execute()).unwrap();
    assert!(report.is_clean(), "{report:?}");
    assert_eq!(contents(fx.root()), before, "one of the two rips was lost");
    assert!(fx.root().join("Kino - 1990 (Best)").is_dir());
    assert!(fx.root().join("Kino - 1990 (Best) (2)").is_dir());
}

#[test]
fn a_destination_that_already_exists_on_disk_is_not_touched() {
    let mut fx = Fixture::new();
    fx.track("incoming/01.mp3", "Kino", "Best", Some(1990), b"new");
    // Someone already has a folder under exactly the name we would pick.
    fx.file("Kino - 1990 (Best)/precious.mp3", b"precious");

    let before = contents(fx.root());
    let plan = fx.plan();
    let report = apply(&plan, &execute()).unwrap();

    assert!(report.is_clean(), "{report:?}");
    assert_eq!(contents(fx.root()), before);
    assert_eq!(std::fs::read(fx.root().join("Kino - 1990 (Best)/precious.mp3")).unwrap(), b"precious");
    assert!(fx.root().join("Kino - 1990 (Best) (2)/01.mp3").exists());
}

#[test]
fn a_folder_already_named_correctly_is_left_alone() {
    let mut fx = Fixture::new();
    fx.track("Kino - 1990 (Best)/01.mp3", "Kino", "Best", Some(1990), b"x");

    let plan = fx.plan();
    assert!(plan.moves.is_empty(), "{:?}", plan.moves);
    assert!(plan.skipped.iter().any(|s| s.reason == "already named correctly"));
}

#[test]
fn nested_albums_move_out_before_their_container() {
    let mut fx = Fixture::new();
    fx.track("dump/kino/01.mp3", "Kino", "Best", Some(1990), b"k");
    fx.track("dump/aria/01.mp3", "Aria", "Hero", Some(1987), b"a");

    let before = contents(fx.root());
    let plan = fx.plan();
    let report = apply(&plan, &execute()).unwrap();

    assert!(report.is_clean(), "{report:?}");
    assert_eq!(contents(fx.root()), before);
    assert!(fx.root().join("Kino - 1990 (Best)/01.mp3").exists());
    assert!(fx.root().join("Aria - 1987 (Hero)/01.mp3").exists());
}

#[test]
fn a_dry_run_changes_absolutely_nothing() {
    let mut fx = Fixture::new();
    fx.track("incoming/01.mp3", "Kino", "Best", Some(1990), b"x");
    fx.track("other/01.mp3", "Aria", "Hero", Some(1987), b"y");

    let before_shape = shape(fx.root());
    let before_contents = contents(fx.root());

    let plan = fx.plan();
    let report = apply(&plan, &ApplyOptions { execute: false, ..execute() }).unwrap();

    assert!(report.dry_run);
    assert_eq!(report.applied.len(), 2);
    assert!(report.journal.is_none(), "a dry run must not even create a journal");
    assert_eq!(shape(fx.root()), before_shape);
    assert_eq!(contents(fx.root()), before_contents);
}

#[test]
fn a_plan_built_against_a_tree_that_has_since_changed_is_refused() {
    let mut fx = Fixture::new();
    fx.track("incoming/01.mp3", "Kino", "Best", Some(1990), b"x");

    let plan = fx.plan();

    // Someone drops another track in while we were thinking.
    std::fs::write(fx.root().join("incoming/02.mp3"), b"later").unwrap();
    let before = contents(fx.root());

    let report = apply(&plan, &execute()).unwrap();

    assert_eq!(report.applied.len(), 0);
    assert_eq!(report.refused.len(), 1);
    assert!(report.refused[0].reason.contains("changed since the plan"));
    assert_eq!(contents(fx.root()), before);
    assert!(fx.root().join("incoming/02.mp3").exists());
}

#[test]
fn ignoring_stamps_is_possible_but_has_to_be_asked_for() {
    let mut fx = Fixture::new();
    fx.track("incoming/01.mp3", "Kino", "Best", Some(1990), b"x");
    let plan = fx.plan();
    std::fs::write(fx.root().join("incoming/02.mp3"), b"later").unwrap();
    let before = contents(fx.root());

    let report = apply(&plan, &ApplyOptions { check_stamps: false, ..execute() }).unwrap();

    assert_eq!(report.applied.len(), 1);
    // Even then, the late arrival travels with the folder — nothing is dropped.
    assert_eq!(contents(fx.root()), before);
    assert!(fx.root().join("Kino - 1990 (Best)/02.mp3").exists());
}

#[test]
fn undo_puts_everything_back_where_it_was() {
    let mut fx = Fixture::new();
    fx.track("incoming/01.mp3", "Kino", "Best", Some(1990), b"x");
    fx.track("second/01.mp3", "Aria", "Hero", Some(1987), b"y");

    let before_shape = shape(fx.root());
    let before_contents = contents(fx.root());

    let plan = fx.plan();
    let report = apply(&plan, &execute()).unwrap();
    assert_eq!(report.applied.len(), 2);
    let journal = report.journal.clone().unwrap();
    assert_ne!(shape(fx.root()), before_shape);

    let undo = mus_fs::undo(
        &journal,
        &mus_fs::UndoOptions {
            execute: true,
            verify: VerifyLevel::Hash,
            cross_device: CrossDevice::Refuse,
            // The undo's own journal would show up in the tree comparison below.
            journal: false,
        },
    )
    .unwrap();

    assert!(undo.errors.is_empty(), "{:?}", undo.errors);
    assert_eq!(undo.restored.len(), 2);
    assert_eq!(contents(fx.root()), before_contents);

    // The only difference from the original tree is the journal we wrote.
    let after: Vec<String> =
        shape(fx.root()).into_iter().filter(|p| !p.starts_with(".mus_renamer")).collect();
    assert_eq!(after, before_shape);
}

#[test]
fn undo_refuses_to_overwrite_a_path_that_was_reoccupied() {
    let mut fx = Fixture::new();
    fx.track("incoming/01.mp3", "Kino", "Best", Some(1990), b"x");

    let plan = fx.plan();
    let report = apply(&plan, &execute()).unwrap();
    let journal = report.journal.clone().unwrap();

    // Someone recreated `incoming` and put something of their own in it.
    std::fs::create_dir(fx.root().join("incoming")).unwrap();
    std::fs::write(fx.root().join("incoming/mine.mp3"), b"mine").unwrap();
    let before = contents(fx.root());

    let undo = mus_fs::undo(&journal, &mus_fs::UndoOptions { execute: true, ..Default::default() }).unwrap();

    assert_eq!(undo.restored.len(), 0);
    assert_eq!(undo.errors.len(), 1);
    assert!(undo.errors[0].contains("already exists"), "{:?}", undo.errors);
    assert_eq!(contents(fx.root()), before);
    assert_eq!(std::fs::read(fx.root().join("incoming/mine.mp3")).unwrap(), b"mine");
}

#[test]
fn applying_the_same_plan_twice_moves_nothing_the_second_time() {
    let mut fx = Fixture::new();
    fx.track("incoming/01.mp3", "Kino", "Best", Some(1990), b"x");

    let plan = fx.plan();
    let first = apply(&plan, &execute()).unwrap();
    assert_eq!(first.applied.len(), 1);
    let after_first = contents(fx.root());

    let second = apply(&plan, &execute()).unwrap();
    assert_eq!(second.applied.len(), 0);
    assert_eq!(second.refused.len(), 1);
    assert!(second.failed.is_empty());
    assert_eq!(contents(fx.root()), after_first);
}

#[test]
fn re_scanning_after_a_run_produces_an_empty_plan() {
    let mut fx = Fixture::new();
    fx.track("incoming/01.mp3", "Kino", "Best", Some(1990), b"x");
    fx.track("more/01.mp3", "Aria", "Hero", Some(1987), b"y");

    let plan = fx.plan();
    apply(&plan, &execute()).unwrap();

    // The reader is keyed by path, and the paths just changed — re-register.
    let mut fx2 = Fixture::new();
    let dir = std::mem::replace(&mut fx.dir, tempfile::tempdir().unwrap());
    fx2.dir = dir;
    fx2.track("Kino - 1990 (Best)/01.mp3", "Kino", "Best", Some(1990), b"x");
    fx2.track("Aria - 1987 (Hero)/01.mp3", "Aria", "Hero", Some(1987), b"y");

    let plan = fx2.plan();
    assert!(plan.moves.is_empty(), "second pass wants to move {:?}", plan.moves);
}

#[test]
fn the_state_directory_is_never_scanned_or_moved() {
    let mut fx = Fixture::new();
    fx.track("incoming/01.mp3", "Kino", "Best", Some(1990), b"x");
    fx.file(".mus_renamer/trash/old/leftover.mp3", b"leftover");

    let plan = fx.plan();
    assert!(
        plan.moves.iter().all(|m| !m.from.starts_with(fx.root().join(".mus_renamer"))),
        "{:?}",
        plan.moves
    );
    apply(&plan, &execute()).unwrap();
    assert_eq!(std::fs::read(fx.root().join(".mus_renamer/trash/old/leftover.mp3")).unwrap(), b"leftover");
}

#[test]
fn a_folder_with_mixed_tags_is_left_alone_rather_than_guessed_at() {
    let mut fx = Fixture::new();
    fx.track("mixed/01.mp3", "Kino", "Best", Some(1990), b"a");
    fx.track("mixed/02.mp3", "Aria", "Hero", Some(1987), b"b");

    let plan = fx.plan();
    assert!(plan.moves.is_empty());
    assert_eq!(plan.skipped.len(), 1);
    assert!(plan.skipped[0].reason.contains("2 artists and 2 albums"));
}

#[test]
fn cleaning_then_undoing_restores_the_junk() {
    let mut fx = Fixture::new();
    fx.track("album/01.mp3", "Kino", "Best", Some(1990), b"x");
    fx.file("album/cover.jpg", b"jpeg");
    fx.file("album/notes.txt", b"notes");

    let before = contents(fx.root());

    let report = mus_fs::clean(
        fx.root(),
        &fx.cfg,
        &mus_fs::CleanOptions { execute: true, non_audio: true, empty_dirs: false, state_dir: None },
    )
    .unwrap();
    assert_eq!(report.trashed.len(), 2);
    // Trashing is a move: the bytes are all still there.
    assert_eq!(contents(fx.root()), before);

    let journal = report.journal.clone().unwrap();
    let undo =
        mus_fs::undo(&journal, &mus_fs::UndoOptions { execute: true, journal: false, ..Default::default() })
            .unwrap();

    assert!(undo.errors.is_empty(), "{:?}", undo.errors);
    assert_eq!(std::fs::read(fx.root().join("album/cover.jpg")).unwrap(), b"jpeg");
    assert_eq!(std::fs::read(fx.root().join("album/notes.txt")).unwrap(), b"notes");
    assert_eq!(contents(fx.root()), before);
}

#[test]
fn cyrillic_folders_are_transliterated_for_a_car_stereo() {
    let mut fx = Fixture::new();
    fx.track("вход/01.mp3", "Ария", "Герой асфальта", Some(1987), b"x");

    let plan = fx.plan();
    assert_eq!(plan.moves.len(), 1);
    assert_eq!(plan.moves[0].to, fx.root().join("Ariya - 1987 (Geroy asfalta)"));

    let report = apply(&plan, &execute()).unwrap();
    assert!(report.is_clean(), "{report:?}");
    assert!(fx.root().join("Ariya - 1987 (Geroy asfalta)/01.mp3").exists());
}

#[test]
fn a_slash_in_a_tag_cannot_escape_into_the_path() {
    let mut fx = Fixture::new();
    fx.track("incoming/01.mp3", "AC/DC", "Back/Black", Some(1980), b"x");

    let plan = fx.plan();
    assert_eq!(plan.moves.len(), 1);
    let name = plan.moves[0].to.file_name().unwrap().to_string_lossy().to_string();
    assert!(!name.contains('/'), "{name}");
    assert_eq!(plan.moves[0].to.parent().unwrap(), fx.root());

    apply(&plan, &execute()).unwrap();
    assert!(fx.root().join("AC DC - 1980 (Back Black)/01.mp3").exists());
}

#[test]
fn the_journal_records_every_move_that_happened() {
    let mut fx = Fixture::new();
    fx.track("a/01.mp3", "Kino", "Best", Some(1990), b"x");
    fx.track("b/01.mp3", "Aria", "Hero", Some(1987), b"y");

    let plan = fx.plan();
    let report = apply(&plan, &execute()).unwrap();
    let records = mus_fs::read_journal(&report.journal.clone().unwrap()).unwrap();

    let done: Vec<_> = records.iter().filter(|r| matches!(r, mus_fs::Record::MoveDone { .. })).collect();
    assert_eq!(done.len(), 2);
    assert!(matches!(records.first(), Some(mus_fs::Record::Header { .. })));
    assert!(matches!(records.last(), Some(mus_fs::Record::Footer { .. })));
}

#[test]
fn symlinked_tracks_are_reported_not_followed() {
    let mut fx = Fixture::new();
    fx.track("album/01.mp3", "Kino", "Best", Some(1990), b"x");
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/passwd", fx.root().join("album/link.mp3")).unwrap();

    let scans = scan_tree(fx.root(), &fx.cfg, &fx.reader).unwrap();
    let album = scans.iter().find(|s| s.dir.ends_with("album")).unwrap();

    #[cfg(unix)]
    {
        assert_eq!(album.unreadable.len(), 1);
        assert!(album.unreadable[0].error.contains("symlink"));
        // An unreadable track makes the folder ambiguous, so it stays put.
        let plan = build_plan(fx.root(), &scans, &fx.cfg).unwrap();
        assert!(plan.moves.is_empty());
    }
    #[cfg(not(unix))]
    let _ = album;
}
