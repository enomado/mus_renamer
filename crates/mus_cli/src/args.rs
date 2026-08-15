//! Command line surface. Every option maps to exactly one field of
//! [`mus_core::Config`] or [`mus_fs::ApplyOptions`] — the CLI decides nothing on
//! its own.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use mus_core::config::{ArtistSource, Layout, LegacyEncoding, TranslitMode, UntaggedPolicy};
use mus_core::Config;

#[derive(Debug, Parser)]
#[command(
    name = "mp3rnm",
    version,
    about = "Arrange music folders by their tags",
    long_about = "Arrange music folders by their tags.\n\n\
                  Nothing is written unless --execute is given, nothing is ever overwritten, \
                  and every executed run is journalled into <root>/.mus_renamer so it can be undone."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Read the tree and report what each folder looks like.
    Scan {
        folder: PathBuf,
        #[command(flatten)]
        config: ConfigArgs,
        /// Emit JSON instead of a table.
        #[arg(long)]
        json: bool,
    },

    /// Build a rename plan and print it (optionally save it as JSON).
    Plan {
        folder: PathBuf,
        #[command(flatten)]
        config: ConfigArgs,
        /// Write the plan here so `apply --plan` can replay exactly this decision.
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },

    /// Apply a plan. Without --execute this is a dry run that still runs every safety check.
    Apply {
        /// Music root to scan and apply. Omit when using --plan.
        folder: Option<PathBuf>,
        /// Apply a plan saved by `plan --out` instead of scanning again.
        #[arg(long, conflicts_with = "folder")]
        plan: Option<PathBuf>,
        #[command(flatten)]
        config: ConfigArgs,
        #[command(flatten)]
        exec: ExecArgs,
    },

    /// Move non-audio files to the trash and remove empty directories.
    Clean {
        folder: PathBuf,
        #[command(flatten)]
        config: ConfigArgs,
        /// Move every file that is not audio into <root>/.mus_renamer/trash/.
        #[arg(long)]
        non_audio: bool,
        /// Remove directories that contain nothing (rmdir; never recursive).
        #[arg(long)]
        empty_dirs: bool,
        #[command(flatten)]
        exec: ExecArgs,
    },

    /// Undo a journalled run: pass a journal file, or a music root to take its latest.
    Undo {
        target: PathBuf,
        #[command(flatten)]
        exec: ExecArgs,
    },

    /// List the journals of a music root, newest last.
    Journals { folder: PathBuf },

    /// Print the effective configuration (after config file and flags).
    Config {
        #[command(flatten)]
        config: ConfigArgs,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Args)]
