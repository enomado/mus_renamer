//! Turning reports into something readable in a terminal.

use std::path::Path;

use mus_core::{FolderScan, Plan};
use mus_fs::apply::ApplyReport;

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Shorten a path for display: everything below the root, or the path itself.
pub fn rel<'a>(root: &Path, path: &'a Path) -> std::borrow::Cow<'a, str> {
    match path.strip_prefix(root) {
        Ok(rel) if !rel.as_os_str().is_empty() => rel.to_string_lossy(),
        _ => path.to_string_lossy(),
    }
}

pub fn print_scan(root: &Path, scans: &[FolderScan], cfg: &mus_core::Config) {
    let mut renameable = 0usize;
    for scan in scans {
        if scan.depth == 0 {
            continue;
        }
        let kind = mus_core::classify(scan, &cfg.classify);
        if kind.is_renameable() {
            renameable += 1;
        }
        println!("{:<14} {:>4} tracks  {}", kind.label(), scan.audio_count(), rel(root, &scan.dir));
        match &kind {
            mus_core::FolderKind::Album { artist, album, year } => {
                println!("               {artist} — {album} ({year})");
            }
            mus_core::FolderKind::Collection { artist, .. } => println!("               {artist}"),
            mus_core::FolderKind::VariousArtists { album, .. } => println!("               {album}"),
            mus_core::FolderKind::Ambiguous { reason } => println!("               {reason}"),
            _ => {}
        }
        for bad in &scan.unreadable {
            println!("               unreadable: {} ({})", rel(root, &bad.path), bad.error);
        }
    }
    println!();
    println!("{} folders scanned, {renameable} would be renamed", scans.len().saturating_sub(1));
}

pub fn print_plan(plan: &Plan, verbose_skips: bool) {
    if plan.moves.is_empty() {
        println!("Nothing to move.");
    }
    for m in &plan.moves {
        println!("{:<12} {}", m.kind.label(), rel(&plan.root, &m.from));
        println!("          -> {}", rel(&plan.root, &m.to));
    }

    if !plan.skipped.is_empty() {
        let mut by_reason: std::collections::BTreeMap<&str, usize> = Default::default();
        for skip in &plan.skipped {
            *by_reason.entry(skip.kind.label()).or_default() += 1;
        }
        println!();
        println!("Left alone:");
        for (label, count) in by_reason {
            println!("  {label}: {count}");
        }
        if verbose_skips {
            for skip in &plan.skipped {
                println!("  {} — {}", rel(&plan.root, &skip.dir), skip.reason);
            }
        }
    }

    println!();
    println!("{} folders to move", plan.moves.len());
}

pub fn print_apply_report(root: &Path, report: &ApplyReport) {
    println!();
    if report.dry_run {
        println!(
            "Dry run: {} folders would move ({} files, {}). Nothing was touched.",
            report.applied.len(),
            report.files(),
            human_bytes(report.bytes())
        );
    } else {
        println!(
            "Moved {} folders ({} files, {}).",
            report.applied.len(),
            report.files(),
            human_bytes(report.bytes())
        );
    }

    if !report.refused.is_empty() {
        println!();
        println!("Refused ({}) — source untouched:", report.refused.len());
        for refused in &report.refused {
            println!("  {}: {}", rel(root, &refused.from), refused.reason);
        }
    }

    if !report.failed.is_empty() {
        println!();
        println!("FAILED ({}):", report.failed.len());
        for failed in &report.failed {
            println!(
                "  {}: {} [source {}]",
                rel(root, &failed.from),
                failed.error,
                if failed.source_intact { "intact" } else { "NEEDS A LOOK" }
            );
        }
    }

    if let Some(journal) = &report.journal {
        println!();
        println!("Journal: {}", journal.display());
        println!("Undo with: mp3rnm undo {} --execute", journal.display());
    }
}

/// Ask before doing anything irreversible. Only a literal `yes` counts.
pub fn confirm(question: &str) -> std::io::Result<bool> {
    use std::io::Write;
    print!("{question} [type 'yes' to continue] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer)? == 0 {
        return Ok(false);
    }
    Ok(answer.trim().eq_ignore_ascii_case("yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_are_readable() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MiB");
    }

    #[test]
    fn paths_are_shown_relative_to_the_root() {
        assert_eq!(rel(Path::new("/music"), Path::new("/music/a/b")), "a/b");
        assert_eq!(rel(Path::new("/music"), Path::new("/other")), "/other");
        assert_eq!(rel(Path::new("/music"), Path::new("/music")), "/music");
    }
}
