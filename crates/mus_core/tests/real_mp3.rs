//! The tag reader against real files, not fixtures.
//!
//! Everywhere else the core is tested through `MapReader`; this is the one place
//! that proves the `lofty` path actually parses an mp3 with an ID3v2 tag, so a
//! reader change cannot pass the suite while breaking the tool.

use std::path::Path;

use mus_core::config::LegacyEncoding;
use mus_core::tags::{LoftyReader, TagReader};
use mus_core::testing::{write_silent_mp3 as silent_mp3, write_tagged_mp3};

fn write_tags(path: &Path, artist: &str, album: &str, year: u32) {
    write_tagged_mp3(path, artist, album, Some(year));
}

#[test]
fn an_id3v2_tag_is_read_back_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("01.mp3");
    write_tags(&path, "Kino", "Gruppa krovi", 1988);

    let tags = LoftyReader::default().read(&path).unwrap();

    assert_eq!(tags.artist.as_deref(), Some("Kino"));
    assert_eq!(tags.album.as_deref(), Some("Gruppa krovi"));
    assert_eq!(tags.year, Some(1988));
    assert_eq!(tags.title.as_deref(), Some("01"));
}

#[test]
fn cyrillic_tags_survive_the_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("01.mp3");
    write_tags(&path, "Ария", "Герой асфальта", 1987);

    let tags = LoftyReader::new(LegacyEncoding::None).read(&path).unwrap();
    assert_eq!(tags.artist.as_deref(), Some("Ария"));
    assert_eq!(tags.album.as_deref(), Some("Герой асфальта"));
}

#[test]
fn an_untagged_file_reads_as_empty_rather_than_failing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("01.mp3");
    silent_mp3(&path);

    let tags = LoftyReader::default().read(&path).unwrap();
    assert!(tags.is_empty(), "{tags:?}");
}

#[test]
fn a_file_that_is_not_audio_at_all_is_an_error_not_a_silent_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken.mp3");
    std::fs::write(&path, b"this is not an mp3").unwrap();

    let err = LoftyReader::default().read(&path).unwrap_err();
    assert_eq!(err.path(), path);
}

#[test]
fn a_scan_of_real_files_classifies_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    let album = dir.path().join("incoming");
    std::fs::create_dir(&album).unwrap();
    for n in 1..=3 {
        write_tags(&album.join(format!("{n:02}.mp3")), "Kino", "Gruppa krovi", 1988);
    }

    let cfg = mus_core::Config::default();
    let reader = LoftyReader::default();
    let scans = mus_core::scan_tree(dir.path(), &cfg, &reader).unwrap();
    let plan = mus_core::build_plan(dir.path(), &scans, &cfg).unwrap();

    assert_eq!(plan.moves.len(), 1);
    assert_eq!(plan.moves[0].to, dir.path().join("Kino - 1988 (Gruppa krovi)"));
    assert_eq!(plan.moves[0].audio_files, 3);
}
