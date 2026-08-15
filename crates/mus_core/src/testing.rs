//! Fixture builders shared by the test suites of every crate in the workspace.
//!
//! Real mp3 files, not mocks: a silent but structurally valid MPEG-1 Layer III
//! stream with a real ID3v2 tag, so tests exercise the same parser the tool uses.

use std::path::Path;

use lofty::config::WriteOptions;
use lofty::prelude::{Accessor, TagExt};
use lofty::tag::{Tag, TagType};

/// Five frames of silence at 128 kbps, 44.1 kHz — about 2 KiB.
pub fn write_silent_mp3(path: &Path) {
    const FRAME_LEN: usize = 417; // 144 * 128000 / 44100
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("creating the fixture directory");
    }
    let mut bytes = Vec::with_capacity(FRAME_LEN * 5);
    for _ in 0..5 {
        // sync + MPEG1 + Layer III + no CRC, 128 kbps, 44.1 kHz, joint stereo
        bytes.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
        bytes.extend(std::iter::repeat_n(0u8, FRAME_LEN - 4));
    }
    std::fs::write(path, bytes).expect("writing the fixture mp3");
}

/// A silent mp3 carrying the given ID3v2 tags.
pub fn write_tagged_mp3(path: &Path, artist: &str, album: &str, year: Option<u32>) {
    write_silent_mp3(path);
    let mut tag = Tag::new(TagType::Id3v2);
    tag.set_artist(artist.to_string());
    tag.set_album(album.to_string());
    if let Some(year) = year {
        tag.set_year(year);
    }
    tag.set_title(
        path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "track".to_string()),
    );
    tag.save_to_path(path, WriteOptions::default()).expect("writing the fixture tags");
}
