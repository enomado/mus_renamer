//! Reading track metadata.
//!
//! [`TagReader`] is a trait so the rest of the core can be tested without
//! carrying binary mp3 fixtures around; [`LoftyReader`] is the real one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lofty::file::TaggedFileExt;
use lofty::prelude::{Accessor, ItemKey};
use lofty::probe::Probe;
use lofty::tag::Tag;
use serde::{Deserialize, Serialize};

use crate::config::LegacyEncoding;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackTags {
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub title: Option<String>,
    pub year: Option<u32>,
    pub track: Option<u32>,
}

impl TrackTags {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TagError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Decode { path: PathBuf, message: String },
}

impl TagError {
    pub fn path(&self) -> &Path {
        match self {
            TagError::Io { path, .. } | TagError::Decode { path, .. } => path,
        }
    }
}

pub trait TagReader: Send + Sync {
    fn read(&self, path: &Path) -> Result<TrackTags, TagError>;
}

/// The real reader, backed by `lofty` (mp3/flac/ogg/opus/m4a/wma/wav).
#[derive(Debug, Clone, Default)]
pub struct LoftyReader {
    /// Re-interpret strings that survived a latin-1 round trip. Opt-in.
    pub legacy_encoding: LegacyEncoding,
}

impl LoftyReader {
    pub fn new(legacy_encoding: LegacyEncoding) -> Self {
        Self { legacy_encoding }
    }

    fn field(&self, tag: &Tag, key: &ItemKey) -> Option<String> {
        let raw = tag.get_string(key)?.trim().to_string();
        if raw.is_empty() {
            return None;
        }
        Some(repair_legacy_encoding(&raw, self.legacy_encoding))
    }
}

impl TagReader for LoftyReader {
    fn read(&self, path: &Path) -> Result<TrackTags, TagError> {
        let tagged = Probe::open(path)
            .map_err(|e| TagError::Decode { path: path.to_path_buf(), message: e.to_string() })?
            .read()
            .map_err(|e| TagError::Decode { path: path.to_path_buf(), message: e.to_string() })?;

        // `primary_tag` is the format's native tag (ID3v2 for mp3); `first_tag`
        // falls back to whatever else was found (e.g. a lone ID3v1 block).
        let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) else {
            return Ok(TrackTags::default());
        };

        let year = tag
            .get_string(&ItemKey::RecordingDate)
            .and_then(parse_year)
            .or_else(|| tag.year())
            .or_else(|| tag.get_string(&ItemKey::OriginalReleaseDate).and_then(parse_year));

        Ok(TrackTags {
            artist: self.field(tag, &ItemKey::TrackArtist),
            album_artist: self.field(tag, &ItemKey::AlbumArtist),
            album: self.field(tag, &ItemKey::AlbumTitle),
            title: self.field(tag, &ItemKey::TrackTitle),
            year,
            track: tag.track(),
        })
    }
}

/// Leading four digits of an ID3 date (`1999`, `1999-06-01`, `1999/06`).
fn parse_year(value: &str) -> Option<u32> {
    let digits: String = value.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() != 4 {
        return None;
    }
    digits.parse().ok()
}

/// Undo the classic mojibake: bytes that were really CP1251/KOI8-R got decoded
/// as latin-1, so every character landed in U+0000..U+00FF. If *all* characters
/// fit in one byte we can put the bytes back and decode them properly.
///
/// Only runs when the user explicitly asked for a codepage, and only when the
/// re-decode produces something different and lossless — otherwise the original
/// string is returned untouched.
pub fn repair_legacy_encoding(value: &str, encoding: LegacyEncoding) -> String {
    let enc = match encoding {
        LegacyEncoding::None => return value.to_string(),
        LegacyEncoding::Cp1251 => encoding_rs::WINDOWS_1251,
        LegacyEncoding::Cp1252 => encoding_rs::WINDOWS_1252,
        LegacyEncoding::Koi8R => encoding_rs::KOI8_R,
    };
    if value.is_empty() || !value.chars().all(|c| (c as u32) < 0x100) {
        return value.to_string();
    }
    let bytes: Vec<u8> = value.chars().map(|c| c as u8).collect();
    let (decoded, _, had_errors) = enc.decode(&bytes);
    if had_errors {
        return value.to_string();
    }
    decoded.into_owned()
}

/// In-memory reader for tests: a path → tags table, missing paths read as empty.
#[derive(Debug, Clone, Default)]
pub struct MapReader {
    pub tags: HashMap<PathBuf, TrackTags>,
    pub failures: HashMap<PathBuf, String>,
}

impl MapReader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, path: impl Into<PathBuf>, tags: TrackTags) -> Self {
        self.tags.insert(path.into(), tags);
        self
    }

    pub fn failing(mut self, path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        self.failures.insert(path.into(), message.into());
        self
    }
}

impl TagReader for MapReader {
    fn read(&self, path: &Path) -> Result<TrackTags, TagError> {
        if let Some(message) = self.failures.get(path) {
            return Err(TagError::Decode { path: path.to_path_buf(), message: message.clone() });
        }
        Ok(self.tags.get(path).cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn year_is_the_leading_four_digits() {
        assert_eq!(parse_year("1999"), Some(1999));
        assert_eq!(parse_year("1999-06-01"), Some(1999));
        assert_eq!(parse_year(" 2001/03 "), Some(2001));
        assert_eq!(parse_year("99"), None);
        assert_eq!(parse_year(""), None);
        assert_eq!(parse_year("nineteen"), None);
    }

    #[test]
    fn cp1251_mojibake_is_repaired_only_when_asked() {
        // "Ария" encoded in cp1251, then wrongly decoded as latin-1.
        let bytes = encoding_rs::WINDOWS_1251.encode("Ария").0.into_owned();
        let mojibake: String = bytes.iter().map(|b| *b as char).collect();

        assert_eq!(repair_legacy_encoding(&mojibake, LegacyEncoding::None), mojibake);
        assert_eq!(repair_legacy_encoding(&mojibake, LegacyEncoding::Cp1251), "Ария");
    }

    #[test]
    fn already_correct_text_survives_the_repair() {
        // Real Cyrillic is above U+00FF, so the repair declines to touch it.
        assert_eq!(repair_legacy_encoding("Ария", LegacyEncoding::Cp1251), "Ария");
        // Pure ASCII decodes to itself under cp1251.
        assert_eq!(repair_legacy_encoding("Metallica", LegacyEncoding::Cp1251), "Metallica");
    }
}
