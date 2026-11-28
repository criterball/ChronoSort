//! "Contact sheet, PDF or ZIP…" in the browser, like the Tauri app's Create dialog: a contact sheet as a
//! picture (one or more pages), one picture joined from several (a strip), a PDF with a photo per page, or a
//! ZIP to send. The new file goes next to the first file; nothing is replaced; Ctrl+Z removes it again.
//! The layouts are calculated here and the text (title, captions) drawn with the ChronoSort font.

use crate::{bold, colors, raster};
use cs_app::create::{Cell, PdfOpts, SheetOpts};
use egui::{Color32, RichText};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Tab {
    Sheet,
    Strip,
    Pdf,
    Zip,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum CaptionKind {
    None,
    Name,
    NameDate,
}

pub struct CreateDlg {
    tab: Tab,
    /// All files (ZIP), and the photos and videos among them (the rest).
    files: Vec<PathBuf>,
    media: Vec<(PathBuf, i64)>,
    folder: String,
    names: [String; 4],
    letter: bool,
    landscape: bool,
    dpi: u32,
    cols: u32,
    caption: CaptionKind,
    title: bool,
    row: bool,
    side: u32,
    gap: u32,
    bg: [u8; 3],
    png: bool,
    pdf_letter: bool,
    margin: f32,
    job: Option<Receiver<Result<String, String>>>,
    progress: Arc<Mutex<cs_app::batch::Progress>>,
    result: Option<String>,
    error: Option<String>,
}

pub enum Outcome {
    Open,
    Closed { changed: bool },
}

/// The contact sheet layout in pixels.
struct Layout {
    w: u32,
    h: u32,
    m: u32,
    g: u32,
    font: f32,
    title_h: u32,
    cols: u32,
    cw: u32,
    bh: u32,
    row_h: u32,
    per_page: usize,
}

impl CreateDlg {
    pub fn new(tab: Tab, files: Vec<PathBuf>, media: Vec<(PathBuf, i64)>, folder: &str) -> Self {
        let f = folder.to_string();
        Self {
            tab,
            files,
            media,
            names: [
                format!("Contact sheet {f}").trim().to_string(),
                format!("{} joined", if f.is_empty() { "Pictures" } else { &f }),
                if f.is_empty() { "Photos".into() } else { f.clone() },
                if f.is_empty() { "Photos".into() } else { f.clone() },
            ],
            folder: f,
            letter: false,
            landscape: false,
            dpi: 150,
            cols: 5,
            caption: CaptionKind::Name,
            title: true,
            row: true,
            side: 1200,
            gap: 12,
            bg: [255, 255, 255],
            png: false,
            pdf_letter: false,
            margin: 10.0,
            job: None,
            progress: Arc::new(Mutex::new(Default::default())),
            result: None,
            error: None,
        }
    }

    fn layout(&self) -> Layout {
        let (mut wmm, mut hmm) = if self.letter { (215.9f32, 279.4f32) } else { (210.0, 297.0) };
        if self.landscape {
            std::mem::swap(&mut wmm, &mut hmm);
        }
        let w = (wmm / 25.4 * self.dpi as f32).round() as u32;
        let h = (hmm / 25.4 * self.dpi as f32).round() as u32;
        let m = (w as f32 * 0.045).round() as u32;
        let g = (w as f32 * 0.018).round() as u32;
        let font = (w as f32 * 0.0125).round().max(10.0);
        let title_h = if self.title { (font * 3.0).round() as u32 } else { 0 };
        let lines = match self.caption {
            CaptionKind::None => 0.0,
            CaptionKind::Name => 1.0,
            CaptionKind::NameDate => 2.0,
        };
        let cap_h = if lines > 0.0 { (font * 1.35 * lines + font * 0.4).round() as u32 } else { 0 };
        let cols = self.cols.clamp(1, 12);
        let cw = (w.saturating_sub(2 * m + (cols - 1) * g)) / cols;
        let bh = (cw as f32 * 0.75).round() as u32;
        let row_h = bh + cap_h;
        let rows = ((h.saturating_sub(2 * m + title_h) + g) / (row_h + g)).max(1);
        Layout { w, h, m, g, font, title_h, cols, cw, bh, row_h, per_page: (cols * rows) as usize }
    }

    fn pages(&self) -> usize {
        self.media.len().div_ceil(self.layout().per_page.max(1)).max(1)
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        if let Some(r) = self.job.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.job = None;
            match r {
                Ok(s) => self.result = Some(s),
                Err(e) => self.error = Some(e),
            }
        }
        let busy = self.job.is_some();
        if busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !busy;
        egui::Modal::new(egui::Id::unique("create")).show(ctx, |ui| {
            ui.set_width(620.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Create").font(bold(20.0)));
                ui.add_space(12.0);
                for (t, l) in [(Tab::Sheet, "Contact sheet"), (Tab::Strip, "Join into one picture"), (Tab::Pdf, "PDF"), (Tab::Zip, "ZIP")] {
                    if ui.add_enabled(!busy, egui::Button::selectable(self.tab == t, l)).clicked() {
                        self.tab = t;
                        self.result = None;
                        self.error = None;
                    }
                }
            });
            ui.separator();
            let hint = |ui: &mut egui::Ui, t: &str| {
                ui.label(RichText::new(t).size(13.0).color(colors::MUTED));
            };
            let idx = self.tab as usize;
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut self.names[idx]).desired_width(360.0));
            });
            match self.tab {
                Tab::Sheet => {
                    let n = self.media.len();
                    hint(ui, &format!("{n} photos and videos as small pictures on a page, with their names — to see a whole folder at a glance."));
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.letter, false, "A4");
                        ui.radio_value(&mut self.letter, true, "Letter");
                        ui.add_space(10.0);
                        ui.radio_value(&mut self.landscape, false, "Portrait");
                        ui.radio_value(&mut self.landscape, true, "Landscape");
                        ui.add_space(10.0);
                        ui.radio_value(&mut self.dpi, 150, "150 dpi");
                        ui.radio_value(&mut self.dpi, 300, "300 dpi (print)");
                    });
                    ui.add(egui::Slider::new(&mut self.cols, 2..=10).text("pictures across"));
                    ui.horizontal(|ui| {
                        ui.label("Under each");
                        ui.radio_value(&mut self.caption, CaptionKind::None, "nothing");
                        ui.radio_value(&mut self.caption, CaptionKind::Name, "the name");
                        ui.radio_value(&mut self.caption, CaptionKind::NameDate, "name and date");
                        ui.checkbox(&mut self.title, "Folder name on top");
                    });
                    let l = self.layout();
                    hint(ui, &format!("{} per page · {} page{} · {} × {} pixels", l.per_page, self.pages(), if self.pages() == 1 { "" } else { "s" }, l.w, l.h));
                }
                Tab::Strip => {
                    hint(ui, "The photos side by side (or one below the other) as one picture.");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.row, true, "Side by side");
                        ui.radio_value(&mut self.row, false, "One below the other");
                    });
                    ui.add(egui::Slider::new(&mut self.side, 200..=4000).suffix(" px").text(if self.row { "Height" } else { "Width" }));
                    ui.add(egui::Slider::new(&mut self.gap, 0..=100).suffix(" px").text("Gap"));
                }
                Tab::Pdf => {
                    hint(ui, "One photo per page; each page turns to suit its photo.");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.pdf_letter, false, "A4");
                        ui.radio_value(&mut self.pdf_letter, true, "Letter");
                    });
                    ui.add(egui::Slider::new(&mut self.margin, 0.0..=30.0).suffix(" mm").text("Margin"));
                }
                Tab::Zip => {
                    let n = self.files.len();
                    hint(ui, &format!("{n} file{} packed into one ZIP file, to send or keep. Photos and videos are stored as they are.", if n == 1 { "" } else { "s" }));
                }
            }
            if matches!(self.tab, Tab::Sheet | Tab::Strip) {
                ui.horizontal(|ui| {
                    ui.label("Background");
                    for c in [[255u8, 255, 255], [0, 0, 0], [40, 42, 46]] {
                        let (r, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                        ui.painter().rect_filled(r, 3.0, Color32::from_rgb(c[0], c[1], c[2]));
                        if self.bg == c {
                            ui.painter().rect_stroke(r, 3.0, egui::Stroke::new(2.0, colors::ACCENT), egui::StrokeKind::Outside);
                        }
                        if resp.clicked() {
                            self.bg = c;
                        }
                    }
                    ui.add_space(10.0);
                    ui.radio_value(&mut self.png, false, "JPEG");
                    ui.radio_value(&mut self.png, true, "PNG");
                });
            }
            if let Some(e) = &self.error {
                ui.colored_label(colors::RED, e);
            }
            if busy {
                let p = self.progress.lock().map(|p| p.clone()).unwrap_or_default();
                let f = if p.total > 0 { p.done as f32 / p.total as f32 } else { 0.0 };
                ui.add(egui::ProgressBar::new(f).show_percentage().text(format!("{} of {} · {}", p.done, p.total, p.name)));
            }
            if let Some(r) = &self.result {
                ui.label(RichText::new(r).color(colors::GREEN));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy, egui::Button::new(if self.result.is_some() { "Close" } else { "Cancel" })).clicked() {
                    close = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let none = if self.tab == Tab::Zip { self.files.is_empty() } else { self.media.is_empty() };
                    if self.result.is_none()
                        && ui.add_enabled(!busy && !none, egui::Button::new(RichText::new("Make it").color(Color32::WHITE)).fill(colors::ACCENT)).clicked()
                    {
                        self.run(ctx);
                    }
                });
            });
        });
        if close { Outcome::Closed { changed: self.result.is_some() } } else { Outcome::Open }
    }

    fn run(&mut self, ctx: &egui::Context) {
        self.error = None;
        let prog = self.progress.clone();
        let report = move |p: cs_app::batch::Progress| {
            if let Ok(mut g) = prog.lock() {
                *g = p;
            }
        };
        let (tx, rx) = channel();
        let c = ctx.clone();
        let name = self.names[self.tab as usize].clone();
        let job: Box<dyn FnOnce() -> Result<String, String> + Send> = match self.tab {
            Tab::Sheet => {
                let l = self.layout();
                let (media, folder, title, caption, bg, png) =
                    (self.media.clone(), self.folder.clone(), self.title, self.caption, self.bg, self.png);
                Box::new(move || {
                    let pages = media.chunks(l.per_page.max(1)).count();
                    let mut made = Vec::new();
                    for (p, list) in media.chunks(l.per_page.max(1)).enumerate() {
                        let cells: Vec<Cell> = (0..list.len() as u32)
                            .map(|i| Cell {
                                x: l.m + (i % l.cols) * (l.cw + l.g),
                                y: l.m + l.title_h + (i / l.cols) * (l.row_h + l.g),
                                w: l.cw,
                                h: l.bh,
                            })
                            .collect();
                        let overlay = (title || caption != CaptionKind::None)
                            .then(|| overlay(&l, p, pages, list, &folder, title, caption, bg))
                            .transpose()?;
                        let n = if pages > 1 { format!("{name} - page {}", p + 1) } else { name.clone() };
                        let files: Vec<PathBuf> = list.iter().map(|(f, _)| f.clone()).collect();
                        let opts = SheetOpts {
                            width: l.w,
                            height: l.h,
                            background: bg,
                            cells,
                            fit: "contain".into(),
                            overlay,
                            format: if png { "PNG" } else { "JPEG" }.into(),
                            quality: 90,
                            name: n,
                        };
                        made.push(cs_app::create::sheet(&files, opts, &report)?);
                    }
                    Ok(if made.len() == 1 {
                        format!("“{}” made. Ctrl+Z removes it.", made[0].name)
                    } else {
                        format!("{} sheets made. Ctrl+Z removes the last one.", made.len())
                    })
                })
            }
            Tab::Strip => {
                let (files, row, side, gap, bg, png): (Vec<PathBuf>, _, _, _, _, _) = (
                    self.media.iter().map(|(f, _)| f.clone()).collect(),
                    self.row,
                    self.side,
                    self.gap,
                    self.bg,
                    self.png,
                );
                Box::new(move || {
                    let sizes = cs_app::create::sizes(&files);
                    let ok: Vec<(PathBuf, (u32, u32))> =
                        files.iter().zip(sizes).filter_map(|(f, s)| s.map(|s| (f.clone(), s))).collect();
                    if ok.is_empty() {
                        return Err("The sizes of these pictures could not be read.".into());
                    }
                    let mut pos = gap;
                    let mut cells = Vec::new();
                    for (_, (w, h)) in &ok {
                        if row {
                            let cw = ((f64::from(*w) / f64::from(*h)) * f64::from(side)).round().max(1.0) as u32;
                            cells.push(Cell { x: pos, y: gap, w: cw, h: side });
                            pos += cw + gap;
                        } else {
                            let ch = ((f64::from(*h) / f64::from(*w)) * f64::from(side)).round().max(1.0) as u32;
                            cells.push(Cell { x: gap, y: pos, w: side, h: ch });
                            pos += ch + gap;
                        }
                    }
                    let (mut w, mut h) = if row { (pos, side + 2 * gap) } else { (side + 2 * gap, pos) };
                    // At most ~120 megapixels: shrink evenly if needed.
                    let k = (118e6 / (f64::from(w) * f64::from(h))).sqrt().min(1.0);
                    if k < 1.0 {
                        let s = |v: u32| (f64::from(v) * k).floor() as u32;
                        cells = cells
                            .iter()
                            .map(|c| Cell { x: s(c.x), y: s(c.y), w: s(c.w).max(1), h: s(c.h).max(1) })
                            .collect();
                        (w, h) = (s(w), s(h));
                    }
                    let files: Vec<PathBuf> = ok.into_iter().map(|(f, _)| f).collect();
                    let opts = SheetOpts {
                        width: w,
                        height: h,
                        background: bg,
                        cells,
                        fit: "contain".into(),
                        overlay: None,
                        format: if png { "PNG" } else { "JPEG" }.into(),
                        quality: 92,
                        name,
                    };
                    let r = cs_app::create::sheet(&files, opts, &report)?;
                    Ok(format!("“{}” made ({w} × {h} pixels). Ctrl+Z removes it.", r.name))
                })
            }
            Tab::Pdf => {
                let files: Vec<PathBuf> = self.media.iter().map(|(f, _)| f.clone()).collect();
                let opts = PdfOpts {
                    page: if self.pdf_letter { "letter" } else { "a4" }.into(),
                    margin_mm: self.margin,
                    name,
                };
                Box::new(move || {
                    let n = files.len();
                    let r = cs_app::create::pdf(&files, opts, &report)?;
                    Ok(format!("“{}” made, {} page(s). Ctrl+Z removes it.", r.name, n - r.skipped.len()))
                })
            }
            Tab::Zip => {
                let files = self.files.clone();
                Box::new(move || {
                    let r = cs_app::create::zip(&files, &name, &report)?;
                    Ok(format!("“{}” made with {} file(s). Ctrl+Z removes it.", r.name, files.len()))
                })
            }
        };
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))
                .unwrap_or_else(|_| Err("Making it failed unexpectedly.".into()));
            let _ = tx.send(r);
            c.request_repaint();
        });
        self.job = Some(rx);
    }
}

