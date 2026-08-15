//! Deciding what a folder *is*, from the tags of the tracks inside it.
//!
//! The rule is the one the original tool used, made explicit and total:
//!
//! | distinct artists | distinct albums | verdict          |
//! |------------------|-----------------|------------------|
//! | 1                | 1               | [`FolderKind::Album`]           |
//! | 1                | >1              | [`FolderKind::Collection`]      |
//! | >1               | 1               | [`FolderKind::VariousArtists`]  |
//! | anything else    |                 | [`FolderKind::Ambiguous`]       |
//!
//! Anything that is not a clear verdict stays [`FolderKind::Ambiguous`] and is
//! never renamed. Missing tags do not silently become a fourth rule — the
//! [`UntaggedPolicy`] knobs say what they mean.

use serde::{Deserialize, Serialize};

use crate::config::{ArtistSource, ClassifyConfig, UntaggedPolicy};
use crate::scan::FolderScan;
use crate::tags::TrackTags;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum FolderKind {
    /// One artist, one album.
    Album { artist: String, album: String, year: String },
    /// One artist, several albums — a discography dump.
    Collection { artist: String, year: String },
    /// Several artists, one album — a compilation.
    VariousArtists { album: String, year: String },
    /// Holds sub-folders; it is a container, not a release.
    Container,
    /// Not enough audio to judge.
    TooFewTracks { audio_files: usize },
    /// Judged, and the answer is "do not touch".
    Ambiguous { reason: String },
}

impl FolderKind {
    pub fn is_renameable(&self) -> bool {
        matches!(
            self,
            FolderKind::Album { .. } | FolderKind::Collection { .. } | FolderKind::VariousArtists { .. }
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            FolderKind::Album { .. } => "album",
            FolderKind::Collection { .. } => "collection",
            FolderKind::VariousArtists { .. } => "various",
            FolderKind::Container => "container",
            FolderKind::TooFewTracks { .. } => "too-few-tracks",
            FolderKind::Ambiguous { .. } => "ambiguous",
        }
    }
}

pub fn classify(scan: &FolderScan, cfg: &ClassifyConfig) -> FolderKind {
    if cfg.skip_folders_with_subdirs && !scan.subdirs.is_empty() {
        return FolderKind::Container;
    }
    let audio_files = scan.audio_count();
    if audio_files == 0 || audio_files < cfg.min_tracks {
        return FolderKind::TooFewTracks { audio_files };
    }

    // A file whose tags could not be read is a file with no tags — the untagged
    // policies decide its fate, rather than a separate silent rule.
    let mut all: Vec<TrackTags> = scan.tracks.iter().map(|t| t.tags.clone()).collect();
    all.extend(std::iter::repeat_n(TrackTags::default(), scan.unreadable.len()));

    let artists = match collect(
        all.iter().map(|t| artist_of(t, cfg.artist_source)),
        cfg.untagged_artist,
        &cfg.unknown_artist,
    ) {
        Ok(v) => v,
        Err(missing) => {
            return FolderKind::Ambiguous {
                reason: format!("{missing} of {} tracks have no artist tag", all.len()),
            }
        }
    };
    let albums = match collect(all.iter().map(|t| t.album.clone()), cfg.untagged_album, &cfg.unknown_album) {
        Ok(v) => v,
        Err(missing) => {
            return FolderKind::Ambiguous {
                reason: format!("{missing} of {} tracks have no album tag", all.len()),
            }
        }
    };
    let years = match collect(
        all.iter().map(|t| t.year.map(|y| y.to_string())),
        cfg.untagged_year,
        &cfg.unknown_year,
    ) {
        Ok(v) => v,
        Err(missing) => {
            return FolderKind::Ambiguous {
                reason: format!("{missing} of {} tracks have no year tag", all.len()),
            }
        }
    };

    // One year for the whole folder, or the unknown label — never "the first one
    // we happened to see".
    let year = match years.len() {
        1 => years[0].clone(),
        _ => cfg.unknown_year.clone(),
    };

    match (artists.len(), albums.len()) {
        (0, _) | (_, 0) => FolderKind::Ambiguous { reason: "no usable artist/album values".to_string() },
        (1, 1) => FolderKind::Album { artist: artists[0].clone(), album: albums[0].clone(), year },
        (1, _) => FolderKind::Collection { artist: artists[0].clone(), year },
        (_, 1) => FolderKind::VariousArtists { album: albums[0].clone(), year },
        (a, b) => FolderKind::Ambiguous { reason: format!("{a} artists and {b} albums in one folder") },
    }
}

fn artist_of(tags: &TrackTags, source: ArtistSource) -> Option<String> {
    match source {
        ArtistSource::Artist => tags.artist.clone(),
        ArtistSource::AlbumArtist => tags.album_artist.clone(),
        ArtistSource::AlbumArtistThenArtist => tags.album_artist.clone().or_else(|| tags.artist.clone()),
    }
}

