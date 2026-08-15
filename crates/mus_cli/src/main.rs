//! `mp3rnm` — the command line front end.
//!
//! Safety posture, in one place:
//! * every command is a dry run unless `--execute` is given;
//! * `--execute` asks for confirmation unless `-y` is given;
//! * an executed run leaves a journal in `<root>/.mus_renamer/` and can be undone.

mod args;
mod report;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use args::{Cli, Command, ConfigArgs, ExecArgs};
use clap::Parser;
use mus_core::tags::LoftyReader;
use mus_core::{build_plan, scan_tree, Config, Plan};
use mus_fs::apply::{apply_with, Event};

/// 0 — everything worked. 1 — something failed. 2 — the command was wrong.
/// 3 — the user said no.
fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

type Failure = Box<dyn std::error::Error>;

fn run() -> Result<ExitCode, Failure> {
    let cli = Cli::parse();

    match cli.command {
        Command::Scan { folder, config, json } => {
            let cfg = config.resolve()?;
            let root = canonical(&folder)?;
            let scans = scan_tree(&root, &cfg, &LoftyReader::new(cfg.scan.legacy_encoding))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&scans)?);
            } else {
                report::print_scan(&root, &scans, &cfg);
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Plan { folder, config, out, json } => {
            let cfg = config.resolve()?;
            let root = canonical(&folder)?;
            let plan = make_plan(&root, &cfg)?;
            if json {
                println!("{}", plan.to_json()?);
            } else {
                report::print_plan(&plan, true);
            }
            if let Some(out) = out {
                std::fs::write(&out, plan.to_json()?)?;
                eprintln!("plan written to {}", out.display());
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Apply { folder, plan: plan_path, config, exec } => {
            cmd_apply(folder, plan_path, config, exec)
        }

        Command::Clean { folder, config, non_audio, empty_dirs, exec } => {
            cmd_clean(folder, config, non_audio, empty_dirs, exec)
        }

        Command::Undo { target, exec } => cmd_undo(&target, &exec),

        Command::Journals { folder } => {
            let root = canonical(&folder)?;
            let journals = mus_fs::list_journals(&mus_fs::apply::state_dir_of(&root))?;
            if journals.is_empty() {
                println!("no journals in {}", root.display());
            }
            for journal in journals {
                println!("{}", journal.display());
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Config { config, json } => {
            let cfg = config.resolve()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&cfg)?);
            } else {
                println!("{}", toml::to_string_pretty(&cfg)?);
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn cmd_apply(
    folder: Option<PathBuf>,
    plan_path: Option<PathBuf>,
    config: ConfigArgs,
    exec: ExecArgs,
) -> Result<ExitCode, Failure> {
    let plan = match (&folder, &plan_path) {
        (Some(folder), None) => {
            let cfg = config.resolve()?;
            let root = canonical(folder)?;
            make_plan(&root, &cfg)?
        }
        (None, Some(path)) => Plan::from_json(&std::fs::read_to_string(path)?)?,
        _ => return Err("give either a folder or --plan <FILE>".into()),
    };

    report::print_plan(&plan, false);
    if plan.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }

    if exec.execute && !exec.yes {
        let question = format!("\nMove {} folders inside {}?", plan.moves.len(), plan.root.display());
        if !report::confirm(&question)? {
            println!("Nothing done.");
            return Ok(ExitCode::from(3));
        }
    }

    let opts = exec.apply_options();
    let root = plan.root.clone();
    let mut observer = |event: Event<'_>| match event {
        Event::Applied(m) => println!(
            "{} {} -> {}",
            match m.method {
                Some(method) => method.as_str(),
                None => "would move",
            },
            report::rel(&root, &m.from),
            report::rel(&root, &m.to)
        ),
        Event::Refused(m) => println!("refused  {}: {}", report::rel(&root, &m.from), m.reason),
        Event::Failed(m) => eprintln!("FAILED   {}: {}", report::rel(&root, &m.from), m.error),
        _ => {}
    };

    let result = apply_with(&plan, &opts, &mut observer)?;
    report::print_apply_report(&plan.root, &result);

    if !exec.execute {
        println!("\nThis was a dry run. Re-run with --execute to move anything.");
    }
    Ok(if result.is_clean() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

fn cmd_clean(
    folder: PathBuf,
    config: ConfigArgs,
    non_audio: bool,
    empty_dirs: bool,
    exec: ExecArgs,
) -> Result<ExitCode, Failure> {
    if !non_audio && !empty_dirs {
        return Err("nothing to do: pass --non-audio and/or --empty-dirs".into());
    }
    let cfg = config.resolve()?;
    let root = canonical(&folder)?;

    if exec.execute && !exec.yes {
        let question = format!("\nClean {}?", root.display());
        if !report::confirm(&question)? {
            println!("Nothing done.");
            return Ok(ExitCode::from(3));
        }
    }

    let opts = mus_fs::CleanOptions { execute: exec.execute, non_audio, empty_dirs, state_dir: None };
    let result = mus_fs::clean(&root, &cfg, &opts)?;

    for (from, to) in &result.trashed {
        println!("trash    {} -> {}", report::rel(&root, from), report::rel(&root, to));
    }
    for dir in &result.removed_dirs {
        println!("rmdir    {}", report::rel(&root, dir));
    }
    for error in &result.errors {
        eprintln!("error    {error}");
    }
    println!();
    println!(
        "{} files {} the trash, {} empty directories {}",
        result.trashed.len(),
        if result.dry_run { "would move to" } else { "moved to" },
        result.removed_dirs.len(),
        if result.dry_run { "would be removed" } else { "removed" }
    );
    if let Some(journal) = &result.journal {
        println!("Journal: {}", journal.display());
    }
    if result.dry_run {
        println!("\nThis was a dry run. Re-run with --execute to move anything.");
    }

    Ok(if result.errors.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

fn cmd_undo(target: &Path, exec: &ExecArgs) -> Result<ExitCode, Failure> {
    let journal = if target.is_dir() {
        let state_dir = mus_fs::apply::state_dir_of(&canonical(target)?);
        mus_fs::list_journals(&state_dir)?
            .pop()
            .ok_or_else(|| format!("no journal found in {}", state_dir.display()))?
    } else {
        target.to_path_buf()
    };
    println!("Undoing {}", journal.display());

    if exec.execute && !exec.yes && !report::confirm("\nUndo this run?")? {
        println!("Nothing done.");
        return Ok(ExitCode::from(3));
    }

    let opts = mus_fs::UndoOptions {
        execute: exec.execute,
        verify: exec.verify.into(),
        cross_device: exec.cross_device.into(),
        journal: !exec.no_journal,
    };
    let result = mus_fs::undo(&journal, &opts)?;

    for (from, to) in &result.restored {
        println!("restore  {} -> {}", from.display(), to.display());
    }
    for dir in &result.removed_dirs {
        println!("rmdir    {}", dir.display());
    }
    for error in &result.errors {
        eprintln!("error    {error}");
    }
    println!();
    println!(
        "{} entries {}",
        result.restored.len(),
        if result.dry_run { "would be restored" } else { "restored" }
    );
    if result.dry_run {
        println!("\nThis was a dry run. Re-run with --execute to restore anything.");
    }

    Ok(if result.errors.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

fn make_plan(root: &Path, cfg: &Config) -> Result<Plan, Failure> {
    let reader = LoftyReader::new(cfg.scan.legacy_encoding);
    let scans = scan_tree(root, cfg, &reader)?;
    Ok(build_plan(root, &scans, cfg)?)
}

/// Resolve the root once, so every path in the plan and the journal is absolute.
fn canonical(path: &Path) -> Result<PathBuf, Failure> {
    let canonical = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!("{} is not a directory", canonical.display()).into());
    }
    Ok(canonical)
}
