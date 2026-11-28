//! Editing in the viewer: a worker thread owns the `cs_app::edit::Session` (decode, edits, preview, save), the
//! UI thread shows its preview and the tool forms in the left fly-out — the same tools as the Tauri app.
//!
//! Slider drags send many requests; the worker only renders the newest colour settings, so it never lags behind.

use crate::{bold, colors, icons, raster};
use cs_app::edit::{Edit, EditState, Retouch, SaveMode, Saved, Session, Watermark};
use egui::{Color32, Pos2, Rect, RichText, Stroke, StrokeKind, Vec2, pos2, vec2};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

#[allow(dead_code)]
pub enum Req {
    Open(PathBuf),
    Apply(Edit),
    StepBack,
    Reset,
    Adjust(Box<cs_adjust::Adjust>),
    Effects(Box<cs_adjust::effects::Effects>),
    Watermark(Watermark),
    Retouch(Retouch),
    Dpi(Option<u32>),
    Auto,
    /// Show the preview without frame / watermark (while cropping or straightening).
    Raw(bool),
    Save {
        mode: SaveMode,
        format: String,
        quality: u8,
    },
    /// Cut the JPEG file itself into a new file (no re-compression), upright pixels.
    LosslessCrop {
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    },
    Close,
}

enum Reply {
    Cropped(cs_app::privacy::Cropped),
    State { state: Box<EditState>, preview: Option<image::RgbImage> },
    Auto(Box<cs_adjust::Adjust>),
    Saved(Saved),
    Error(String),
}

/// Something the viewer should react to.
pub enum Event {
    Cropped(cs_app::privacy::Cropped),
    Saved(Saved),
    Error(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Grab {
    Move,
    New,
    Corner(u8),
}

pub struct Crop {
    pub ratio: usize,
    /// Selection in 0..1 of the picture.
    pub sel: Rect,
    grab: Option<(Grab, Pos2, Rect)>,
}

pub enum Tool {
    None,
    Crop(Crop),
    Resize { w: u32, h: u32, keep: bool },
    Straighten { angle: f32 },
    Canvas { w: u32, h: u32, anchor: u8, color: [u8; 3] },
    Dpi { dpi: u32 },
    Adjust { a: Box<cs_adjust::Adjust> },
    Save { mode: SaveMode, format: String, quality: u8 },
    Retouch(Box<Rt>),
    Effects(Box<Fx>),
}

impl Tool {
    pub fn is_none(&self) -> bool {
        matches!(self, Tool::None)
    }
    /// Tools whose marks sit on the picture itself: show it without frame / watermark.
    fn wants_raw(&self) -> bool {
        matches!(self, Tool::Crop(_) | Tool::Straighten { .. } | Tool::Retouch(_))
    }
}

const RATIOS: [(&str, f32); 8] = [
    ("Free", 0.0),
    ("Original", -1.0),
    ("1 : 1", 1.0),
    ("4 : 3", 4.0 / 3.0),
    ("3 : 2", 1.5),
    ("16 : 9", 16.0 / 9.0),
    ("3 : 4 (portrait)", 0.75),
    ("2 : 3 (portrait)", 2.0 / 3.0),
];

pub struct Editor {
    tx: Sender<(u64, Req)>,
    rx: Receiver<(u64, Reply)>,
    generation: u64,
    pub path: Option<PathBuf>,
    pub state: Option<EditState>,
    pub tex: Option<egui::TextureHandle>,
    pub pending: usize,
    pub tool: Tool,
    controls: Vec<cs_adjust::Control>,
    formats: Vec<&'static str>,
    events: Vec<Event>,
}

impl Editor {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, wrx) = channel::<(u64, Req)>();
        let (wtx, rx) = channel::<(u64, Reply)>();
        std::thread::Builder::new()
            .name("editor".into())
            .spawn(move || worker(&wrx, &wtx, &ctx))
            .expect("start the editor thread");
        Self {
            tx,
            rx,
            generation: 0,
            path: None,
            state: None,
            tex: None,
            pending: 0,
            tool: Tool::None,
            controls: cs_adjust::controls(),
            formats: cs_app::edit::formats(),
            events: Vec::new(),
        }
    }

    fn send(&mut self, r: Req) {
        self.pending += 1;
        let _ = self.tx.send((self.generation, r));
    }

    /// Edit this photo (opens it first when needed), then send `r`.
    pub fn request(&mut self, path: &Path, r: Req) {
        if self.path.as_deref() != Some(path) {
            self.close();
            self.path = Some(path.to_path_buf());
            self.send(Req::Open(path.to_path_buf()));
        }
        self.send(r);
    }

    pub fn open(&mut self, path: &Path) {
        if self.path.as_deref() != Some(path) {
            self.close();
            self.path = Some(path.to_path_buf());
            self.send(Req::Open(path.to_path_buf()));
        }
    }

    pub fn close(&mut self) {
        if self.path.is_some() {
            let _ = self.tx.send((self.generation, Req::Close));
        }
        self.generation += 1;
        self.path = None;
        self.state = None;
        self.tex = None;
        self.pending = 0;
        self.tool = Tool::None;
    }

    /// Editing this photo right now?
    pub fn on(&self, path: &Path) -> bool {
        self.path.as_deref() == Some(path)
    }

    pub fn dirty(&self) -> bool {
        self.state.as_ref().is_some_and(|s| s.edits > 0 || s.adjusted || s.has_effects || s.dpi.is_some())
    }

    /// How many changes the photo has: the steps, plus colour settings, frame / watermark and print size.
    pub fn changes(&self) -> usize {
        self.state.as_ref().map_or(0, |s| {
            s.edits + usize::from(s.adjusted) + usize::from(s.has_effects) + usize::from(s.dpi.is_some())
        })
    }

    pub fn busy(&self) -> bool {
        self.pending > 0
    }

