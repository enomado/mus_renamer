//! The window.
//!
//! It mirrors the command line exactly — scan, look at the plan, dry run, apply,
//! undo — with the same defaults: nothing is executed until the Apply button is
//! pressed and the confirmation is answered, and every row can be unticked.

use std::path::PathBuf;

use eframe::egui;
use mus_core::config::{ArtistSource, Layout, LegacyEncoding, TranslitMode, UntaggedPolicy};
use mus_core::{Config, Plan};
use mus_fs::{ApplyOptions, ApplyReport, CrossDevice, VerifyLevel};

use crate::worker::{rel, Message, Worker};

pub struct App {
    root: String,
    cfg: Config,
    verify: VerifyLevel,
    cross_device: CrossDevice,
    check_stamps: bool,
    write_journal: bool,

    plan: Option<Plan>,
    include: Vec<bool>,
    last_report: Option<Box<ApplyReport>>,
    log: Vec<String>,
    busy: bool,
    confirming: bool,
    /// A folder given on the command line is scanned on the first frame.
    scan_on_start: bool,

    worker: Worker,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let root = std::env::args().nth(1).unwrap_or_default();
        Self {
            scan_on_start: !root.trim().is_empty(),
            root,
            cfg: Config::default(),
            verify: VerifyLevel::Metadata,
            cross_device: CrossDevice::Refuse,
            check_stamps: true,
            write_journal: true,
            plan: None,
            include: Vec::new(),
            last_report: None,
            log: Vec::new(),
            busy: false,
            confirming: false,
            worker: Worker::default(),
        }
    }

    fn root_path(&self) -> PathBuf {
        PathBuf::from(self.root.trim())
    }

    /// The plan reduced to the ticked rows. Dropping moves can only remove
    /// collisions, never create one, so the subset is as safe as the whole.
    fn selected_plan(&self) -> Option<Plan> {
        let plan = self.plan.as_ref()?;
        let mut selected = plan.clone();
        selected.moves = plan
            .moves
            .iter()
            .enumerate()
            .filter(|(i, _)| self.include.get(*i).copied().unwrap_or(true))
            .map(|(_, m)| m.clone())
            .collect();
        Some(selected)
    }

    fn start_scan(&mut self, ctx: &egui::Context) {
        let root = self.root_path();
        if !root.is_dir() {
            self.log.push(format!("{} is not a directory", root.display()));
            return;
        }
        self.busy = true;
        self.plan = None;
        self.last_report = None;
        let ctx = ctx.clone();
        self.worker.plan(root, self.cfg.clone(), move || ctx.request_repaint());
    }

    fn start_apply(&mut self, ctx: &egui::Context, execute: bool) {
        let Some(plan) = self.selected_plan() else { return };
        if plan.moves.is_empty() {
            self.log.push("nothing selected".to_string());
            return;
        }
        self.busy = true;
        let opts = ApplyOptions {
            execute,
            verify: self.verify,
            cross_device: self.cross_device,
            journal: self.write_journal,
            state_dir: None,
            check_stamps: self.check_stamps,
            stop_on_first_error: false,
        };
        let ctx = ctx.clone();
        self.worker.apply(plan, opts, move || ctx.request_repaint());
    }

    fn drain_messages(&mut self) {
        while let Ok(message) = self.worker.rx.try_recv() {
            match message {
                Message::Log(line) => self.log.push(line),
                Message::Planned(Ok(plan)) => {
                    self.log.push(format!("{} folders to move", plan.moves.len()));
                    self.include = vec![true; plan.moves.len()];
                    self.plan = Some(*plan);
                    self.busy = false;
                }
                Message::Planned(Err(e)) => {
                    self.log.push(format!("scan failed: {e}"));
                    self.busy = false;
                }
                Message::Applied(Ok(report)) => {
                    self.log.push(format!(
                        "{}: {} moved, {} refused, {} failed",
                        if report.dry_run { "dry run" } else { "done" },
                        report.applied.len(),
                        report.refused.len(),
                        report.failed.len()
                    ));
                    if let Some(journal) = &report.journal {
                        self.log.push(format!("journal: {}", journal.display()));
                    }
                    // After a real run the plan describes paths that no longer
                    // exist; force a rescan rather than showing a stale list.
                    if !report.dry_run {
                        self.plan = None;
                        self.include.clear();
                    }
                    self.last_report = Some(report);
                    self.busy = false;
                }
                Message::Applied(Err(e)) => {
                    self.log.push(format!("apply failed: {e}"));
                    self.busy = false;
                }
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_messages();
        let ctx = ui.ctx().clone();
        let ctx = &ctx;

        if std::mem::take(&mut self.scan_on_start) {
            self.start_scan(ctx);
        }

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Music folder:");
                ui.add(egui::TextEdit::singleline(&mut self.root).desired_width(360.0));
                if ui.button("Browse…").clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        self.root = dir.to_string_lossy().to_string();
                    }
                }
                ui.add_enabled_ui(!self.busy, |ui| {
                    if ui.button("Scan").clicked() {
                        self.start_scan(ctx);
                    }
                });
                if self.busy {
                    ui.spinner();
                }
            });
            ui.add_space(4.0);
        });

        egui::Panel::right("settings").default_size(280.0).show(ui, |ui| {
            ui.heading("Naming");
            egui::ComboBox::from_label("Layout")
                .selected_text(match self.cfg.layout.layout {
                    Layout::Flat => "flat",
                    Layout::ArtistFolders => "artist folders",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.cfg.layout.layout, Layout::Flat, "flat");
                    ui.selectable_value(&mut self.cfg.layout.layout, Layout::ArtistFolders, "artist folders");
                });
            egui::ComboBox::from_label("Transliteration")
                .selected_text(match self.cfg.naming.translit {
                    TranslitMode::Off => "off",
                    TranslitMode::Cyrillic => "cyrillic -> latin",
                    TranslitMode::AsciiOnly => "ascii only",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.cfg.naming.translit, TranslitMode::Off, "off");
                    ui.selectable_value(
                        &mut self.cfg.naming.translit,
                        TranslitMode::Cyrillic,
                        "cyrillic -> latin",
                    );
                    ui.selectable_value(&mut self.cfg.naming.translit, TranslitMode::AsciiOnly, "ascii only");
                });
            ui.checkbox(&mut self.cfg.naming.fat_safe, "FAT-safe names");
            ui.label("Album template:");
            ui.add(
                egui::TextEdit::singleline(&mut self.cfg.naming.album_template).desired_width(f32::INFINITY),
            );
            ui.label("Collection template:");
            ui.add(
                egui::TextEdit::singleline(&mut self.cfg.naming.collection_template)
                    .desired_width(f32::INFINITY),
            );
            ui.label("Compilation template:");
            ui.add(
                egui::TextEdit::singleline(&mut self.cfg.naming.various_template)
                    .desired_width(f32::INFINITY),
            );

            ui.separator();
            ui.heading("Reading");
            egui::ComboBox::from_label("Artist from")
                .selected_text(match self.cfg.classify.artist_source {
                    ArtistSource::Artist => "artist",
                    ArtistSource::AlbumArtist => "album artist",
                    ArtistSource::AlbumArtistThenArtist => "album artist, else artist",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.cfg.classify.artist_source, ArtistSource::Artist, "artist");
                    ui.selectable_value(
                        &mut self.cfg.classify.artist_source,
                        ArtistSource::AlbumArtist,
                        "album artist",
                    );
                    ui.selectable_value(
                        &mut self.cfg.classify.artist_source,
                        ArtistSource::AlbumArtistThenArtist,
                        "album artist, else artist",
                    );
                });
            egui::ComboBox::from_label("Missing tags")
                .selected_text(match self.cfg.classify.untagged_artist {
                    UntaggedPolicy::Ambiguous => "leave the folder alone",
                    UntaggedPolicy::AsUnknown => "count as \"unknown\"",
                    UntaggedPolicy::Ignore => "ignore the track",
                })
                .show_ui(ui, |ui| {
                    for (policy, label) in [
                        (UntaggedPolicy::Ambiguous, "leave the folder alone"),
                        (UntaggedPolicy::AsUnknown, "count as \"unknown\""),
                        (UntaggedPolicy::Ignore, "ignore the track"),
                    ] {
                        if ui.selectable_label(self.cfg.classify.untagged_artist == policy, label).clicked() {
                            self.cfg.classify.untagged_artist = policy;
                            self.cfg.classify.untagged_album = policy;
                        }
                    }
                });
            egui::ComboBox::from_label("Legacy encoding")
                .selected_text(match self.cfg.scan.legacy_encoding {
                    LegacyEncoding::None => "none",
                    LegacyEncoding::Cp1251 => "cp1251",
                    LegacyEncoding::Cp1252 => "cp1252",
                    LegacyEncoding::Koi8R => "koi8-r",
                })
                .show_ui(ui, |ui| {
                    for (encoding, label) in [
                        (LegacyEncoding::None, "none"),
                        (LegacyEncoding::Cp1251, "cp1251"),
                        (LegacyEncoding::Cp1252, "cp1252"),
                        (LegacyEncoding::Koi8R, "koi8-r"),
                    ] {
                        ui.selectable_value(&mut self.cfg.scan.legacy_encoding, encoding, label);
                    }
                });
            ui.add(
                egui::Slider::new(&mut self.cfg.classify.min_tracks, 1..=20).text("min tracks per folder"),
            );

            ui.separator();
            ui.heading("Safety");
            egui::ComboBox::from_label("Verify")
                .selected_text(match self.verify {
                    VerifyLevel::Metadata => "file tree and sizes",
                    VerifyLevel::Hash => "hash every file",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.verify, VerifyLevel::Metadata, "file tree and sizes");
                    ui.selectable_value(&mut self.verify, VerifyLevel::Hash, "hash every file");
                });
            egui::ComboBox::from_label("Other filesystem")
                .selected_text(match self.cross_device {
                    CrossDevice::Refuse => "refuse the move",
                    CrossDevice::CopyVerifyDelete => "copy, verify, delete",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.cross_device, CrossDevice::Refuse, "refuse the move");
                    ui.selectable_value(
                        &mut self.cross_device,
                        CrossDevice::CopyVerifyDelete,
                        "copy, verify, delete",
                    );
                });
            ui.checkbox(&mut self.check_stamps, "refuse folders changed since the scan");
            ui.checkbox(&mut self.write_journal, "write a journal (undoable)");
        });

        egui::Panel::bottom("log").resizable(true).default_size(160.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Log");
                if ui.button("clear").clicked() {
                    self.log.clear();
                }
            });
            // Without a floor the panel shrinks to its content, and an empty log
            // leaves nothing but the heading.
            ui.set_min_height(120.0);
            egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                for line in &self.log {
                    ui.monospace(line);
                }
            });
        });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            let Some(plan) = self.plan.clone() else {
                ui.centered_and_justified(|ui| {
                    ui.label("Pick a folder and press Scan. Nothing is moved until you say so.");
                });
                return;
            };

            ui.horizontal(|ui| {
                let selected = self.include.iter().filter(|i| **i).count();
                ui.label(format!("{selected} of {} folders selected", plan.moves.len()));
                if ui.button("all").clicked() {
                    self.include.iter_mut().for_each(|i| *i = true);
                }
                if ui.button("none").clicked() {
                    self.include.iter_mut().for_each(|i| *i = false);
                }
                ui.add_enabled_ui(!self.busy && selected > 0, |ui| {
                    if ui.button("Dry run").clicked() {
                        self.start_apply(ctx, false);
                    }
                    let apply = egui::Button::new("Apply").fill(egui::Color32::from_rgb(140, 40, 40));
                    if ui.add(apply).clicked() {
                        self.confirming = true;
                    }
                });
            });
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("moves").num_columns(4).striped(true).show(ui, |ui| {
                    ui.label("");
                    ui.strong("kind");
                    ui.strong("from");
                    ui.strong("to");
                    ui.end_row();

                    for (i, m) in plan.moves.iter().enumerate() {
                        if self.include.len() <= i {
                            self.include.push(true);
                        }
                        ui.checkbox(&mut self.include[i], "");
                        ui.label(m.kind.label());
                        ui.label(rel(&plan.root, &m.from));
                        ui.label(rel(&plan.root, &m.to));
                        ui.end_row();
                    }
                });

                if !plan.skipped.is_empty() {
                    ui.add_space(12.0);
                    egui::CollapsingHeader::new(format!("left alone ({})", plan.skipped.len())).show(
                        ui,
                        |ui| {
                            egui::Grid::new("skipped").num_columns(2).striped(true).show(ui, |ui| {
                                for skip in &plan.skipped {
                                    ui.label(rel(&plan.root, &skip.dir));
                                    ui.label(&skip.reason);
                                    ui.end_row();
                                }
                            });
                        },
                    );
                }
            });
        });

        if self.confirming {
            let selected = self.include.iter().filter(|i| **i).count();
            egui::Modal::new(egui::Id::new("confirm-apply")).show(ctx, |ui| {
                ui.heading("Apply?");
                ui.label(format!("Move {selected} folders inside {}.", self.root));
                ui.label("Nothing will be overwritten, and the run can be undone from the journal.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.confirming = false;
                    }
                    if ui.button("Move them").clicked() {
                        self.confirming = false;
                        self.start_apply(ctx, true);
                    }
                });
            });
        }
    }
}