/// The title and captions of one sheet page, as a transparent PNG the sheet's size.
#[allow(clippy::too_many_arguments)]
fn overlay(
    l: &Layout,
    page: usize,
    pages: usize,
    list: &[(PathBuf, i64)],
    folder: &str,
    title: bool,
    caption: CaptionKind,
    bg: [u8; 3],
) -> Result<Vec<u8>, String> {
    let light = bg.iter().map(|&c| u32::from(c)).sum::<u32>() > 382;
    let ink = if light { [29, 31, 35] } else { [242, 243, 245] };
    let mut marks = Vec::new();
    if title {
        let t = if pages > 1 { format!("{folder}  ·  page {} of {pages}", page + 1) } else { folder.to_string() };
        marks.push(raster::Mark::Text { x: l.m as f32, y: l.m as f32, text: t, size: l.font * 1.6, color: ink });
    }
    let mut captions = Vec::new();
    if caption != CaptionKind::None {
        for (i, (p, modified)) in list.iter().enumerate() {
            let (col, row) = (i as u32 % l.cols, i as u32 / l.cols);
            let x = (l.m + col * (l.cw + l.g)) as f32;
            let y = (l.m + l.title_h + row * (l.row_h + l.g) + l.bh) as f32 + l.font * 0.3;
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            captions.push((x, y, name));
            if caption == CaptionKind::NameDate && *modified > 0 {
                let (yy, mo, d, ..) = cs_model::civil::from_unix(*modified);
                captions.push((x, y + l.font * 1.35, format!("{yy}-{mo:02}-{d:02}")));
            }
        }
    }
    raster::sheet_layer(l.w, l.h, &marks, &captions, l.cw as f32, l.font, ink)
}