    pub fn tool_name(&self) -> &'static str {
        match self.tool {
            Tool::None => "none",
            Tool::Crop(_) => "crop",
            Tool::Resize { .. } => "resize",
            Tool::Straighten { .. } => "straighten",
            Tool::Canvas { .. } => "canvas",
            Tool::Dpi { .. } => "dpi",
            Tool::Adjust { .. } => "adjust",
            Tool::Save { .. } => "save",
            Tool::Retouch(_) => "retouch",
            Tool::Effects(_) => "effects",
        }
    }

    /// Take the worker's answers; returns what the viewer must react to.
    pub fn poll(&mut self, ctx: &egui::Context) -> Vec<Event> {
        while let Ok((g, reply)) = self.rx.try_recv() {
            if g != self.generation {
                continue;
            }
            self.pending = self.pending.saturating_sub(1);
            match reply {
                Reply::State { state, preview } => {
                    if let Some(p) = preview {
                        let (w, h) = p.dimensions();
                        let img = egui::ColorImage::from_rgb([w as usize, h as usize], p.as_raw());
                        self.tex = Some(ctx.load_texture("edit-preview", img, egui::TextureOptions::LINEAR));
                    }
                    self.state = Some(*state);
                }
                Reply::Auto(a) => {
                    if let Tool::Adjust { a: cur } = &mut self.tool {
                        **cur = (*a).clone();
                    }
                    self.send(Req::Adjust(a));
                }
                Reply::Saved(s) => self.events.push(Event::Saved(s)),
                Reply::Cropped(c) => self.events.push(Event::Cropped(c)),
                Reply::Error(e) => {
                    self.events.push(Event::Error(e));
                    // A failed open leaves nothing to edit.
                    if self.state.is_none() {
                        self.close();
                    }
                }
            }
        }
        std::mem::take(&mut self.events)
    }

    // ------------------------------------------------------------------ starting the tools

    pub fn start(&mut self, path: &Path, tool: &str) {
        self.open(path);
        let st = self.state.clone();
        let (w, h) = st.as_ref().map_or((0, 0), |s| (s.width, s.height));
        self.tool = match tool {
            "crop" => {
                Tool::Crop(Crop { ratio: 0, sel: Rect::from_min_max(pos2(0.1, 0.1), pos2(0.9, 0.9)), grab: None })
            }
            "resize" => Tool::Resize { w, h, keep: true },
            "straighten" => Tool::Straighten { angle: 0.0 },
            "canvas" => Tool::Canvas { w, h, anchor: 4, color: [255, 255, 255] },
            "dpi" => Tool::Dpi {
                dpi: st.as_ref().and_then(|s| s.dpi.or(s.dpi_original.map(|d| d.round() as u32))).unwrap_or(300),
            },
            "adjust" => Tool::Adjust { a: Box::new(st.as_ref().map(|s| s.adjust.clone()).unwrap_or_default()) },
            "save" => Tool::Save {
                mode: SaveMode::Copy,
                format: if st.as_ref().is_some_and(|s| s.format == "Jpeg") || st.is_none() {
                    "JPEG".into()
                } else {
                    "PNG".into()
                },
                quality: 92,
            },
            "retouch" => Tool::Retouch(Box::new(Rt {
                mode: RtMode::Heal,
                size: 18.0,
                hardness: 50.0,
                pupil: 50.0,
                darken: 50.0,
                src: None,
                offset: None,
                pick_source: false,
                shape: raster::Shape::Pen,
                color: [255, 255, 255],
                width: 6.0,
                font: 48.0,
                text: String::new(),
                marks: Vec::new(),
                stroke: None,
                hint: String::new(),
            })),
            "effects" => {
                let e = st.as_ref().map(|s| s.effects).unwrap_or_default();
                Tool::Effects(Box::new(Fx {
                    effects: e,
                    use_logo: false,
                    logo_shown: false,
                    logo: None,
                    text: String::new(),
                    bold: true,
                    text_color: [255, 255, 255],
                    outline: true,
                    sent_layer: format!("{:?}", st.as_ref().and_then(|s| s.watermark.clone())),
                }))
            }
            _ => Tool::None,
        };
        let raw = self.tool.wants_raw();
        self.send(Req::Raw(raw));
    }

    /// The tool's size fields start at 0 while the photo is still opening: fill them in once known.
    fn fill_sizes(&mut self) {
        let Some(s) = &self.state else { return };
        match &mut self.tool {
            Tool::Resize { w, h, .. } | Tool::Canvas { w, h, .. } if *w == 0 => {
                *w = s.width;
                *h = s.height;
            }
            _ => {}
        }
    }

    pub fn cancel_tool(&mut self) {
        let raw = self.tool.wants_raw();
        if let Tool::Adjust { .. } = self.tool {
            // Leave the colour settings as they are (they are part of the edit, like in the old app).
        }
        self.tool = Tool::None;
        if raw {
            self.send(Req::Raw(false));
        }
    }

    /// Enter in the viewer: apply the open tool. `true` = it took the key.
    pub fn enter(&mut self) -> bool {
        match &self.tool {
            Tool::Crop(_) => {
                self.apply_crop();
                true
            }
            Tool::Straighten { .. } => {
                self.apply_straighten();
                true
            }
            _ => false,
        }
    }

    /// The crop box in picture pixels.
    fn crop_rect(&self) -> Option<(u32, u32, u32, u32)> {
        let (Tool::Crop(c), Some(s)) = (&self.tool, &self.state) else { return None };
        let (w, h) = (s.width as f32, s.height as f32);
        let x = (c.sel.min.x * w).round().clamp(0.0, w - 1.0) as u32;
        let y = (c.sel.min.y * h).round().clamp(0.0, h - 1.0) as u32;
        let cw = ((c.sel.width() * w).round() as u32).clamp(1, s.width - x);
        let ch = ((c.sel.height() * h).round() as u32).clamp(1, s.height - y);
        Some((x, y, cw, ch))
    }

    fn apply_crop(&mut self) {
        if let Some((x, y, w, h)) = self.crop_rect() {
            self.send(Req::Apply(Edit::Crop { x, y, w, h }));
        }
        self.cancel_tool();
    }

    /// Only for a JPEG with no other changes: the file is cut without re-compressing it.
    fn can_crop_losslessly(&self) -> bool {
        self.state.as_ref().is_some_and(|s| s.format == "Jpeg" && s.edits == 0 && !s.adjusted && !s.has_effects)
    }

    fn lossless_crop(&mut self) {
        if let Some((x, y, w, h)) = self.crop_rect() {
            self.send(Req::LosslessCrop { x, y, w, h });
        }
        self.cancel_tool();
    }

    fn apply_straighten(&mut self) {
        let Tool::Straighten { angle } = self.tool else { return };
        let a100 = (angle * 100.0).round() as i32;
        if a100 != 0 {
            self.send(Req::Apply(Edit::Straighten { angle100: a100 }));
        }
        self.cancel_tool();
    }

    pub fn step_back(&mut self) {
        if self.state.as_ref().is_some_and(|s| s.edits > 0) {
            self.send(Req::StepBack);
        }
    }

    pub fn reset(&mut self) {
        self.send(Req::Reset);
        self.tool = Tool::None;
    }

    pub fn save(&mut self, mode: SaveMode, format: &str, quality: u8) {
        self.send(Req::Save { mode, format: format.into(), quality });
    }

    // ------------------------------------------------------------------ on the picture

    /// The straighten angle to show the picture turned by (degrees), while that tool is open.
    pub fn preview_angle(&self) -> f32 {
        if let Tool::Straighten { angle } = self.tool { angle } else { 0.0 }
    }

    /// Crop box on the picture (`pic` = where the picture is drawn). Returns true while it uses the mouse.
    pub fn crop_overlay(&mut self, ui: &egui::Ui, pic: Rect, resp: &egui::Response) -> bool {
        let ratio = self.ratio();
        let Tool::Crop(c) = &mut self.tool else { return false };
        let to_screen =
            |r: Rect| Rect::from_min_max(pic.lerp_inside(r.min.to_vec2()), pic.lerp_inside(r.max.to_vec2()));
        let to_norm = |p: Pos2| {
            pos2(((p.x - pic.min.x) / pic.width()).clamp(0.0, 1.0), ((p.y - pic.min.y) / pic.height()).clamp(0.0, 1.0))
        };
        let pointer = ui.ctx().input(|i| i.pointer.interact_pos());
        let corners = |r: Rect| [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()];
        if resp.drag_started()
            && let Some(p) = pointer
        {
            let box_s = to_screen(c.sel);
            let grab = if let Some(i) = corners(box_s).iter().position(|q| q.distance(p) < 16.0) {
                Grab::Corner(i as u8)
            } else if box_s.contains(p) {
                Grab::Move
            } else {
                Grab::New
            };
            c.grab = Some((grab, to_norm(p), c.sel));
        }
        if resp.dragged()
            && let (Some((grab, start, sel0)), Some(p)) = (c.grab, pointer)
        {
            let now = to_norm(p);
            let pic_ratio = pic.width() / pic.height();
            c.sel = match grab {
                Grab::Move => {
                    let d = now - start;
                    let d = vec2(d.x.clamp(-sel0.min.x, 1.0 - sel0.max.x), d.y.clamp(-sel0.min.y, 1.0 - sel0.max.y));
                    sel0.translate(d)
                }
                Grab::New => shaped(start, now, ratio, pic_ratio),
                Grab::Corner(i) => {
                    let fixed = corners(sel0)[3 - i as usize];
                    shaped(fixed, now, ratio, pic_ratio)
                }
            };
        }
        if resp.drag_stopped() {
            c.grab = None;
        }
        // Draw: dim outside, frame, thirds, corner handles.
        let s = to_screen(c.sel);
        let dim = Color32::from_black_alpha(140);
        let p = ui.painter();
        p.rect_filled(Rect::from_min_max(pic.min, pos2(pic.max.x, s.min.y)), 0.0, dim);
        p.rect_filled(Rect::from_min_max(pos2(pic.min.x, s.max.y), pic.max), 0.0, dim);
        p.rect_filled(Rect::from_min_max(pos2(pic.min.x, s.min.y), pos2(s.min.x, s.max.y)), 0.0, dim);
        p.rect_filled(Rect::from_min_max(pos2(s.max.x, s.min.y), pos2(pic.max.x, s.max.y)), 0.0, dim);
        p.rect_stroke(s, 0.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Middle);
        for t in [1.0 / 3.0, 2.0 / 3.0] {
            let x = s.min.x + s.width() * t;
            let y = s.min.y + s.height() * t;
            p.line_segment([pos2(x, s.min.y), pos2(x, s.max.y)], Stroke::new(1.0, Color32::from_white_alpha(110)));
            p.line_segment([pos2(s.min.x, y), pos2(s.max.x, y)], Stroke::new(1.0, Color32::from_white_alpha(110)));
        }
        for q in corners(s) {
            p.rect_filled(Rect::from_center_size(q, vec2(12.0, 12.0)), 2.0, Color32::WHITE);
        }
        if let Some(st) = &self.state {
            let (w, h) = ((c.sel.width() * st.width as f32).round(), (c.sel.height() * st.height as f32).round());
            p.text(
                s.center_bottom() + vec2(0.0, 14.0),
                egui::Align2::CENTER_CENTER,
                format!("{w} × {h}"),
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            );
        }
        true
    }

    /// The chosen crop shape as width / height of the picture's pixels (0 = free).
    fn ratio(&self) -> f32 {
        let Tool::Crop(c) = &self.tool else { return 0.0 };
        match RATIOS[c.ratio].1 {
            r if r < 0.0 => self.state.as_ref().map_or(0.0, |s| s.width as f32 / s.height as f32),
            r => r,
        }
    }

    /// Apply the crop shape to the current box (keeps its centre).
    fn reshape_crop(&mut self) {
        let ratio = self.ratio();
        let pic_ratio = self.state.as_ref().map_or(1.0, |s| s.width as f32 / s.height as f32);
        if let Tool::Crop(c) = &mut self.tool
            && ratio > 0.0
        {
            // In 0..1 units the box's shape is ratio / pic_ratio.
            let want = ratio / pic_ratio;
            let (mut w, mut h) = (c.sel.width(), c.sel.height());
            if w / h > want {
                w = h * want
            } else {
                h = w / want
            }
            let s = (1.0f32).min(1.0 / w).min(1.0 / h);
            let (w, h) = (w * s.min(1.0), h * s.min(1.0));
            let cen = c.sel.center();
            let cen = pos2(cen.x.clamp(w / 2.0, 1.0 - w / 2.0), cen.y.clamp(h / 2.0, 1.0 - h / 2.0));
            c.sel = Rect::from_center_size(cen, vec2(w, h));
        }
    }

    // ------------------------------------------------------------------ the forms in the left fly-out

    /// The open tool's form. `false` = closed.
    pub fn tool_panel(&mut self, ui: &mut egui::Ui) {
        self.fill_sizes();
        let st = self.state.clone();
        let head = |ui: &mut egui::Ui, t: &str| {
            ui.label(RichText::new(t).font(bold(17.0)).color(Color32::WHITE));
            ui.add_space(4.0);
        };
        let hint = |ui: &mut egui::Ui, t: &str| {
            ui.label(RichText::new(t).size(13.0).color(colors::MUTED));
        };
        let lossless_ok = self.can_crop_losslessly();
        let mut action: Option<Act> = None;
        egui::ScrollArea::vertical().id_salt("tool").show(ui, |ui| {
            ui.spacing_mut().slider_width = 170.0;
            match &mut self.tool {
                Tool::None => {}
                Tool::Crop(c) => {
                    head(ui, "Crop");
                    let mut r = c.ratio;
                    egui::ComboBox::from_label("Shape").selected_text(RATIOS[r].0).show_ui(ui, |ui| {
                        for (i, (t, _)) in RATIOS.iter().enumerate() {
                            ui.selectable_value(&mut r, i, *t);
                        }
                    });
                    if r != c.ratio {
                        c.ratio = r;
                        action = Some(Act::Reshape);
                    }
                    hint(ui, "Drag the box or its corners; drag outside it to draw a new one.");
                    if let Some(s) = &st {
                        hint(ui, &format!("{} × {} pixels", (c.sel.width() * s.width as f32).round(), (c.sel.height() * s.height as f32).round()));
                    }
                    buttons(ui, "Crop  (Enter)", &mut action, Act::Crop);
                    if lossless_ok {
                        ui.add_space(6.0);
                        if ui.button("Lossless crop to a new file").on_hover_text("Cuts the JPEG without re-compressing it: no quality is lost. The edges snap to JPEG's 8/16-pixel grid.").clicked() {
                            action = Some(Act::Lossless);
                        }
                        hint(ui, "Keeps the photo's full quality; the original stays as it is.");
                    }
                }
                Tool::Resize { w, h, keep } => {
                    head(ui, "Resize");
                    if let Some(s) = &st {
                        hint(ui, &format!("Now {} × {} pixels", s.width, s.height));
                        let ratio = s.width as f64 / s.height.max(1) as f64;
                        let (ow, oh) = (*w, *h);
                        ui.horizontal(|ui| {
                            ui.label("Width");
                            ui.add(egui::DragValue::new(w).range(1..=60000));
                            ui.label("Height");
                            ui.add(egui::DragValue::new(h).range(1..=60000));
                        });
                        ui.checkbox(keep, "Keep the shape (aspect ratio)");
                        if *keep && *w != ow {
                            *h = ((*w as f64 / ratio).round() as u32).max(1);
                        } else if *keep && *h != oh {
                            *w = ((*h as f64 * ratio).round() as u32).max(1);
                        }
                        ui.horizontal(|ui| {
                            for p in [25u32, 50, 75] {
                                if ui.button(format!("{p}%")).clicked() {
                                    *w = (s.width * p / 100).max(1);
                                    *h = (s.height * p / 100).max(1);
                                }
                            }
                        });
                    }
                    buttons(ui, "Resize", &mut action, Act::Resize);
                }
                Tool::Straighten { angle } => {
                    head(ui, "Straighten");
                    hint(ui, "Turn the picture until it is level; the corners are cropped so nothing is left blank.");
                    ui.add(egui::Slider::new(angle, -45.0..=45.0).step_by(0.1).suffix("°").text("Angle"));
                    ui.horizontal(|ui| {
                        for d in [-1.0f32, -0.1, 0.1, 1.0] {
                            if ui.button(format!("{}{}°", if d > 0.0 { "+" } else { "−" }, d.abs())).clicked() {
                                *angle = (*angle + d).clamp(-45.0, 45.0);
                            }
                        }
                        if ui.button("0°").clicked() {
                            *angle = 0.0;
                        }
                    });
                    buttons(ui, "Straighten  (Enter)", &mut action, Act::Straighten);
                }
                Tool::Canvas { w, h, anchor, color } => {
                    head(ui, "Canvas size");
                    if let Some(s) = &st {
                        hint(ui, &format!("Now {} × {}. A bigger canvas adds a border; a smaller one trims the picture.", s.width, s.height));
                        ui.horizontal(|ui| {
                            for p in [2u32, 5, 10] {
                                if ui.button(format!("Border {p}%")).clicked() {
                                    let b = (s.width.max(s.height) * p / 100).max(1);
                                    *w = s.width + 2 * b;
                                    *h = s.height + 2 * b;
                                }
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label("Width");
                        ui.add(egui::DragValue::new(w).range(1..=60000));
                        ui.label("Height");
                        ui.add(egui::DragValue::new(h).range(1..=60000));
                    });
                    hint(ui, "Picture position");
                    egui::Grid::new("anchor").spacing([4.0, 4.0]).show(ui, |ui| {
                        for i in 0..9u8 {
                            if ui.selectable_label(*anchor == i, if *anchor == i { "●" } else { "○" }).clicked() {
                                *anchor = i;
                            }
                            if i % 3 == 2 {
                                ui.end_row();
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Border colour");
                        for c in [[255u8, 255, 255], [0, 0, 0], [128, 128, 128]] {
                            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
                            ui.painter().rect_filled(r, 3.0, Color32::from_rgb(c[0], c[1], c[2]));
                            if *color == c {
                                ui.painter().rect_stroke(r, 3.0, Stroke::new(2.0, colors::ACCENT), StrokeKind::Outside);
                            }
                            if resp.clicked() {
                                *color = c;
                            }
                        }
                        ui.color_edit_button_srgb(color);
                    });
                    buttons(ui, "Apply", &mut action, Act::Canvas);
                }
                Tool::Dpi { dpi } => {
                    head(ui, "Print size");
                    if let Some(s) = &st {
                        hint(ui, &match s.dpi_original {
                            Some(d) => format!("The file says {} dots per inch.", d.round()),
                            None => "The file does not say how big to print.".into(),
                        });
                        hint(ui, "This only changes how big the picture prints; the pixels stay the same.");
                        ui.horizontal(|ui| {
                            ui.label("Dots per inch");
                            ui.add(egui::DragValue::new(dpi).range(1..=10000));
                        });
                        ui.horizontal(|ui| {
                            for v in [72u32, 150, 300, 600] {
                                ui.selectable_value(dpi, v, v.to_string());
                            }
                        });
                        let cm = |px: u32| px as f32 / *dpi as f32 * 2.54;
                        hint(ui, &format!("Prints at {:.1} × {:.1} cm", cm(s.width), cm(s.height)));
                    }
                    buttons(ui, "Apply", &mut action, Act::Dpi);
                }
                Tool::Adjust { a } => {
                    head(ui, "Colour and light");
                    ui.horizontal(|ui| {
                        if ui.button(format!("{}  Auto", icons::WAND)).on_hover_text("Suggested settings for this photo").clicked() {
                            action = Some(Act::Auto);
                        }
                        if ui.button("Reset all").clicked() {
                            **a = cs_adjust::Adjust::default();
                            action = Some(Act::AdjustNow);
                        }
                    });
                    let before = serde_json::to_value(&**a).unwrap_or_default();
                    let mut v = before.clone();
                    let mut section = "";
                    for c in &self.controls {
                        if c.section != section {
                            section = c.section;
                            ui.add_space(6.0);
                            ui.label(RichText::new(section.to_uppercase()).font(bold(12.0)).color(colors::MUTED));
                        }
                        let mut x = v.get(c.key).and_then(|x| x.as_f64()).unwrap_or(c.default);
                        let r = ui.add(egui::Slider::new(&mut x, c.min..=c.max).step_by(c.step).text(c.label));
                        if r.double_clicked() {
                            x = c.default;
                        }
                        if let Some(m) = v.as_object_mut() {
                            m.insert(c.key.into(), serde_json::json!(x));
                        }
                    }
                    if v != before
                        && let Ok(n) = serde_json::from_value::<cs_adjust::Adjust>(v)
                    {
                        **a = n;
                        action = Some(Act::AdjustNow);
                    }
                    ui.add_space(8.0);
                    hint(ui, "Double-click a slider to reset it. The changes are saved with “Save as…”.");
                    if ui.button("Done").clicked() {
                        action = Some(Act::Cancel);
                    }
                }
                Tool::Save { mode, format, quality } => {
                    head(ui, "Save the edited picture");
                    ui.radio_value(mode, SaveMode::Copy, "As a new copy next to the original (recommended)");
                    ui.radio_value(mode, SaveMode::Replace, "Replace the original (it is kept in a hidden backup folder, so Undo can bring it back)");
                    ui.add_space(4.0);
                    egui::ComboBox::from_label("Format").selected_text(format.as_str()).show_ui(ui, |ui| {
                        for f in &self.formats {
                            ui.selectable_value(format, (*f).to_string(), *f);
                        }
                    });
                    let lossless = st.as_ref().is_some_and(|s| s.lossless) && format == "JPEG";
                    if (format == "JPEG" || format == "WebP") && !lossless {
                        ui.add(egui::Slider::new(quality, 50..=100).text("Quality"));
                    }
                    if lossless {
                        ui.label(RichText::new("Only turned, flipped or given a print size: saved without any quality loss.").color(colors::GREEN));
                    }
                    buttons(ui, "Save", &mut action, Act::Save);
                }
                Tool::Retouch(rt) => {
                    head(ui, "Retouch");
                    ui.horizontal_wrapped(|ui| {
                        for (m, t) in [(RtMode::RedEye, "Red eye"), (RtMode::Heal, "Heal"), (RtMode::Clone, "Clone"), (RtMode::Draw, "Draw / text")] {
                            if ui.selectable_label(rt.mode == m, t).clicked() {
                                rt.mode = m;
                                rt.hint.clear();
                            }
                        }
                    });
                    ui.add_space(4.0);
                    match rt.mode {
                        RtMode::RedEye => {
                            hint(ui, "Drag a box around each red eye. Only the red in the pupil changes.");
                            ui.add(egui::Slider::new(&mut rt.pupil, 0.0..=100.0).text("Pupil size"));
                            ui.add(egui::Slider::new(&mut rt.darken, 0.0..=100.0).text("Darken"));
                        }
                        RtMode::Heal => {
                            hint(ui, "Paint over a spot, scratch or speck of dust. When you let go, it is filled from the area around it.");
                            ui.add(egui::Slider::new(&mut rt.size, 3.0..=120.0).text("Brush size"));
                        }
                        RtMode::Clone => {
                            hint(ui, if rt.src.is_some() {
                                "Paint to copy from the blue circle. Alt+click (or the button) picks another source."
                            } else {
                                "First click the place to copy FROM (or Alt+click), then paint where it should go."
                            });
                            if ui.selectable_label(rt.pick_source, "Set the source…").clicked() {
                                rt.pick_source = true;
                            }
                            ui.add(egui::Slider::new(&mut rt.size, 3.0..=120.0).text("Brush size"));
                            ui.add(egui::Slider::new(&mut rt.hardness, 0.0..=100.0).text("Hardness"));
                        }
                        RtMode::Draw => {
                            ui.horizontal_wrapped(|ui| {
                                for (s, t) in raster::Shape::ALL {
                                    ui.selectable_value(&mut rt.shape, s, t);
                                }
                            });
                            ui.horizontal(|ui| {
                                for c in [[255u8, 255, 255], [0, 0, 0], [229, 57, 53], [253, 216, 53], [67, 160, 71], [30, 136, 229]] {
                                    swatch(ui, &mut rt.color, c);
                                }
                                ui.color_edit_button_srgb(&mut rt.color);
                            });
                            if rt.shape == raster::Shape::Text {
                                ui.label("Text");
                                ui.add(egui::TextEdit::multiline(&mut rt.text).desired_rows(2).hint_text("e.g. Summer 1987"));
                                ui.add(egui::Slider::new(&mut rt.font, 10.0..=200.0).text("Size"));
                                hint(ui, "Then click where the text should start.");
                            } else {
                                ui.add(egui::Slider::new(&mut rt.width, 1.0..=40.0).text("Line width"));
                            }
                            ui.horizontal(|ui| {
                                if ui.add_enabled(!rt.marks.is_empty(), egui::Button::new("Clear")).clicked() {
                                    rt.marks.clear();
                                }
                                let put = egui::Button::new(RichText::new("Put on the picture").color(Color32::WHITE)).fill(colors::ACCENT);
                                if ui.add_enabled(!rt.marks.is_empty(), put).clicked() {
                                    action = Some(Act::Draw);
                                }
                            });
                        }
                    }
                    if !rt.hint.is_empty() {
                        ui.label(RichText::new(&rt.hint).color(colors::RED));
                    }
                    hint(ui, "Each change can be taken back with Ctrl+Z.");
                    if ui.add(egui::Button::new(RichText::new("Done").color(Color32::WHITE)).fill(colors::ACCENT)).clicked() {
                        action = Some(if rt.marks.is_empty() { Act::Cancel } else { Act::DrawDone });
                    }
                }
                Tool::Effects(fx) => {
                    head(ui, "Frame and watermark");
                    use cs_adjust::effects::Frame;
                    let before = (fx.effects, fx.text.clone(), fx.bold, fx.text_color, fx.outline, fx.logo.clone());
                    ui.label(RichText::new("FRAME").font(bold(12.0)).color(colors::MUTED));
                    egui::ComboBox::from_label("Frame").selected_text(frame_name(fx.effects.frame)).show_ui(ui, |ui| {
                        for f in [Frame::None, Frame::Border, Frame::Matte, Frame::Instant, Frame::Rounded, Frame::Shadow, Frame::Vignette] {
                            ui.selectable_value(&mut fx.effects.frame, f, frame_name(f));
                        }
                    });
                    if fx.effects.frame != Frame::None {
                        let label = if fx.effects.frame == Frame::Vignette { "Strength" } else { "Size %" };
                        ui.add(egui::Slider::new(&mut fx.effects.size, 0.5..=30.0).text(label));
                        ui.horizontal(|ui| {
                            ui.label("Colour");
                            for c in [[255u8, 255, 255], [0, 0, 0], [240, 234, 220], [128, 128, 128]] {
                                swatch(ui, &mut fx.effects.color, c);
                            }
                            ui.color_edit_button_srgb(&mut fx.effects.color);
                        });
                        if matches!(fx.effects.frame, Frame::Matte | Frame::Shadow) {
                            ui.horizontal(|ui| {
                                ui.label(if fx.effects.frame == Frame::Matte { "Line colour" } else { "Shadow colour" });
                                ui.color_edit_button_srgb(&mut fx.effects.accent);
                            });
                        }
                    }
                    ui.add_space(8.0);
                    ui.label(RichText::new("WATERMARK").font(bold(12.0)).color(colors::MUTED));
                    ui.checkbox(&mut fx.effects.watermark.on, "Put a watermark on the photo");
                    if fx.effects.watermark.on {
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut fx.use_logo, false, "Text");
                            ui.radio_value(&mut fx.use_logo, true, "Logo picture");
                        });
                        if fx.use_logo {
                            ui.horizontal(|ui| {
                                let name = fx.logo.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
                                ui.label(name.unwrap_or_else(|| "No logo chosen".into()));
                                if ui.button("Choose…").clicked()
                                    && let Some(p) = rfd::FileDialog::new()
                                        .set_title("Choose a logo picture")
                                        .add_filter("Pictures", &["png", "jpg", "jpeg", "webp", "tif", "tiff", "bmp", "gif"])
                                        .pick_file()
                                {
                                    fx.logo = Some(p);
                                }
                            });
                        } else {
                            ui.add(egui::TextEdit::multiline(&mut fx.text).desired_rows(2).hint_text("e.g. © Smith family 2024"));
                            ui.horizontal(|ui| {
                                for c in [[255u8, 255, 255], [0, 0, 0], [253, 216, 53]] {
                                    swatch(ui, &mut fx.text_color, c);
                                }
                                ui.color_edit_button_srgb(&mut fx.text_color);
                                ui.checkbox(&mut fx.bold, "Bold");
                                ui.checkbox(&mut fx.outline, "Halo");
                            });
                        }
                        let wm = &mut fx.effects.watermark;
                        hint(ui, "Position");
                        egui::Grid::new("wm-anchor").spacing([4.0, 4.0]).show(ui, |ui| {
                            for i in 0..9u8 {
                                if ui.selectable_label(wm.anchor == i, if wm.anchor == i { "●" } else { "○" }).clicked() {
                                    wm.anchor = i;
                                }
                                if i % 3 == 2 {
                                    ui.end_row();
                                }
                            }
                        });
                        ui.add(egui::Slider::new(&mut wm.width, 2.0..=90.0).text("Width %"));
                        ui.add(egui::Slider::new(&mut wm.margin, 0.0..=20.0).text("Margin %"));
                        ui.add(egui::Slider::new(&mut wm.opacity, 0.0..=100.0).text("Opacity %"));
                    }
                    let after = (fx.effects, fx.text.clone(), fx.bold, fx.text_color, fx.outline, fx.logo.clone());
                    if after != before || fx.logo_shown != fx.use_logo {
                        fx.logo_shown = fx.use_logo;
                        action = Some(Act::EffectsNow);
                    }
                    ui.add_space(8.0);
                    hint(ui, "The frame and watermark are put on when you save; the original stays unchanged.");
                    ui.horizontal(|ui| {
                        if ui.button("Remove both").clicked() {
                            fx.effects = Default::default();
                            fx.text.clear();
                            action = Some(Act::EffectsNow);
                        }
                        if ui.add(egui::Button::new(RichText::new("Done").color(Color32::WHITE)).fill(colors::ACCENT)).clicked() {
                            action = Some(Act::Cancel);
                        }
                    });
                }
            }
        });
        match action {
            None => {}
            Some(Act::Cancel) => self.cancel_tool(),
            Some(Act::Reshape) => self.reshape_crop(),
            Some(Act::Crop) => self.apply_crop(),
            Some(Act::Lossless) => self.lossless_crop(),
            Some(Act::Straighten) => self.apply_straighten(),
            Some(Act::Resize) => {
                if let Tool::Resize { w, h, .. } = self.tool {
                    self.send(Req::Apply(Edit::Resize { w, h }));
                }
                self.cancel_tool();
            }
            Some(Act::Canvas) => {
                if let Tool::Canvas { w, h, anchor, color } = self.tool {
                    self.send(Req::Apply(Edit::Canvas { w, h, anchor, color }));
                }
                self.cancel_tool();
            }
            Some(Act::Dpi) => {
                if let Tool::Dpi { dpi } = self.tool {
                    self.send(Req::Dpi(Some(dpi)));
                }
                self.cancel_tool();
            }
            Some(Act::Auto) => self.send(Req::Auto),
            Some(Act::AdjustNow) => {
                if let Tool::Adjust { a } = &self.tool {
                    let a = a.clone();
                    self.send(Req::Adjust(a));
                }
            }
            Some(Act::Save) => {
                if let Tool::Save { mode, format, quality } = &self.tool {
                    let (m, f, q) = (*mode, format.clone(), *quality);
                    self.save(m, &f, q);
                }
                self.tool = Tool::None;
            }
            Some(a @ (Act::Draw | Act::DrawDone)) => {
                self.put_drawing();
                if matches!(a, Act::DrawDone) {
                    self.cancel_tool();
                }
            }
            Some(Act::EffectsNow) => self.send_effects(),
        }
    }

    /// "Put on the picture": the drawn marks become one retouch step.
    fn put_drawing(&mut self) {
        let (Tool::Retouch(rt), Some(s)) = (&mut self.tool, &self.state) else { return };
        if rt.marks.is_empty() {
            return;
        }
        match raster::layer_png(s.width, s.height, &rt.marks) {
            Ok(png) => {
                rt.marks.clear();
                self.send(Req::Retouch(Retouch::Draw { png }));
            }
            Err(e) => rt.hint = e,
        }
    }

    /// Frame / watermark changed: send the settings and (when it changed) the watermark picture.
    fn send_effects(&mut self) {
        let Tool::Effects(fx) = &mut self.tool else { return };
        let mut fx_settings = fx.effects;
        let wm = if !fx.effects.watermark.on {
            Watermark::None
        } else if fx.use_logo {
            fx.logo.clone().map_or(Watermark::None, Watermark::Logo)
        } else {
            raster::text_watermark_png(&fx.text, fx.bold, fx.text_color, fx.outline)
                .map_or(Watermark::None, Watermark::Text)
        };
        let key = format!("{:?}|{}|{}|{:?}|{}|{:?}", fx.use_logo, fx.text, fx.bold, fx.text_color, fx.outline, fx.logo);
        let layer_changed = key != fx.sent_layer;
        fx.sent_layer = key;
        if matches!(wm, Watermark::None) {
            fx_settings.watermark.on = false;
        }
        if layer_changed {
            self.send(Req::Watermark(wm));
        }
        self.send(Req::Effects(Box::new(fx_settings)));
    }

    /// Retouch on the picture (`pic` = where it is drawn): boxes, brush strokes, drawing. True while active.
    pub fn retouch_overlay(&mut self, ui: &egui::Ui, pic: Rect, resp: &egui::Response) {
        let Some((w, h)) = self.state.as_ref().map(|s| (s.width as f32, s.height as f32)) else { return };
        let Tool::Retouch(rt) = &mut self.tool else { return };
        let k = pic.width() / w; // screen points per picture pixel
        let to_pic = |p: Pos2| (((p.x - pic.min.x) / k).clamp(0.0, w), ((p.y - pic.min.y) / k).clamp(0.0, h));
        let to_scr = |(x, y): (f32, f32)| pos2(pic.min.x + x * k, pic.min.y + y * k);
        let (pointer, alt) = ui.ctx().input(|i| (i.pointer.interact_pos().or(i.pointer.hover_pos()), i.modifiers.alt));
        let mut out: Option<Retouch> = None;
        let brush = rt.size / 2.0;

        if (resp.drag_started() || (resp.clicked() && !resp.dragged()))
            && let Some(p) = pointer.filter(|p| pic.contains(*p))
        {
            {
                let q = to_pic(p);
                if rt.mode == RtMode::Clone && (alt || rt.pick_source || rt.src.is_none()) {
                    rt.src = Some(q);
                    rt.offset = None;
                    rt.pick_source = false;
                } else if rt.mode == RtMode::Draw && rt.shape == raster::Shape::Text {
                    if rt.text.trim().is_empty() {
                        rt.hint = "Type the text in the panel first, then click where it should go.".into();
                    } else if resp.clicked() || resp.drag_started() {
                        rt.marks.push(raster::Mark::Text {
                            x: q.0,
                            y: q.1,
                            text: rt.text.clone(),
                            size: rt.font,
                            color: rt.color,
                        });
                    }
                } else {
                    rt.stroke = Some(vec![q]);
                }
            }
        }
        if resp.dragged()
            && let (Some(st), Some(p)) = (rt.stroke.as_mut(), pointer)
        {
            let q = to_pic(p);
            match rt.mode {
                RtMode::Heal | RtMode::Clone => {
                    let last = *st.last().unwrap_or(&q);
                    if ((q.0 - last.0).powi(2) + (q.1 - last.1).powi(2)).sqrt() > brush / 3.0 {
                        st.push(q);
                    }
                }
                RtMode::Draw if rt.shape == raster::Shape::Pen => st.push(q),
                _ => {
                    st.truncate(1);
                    st.push(q);
                }
            }
        }
        if resp.drag_stopped()
            && let Some(st) = rt.stroke.take()
        {
            let (a, b) = (st[0], *st.last().unwrap_or(&st[0]));
            match rt.mode {
                RtMode::RedEye => {
                    let (x0, y0, x1, y1) = (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1));
                    if x1 - x0 > 3.0 && y1 - y0 > 3.0 {
                        out = Some(Retouch::RedEye {
                            eyes: vec![cs_adjust::retouch::Eye {
                                cx: f64::from((x0 + x1) / 2.0),
                                cy: f64::from((y0 + y1) / 2.0),
                                rx: f64::from((x1 - x0) / 2.0),
                                ry: f64::from((y1 - y0) / 2.0),
                                pupil_size: f64::from(rt.pupil),
                                darken: f64::from(rt.darken),
                            }],
                        });
                    }
                }
                RtMode::Heal | RtMode::Clone => {
                    let dabs: Vec<cs_adjust::retouch::Dab> = st
                        .iter()
                        .map(|&(x, y)| cs_adjust::retouch::Dab {
                            x: f64::from(x),
                            y: f64::from(y),
                            r: f64::from(brush),
                        })
                        .collect();
                    if rt.mode == RtMode::Heal {
                        out = Some(Retouch::Heal { dabs });
                    } else if let Some(src) = rt.src {
                        // Aligned: the offset is fixed by the first stroke after the source was set.
                        let off = *rt.offset.get_or_insert((src.0 - a.0, src.1 - a.1));
                        out = Some(Retouch::Clone {
                            dabs,
                            dx: f64::from(off.0),
                            dy: f64::from(off.1),
                            hardness: f64::from(rt.hardness),
                        });
                    }
                }
                RtMode::Draw => {
                    if rt.shape == raster::Shape::Pen {
                        rt.marks.push(raster::Mark::Path {
                            points: st,
                            width: rt.width,
                            color: rt.color,
                            arrow: false,
                        });
                    } else if rt.shape != raster::Shape::Text && ((b.0 - a.0).abs() + (b.1 - a.1).abs()) > 2.0 {
                        rt.marks.push(raster::Mark::shape(rt.shape, a, b, rt.width, rt.color));
                    }
                }
            }
        }

        // Draw what is in progress and the marks not yet put on the picture.
        let p = ui.painter().with_clip_rect(pic);
        let col = |c: [u8; 3]| Color32::from_rgb(c[0], c[1], c[2]);
        for m in rt.marks.iter().chain(
            rt.stroke
                .as_ref()
                .filter(|_| rt.mode == RtMode::Draw)
                .map(|st| {
                    if rt.shape == raster::Shape::Pen {
                        raster::Mark::Path { points: st.clone(), width: rt.width, color: rt.color, arrow: false }
                    } else {
                        raster::Mark::shape(rt.shape, st[0], *st.last().unwrap_or(&st[0]), rt.width, rt.color)
                    }
                })
                .iter(),
        ) {
            match m {
                raster::Mark::Path { points, width, color, arrow } => {
                    let pts: Vec<Pos2> = points.iter().map(|&q| to_scr(q)).collect();
                    p.add(egui::Shape::line(pts.clone(), Stroke::new(width * k, col(*color))));
                    if *arrow && pts.len() >= 2 {
                        let (a, b) = (pts[0], pts[pts.len() - 1]);
                        let ang = (b - a).angle();
                        let len = (width * 4.0).max(12.0) * k;
                        for s in [-0.5f32, 0.5] {
                            let t = ang + std::f32::consts::PI + s;
                            p.line_segment([b, b + vec2(t.cos(), t.sin()) * len], Stroke::new(width * k, col(*color)));
                        }
                    }
                }
                raster::Mark::Text { x, y, text, size, color } => {
                    p.text(
                        to_scr((*x, *y)),
                        egui::Align2::LEFT_TOP,
                        text,
                        egui::FontId::proportional(size * k),
                        col(*color),
                    );
                }
            }
        }
        if let Some(st) = &rt.stroke {
            match rt.mode {
                RtMode::RedEye => {
                    let r = Rect::from_two_pos(to_scr(st[0]), to_scr(*st.last().unwrap_or(&st[0])));
                    p.rect_stroke(r, 0.0, Stroke::new(2.0, Color32::from_rgb(255, 80, 80)), StrokeKind::Middle);
                }
                RtMode::Heal | RtMode::Clone => {
                    for &q in st {
                        p.circle_filled(to_scr(q), brush * k, Color32::from_rgba_unmultiplied(255, 255, 255, 60));
                    }
                }
                RtMode::Draw => {}
            }
        }
        if let Some(src) = rt.src.filter(|_| rt.mode == RtMode::Clone) {
            p.circle_stroke(to_scr(src), brush * k, Stroke::new(2.0, Color32::from_rgb(60, 140, 255)));
        }
        if let Some(pt) = pointer.filter(|q| pic.contains(*q))
            && matches!(rt.mode, RtMode::Heal | RtMode::Clone)
        {
            p.circle_stroke(pt, brush * k, Stroke::new(1.5, Color32::WHITE));
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        if let Some(r) = out {
            self.send(Req::Retouch(r));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RtMode {
    RedEye,
    Heal,
    Clone,
    Draw,
}

pub struct Rt {
    pub mode: RtMode,
    size: f32,
    hardness: f32,
    pupil: f32,
    darken: f32,
    /// Clone source and the fixed offset (picture pixels).
    src: Option<(f32, f32)>,
    offset: Option<(f32, f32)>,
    pick_source: bool,
    shape: raster::Shape,
    color: [u8; 3],
    width: f32,
    font: f32,
    text: String,
    marks: Vec<raster::Mark>,
    /// The stroke / box being dragged (picture pixels).
    stroke: Option<Vec<(f32, f32)>>,
    hint: String,
}

pub struct Fx {
    effects: cs_adjust::effects::Effects,
    use_logo: bool,
    logo_shown: bool,
    logo: Option<PathBuf>,
    text: String,
    bold: bool,
    text_color: [u8; 3],
    outline: bool,
    /// What the watermark picture last sent was made from (to send it only when it changes).
    sent_layer: String,
}

fn frame_name(f: cs_adjust::effects::Frame) -> &'static str {
    use cs_adjust::effects::Frame::*;
    match f {
        None => "No frame",
        Border => "Border",
        Matte => "Mat (passe-partout)",
        Instant => "Instant photo",
        Rounded => "Rounded corners",
        Shadow => "Shadow",
        Vignette => "Vignette",
    }
}

/// A colour swatch that sets `target` when clicked.
fn swatch(ui: &mut egui::Ui, target: &mut [u8; 3], c: [u8; 3]) {
    let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
    ui.painter().rect_filled(r, 3.0, Color32::from_rgb(c[0], c[1], c[2]));
    let ring = if *target == c { Stroke::new(2.0, colors::ACCENT) } else { Stroke::new(1.0, Color32::from_gray(90)) };
    ui.painter().rect_stroke(r, 3.0, ring, StrokeKind::Outside);
    if resp.clicked() {
        *target = c;
    }
}

enum Act {
    Lossless,
    Draw,
    DrawDone,
    EffectsNow,
    Cancel,
    Reshape,
    Crop,
    Straighten,
    Resize,
    Canvas,
    Dpi,
    Auto,
    AdjustNow,
    Save,
}

/// Cancel + the main button.
fn buttons(ui: &mut egui::Ui, main: &str, out: &mut Option<Act>, act: Act) {
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if ui.button("Cancel").clicked() {
            *out = Some(Act::Cancel);
        }
        if ui.add(egui::Button::new(RichText::new(main).color(Color32::WHITE)).fill(colors::ACCENT)).clicked() {
            *out = Some(act);
        }
    });
}