/// Distinct values in first-seen order, with the untagged policy applied.
/// `Err(n)` means the policy is [`UntaggedPolicy::Ambiguous`] and `n` tracks were missing the tag.
fn collect<I>(values: I, policy: UntaggedPolicy, unknown: &str) -> Result<Vec<String>, usize>
where
    I: Iterator<Item = Option<String>>,
{
    let mut out: Vec<String> = Vec::new();
    let mut missing = 0usize;
    for value in values {
        let value = match (value, policy) {
            (Some(v), _) => v,
            (None, UntaggedPolicy::Ignore) => {
                missing += 1;
                continue;
            }
            (None, UntaggedPolicy::AsUnknown) => {
                missing += 1;
                unknown.to_string()
            }
            (None, UntaggedPolicy::Ambiguous) => {
                missing += 1;
                return Err(missing);
            }
        };
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        if !out.iter().any(|v| v.eq_ignore_ascii_case(&value)) {
            out.push(value);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{DirStamp, TrackEntry, UnreadableTrack};
    use std::path::PathBuf;

    fn tags(artist: &str, album: &str, year: Option<u32>) -> TrackTags {
        TrackTags {
            artist: Some(artist.to_string()),
            album: Some(album.to_string()),
            year,
            ..Default::default()
        }
    }

    fn scan_of(tracks: Vec<TrackTags>) -> FolderScan {
        FolderScan {
            dir: PathBuf::from("/music/x"),
            depth: 1,
            stamp: DirStamp { dev: 1, ino: 2, entries: tracks.len(), mtime_ns: 0 },
            tracks: tracks
                .into_iter()
                .enumerate()
                .map(|(i, tags)| TrackEntry { path: PathBuf::from(format!("/music/x/{i}.mp3")), tags })
                .collect(),
            unreadable: vec![],
            non_audio_files: vec![],
            subdirs: vec![],
        }
    }

    #[test]
    fn one_artist_one_album_is_an_album() {
        let scan =
            scan_of(vec![tags("Kino", "Gruppa krovi", Some(1988)), tags("Kino", "Gruppa krovi", Some(1988))]);
        assert_eq!(
            classify(&scan, &ClassifyConfig::default()),
            FolderKind::Album { artist: "Kino".into(), album: "Gruppa krovi".into(), year: "1988".into() }
        );
    }

    #[test]
    fn one_artist_many_albums_is_a_collection() {
        let scan = scan_of(vec![tags("Kino", "A", Some(1988)), tags("Kino", "B", Some(1988))]);
        assert_eq!(
            classify(&scan, &ClassifyConfig::default()),
            FolderKind::Collection { artist: "Kino".into(), year: "1988".into() }
        );
    }

    #[test]
    fn many_artists_one_album_is_various() {
        let scan = scan_of(vec![tags("A", "Hits 99", Some(1999)), tags("B", "Hits 99", Some(1999))]);
        assert_eq!(
            classify(&scan, &ClassifyConfig::default()),
            FolderKind::VariousArtists { album: "Hits 99".into(), year: "1999".into() }
        );
    }

    #[test]
    fn many_artists_many_albums_is_ambiguous() {
        let scan = scan_of(vec![tags("A", "X", None), tags("B", "Y", None)]);
        assert!(matches!(classify(&scan, &ClassifyConfig::default()), FolderKind::Ambiguous { .. }));
    }

    #[test]
    fn disagreeing_years_fall_back_to_unknown_not_to_the_first_one() {
        let scan = scan_of(vec![tags("Kino", "Best", Some(1988)), tags("Kino", "Best", Some(1990))]);
        assert_eq!(
            classify(&scan, &ClassifyConfig::default()),
            FolderKind::Album { artist: "Kino".into(), album: "Best".into(), year: "unknown".into() }
        );
    }

    #[test]
    fn a_missing_artist_tag_makes_the_folder_ambiguous_by_default() {
        let mut t = tags("Kino", "Best", Some(1988));
        t.artist = None;
        let scan = scan_of(vec![t, tags("Kino", "Best", Some(1988))]);
        assert!(matches!(classify(&scan, &ClassifyConfig::default()), FolderKind::Ambiguous { .. }));
    }

    #[test]
    fn an_unreadable_file_counts_as_an_untagged_track() {
        let mut scan = scan_of(vec![tags("Kino", "Best", Some(1988))]);
        scan.unreadable
            .push(UnreadableTrack { path: PathBuf::from("/music/x/broken.mp3"), error: "truncated".into() });
        assert!(matches!(classify(&scan, &ClassifyConfig::default()), FolderKind::Ambiguous { .. }));
    }

    #[test]
    fn folders_with_subdirs_are_containers() {
        let mut scan = scan_of(vec![tags("Kino", "Best", Some(1988))]);
        scan.subdirs.push(PathBuf::from("/music/x/cd2"));
        assert_eq!(classify(&scan, &ClassifyConfig::default()), FolderKind::Container);
    }

    #[test]
    fn artist_case_differences_are_one_artist() {
        let scan = scan_of(vec![tags("Kino", "Best", Some(1988)), tags("KINO", "Best", Some(1988))]);
        assert!(matches!(classify(&scan, &ClassifyConfig::default()), FolderKind::Album { .. }));
    }

    #[test]
    fn album_artist_wins_over_per_track_artist_by_default() {
        let mut a = tags("Vasya feat. Petya", "Album", Some(2000));
        a.album_artist = Some("Vasya".into());
        let mut b = tags("Vasya", "Album", Some(2000));
        b.album_artist = Some("Vasya".into());
        let scan = scan_of(vec![a, b]);
        assert_eq!(
            classify(&scan, &ClassifyConfig::default()),
            FolderKind::Album { artist: "Vasya".into(), album: "Album".into(), year: "2000".into() }
        );
    }
}
