//! Scanning and applying happen on a worker thread; the UI thread only ever
//! reads messages off a channel, so a slow disk never freezes the window.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use mus_core::tags::LoftyReader;
use mus_core::{build_plan, scan_tree, Config, Plan};
use mus_fs::{apply_with, ApplyOptions, ApplyReport, Event};

pub enum Message {
    Log(String),
    Planned(Result<Box<Plan>, String>),
    Applied(Result<Box<ApplyReport>, String>),
}

pub struct Worker {
    pub rx: Receiver<Message>,
    tx: Sender<Message>,
}

impl Default for Worker {
    fn default() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self { rx, tx }
    }
}

impl Worker {
    /// Scan `root` and build a plan. `repaint` wakes the UI when a message lands.
    pub fn plan(&self, root: PathBuf, cfg: Config, repaint: impl Fn() + Send + 'static) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Message::Log(format!("scanning {}…", root.display())));
            let result = (|| {
                let reader = LoftyReader::new(cfg.scan.legacy_encoding);
                let scans = scan_tree(&root, &cfg, &reader).map_err(|e| e.to_string())?;
                let _ = tx.send(Message::Log(format!("{} folders scanned", scans.len())));
                build_plan(&root, &scans, &cfg).map(Box::new).map_err(|e| e.to_string())
            })();
            let _ = tx.send(Message::Planned(result));
            repaint();
        });
    }

    pub fn apply(&self, plan: Plan, opts: ApplyOptions, repaint: impl Fn() + Send + 'static) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let root = plan.root.clone();
            let mut observer = |event: Event<'_>| {
                let line = match event {
                    Event::Started { moves, dry_run } => {
                        Some(format!("{} {moves} folders", if dry_run { "checking" } else { "moving" }))
                    }
                    Event::Applied(m) => Some(format!(
                        "{}  {} → {}",
                        m.method.map(|m| m.as_str()).unwrap_or("would move"),
                        rel(&root, &m.from),
                        rel(&root, &m.to)
                    )),
                    Event::Refused(m) => Some(format!("refused  {}: {}", rel(&root, &m.from), m.reason)),
                    Event::Failed(m) => Some(format!("FAILED   {}: {}", rel(&root, &m.from), m.error)),
                    Event::Finished(_) => None,
                };
                if let Some(line) = line {
                    let _ = tx.send(Message::Log(line));
                }
            };
            let result = apply_with(&plan, &opts, &mut observer).map(Box::new).map_err(|e| e.to_string());
            let _ = tx.send(Message::Applied(result));
            repaint();
        });
    }
}

pub fn rel(root: &std::path::Path, path: &std::path::Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().to_string()
}
