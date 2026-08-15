//! Turning a verdict into a folder name: template → transliteration → sanitize
//! → collision-free allocation.
//!
//! Every step is total and deterministic. There is no language autodetection:
//! the Cyrillic table is applied per character, so the same input always yields
//! the same name regardless of what else is in the folder.

use std::collections::HashSet;

use crate::classify::FolderKind;
use crate::config::{NamingConfig, TranslitMode};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fields {
    pub artist: String,
    pub album: String,
    pub year: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NameError {
    #[error("template `{template}` uses unknown placeholder `{{{key}}}` (known: artist, album, year)")]
    UnknownPlaceholder { template: String, key: String },
    #[error("template `{template}` has an unclosed `{{`")]
    UnclosedPlaceholder { template: String },
}

/// Expand `{artist}` / `{album}` / `{year}`. `{{` and `}}` are literal braces.
pub fn render(template: &str, fields: &Fields) -> Result<String, NameError> {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut key = String::new();
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == '}' {
                        closed = true;
                        break;
                    }
                    key.push(c);
                }
                if !closed {
                    return Err(NameError::UnclosedPlaceholder { template: template.to_string() });
                }
                match key.trim() {
                    "artist" => out.push_str(&fields.artist),
                    "album" => out.push_str(&fields.album),
                    "year" => out.push_str(&fields.year),
                    other => {
                        return Err(NameError::UnknownPlaceholder {
                            template: template.to_string(),
                            key: other.to_string(),
                        })
                    }
                }
            }
            c => out.push(c),
        }
    }
    Ok(out)
}

/// The fields a verdict contributes to a name. `None` for verdicts that are not renamed.
pub fn fields_of(kind: &FolderKind, cfg: &crate::config::ClassifyConfig) -> Option<Fields> {
    match kind {
        FolderKind::Album { artist, album, year } => {
            Some(Fields { artist: artist.clone(), album: album.clone(), year: year.clone() })
        }
        FolderKind::Collection { artist, year } => {
            Some(Fields { artist: artist.clone(), album: cfg.unknown_album.clone(), year: year.clone() })
        }
        FolderKind::VariousArtists { album, year } => {
            Some(Fields { artist: cfg.unknown_artist.clone(), album: album.clone(), year: year.clone() })
        }
        _ => None,
    }
}

pub fn transliterate(value: &str, mode: TranslitMode) -> String {
    match mode {
        TranslitMode::Off => value.to_string(),
        TranslitMode::Cyrillic => translit_cyrillic(value),
        TranslitMode::AsciiOnly => {
            translit_cyrillic(value).chars().filter(|c| c.is_ascii()).collect::<String>()
        }
    }
}

/// Per-character Cyrillic → Latin (Russian + Ukrainian/Belarusian extras).
/// Uppercase letters keep their case; a soft/hard sign disappears.
fn translit_cyrillic(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        let lower = c.to_lowercase().next().unwrap_or(c);
        let Some(mapped) = cyrillic_letter(lower) else {
            out.push(c);
            continue;
        };
        if c == lower {
            out.push_str(mapped);
        } else {
            // Uppercase source → capitalize the (possibly multi-letter) result.
            let mut it = mapped.chars();
            if let Some(first) = it.next() {
                out.extend(first.to_uppercase());
                out.push_str(it.as_str());
            }
        }
    }
    out
}

fn cyrillic_letter(c: char) -> Option<&'static str> {
    Some(match c {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'ґ' => "g",
        'д' => "d",
        'е' => "e",
        'ё' => "yo",
        'є' => "ye",
        'ж' => "zh",
        'з' => "z",
        'и' => "i",
        'і' => "i",
        'ї' => "yi",
        'й' => "y",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ў' => "u",
        'ф' => "f",
        'х' => "kh",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "shch",
        'ъ' => "",
        'ы' => "y",
        'ь' => "",
        'э' => "e",
        'ю' => "yu",
        'я' => "ya",
        _ => return None,
    })
}

const FAT_FORBIDDEN: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
const DOS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Make one path component safe to create: no separators, no control characters,
/// no DOS-reserved stem, no trailing dot/space (Windows drops them silently),
/// never empty, never longer than the configured byte cap.
pub fn sanitize(name: &str, cfg: &NamingConfig) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        let bad = c == '/' || c == '\0' || (c as u32) < 0x20 || (cfg.fat_safe && FAT_FORBIDDEN.contains(&c));
        if bad {
            out.push(' ');
        } else {
            out.push(c);
        }
    }

    // Collapse runs of whitespace so removed characters do not leave holes.
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut name = collapsed.trim().trim_end_matches(['.', ' ']).trim().to_string();

    if cfg.fat_safe {
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        if DOS_RESERVED.contains(&stem.as_str()) {
            name = format!("_{name}");
        }
    }
    // `.` and `..` are not names at all; a leading dot hides the folder.
    if name.chars().all(|c| c == '.') {
        name.clear();
    }
    if name.is_empty() {
        name = cfg.fallback_name.clone();
    }
    truncate_bytes(&name, cfg.max_component_bytes)
}

/// Cut to at most `max` bytes without splitting a character.
pub fn truncate_bytes(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].trim_end().to_string()
}

/// Full pipeline for one folder name.
pub fn build_name(template: &str, fields: &Fields, cfg: &NamingConfig) -> Result<String, NameError> {
    let rendered = render(template, fields)?;
    let translited = transliterate(&rendered, cfg.translit);
    Ok(sanitize(&translited, cfg))
}

/// Hands out names that are unique inside one directory.
///
/// Names already present on disk are `reserve`d first, so a plan can never aim
/// two folders at one destination, and never at an existing entry.
#[derive(Debug, Clone)]
pub struct NameAllocator {
    taken: HashSet<String>,
    case_insensitive: bool,
    max_bytes: usize,
}

