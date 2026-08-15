//! `mp3rnm-ui` — the same tool with a window around it.
//!
//! Optional argument: the folder to start with.

mod app;
mod worker;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 700.0])
            .with_min_inner_size([700.0, 450.0])
            .with_title("mus_renamer"),
        ..Default::default()
    };
    eframe::run_native("mus_renamer", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
