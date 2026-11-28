//! The print dialog (Ctrl+P in the viewer, later also for photos picked in the browser): one / two / four
//! photos per page or exact photo sizes, fit or fill, captions, A4 or Letter, a margin — with a live preview of
//! the first page and the page count. "Print…" opens the standard Windows Print dialog (printer, copies,
//! preferences); "Save as PDF" writes `<name>_print.pdf` next to the first photo (Ctrl+Z removes it).
//! Pages are drawn on worker threads; the window never freezes.

use crate::{bold, colors, icons, raster};
use cs_app::print::{Layout, Paper, PrintOpts, SIZES};
use egui::{Color32, RichText};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

enum Done {
    Preview { page: image::RgbImage, pages: usize },
    Printed(bool),
    Saved(PathBuf),
    Failed(String),
}

pub struct PrintDlg {
    files: Vec<PathBuf>,
    /// The shown photo and all photos of the folder (choice "this photo" / "all").
    one: Vec<PathBuf>,
    all: Vec<PathBuf>,
    use_all: bool,
    prev_kind: usize,
    /// 0 one per page, 1 two, 2 four, 3 exact size.
    kind: usize,
    size: usize,
    paper: Paper,
    fill: bool,
    captions: bool,
    margin_mm: f32,
    preview: Option<egui::TextureHandle>,
    pages: usize,
    /// The settings the preview shows (redrawn when they change).
    shown: Option<String>,
    preview_rx: Option<Receiver<Done>>,
    job_rx: Option<Receiver<Done>>,
    busy_text: &'static str,
    error: Option<String>,
}

pub enum Outcome {
    Open,
    Closed(Option<String>),
}

impl PrintDlg {
    /// `one`: the photo shown (or the picked ones); `all`: every photo of the folder.
    pub fn new(one: Vec<PathBuf>, all: Vec<PathBuf>) -> Self {
        Self {
            files: one.clone(),
            one,
            all,
            use_all: false,
            prev_kind: 0,
            kind: 0,
            size: 1,
            paper: Paper::A4,
            fill: false,
            captions: false,
            margin_mm: 10.0,
            preview: None,
            pages: 0,
            shown: None,
            preview_rx: None,
            job_rx: None,
            busy_text: "",
            error: None,
        }
    }

    fn opts(&self) -> PrintOpts {
        let layout = match self.kind {
            0 => Layout::One,
            1 => Layout::Two,
            2 => Layout::Four,
            4 => Layout::Grid { cols: 3, rows: 4 },
            5 => Layout::Grid { cols: 4, rows: 6 },
            _ => Layout::Size { long_cm: SIZES[self.size].0, short_cm: SIZES[self.size].1 },
        };
        PrintOpts { layout, paper: self.paper, fill: self.fill, captions: self.captions, margin_mm: self.margin_mm }
    }