pub struct ExecArgs {
    /// Actually touch the disk. Without it nothing is moved, created or removed.
    #[arg(long)]
    pub execute: bool,
    /// Skip the confirmation prompt (for scripts).
    #[arg(short = 'y', long)]
    pub yes: bool,
    /// How thoroughly a move is verified afterwards.
    #[arg(long, value_enum, default_value_t = VerifyArg::Metadata)]
    pub verify: VerifyArg,
    /// What to do when the destination is on another filesystem.
    #[arg(long, value_enum, default_value_t = CrossDeviceArg::Refuse)]
    pub cross_device: CrossDeviceArg,
    /// Do not write a journal. The run then cannot be undone.
    #[arg(long)]
    pub no_journal: bool,
    /// Stop at the first failed move instead of continuing with the rest.
    #[arg(long)]
    pub stop_on_error: bool,
    /// Apply the plan even if folders changed since it was built. Dangerous.
    #[arg(long)]
    pub ignore_stamps: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum VerifyArg {
    /// Compare the file tree and sizes after the move.
    Metadata,
    /// Also compare a blake3 hash of every file. Slower, reads everything twice.
    Hash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CrossDeviceArg {
    /// Fail the move.
    Refuse,
    /// Copy, verify by hash, then delete the original.
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TranslitArg {
    Off,
    Cyrillic,
    Ascii,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LayoutArg {
    /// `root/Artist - 1999 (Album)`
    Flat,
    /// `root/Artist/1999 (Album)`
    ArtistFolders,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ArtistSourceArg {
    Artist,
    AlbumArtist,
    AlbumArtistThenArtist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum UntaggedArg {
    /// A missing tag makes the folder ambiguous, so it is left alone.
    Ambiguous,
    /// A missing tag counts as the value "unknown".
    Unknown,
    /// Tracks with a missing tag do not take part in the decision.
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum EncodingArg {
    None,
    Cp1251,
    Cp1252,
    Koi8r,
}

/// Overrides on top of a config file (or the defaults).
#[derive(Debug, Clone, Args)]
pub struct ConfigArgs {
    /// TOML file with the full configuration; flags below override its fields.
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Comma-separated audio extensions, without dots.
    #[arg(long, value_name = "LIST", value_delimiter = ',')]
    pub ext: Option<Vec<String>>,

    #[arg(long, value_enum)]
    pub layout: Option<LayoutArg>,

    #[arg(long, value_enum)]
    pub translit: Option<TranslitArg>,

    /// Name template for a single-album folder.
    #[arg(long, value_name = "TEMPLATE")]
    pub album_template: Option<String>,

    /// Name template for a one-artist, many-albums folder.
    #[arg(long, value_name = "TEMPLATE")]
    pub collection_template: Option<String>,

    /// Name template for a compilation.
    #[arg(long, value_name = "TEMPLATE")]
    pub various_template: Option<String>,

    #[arg(long, value_enum)]
    pub artist_source: Option<ArtistSourceArg>,

    /// What a missing artist tag means.
    #[arg(long, value_enum)]
    pub untagged_artist: Option<UntaggedArg>,

    /// What a missing album tag means.
    #[arg(long, value_enum)]
    pub untagged_album: Option<UntaggedArg>,

    /// Re-decode tags that were written in a legacy single-byte codepage.
    #[arg(long, value_enum)]
    pub legacy_encoding: Option<EncodingArg>,

    /// Folders with fewer audio files than this are left alone.
    #[arg(long, value_name = "N")]
    pub min_tracks: Option<usize>,

    /// Also classify folders that contain sub-folders.
    #[arg(long)]
    pub allow_subdirs: bool,

    /// Follow symlinked directories while scanning. Off by default.
    #[arg(long)]
    pub follow_symlinks: bool,

    /// Include dot-files and dot-directories.
    #[arg(long)]
    pub include_hidden: bool,

    /// Keep characters that FAT/NTFS forbid (only safe on ext4/btrfs targets).
    #[arg(long)]
    pub no_fat_safe: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigLoadError {
    #[error("reading {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

impl ConfigArgs {
    pub fn resolve(&self) -> Result<Config, ConfigLoadError> {
        let mut cfg = match &self.config {
            None => Config::default(),
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .map_err(|source| ConfigLoadError::Read { path: path.clone(), source })?;
                toml::from_str(&text)
                    .map_err(|source| ConfigLoadError::Parse { path: path.clone(), source })?
            }
        };

        if let Some(ext) = &self.ext {
            cfg.scan.audio_extensions =
                ext.iter().map(|e| e.trim().trim_start_matches('.').to_lowercase()).collect();
        }
        if let Some(layout) = self.layout {
            cfg.layout.layout = match layout {
                LayoutArg::Flat => Layout::Flat,
                LayoutArg::ArtistFolders => Layout::ArtistFolders,
            };
        }
        if let Some(translit) = self.translit {
            cfg.naming.translit = match translit {
                TranslitArg::Off => TranslitMode::Off,
                TranslitArg::Cyrillic => TranslitMode::Cyrillic,
                TranslitArg::Ascii => TranslitMode::AsciiOnly,
            };
        }
        if let Some(t) = &self.album_template {
            cfg.naming.album_template = t.clone();
        }
        if let Some(t) = &self.collection_template {
            cfg.naming.collection_template = t.clone();
        }
        if let Some(t) = &self.various_template {
            cfg.naming.various_template = t.clone();
        }
        if let Some(source) = self.artist_source {
            cfg.classify.artist_source = match source {
                ArtistSourceArg::Artist => ArtistSource::Artist,
                ArtistSourceArg::AlbumArtist => ArtistSource::AlbumArtist,
                ArtistSourceArg::AlbumArtistThenArtist => ArtistSource::AlbumArtistThenArtist,
            };
        }
        if let Some(policy) = self.untagged_artist {
            cfg.classify.untagged_artist = policy.into();
        }
        if let Some(policy) = self.untagged_album {
            cfg.classify.untagged_album = policy.into();
        }
        if let Some(encoding) = self.legacy_encoding {
            cfg.scan.legacy_encoding = match encoding {
                EncodingArg::None => LegacyEncoding::None,
                EncodingArg::Cp1251 => LegacyEncoding::Cp1251,
                EncodingArg::Cp1252 => LegacyEncoding::Cp1252,
                EncodingArg::Koi8r => LegacyEncoding::Koi8R,
            };
        }
        if let Some(n) = self.min_tracks {
            cfg.classify.min_tracks = n;
        }
        if self.allow_subdirs {
            cfg.classify.skip_folders_with_subdirs = false;
        }
        if self.follow_symlinks {
            cfg.scan.follow_symlinks = true;
        }
        if self.include_hidden {
            cfg.scan.include_hidden = true;
        }
        if self.no_fat_safe {
            cfg.naming.fat_safe = false;
        }

        Ok(cfg)
    }
}

impl From<UntaggedArg> for UntaggedPolicy {
    fn from(value: UntaggedArg) -> Self {
        match value {
            UntaggedArg::Ambiguous => UntaggedPolicy::Ambiguous,
            UntaggedArg::Unknown => UntaggedPolicy::AsUnknown,
            UntaggedArg::Ignore => UntaggedPolicy::Ignore,
        }
    }
}

impl From<VerifyArg> for mus_fs::VerifyLevel {
    fn from(value: VerifyArg) -> Self {
        match value {
            VerifyArg::Metadata => mus_fs::VerifyLevel::Metadata,
            VerifyArg::Hash => mus_fs::VerifyLevel::Hash,
        }
    }
}

impl From<CrossDeviceArg> for mus_fs::CrossDevice {
    fn from(value: CrossDeviceArg) -> Self {
        match value {
            CrossDeviceArg::Refuse => mus_fs::CrossDevice::Refuse,
            CrossDeviceArg::Copy => mus_fs::CrossDevice::CopyVerifyDelete,
        }
    }
}

impl ExecArgs {
    pub fn apply_options(&self) -> mus_fs::ApplyOptions {
        mus_fs::ApplyOptions {
            execute: self.execute,
            verify: self.verify.into(),
            cross_device: self.cross_device.into(),
            journal: !self.no_journal,
            state_dir: None,
            check_stamps: !self.ignore_stamps,
            stop_on_first_error: self.stop_on_error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn nothing_executes_by_default() {
        let cli = Cli::try_parse_from(["mp3rnm", "apply", "/music"]).unwrap();
        let Command::Apply { exec, .. } = cli.command else { panic!() };
        assert!(!exec.execute);
        assert!(!exec.apply_options().execute);
        assert!(exec.apply_options().check_stamps);
    }

    #[test]
    fn flags_override_the_config_file() {
        let cli = Cli::try_parse_from([
            "mp3rnm",
            "plan",
            "/music",
            "--translit",
            "off",
            "--ext",
            "mp3,flac",
            "--album-template",
            "{artist}/{album}",
        ])
        .unwrap();
        let Command::Plan { config, .. } = cli.command else { panic!() };
        let cfg = config.resolve().unwrap();
        assert_eq!(cfg.naming.translit, TranslitMode::Off);
        assert_eq!(cfg.scan.audio_extensions, vec!["mp3", "flac"]);
        assert_eq!(cfg.naming.album_template, "{artist}/{album}");
    }

    #[test]
    fn folder_and_plan_are_mutually_exclusive() {
        assert!(Cli::try_parse_from(["mp3rnm", "apply", "/music", "--plan", "p.json"]).is_err());
    }
}
