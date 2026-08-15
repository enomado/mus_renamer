//! The binary, driven the way a user drives it.
//!
//! These tests care about the two things a command line promises: what it does
//! to the disk, and what it says on the way out (exit codes included).

use std::path::Path;
use std::process::Command;

use mus_core::testing::write_tagged_mp3;

fn mp3rnm() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mp3rnm"))
}

struct Run {
    code: i32,
    stdout: String,
}

fn run(args: &[&str]) -> Run {
    let output = mp3rnm().args(args).output().expect("running mp3rnm");
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
    }
}

fn entries(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write_tagged_mp3(&dir.path().join("incoming/01.mp3"), "Kino", "Gruppa krovi", Some(1988));
    write_tagged_mp3(&dir.path().join("incoming/02.mp3"), "Kino", "Gruppa krovi", Some(1988));
    dir
}

#[test]
fn plan_reports_the_new_name_without_touching_anything() {
    let dir = fixture();
    let before = entries(dir.path());

    let run = run(&["plan", dir.path().to_str().unwrap()]);

    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(run.stdout.contains("Kino - 1988 (Gruppa krovi)"), "{}", run.stdout);
    assert_eq!(entries(dir.path()), before);
}

#[test]
fn apply_is_a_dry_run_unless_execute_is_given() {
    let dir = fixture();
    let before = entries(dir.path());

    let run = run(&["apply", dir.path().to_str().unwrap()]);

    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(run.stdout.contains("dry run"), "{}", run.stdout);
    assert_eq!(entries(dir.path()), before);
}

#[test]
fn apply_execute_renames_and_leaves_an_undoable_journal() {
    let dir = fixture();
    let root = dir.path().to_str().unwrap();

    let applied = run(&["apply", root, "--execute", "-y", "--verify", "hash"]);
    assert_eq!(applied.code, 0, "{}", applied.stdout);
    assert_eq!(
        entries(dir.path()),
        vec![".mus_renamer".to_string(), "Kino - 1988 (Gruppa krovi)".to_string()]
    );
    assert!(dir.path().join("Kino - 1988 (Gruppa krovi)/01.mp3").exists());

    let journals = run(&["journals", root]);
    assert_eq!(journals.code, 0);
    assert!(journals.stdout.contains("journal-"), "{}", journals.stdout);

    let undone = run(&["undo", root, "--execute", "-y"]);
    assert_eq!(undone.code, 0, "{}", undone.stdout);
    assert!(dir.path().join("incoming/01.mp3").exists());
    assert!(!dir.path().join("Kino - 1988 (Gruppa krovi)").exists());
}

#[test]
fn a_saved_plan_can_be_applied_later() {
    let dir = fixture();
    let root = dir.path().to_str().unwrap();
    // The plan file lives outside the tree it describes, so writing it cannot
    // change what the plan is about.
    let elsewhere = tempfile::tempdir().unwrap();
    let plan_file = elsewhere.path().join("plan.json");

    let planned = run(&["plan", root, "--out", plan_file.to_str().unwrap()]);
    assert_eq!(planned.code, 0, "{}", planned.stdout);
    assert!(plan_file.exists());

    let applied = run(&["apply", "--plan", plan_file.to_str().unwrap(), "--execute", "-y"]);
    assert_eq!(applied.code, 0, "{}", applied.stdout);
    assert!(dir.path().join("Kino - 1988 (Gruppa krovi)/01.mp3").exists());
}

#[test]
fn clean_moves_junk_to_the_trash_and_undo_brings_it_back() {
    let dir = fixture();
    let root = dir.path().to_str().unwrap();
    std::fs::write(dir.path().join("incoming/cover.jpg"), b"jpeg").unwrap();

    let cleaned = run(&["clean", root, "--non-audio", "--execute", "-y"]);
    assert_eq!(cleaned.code, 0, "{}", cleaned.stdout);
    assert!(!dir.path().join("incoming/cover.jpg").exists());

    // Still on disk, in the trash.
    let trashed: Vec<_> = walkdir::WalkDir::new(dir.path().join(".mus_renamer/trash"))
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .collect();
    assert_eq!(trashed.len(), 1);
    assert_eq!(std::fs::read(trashed[0].path()).unwrap(), b"jpeg");

    let undone = run(&["undo", root, "--execute", "-y"]);
    assert_eq!(undone.code, 0, "{}", undone.stdout);
    assert_eq!(std::fs::read(dir.path().join("incoming/cover.jpg")).unwrap(), b"jpeg");
}

#[test]
fn a_folder_with_two_artists_is_reported_and_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    write_tagged_mp3(&dir.path().join("mixed/01.mp3"), "Kino", "A", Some(1988));
    write_tagged_mp3(&dir.path().join("mixed/02.mp3"), "Aria", "B", Some(1987));
    let before = entries(dir.path());

    let run = run(&["apply", dir.path().to_str().unwrap(), "--execute", "-y"]);

    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(run.stdout.contains("Nothing to move"), "{}", run.stdout);
    assert_eq!(entries(dir.path()), before);
}

#[test]
fn a_missing_folder_is_a_usage_error() {
    let run = run(&["plan", "/definitely/not/here"]);
    assert_eq!(run.code, 2);
}

#[test]
fn apply_needs_either_a_folder_or_a_plan() {
    let run = run(&["apply"]);
    assert_eq!(run.code, 2);
}

#[test]
fn config_prints_something_that_can_be_fed_back_in() {
    let dir = fixture();
    let printed = run(&["config"]);
    assert_eq!(printed.code, 0);

    let path = dir.path().join("cfg.toml");
    std::fs::write(&path, &printed.stdout).unwrap();

    let used = run(&["plan", dir.path().to_str().unwrap(), "--config", path.to_str().unwrap()]);
    assert_eq!(used.code, 0, "{}", used.stdout);
    assert!(used.stdout.contains("Kino - 1988 (Gruppa krovi)"), "{}", used.stdout);
}

#[test]
fn templates_and_layout_come_from_the_command_line() {
    let dir = fixture();
    let run = run(&["plan", dir.path().to_str().unwrap(), "--layout", "artist-folders"]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    assert!(run.stdout.contains("Kino/1988 (Gruppa krovi)"), "{}", run.stdout);
}
