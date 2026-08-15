//! Every knob the core obeys. No hidden defaults live in the algorithms — if
//! behaviour can differ, it is a field here, serializable to/from TOML+JSON so a
//! run can be reproduced exactly.

use serde::{Deserialize, Serialize};

/// Which tag frame decides "who is the artist of this folder".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtistSource {
    /// `TPE1` only.
    Artist,
    /// `TPE2` only. Folders of a single band ripped as "feat." tracks stay one artist.
    AlbumArtist,
    /// `TPE2` when present on a track, `TPE1` otherwise.
    AlbumArtistThenArtist,
}

/// What to do with tracks whose relevant tag is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UntaggedPolicy {
    /// A missing value makes the whole folder ambiguous — it will not be renamed.
    Ambiguous,
    /// A missing value is a distinct value equal to the configured unknown label.
    AsUnknown,
    /// The track does not participate in the decision at all.
    Ignore,
}

/// Cyrillic → Latin conversion. Deliberately *not* language autodetection: the
/// table is applied per character, so the result never depends on a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TranslitMode {
    /// Leave names as they are (still sanitized).
    Off,
    /// Transliterate Cyrillic letters, leave everything else untouched.
    Cyrillic,
    /// Transliterate Cyrillic, then drop every remaining non-ASCII character.
    AsciiOnly,
}

/// Re-decoding of tags stored in a legacy single-byte codepage (ID3v1 and old
/// taggers). Opt-in only — it is a lossy reinterpretation of bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LegacyEncoding {
    #[default]
    None,
    Cp1251,
    Cp1252,
    Koi8R,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScanConfig {
    /// Extensions counted as music, lowercase, without the dot.
    pub audio_extensions: Vec<String>,
    /// Descend into symlinked directories. Off by default: a symlink loop or a
    /// link pointing outside the root would put moves outside the tree.
    pub follow_symlinks: bool,
    /// Visit entries whose name starts with a dot.
    pub include_hidden: bool,
    /// Directory names never scanned nor moved (the tool's own state lives here).
    pub excluded_dirs: Vec<String>,
    /// Re-decode tag strings that fit in one byte with this codepage.
    pub legacy_encoding: LegacyEncoding,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            audio_extensions: ["mp3", "flac", "ogg", "opus", "m4a", "wma", "wav", "aac"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            follow_symlinks: false,
            include_hidden: false,
            excluded_dirs: vec![crate::plan::STATE_DIR.to_string()],
            legacy_encoding: LegacyEncoding::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClassifyConfig {
    pub artist_source: ArtistSource,
    pub untagged_artist: UntaggedPolicy,
    pub untagged_album: UntaggedPolicy,
    pub untagged_year: UntaggedPolicy,
    /// A folder with fewer audio files than this is never classified.
    pub min_tracks: usize,
    /// Folders that contain sub-folders are left alone (they are containers, not albums).
    pub skip_folders_with_subdirs: bool,
    pub unknown_artist: String,
    pub unknown_album: String,
    pub unknown_year: String,
}

impl Default for ClassifyConfig {
    fn default() -> Self {
        Self {
            artist_source: ArtistSource::AlbumArtistThenArtist,
            untagged_artist: UntaggedPolicy::Ambiguous,
            untagged_album: UntaggedPolicy::Ambiguous,
            untagged_year: UntaggedPolicy::AsUnknown,
            min_tracks: 1,
            skip_folders_with_subdirs: true,
            unknown_artist: "unknown".to_string(),
            unknown_album: "unknown".to_string(),
            unknown_year: "unknown".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NamingConfig {
    /// Placeholders: `{artist}`, `{album}`, `{year}`.
    pub album_template: String,
    pub collection_template: String,
    pub various_template: String,
    pub translit: TranslitMode,
    /// Strip characters illegal on FAT/NTFS and refuse reserved DOS names.
    /// Kept on by default — the tool exists for car-audio flash drives.
    pub fat_safe: bool,
    /// Hard cap on one path component, in bytes (255 is the ext4/FAT limit).
    pub max_component_bytes: usize,
    /// Name used when sanitizing leaves nothing behind.
    pub fallback_name: String,
    /// Treat `ABBA` and `abba` as the same destination. FAT and NTFS do, ext4 does not.
    pub case_insensitive_collisions: bool,
}

impl Default for NamingConfig {
    fn default() -> Self {
        Self {
            album_template: "{artist} - {year} ({album})".to_string(),
            collection_template: "{artist} - collection".to_string(),
            various_template: "VA - {album}".to_string(),
            translit: TranslitMode::Cyrillic,
            fat_safe: true,
            max_component_bytes: 255,
            fallback_name: "unnamed".to_string(),
            case_insensitive_collisions: true,
        }
    }
}

/// Where classified folders end up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    /// Everything flat in the root: `root/Artist - 1999 (Album)`. Car-audio layout.
    Flat,
    /// `root/Artist/1999 (Album)`. Collections and VA stay flat in the root.
    ArtistFolders,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub scan: ScanConfig,
    pub classify: ClassifyConfig,
    pub naming: NamingConfig,
    pub layout: LayoutConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LayoutConfig {
    pub layout: Layout,
    /// In `artist-folders` layout, the name of the album folder inside the artist folder.
    pub album_subfolder_template: String,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self { layout: Layout::Flat, album_subfolder_template: "{year} ({album})".to_string() }
    }
}

impl Config {
    pub fn is_audio(&self, path: &std::path::Path) -> bool {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return false;
        };
        let ext = ext.to_ascii_lowercase();
        self.scan.audio_extensions.contains(&ext)
    }
}
