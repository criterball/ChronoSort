//! Batch (F3 / F4 in the browser), like the Tauri app's:
//! * Convert / resize: copies into a new "Converted" folder (or a chosen one) — format, quality, size
//!   (longest side, percent, fit inside), optionally the frame and watermark; originals never change.
//! * Rename: a pattern ({name} {num} {year} … {folder}) with a live before → after list; refused while any
//!   name has a problem; sidecar files follow their photo. One Ctrl+Z undoes either.

use crate::{bold, colors, icons};
use cs_app::batch::{ConvertOpts, Progress, RenameOpts, RenameRow, Resize};
use egui::{Color32, RichText};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Tab {
    Convert,
    Rename,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Size {
    Keep,
    Long,
    Pct,
    Fit,
}

/// A finished batch: the message, and a folder to open.
type JobResult = Result<(String, Option<PathBuf>), String>;

const PRESETS: [(&str, &str); 5] = [
    ("{name}", "Keep the names"),
    ("{year}-{month}-{day} {hour}.{minute}.{second}", "Date and time taken"),
    ("{year}-{month}-{day} {num}", "Date taken + number"),
    ("Holiday {num}", "Text + number"),
    ("{folder} {num}", "Folder name + number"),
];

pub struct BatchDlg {
    pub tab: Tab,
    files: Vec<PathBuf>,
    pictures: Vec<PathBuf>,
    // convert
    format: String,
    quality: u8,
    size: Size,
    long: u32,
    pct: u32,
    fit: (u32, u32),
    folder: Option<PathBuf>,
    // rename
    pattern: String,
    start: i64,
    digits: u8,
    find: String,
    replace: String,
    ext_case: &'static str,
    sidecars: bool,
    preview: Vec<RenameRow>,
    preview_key: String,
    preview_rx: Option<Receiver<Result<Vec<RenameRow>, String>>>,
    // running
    job: Option<Receiver<JobResult>>,
    progress: Arc<Mutex<Progress>>,
    result: Option<(String, Option<PathBuf>)>,
    error: Option<String>,
}

pub enum Outcome {
    Open,
    /// Closed; something changed on disk (reload), and a folder to open (the converted copies).
    Closed {
        changed: bool,
        open: Option<PathBuf>,
    },
}

impl BatchDlg {
    /// `files`: the picked files, or every file of the folder.
    pub fn new(tab: Tab, files: Vec<PathBuf>) -> Self {
        let pictures = files
            .iter()
            .filter(|p| {
                let e = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
                matches!(crate::browse::kind_of(&e), crate::browse::Kind::Image | crate::browse::Kind::Raw)
            })
            .cloned()
            .collect();
        Self {
            tab,
            files,
            pictures,
            format: "keep".into(),
            quality: 90,
            size: Size::Long,
            long: 1920,
            pct: 50,
            fit: (1920, 1080),
            folder: None,
            pattern: "{name}".into(),
            start: 1,
            digits: 0,
            find: String::new(),
            replace: String::new(),
            ext_case: "keep",
            sidecars: true,
            preview: Vec::new(),
            preview_key: String::new(),
            preview_rx: None,
            job: None,
            progress: Arc::new(Mutex::new(Progress::default())),
            result: None,
            error: None,
        }
    }

    fn rename_opts(&self) -> RenameOpts {
        RenameOpts {
            pattern: self.pattern.clone(),
            start: self.start,
            step: 1,
            digits: self.digits,
            find: self.find.clone(),
            replace: self.replace.clone(),
            ext_case: self.ext_case.into(),
            sidecars: self.sidecars,
            tz_offset_min: cs_platform::local_utc_offset_min(),
        }
    }

    fn resize(&self) -> Resize {
        match self.size {
            Size::Keep => Resize::None,
            Size::Long => Resize::Long { px: self.long },
            Size::Pct => Resize::Percent { pct: self.pct },
            Size::Fit => Resize::Fit { w: self.fit.0, h: self.fit.1 },
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        // Answers from the workers.
        if let Some(r) = self.preview_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.preview_rx = None;
            match r {
                Ok(rows) => {
                    self.preview = rows;
                    self.error = None;
                }
                Err(e) => {
                    self.preview.clear();
                    self.error = Some(e);
                }
            }
        }
        let mut changed_now = false;
        if let Some(r) = self.job.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.job = None;
            match r {
                Ok(done) => {
                    self.result = Some(done);
                    changed_now = true;
                }
                Err(e) => self.error = Some(e),
            }
        }
        if self.tab == Tab::Rename && self.result.is_none() {
            let key = format!(
                "{:?}",
                (&self.pattern, self.start, self.digits, &self.find, &self.replace, self.ext_case, self.sidecars)
            );
            if key != self.preview_key && self.preview_rx.is_none() {
                self.preview_key = key;
                let (tx, rx) = channel();
                let (files, opts, c) = (self.files.clone(), self.rename_opts(), ctx.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(cs_app::batch::rename_preview(&files, &opts));
                    c.request_repaint();
                });
                self.preview_rx = Some(rx);
            }
        }
        let busy = self.job.is_some();
        if busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        let mut out = Outcome::Open;
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !busy;
        egui::Modal::new(egui::Id::unique("batch")).show(ctx, |ui| {
            ui.set_width(680.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Batch").font(bold(20.0)));
                ui.add_space(12.0);
                for (t, label) in [(Tab::Convert, "Convert / resize  (F3)"), (Tab::Rename, "Rename  (F4)")] {
                    if ui.add_enabled(!busy, egui::Button::selectable(self.tab == t, label)).clicked() {
                        self.tab = t;
                        self.result = None;
                        self.error = None;
                    }
                }
            });
            ui.separator();
            match self.tab {
                Tab::Convert => self.convert_ui(ui),
                Tab::Rename => self.rename_ui(ui),
            }
            if let Some(e) = &self.error {
                ui.colored_label(colors::RED, e);
            }
            if busy {
                let p = self.progress.lock().map(|p| p.clone()).unwrap_or_default();
                let f = if p.total > 0 { p.done as f32 / p.total as f32 } else { 0.0 };
                ui.add(
                    egui::ProgressBar::new(f).show_percentage().text(format!("{} of {} · {}", p.done, p.total, p.name)),
                );
            }
            if let Some((text, folder)) = &self.result {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(text).color(colors::GREEN));
                    if let Some(f) = folder
                        && ui.link("Open the folder").clicked()
                    {
                        out = Outcome::Closed { changed: true, open: Some(f.clone()) };
                    }
                });
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if busy {
                    if self.tab == Tab::Convert && ui.button("Stop").clicked() {
                        cs_app::batch::cancel();
                    }
                } else if ui.button(if self.result.is_some() { "Close" } else { "Cancel" }).clicked() {
                    close = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.result.is_some() {
                        return;
                    }
                    match self.tab {
                        Tab::Convert => {
                            let n = self.pictures.len();
                            let label = format!("Convert {n} photo{}", if n == 1 { "" } else { "s" });
                            if ui
                                .add_enabled(
                                    !busy && n > 0,
                                    egui::Button::new(RichText::new(label).color(Color32::WHITE)).fill(colors::ACCENT),
                                )
                                .clicked()
                            {
                                self.run_convert(ctx);
                            }
                        }
                        Tab::Rename => {
                            let problems = self.preview.iter().filter(|r| r.problem.is_some()).count();
                            let changes = self.preview.iter().filter(|r| r.old != r.new).count();
                            let label = format!("Rename {changes} file{}", if changes == 1 { "" } else { "s" });
                            let ok = !busy && problems == 0 && changes > 0 && self.preview_rx.is_none();
                            if ui
                                .add_enabled(
                                    ok,
                                    egui::Button::new(RichText::new(label).color(Color32::WHITE)).fill(colors::ACCENT),
                                )
                                .clicked()
                            {
                                self.run_rename(ctx);
                            }
                        }
                    }
                });
            });
        });
        if let Outcome::Closed { .. } = out {
            return out;
        }
        if close {
            return Outcome::Closed { changed: self.result.is_some(), open: None };
        }
        if changed_now {
            // Stay open to show the result; the browser reloads when it closes.
        }
        out
    }

    fn convert_ui(&mut self, ui: &mut egui::Ui) {
        let hint = |ui: &mut egui::Ui, t: &str| {
            ui.label(RichText::new(t).size(13.0).color(colors::MUTED));
        };
        let n = self.pictures.len();
        hint(
            ui,
            &format!(
                "{n} photo{} (other files are left out). New copies are made; the originals do not change.",
                if n == 1 { "" } else { "s" }
            ),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Save as");
            let label =
                if self.format == "keep" { "Same type as the original".to_string() } else { self.format.clone() };
            egui::ComboBox::from_id_salt("fmt").selected_text(label).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.format, "keep".into(), "Same type as the original");
                for f in cs_app::edit::formats() {
                    ui.selectable_value(&mut self.format, f.to_string(), f);
                }
            });
            if self.format == "keep" || self.format == "JPEG" || self.format == "WebP" {
                ui.add(egui::Slider::new(&mut self.quality, 50..=100).text("Quality"));
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new("SIZE").font(bold(12.0)).color(colors::MUTED));
        ui.radio_value(&mut self.size, Size::Keep, "Keep the size");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.size, Size::Long, "Longest side");
            if ui.add(egui::DragValue::new(&mut self.long).range(16..=20000).suffix(" px")).changed() {
                self.size = Size::Long;
            }
            for (v, t) in [(1280, "e-mail"), (1920, "screen"), (3840, "4K")] {
                if ui.small_button(t).clicked() {
                    self.size = Size::Long;
                    self.long = v;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.size, Size::Pct, "");
            if ui.add(egui::DragValue::new(&mut self.pct).range(1..=99).suffix(" %")).changed() {
                self.size = Size::Pct;
            }
            ui.label("of the size");
        });
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.size, Size::Fit, "Fit inside");
            let a = ui.add(egui::DragValue::new(&mut self.fit.0).range(16..=20000));
            ui.label("×");
            let b = ui.add(egui::DragValue::new(&mut self.fit.1).range(16..=20000));
            if a.changed() || b.changed() {
                self.size = Size::Fit;
            }
        });
        hint(
            ui,
            "Pictures are only made smaller, never bigger. Dates and photo information are kept. RAW photos use the camera's own JPEG inside them.",
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Put the copies in");
            let where_ = self
                .folder
                .as_ref()
                .map_or("a new folder “Converted” next to them".to_string(), |f| crate::browse::shown(f));
            ui.label(RichText::new(where_).strong());
            if ui.button("Choose…").clicked()
                && let Some(f) = rfd::FileDialog::new().set_title("Put the copies in which folder?").pick_folder()
            {
                self.folder = Some(f);
            }
            if self.folder.is_some() && ui.button("Use “Converted”").clicked() {
                self.folder = None;
            }
        });
    }

    fn rename_ui(&mut self, ui: &mut egui::Ui) {
        let hint = |ui: &mut egui::Ui, t: &str| {
            ui.label(RichText::new(t).size(13.0).color(colors::MUTED));
        };
        ui.horizontal(|ui| {
            ui.label("New name");
            ui.add(egui::TextEdit::singleline(&mut self.pattern).desired_width(360.0).font(egui::TextStyle::Monospace));
            egui::ComboBox::from_id_salt("presets").selected_text("Ready-made…").show_ui(ui, |ui| {
                for (v, t) in PRESETS {
                    if ui.selectable_label(false, t).clicked() {
                        self.pattern = v.into();
                    }
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            for (t, l) in [
                ("{name}", "old name"),
                ("{num}", "number"),
                ("{year}-{month}-{day}", "date taken"),
                ("{hour}.{minute}.{second}", "time"),
                ("{folder}", "folder name"),
            ] {
                if ui.small_button(format!("+ {l}")).on_hover_text(format!("Add {t}")).clicked() {
                    self.pattern = format!("{} {t}", self.pattern).trim().to_string();
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("First number");
            ui.add(egui::DragValue::new(&mut self.start).range(0..=999_999));
            ui.label("Digits");
            egui::ComboBox::from_id_salt("digits")
                .selected_text(if self.digits == 0 {
                    "Automatic".to_string()
                } else {
                    format!("{} ({}1)", self.digits, "0".repeat(usize::from(self.digits) - 1))
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.digits, 0, "Automatic");
                    for d in 1..=5u8 {
                        ui.selectable_value(&mut self.digits, d, format!("{d} ({}1)", "0".repeat(usize::from(d) - 1)));
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Replace");
            ui.add(egui::TextEdit::singleline(&mut self.find).hint_text("text").desired_width(140.0));
            ui.label("with");
            ui.add(egui::TextEdit::singleline(&mut self.replace).desired_width(140.0));
            ui.label("Extension");
            egui::ComboBox::from_id_salt("ext")
                .selected_text(match self.ext_case {
                    "lower" => "small letters (.jpg)",
                    "upper" => "CAPITALS (.JPG)",
                    _ => "as it is",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.ext_case, "keep", "as it is");
                    ui.selectable_value(&mut self.ext_case, "lower", "small letters (.jpg)");
                    ui.selectable_value(&mut self.ext_case, "upper", "CAPITALS (.JPG)");
                });
        });
        ui.checkbox(&mut self.sidecars, "Also rename matching sidecar files (.xmp, .aae, .thm, .json)");
        ui.add_space(4.0);
        let problems = self.preview.iter().filter(|r| r.problem.is_some()).count();
        let changes = self.preview.iter().filter(|r| r.old != r.new).count();
        if problems > 0 {
            ui.label(
                RichText::new(format!(
                    "{problems} name{} a change before renaming.",
                    if problems == 1 { " needs" } else { "s need" }
                ))
                .color(colors::RED),
            );
        } else {
            hint(
                ui,
                &format!(
                    "{changes} file{} a new name. Ctrl+Z in the browser puts the old names back.",
                    if changes == 1 { " gets" } else { "s get" }
                ),
            );
        }
        egui::ScrollArea::vertical().max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
            egui::Grid::new("rename-preview").striped(true).num_columns(3).spacing([10.0, 3.0]).show(ui, |ui| {
                for r in self.preview.iter().take(500) {
                    ui.label(RichText::new(&r.old).size(13.0).color(colors::MUTED));
                    ui.label(icons::FORWARD);
                    let c = if r.problem.is_some() {
                        colors::RED
                    } else if r.old == r.new {
                        colors::MUTED
                    } else {
                        colors::TEXT
                    };
                    let t = match &r.problem {
                        Some(p) => format!("{}   ({p})", r.new),
                        None => r.new.clone(),
                    };
                    ui.label(RichText::new(t).size(13.0).color(c));
                    ui.end_row();
                }
            });
        });
    }

    fn run_convert(&mut self, ctx: &egui::Context) {
        let opts = ConvertOpts {
            format: self.format.clone(),
            quality: self.quality,
            resize: self.resize(),
            folder: self.folder.as_ref().map(|f| f.to_string_lossy().into_owned()),
            effects: None,
            watermark_png: None,
            watermark_path: None,
        };
        let (tx, rx) = channel();
        let (files, prog, c) = (self.pictures.clone(), self.progress.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = cs_app::batch::convert(files, opts, |p| {
                if let Ok(mut g) = prog.lock() {
                    *g = p;
                }
            })
            .map(|rep| {
                let mut s = format!(
                    "{} copies made in “{}”.",
                    rep.done,
                    crate::browse::shown(std::path::Path::new(&rep.folder))
                );
                if rep.cancelled {
                    s += " Stopped.";
                }
                if !rep.failed.is_empty() {
                    s += &format!(" {} could not: {} ({})", rep.failed.len(), rep.failed[0].0, rep.failed[0].1);
                }
                (s + " Ctrl+Z removes them.", Some(PathBuf::from(rep.folder)))
            });
            let _ = tx.send(r);
            c.request_repaint();
        });
        self.job = Some(rx);
        self.error = None;
    }

    fn run_rename(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let (files, opts, prog, c) = (self.files.clone(), self.rename_opts(), self.progress.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = cs_app::batch::rename(&files, &opts, |p| {
                if let Ok(mut g) = prog.lock() {
                    *g = p;
                }
            })
            .map(|rep| {
                let mut s = format!("{} file{} renamed.", rep.renamed, if rep.renamed == 1 { "" } else { "s" });
                if rep.sidecars > 0 {
                    s += &format!(" {} sidecar file(s) followed.", rep.sidecars);
                }
                (s + " Ctrl+Z puts the old names back.", None)
            });
            let _ = tx.send(r);
            c.request_repaint();
        });
        self.job = Some(rx);
        self.error = None;
    }
}