    fn draw_job(
        &self,
        ctx: &egui::Context,
        dpi: f32,
        then: impl FnOnce(Vec<image::RgbImage>) -> Done + Send + 'static,
    ) -> Receiver<Done> {
        let (tx, rx) = channel();
        let (files, opts, ctx) = (self.files.clone(), self.opts(), ctx.clone());
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                match cs_app::print::draw_pages(&files, &opts, dpi, &raster::caption_rgb) {
                    Ok(pages) => then(pages),
                    Err(e) => Done::Failed(e),
                }
            }))
            .unwrap_or_else(|_| Done::Failed("A photo could not be read for printing.".into()));
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        rx
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        // A contact sheet of one photo makes no sense: it takes the whole folder.
        if self.kind >= 4 && self.prev_kind < 4 && self.all.len() > 1 && self.one.len() == 1 {
            self.use_all = true;
        }
        self.prev_kind = self.kind;
        self.files = if self.use_all { self.all.clone() } else { self.one.clone() };
        let key = format!("{:?}|{}", self.opts(), self.files.len());
        if self.shown.as_deref() != Some(key.as_str()) && self.preview_rx.is_none() {
            self.shown = Some(key);
            self.preview_rx = Some(self.draw_job(ctx, 30.0, |pages| {
                let n = pages.len();
                Done::Preview { page: pages.into_iter().next().unwrap_or_else(|| image::RgbImage::new(1, 1)), pages: n }
            }));
        }
        let mut out = Outcome::Open;
        for rx in [&mut self.preview_rx, &mut self.job_rx] {
            if let Some(d) = rx.as_ref().and_then(|r| r.try_recv().ok()) {
                *rx = None;
                match d {
                    Done::Preview { page, pages } => {
                        let (w, h) = page.dimensions();
                        let img = egui::ColorImage::from_rgb([w as usize, h as usize], page.as_raw());
                        self.preview = Some(ctx.load_texture("print-preview", img, egui::TextureOptions::LINEAR));
                        self.pages = pages;
                    }
                    Done::Printed(true) => out = Outcome::Closed(Some("Sent to the printer.".into())),
                    Done::Printed(false) => {}
                    Done::Saved(p) => {
                        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        out = Outcome::Closed(Some(format!(
                            "Saved “{name}” next to the photo. Ctrl+Z in the browser removes it."
                        )));
                    }
                    Done::Failed(e) => self.error = Some(e),
                }
            }
        }
        if !matches!(out, Outcome::Open) {
            return out;
        }
        let busy = self.job_rx.is_some();
        let mut close = false;
        egui::Modal::new(egui::Id::unique("print-dialog")).show(ctx, |ui| {
            ui.set_width(760.0);
            ui.label(RichText::new(format!("{}  Print", icons::PRINT)).font(bold(18.0)));
            let what = if self.files.len() == 1 {
                self.files[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            } else {
                format!("{} photos", self.files.len())
            };
            ui.label(RichText::new(what).color(colors::MUTED));
            ui.add_space(6.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(380.0);
                    if self.all.len() > 1 && self.one.len() == 1 {
                        ui.label(RichText::new("PRINT").font(bold(12.0)).color(colors::MUTED));
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut self.use_all, false, "This photo");
                            ui.radio_value(
                                &mut self.use_all,
                                true,
                                format!("All {} photos in this folder", self.all.len()),
                            );
                        });
                        ui.add_space(4.0);
                    }
                    ui.label(RichText::new("LAYOUT").font(bold(12.0)).color(colors::MUTED));
                    ui.radio_value(&mut self.kind, 0, "One photo per page (the page turns with the photo)");
                    ui.radio_value(&mut self.kind, 1, "Two photos per page");
                    ui.radio_value(&mut self.kind, 2, "Four photos per page");
                    ui.radio_value(&mut self.kind, 4, "Contact sheet: 12 per page, file name under each");
                    ui.radio_value(&mut self.kind, 5, "Contact sheet: 24 per page, file name under each");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.kind, 3, "Exact photo size");
                        ui.add_enabled_ui(self.kind == 3, |ui| {
                            egui::ComboBox::from_id_salt("print-size")
                                .selected_text(format!("{} × {} cm", SIZES[self.size].1, SIZES[self.size].0))
                                .show_ui(ui, |ui| {
                                    for (i, (l, s)) in SIZES.iter().enumerate() {
                                        ui.selectable_value(&mut self.size, i, format!("{s} × {l} cm"));
                                    }
                                });
                        });
                    });
                    ui.add_space(6.0);
                    ui.label(RichText::new("PHOTO").font(bold(12.0)).color(colors::MUTED));
                    ui.radio_value(&mut self.fill, false, "Fit: the whole photo, white edges where needed");
                    ui.radio_value(&mut self.fill, true, "Fill: no white edges, the photo's edges may be cut");
                    if self.kind >= 4 {
                        ui.add_enabled(
                            false,
                            egui::Checkbox::new(
                                &mut true,
                                "Print the file name under each photo (always on a contact sheet)",
                            ),
                        );
                    } else {
                        ui.checkbox(&mut self.captions, "Print the file name under each photo");
                    }
                    ui.add_space(6.0);
                    ui.label(RichText::new("PAPER").font(bold(12.0)).color(colors::MUTED));
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.paper, Paper::A4, "A4");
                        ui.radio_value(&mut self.paper, Paper::Letter, "Letter");
                    });
                    ui.add(
                        egui::Slider::new(&mut self.margin_mm, 0.0..=30.0).step_by(1.0).suffix(" mm").text("Margin"),
                    );
                });
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "PREVIEW · {} page{}",
                            self.pages,
                            if self.pages == 1 { "" } else { "s" }
                        ))
                        .font(bold(12.0))
                        .color(colors::MUTED),
                    );
                    let (r, _) = ui.allocate_exact_size(egui::vec2(340.0, 340.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, 4.0, Color32::from_gray(40));
                    if let Some(t) = &self.preview {
                        let fit = crate::fit(t.size_vec2() * 4.0, r.shrink(8.0));
                        ui.painter().rect_filled(fit.expand(1.0), 0.0, Color32::from_gray(120));
                        ui.painter().image(
                            t.id(),
                            fit,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    if self.preview_rx.is_some() {
                        ui.painter().text(
                            r.center(),
                            egui::Align2::CENTER_CENTER,
                            "…",
                            egui::FontId::proportional(28.0),
                            colors::MUTED,
                        );
                    }
                });
            });
            if let Some(e) = &self.error {
                ui.colored_label(colors::RED, e);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy, egui::Button::new("Cancel")).clicked() {
                    close = true;
                }
                if busy {
                    ui.spinner();
                    ui.label(self.busy_text);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let print =
                        egui::Button::new(RichText::new(format!("{}  Print…", icons::PRINT)).color(Color32::WHITE))
                            .fill(colors::ACCENT);
                    if ui
                        .add_enabled(!busy, print)
                        .on_hover_text("Opens the Windows Print dialog: printer, copies and settings")
                        .clicked()
                    {
                        self.error = None;
                        self.busy_text = "Preparing the pages…";
                        let title = format!(
                            "ChronoSort – {}",
                            self.files
                                .first()
                                .and_then(|p| p.file_name())
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        );
                        self.job_rx = Some(self.draw_job(ctx, 300.0, move |pages| {
                            match cs_app::print::print(&pages, &title, 0) {
                                Ok(sent) => Done::Printed(sent),
                                Err(e) => Done::Failed(e),
                            }
                        }));
                    }
                    if ui
                        .add_enabled(!busy, egui::Button::new("Save as PDF"))
                        .on_hover_text("A PDF file next to the photo, to print later or send")
                        .clicked()
                    {
                        self.error = None;
                        self.busy_text = "Making the PDF…";
                        let paper = self.paper;
                        let first = self.files.first().cloned().unwrap_or_default();
                        self.job_rx = Some(self.draw_job(ctx, 300.0, move |pages| {
                            let dir = first.parent().map(PathBuf::from).unwrap_or_default();
                            let stem = first
                                .file_stem()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "photos".into());
                            match cs_app::print::pdf(&pages, paper)
                                .and_then(|pdf| cs_app::print::save_pdf(&pdf, &dir, &format!("{stem}_print.pdf")))
                            {
                                Ok(p) => Done::Saved(p),
                                Err(e) => Done::Failed(e),
                            }
                        }));
                    }
                });
            });
        });
        if close || (ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !busy) {
            return Outcome::Closed(None);
        }
        Outcome::Open
    }
}