/// A box from `a` to `b` (0..1 units), shaped to `ratio` (picture pixels w/h; 0 = free).
fn shaped(a: Pos2, b: Pos2, ratio: f32, pic_ratio: f32) -> Rect {
    if ratio <= 0.0 {
        return Rect::from_two_pos(a, b);
    }
    let want = ratio / pic_ratio; // width / height in 0..1 units
    let d: Vec2 = b - a;
    let (mut w, mut h) = (d.x.abs(), d.y.abs());
    if w / h.max(1e-6) > want {
        w = h * want
    } else {
        h = w / want
    }
    let b = pos2(a.x + w * d.x.signum(), a.y + h * d.y.signum());
    let r = Rect::from_two_pos(a, b);
    if r.min.x < 0.0 || r.min.y < 0.0 || r.max.x > 1.0 || r.max.y > 1.0 {
        // Shrink to fit inside the picture, keeping the anchored corner.
        let s = [
            if d.x < 0.0 { a.x / w.max(1e-6) } else { (1.0 - a.x) / w.max(1e-6) },
            if d.y < 0.0 { a.y / h.max(1e-6) } else { (1.0 - a.y) / h.max(1e-6) },
        ]
        .into_iter()
        .fold(1.0f32, f32::min);
        return Rect::from_two_pos(a, pos2(a.x + w * s * d.x.signum(), a.y + h * s * d.y.signum()));
    }
    r
}

