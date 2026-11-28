//! "Set as desktop wallpaper" (viewer and browser): the same six choices as Windows. The photo is copied
//! (upright, as JPEG) into `ChronoSortData\wallpaper`, so the wallpaper stays when the photo is moved;
//! Ctrl+Z in the browser puts the previous wallpaper back.

use crate::{bold, colors, icons};
use egui::{Color32, RichText};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

const STYLES: [(&str, &str, &str); 6] = [
    ("fill", "Fill", "Fills the screen; edges may be cut"),
    ("fit", "Fit", "The whole photo, with bars at the sides"),
    ("stretch", "Stretch", "Fills the screen; the photo is pulled out of shape"),
    ("center", "Centre", "Real size in the middle"),
    ("tile", "Tile", "Repeated like tiles"),
    ("span", "Span", "One photo across all screens"),
];

pub struct WallDlg {
    path: PathBuf,
    style: &'static str,
    job: Option<Receiver<Result<String, String>>>,
    error: Option<String>,
}

pub enum Outcome {
    Open,
    Closed(Option<String>),
}

impl WallDlg {
    pub fn new(path: &Path) -> Self {
        Self { path: path.to_path_buf(), style: "fill", job: None, error: None }
    }

    pub fn show(&mut self, ctx: &egui::Context, data_root: &Path) -> Outcome {
        if let Some(r) = self.job.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.job = None;
            match r {
                Ok(name) => {
                    return Outcome::Closed(Some(format!(
                        "“{name}” is now your desktop wallpaper. Ctrl+Z in the browser puts the previous one back."
                    )));
                }
                Err(e) => self.error = Some(e),
            }
        }
        let busy = self.job.is_some();
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !busy;
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        egui::Modal::new(egui::Id::unique("wallpaper")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(RichText::new(format!("{}  Set as desktop wallpaper", icons::WALLPAPER)).font(bold(18.0)));
            ui.label(RichText::new(&name).color(colors::MUTED));
            ui.add_space(8.0);
            for (k, t, hint) in STYLES {
                ui.radio_value(&mut self.style, k, format!("{t}  —  {hint}"));
            }
            if let Some(e) = &self.error {
                ui.colored_label(colors::RED, e);
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy, egui::Button::new("Cancel")).clicked() {
                    close = true;
                }
                let label = if busy { "Setting…" } else { "Set as wallpaper" };
                if ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(RichText::new(label).color(Color32::WHITE)).fill(colors::ACCENT),
                    )
                    .clicked()
                {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let (src, style, data, c) = (self.path.clone(), self.style, data_root.to_path_buf(), ctx.clone());
                    std::thread::spawn(move || {
                        let _ = tx.send(cs_app::wallpaper::set_wallpaper(&src, style, &data));
                        c.request_repaint();
                    });
                    self.job = Some(rx);
                    self.error = None;
                }
            });
        });
        if close { Outcome::Closed(None) } else { Outcome::Open }
    }
}