impl NameAllocator {
    pub fn new(case_insensitive: bool, max_bytes: usize) -> Self {
        Self { taken: HashSet::new(), case_insensitive, max_bytes }
    }

    fn key(&self, name: &str) -> String {
        if self.case_insensitive {
            name.to_lowercase()
        } else {
            name.to_string()
        }
    }

    /// Mark a name as occupied. `false` if it already was.
    pub fn reserve(&mut self, name: &str) -> bool {
        self.taken.insert(self.key(name))
    }

    pub fn is_taken(&self, name: &str) -> bool {
        self.taken.contains(&self.key(name))
    }

    /// `desired`, or `desired (2)`, `desired (3)`, … — the first one free.
    /// The result is reserved before it is returned.
    pub fn allocate(&mut self, desired: &str) -> String {
        if self.reserve(desired) {
            return desired.to_string();
        }
        for n in 2u32.. {
            let suffix = format!(" ({n})");
            let base = truncate_bytes(desired, self.max_bytes.saturating_sub(suffix.len()));
            let candidate = format!("{base}{suffix}");
            if self.reserve(&candidate) {
                return candidate;
            }
        }
        unreachable!("u32 range is exhausted only after 4 billion collisions")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> NamingConfig {
        NamingConfig::default()
    }

    #[test]
    fn template_expands_known_placeholders() {
        let fields = Fields { artist: "Kino".into(), album: "Zvezda".into(), year: "1989".into() };
        assert_eq!(render("{artist} - {year} ({album})", &fields).unwrap(), "Kino - 1989 (Zvezda)");
        assert_eq!(render("{{literal}}", &fields).unwrap(), "{literal}");
    }

    #[test]
    fn unknown_placeholder_is_an_error_not_an_empty_string() {
        let err = render("{artist} {genre}", &Fields::default()).unwrap_err();
        assert_eq!(
            err,
            NameError::UnknownPlaceholder { template: "{artist} {genre}".into(), key: "genre".into() }
        );
        assert!(matches!(render("{artist", &Fields::default()), Err(NameError::UnclosedPlaceholder { .. })));
    }

    #[test]
    fn cyrillic_transliterates_per_character() {
        assert_eq!(translit_cyrillic("Кино"), "Kino");
        assert_eq!(translit_cyrillic("Щёлкунчик"), "Shchyolkunchik");
        assert_eq!(translit_cyrillic("Ария"), "Ariya");
        assert_eq!(translit_cyrillic("ДДТ"), "DDT");
        // Latin text passes through untouched — no language guessing involved.
        assert_eq!(translit_cyrillic("Pink Floyd — The Wall"), "Pink Floyd — The Wall");
    }

    #[test]
    fn ascii_only_drops_what_the_table_could_not_map() {
        assert_eq!(transliterate("Ария — Герой", TranslitMode::AsciiOnly), "Ariya  Geroy");
        assert_eq!(transliterate("Ария — Герой", TranslitMode::Cyrillic), "Ariya — Geroy");
    }

    #[test]
    fn sanitize_removes_separators_and_illegal_characters() {
        assert_eq!(sanitize("AC/DC: Back?", &cfg()), "AC DC Back");
        assert_eq!(sanitize("a\u{0}b", &cfg()), "a b");
        assert_eq!(sanitize("  spaced   out  ", &cfg()), "spaced out");
    }

    #[test]
    fn sanitize_never_returns_something_the_filesystem_would_mangle() {
        assert_eq!(sanitize("trailing.", &cfg()), "trailing");
        assert_eq!(sanitize("trailing ", &cfg()), "trailing");
        assert_eq!(sanitize("..", &cfg()), "unnamed");
        assert_eq!(sanitize("", &cfg()), "unnamed");
        assert_eq!(sanitize("///", &cfg()), "unnamed");
        assert_eq!(sanitize("CON", &cfg()), "_CON");
        assert_eq!(sanitize("nul.mp3", &cfg()), "_nul.mp3");
    }

    #[test]
    fn sanitize_respects_the_byte_cap_on_char_boundaries() {
        let mut c = cfg();
        c.max_component_bytes = 10;
        // Each Cyrillic character is two bytes; the cut must not split one.
        let out = sanitize("ЯЯЯЯЯЯЯЯ", &{
            let mut c2 = c.clone();
            c2.translit = TranslitMode::Off;
            c2
        });
        assert!(out.len() <= 10, "{out:?} is {} bytes", out.len());
        assert!(out.chars().all(|ch| ch == 'Я'));
    }

    #[test]
    fn allocator_never_hands_out_the_same_name_twice() {
        let mut alloc = NameAllocator::new(true, 255);
        assert_eq!(alloc.allocate("Kino"), "Kino");
        assert_eq!(alloc.allocate("Kino"), "Kino (2)");
        assert_eq!(alloc.allocate("Kino"), "Kino (3)");
        // Case-insensitive mode is what FAT and NTFS do.
        assert_eq!(alloc.allocate("KINO"), "KINO (4)");
    }

    #[test]
    fn allocator_can_be_case_sensitive() {
        let mut alloc = NameAllocator::new(false, 255);
        assert_eq!(alloc.allocate("Kino"), "Kino");
        assert_eq!(alloc.allocate("KINO"), "KINO");
    }

    #[test]
    fn suffixed_name_still_fits_the_byte_cap() {
        let mut alloc = NameAllocator::new(true, 8);
        assert_eq!(alloc.allocate("abcdefgh"), "abcdefgh");
        let second = alloc.allocate("abcdefgh");
        assert_eq!(second, "abcd (2)");
        assert!(second.len() <= 8);
    }
}