fn worker(rx: &Receiver<(u64, Req)>, tx: &Sender<(u64, Reply)>, ctx: &egui::Context) {
    let mut session: Option<Session> = None;
    let mut raw = false;
    let mut queue: VecDeque<(u64, Req)> = VecDeque::new();
    loop {
        if queue.is_empty() {
            match rx.recv() {
                Ok(r) => queue.push_back(r),
                Err(_) => return,
            }
        }
        while let Ok(r) = rx.try_recv() {
            queue.push_back(r);
        }
        let Some((g, req)) = queue.pop_front() else { continue };
        // Only the newest colour settings matter: skip this one if another follows right away.
        if matches!(req, Req::Adjust(_) | Req::Effects(_))
            && queue
                .front()
                .is_some_and(|(g2, r)| *g2 == g && std::mem::discriminant(r) == std::mem::discriminant(&req))
        {
            let _ = tx.send((
                g,
                Reply::State {
                    state: Box::new(session.as_ref().map(Session::state).unwrap_or_else(empty_state)),
                    preview: None,
                },
            ));
            continue;
        }
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Reply, String> {
            let s = match req {
                Req::Open(p) => {
                    session = Some(Session::open(&p)?);
                    session.as_mut().ok_or("not open")?
                }
                Req::Close => {
                    session = None;
                    return Ok(Reply::State { state: Box::new(empty_state()), preview: None });
                }
                Req::Auto => {
                    return Ok(Reply::Auto(Box::new(session.as_ref().ok_or("No photo is open for editing.")?.auto()?)));
                }
                Req::LosslessCrop { x, y, w, h } => {
                    let s = session.as_ref().ok_or("No photo is open for editing.")?;
                    return Ok(Reply::Cropped(cs_app::privacy::lossless_crop_file(s.path(), x, y, w, h)?));
                }
                Req::Save { mode, format, quality } => {
                    let s = session.as_ref().ok_or("No photo is open for editing.")?;
                    return Ok(Reply::Saved(s.save(mode, &format, quality, None)?));
                }
                other => {
                    let s = session.as_mut().ok_or("No photo is open for editing.")?;
                    match other {
                        Req::Apply(e) => {
                            s.apply(e)?;
                        }
                        Req::StepBack => {
                            s.step_back()?;
                        }
                        Req::Reset => {
                            s.reset();
                        }
                        Req::Adjust(a) => {
                            s.set_adjust(*a);
                        }
                        Req::Effects(e) => {
                            s.set_effects(*e);
                        }
                        Req::Watermark(w) => {
                            s.set_watermark(w)?;
                        }
                        Req::Retouch(r) => {
                            s.retouch(r)?;
                        }
                        Req::Dpi(d) => {
                            s.set_dpi(d)?;
                        }
                        Req::Raw(r) => raw = r,
                        _ => {}
                    }
                    s
                }
            };
            let preview = s.preview(raw)?;
            Ok(Reply::State { state: Box::new(s.state()), preview: Some(preview) })
        }))
        .unwrap_or_else(|_| Err("Editing failed on this picture.".into()));
        let _ = tx.send((g, out.unwrap_or_else(Reply::Error)));
        ctx.request_repaint();
    }
}

fn empty_state() -> EditState {
    EditState {
        width: 0,
        height: 0,
        out_width: 0,
        out_height: 0,
        format: String::new(),
        edits: 0,
        version: 0,
        lossless: false,
        adjusted: false,
        adjust: Default::default(),
        dpi_original: None,
        dpi: None,
        effects: Default::default(),
        has_effects: false,
        watermark: None,
    }
}
