//! The full-screen viewer, laid out like the Tauri app's (FastStone style): the photo fills the screen and four
//! fly-outs slide in when the mouse touches an edge — top: the folder's photos as a strip; left: the picture
//! menu (and the open edit tool's form); right: information; bottom: navigation and zoom.
//!
//! Mouse: wheel = next / previous · Ctrl+wheel = zoom about the mouse · drag = move a zoomed photo ·
//! double-click = back to the browser. Keys: → Space PageDown next · ← Backspace PageUp previous ·
//! Home / End first / last · 0 or B fit · 1 or A real size · 2–9 = 200–900 % · + / − zoom ·
//! Shift+W / Shift+H fit width / height · I information · Esc closes a tool, a pinned panel, then the viewer.
//! Editing: L / R rotate · H / V flip · X crop · Ctrl+R resize · T straighten · C colour and light ·
//! Ctrl+S save as · Ctrl+Z undo the last change. Marks: Ctrl+1–5 stars (Ctrl+0 removes them) · Q tag ·
//! Ctrl+H histogram · hold the left button on a fitted photo for the magnifier. Edits happen in memory (see `editor.rs`); the file is only
//! written by "Save as…", and leaving an edited photo asks first.

use crate::browse::{self, Kind, Listing};
use crate::editor::{self, Tool};
use crate::loader::Key;
use crate::{App, Tex, bold, colors, icons};
use cs_app::edit::Edit;
use egui::{Align, Align2, Color32, CursorIcon, FontId, Id, Order, Pos2, Rect, RichText, Sense, Vec2, pos2, vec2};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Longest side the original is decoded to (fits a graphics card texture everywhere).
const FULL_SIDE: u32 = 4096;
/// How close to the screen edge the mouse must be to open a fly-out, in points.
const EDGE: f32 = 7.0;
/// Wheel travel for one step to the next / previous photo (one notch of a mouse wheel is ~50).
const WHEEL_STEP: f32 = 40.0;
const FLY: Color32 = Color32::from_rgba_premultiplied(26, 28, 32, 246);
const SOON: &str = "This tool is being moved to the new ChronoSort next. For now use ChronoSort.exe for it.";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Top,
    Left,
    Right,
    Bottom,
}

#[derive(Clone, Copy)]
enum Nav {
    By(isize),
    To(usize),
    First,
    Last,
}

/// What to do once the "unsaved changes" question is answered.
#[derive(Clone, Copy)]
enum After {
    Nav(Nav),
    Close,
}

enum Cmd {
    Nav(Nav),
    Zoom(f32),
    Fit,
    Real,
    Info,
    Explorer,
    Close,
    Edit(Edit),
    Tool(&'static str),
    StepBack,
    Reset,
    Wallpaper,
    Print,
    Slideshow,
    Rate(u8),
    Tag,
    Histo,
}

pub struct Viewer {
    pub index: usize,
    /// Screen points per picture pixel; `None` = fit to the screen.
    scale: Option<f32>,
    /// Picture centre offset from the screen centre, in points.
    pan: Vec2,
    edge: Option<Side>,
    pinned: Option<Side>,
    started: bool,
    was_fullscreen: bool,
    wheel: f32,
    last_move: Instant,
    last_pointer: Option<Pos2>,
    strip_follow: bool,
    info: crate::infopanel::InfoPanel,
    note: Option<(String, Instant)>,
    ask: Option<After>,
    /// The wallpaper dialog: chosen style, the running job, its error.
    wall: Option<WallDlg>,
    printdlg: Option<crate::printdlg::PrintDlg>,
    /// Ctrl+H: the histogram box is shown.
    show_hist: bool,
    /// Stars being written into this photo (path, stars, the answer).
    rate_job: Option<RateJob>,
}

type RateJob = (PathBuf, u8, std::sync::mpsc::Receiver<Result<cs_app::rating::RatingReport, String>>);

struct WallDlg {
    style: &'static str,
    job: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    error: Option<String>,
}

const WALL_STYLES: [(&str, &str, &str); 6] = [
    ("fill", "Fill", "Fills the screen; edges may be cut"),
    ("fit", "Fit", "The whole photo, with bars at the sides"),
    ("stretch", "Stretch", "Fills the screen; the photo is pulled out of shape"),
    ("center", "Centre", "Real size in the middle"),
    ("tile", "Tile", "Repeated like tiles"),
    ("span", "Span", "One photo across all screens"),
];

impl Viewer {
    pub fn new(index: usize) -> Self {
        Self {
            index,
            scale: None,
            pan: Vec2::ZERO,
            edge: None,
            pinned: None,
            started: false,
            was_fullscreen: false,
            wheel: 0.0,
            last_move: Instant::now(),
            last_pointer: None,
            strip_follow: true,
            info: crate::infopanel::InfoPanel::new(),
            note: None,
            ask: None,
            wall: None,
            printdlg: None,
            show_hist: false,
            rate_job: None,
        }
    }

    fn viewable(l: &Listing) -> Vec<usize> {
        l.entries.iter().enumerate().filter(|(_, e)| e.kind.viewable()).map(|(i, _)| i).collect()
    }

    /// The shown picture and its neighbours (kept in memory, decoded ahead).
    pub fn neighbours(&self, l: Option<&Listing>) -> Vec<PathBuf> {
        let Some(l) = l else { return Vec::new() };
        let v = Self::viewable(l);
        let Some(pos) = v.iter().position(|&i| i == self.index) else { return Vec::new() };
        [pos.checked_sub(1), Some(pos), Some(pos + 1)]
            .into_iter()
            .flatten()
            .filter_map(|p| v.get(p).map(|&i| l.entries[i].path.clone()))
            .collect()
    }

    fn navigate(&mut self, list: &[usize], nav: Nav) {
        let Some(pos) = list.iter().position(|&i| i == self.index) else { return };
        let to = match nav {
            Nav::By(by) => (pos as isize + by).clamp(0, list.len() as isize - 1) as usize,
            Nav::To(p) => p.min(list.len().saturating_sub(1)),
            Nav::First => 0,
            Nav::Last => list.len().saturating_sub(1),
        };
        if let Some(&i) = list.get(to)
            && i != self.index
        {
            self.index = i;
            self.scale = None;
            self.pan = Vec2::ZERO;
            self.strip_follow = true;
        }
    }

    fn say(&mut self, text: impl Into<String>) {
        self.note = Some((text.into(), Instant::now()));
    }

    fn close(&self, ctx: &egui::Context, app: &mut App) -> bool {
        app.editor.close();
        app.player = None;
        if !self.was_fullscreen && std::env::var_os("CHRONOSORT_DRIVE").is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        false
    }

    fn is_open(&self, s: Side) -> bool {
        self.edge == Some(s) || self.pinned == Some(s)
    }

    fn toggle_info(&mut self) {
        self.pinned = if self.pinned == Some(Side::Right) { None } else { Some(Side::Right) };
    }

    /// Where each fly-out sits when fully open.
    fn panel_rect(s: Side, screen: Rect, wide_left: bool) -> Rect {
        match s {
            Side::Top => Rect::from_min_size(screen.min, vec2(screen.width(), 156.0)),
            Side::Bottom => Rect::from_min_size(pos2(screen.min.x, screen.max.y - 62.0), vec2(screen.width(), 62.0)),
            Side::Left => {
                let h = (screen.height() * 0.96).min(880.0);
                let w = if wide_left { 360.0 } else { 290.0 };
                Rect::from_min_size(pos2(screen.min.x, screen.center().y - h / 2.0), vec2(w, h))
            }
            Side::Right => {
                let h = (screen.height() * 0.94).min(620.0);
                Rect::from_min_size(pos2(screen.max.x - 380.0, screen.center().y - h / 2.0), vec2(380.0, h))
            }
        }
    }

    /// Leave this photo: ask first when it has unsaved changes. `true` = go ahead now.
    fn may_leave(&mut self, app: &mut App, after: After) -> bool {
        if app.editor.dirty() {
            self.ask = Some(after);
            false
        } else {
            app.editor.close();
            true
        }
    }

    /// Draws the viewer; `false` = close it.
    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut App) -> bool {
        let ctx = ui.ctx().clone();
        if !self.started {
            self.started = true;
            self.was_fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            if std::env::var_os("CHRONOSORT_DRIVE").is_none() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            }
        }
        // Answers from the edit worker.
        for ev in app.editor.poll(&ctx) {
            match ev {
                editor::Event::Error(e) => self.say(e),
                editor::Event::Cropped(c) => {
                    let mut msg = format!("Saved “{}” ({} × {}) without any quality loss.", c.name, c.width, c.height);
                    if c.moved {
                        msg += " The left or top edge moved a few pixels to JPEG's 8/16-pixel grid.";
                    }
                    msg += " The original is unchanged; Ctrl+Z in the browser removes the new file.";
                    self.say(msg);
                    let shown = app.listing.as_ref().and_then(|l| l.entries.get(self.index)).map(|e| e.path.clone());
                    if let Some(i) = app.reload_listing(shown.as_deref()) {
                        self.index = i;
                    }
                }
                editor::Event::Saved(s) => {
                    let name = s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let mut msg = format!("Saved “{name}”");
                    if s.lossless {
                        msg += " without any quality loss";
                    }
                    msg += ". The original is safe; Ctrl+Z in the browser undoes the save.";
                    for w in &s.warnings {
                        msg += &format!("\n{w}");
                    }
                    self.say(msg);
                    let shown = app.listing.as_ref().and_then(|l| l.entries.get(self.index)).map(|e| e.path.clone());
                    app.editor.close();
                    if let Some(p) = &shown {
                        app.forget_pictures(p);
                    }
                    // The folder has a new (or changed) file: list it again, stay on the same photo.
                    if let Some(i) = app.reload_listing(shown.as_deref()) {
                        self.index = i;
                    }
                }
            }
        }
        let Some(list) = app.listing.as_ref().map(Self::viewable) else { return self.close(&ctx, app) };
        if list.is_empty() {
            return self.close(&ctx, app);
        }
        let screen = ui.max_rect();
        let tool_open = !app.editor.tool.is_none();
        if tool_open {
            self.pinned = Some(Side::Left);
        }
        let wide_left = tool_open;
        let pointer = ctx.input(|i| i.pointer.hover_pos());
        let over_panel = pointer.is_some_and(|p| {
            [Side::Top, Side::Left, Side::Right, Side::Bottom]
                .into_iter()
                .any(|s| self.is_open(s) && Self::panel_rect(s, screen, wide_left).contains(p))
        });
        let Some(entry) = app.listing.as_ref().and_then(|l| l.entries.get(self.index)).cloned() else {
            return self.close(&ctx, app);
        };

        // ---------- keys and wheel ----------
        let is_video = entry.kind == Kind::Video;
        let mut play_toggle = false;
        let typing = ctx.text_edit_focused();
        let mut nav = None;
        let mut cmd: Vec<Cmd> = Vec::new();
        let (mut esc, mut enter) = (false, false);
        let mut zoom_by: Option<f32> = None;
        let (mut fit, mut real, mut to_pct, mut fit_w, mut fit_h) = (false, false, None, false, false);
        let mut wheel = self.wheel;
        if let Some(log) = std::env::var_os("CHRONOSORT_KEYLOG") {
            ctx.input(|i| {
                for ev in &i.events {
                    if matches!(
                        ev,
                        egui::Event::Key { .. }
                            | egui::Event::Text(_)
                            | egui::Event::Copy
                            | egui::Event::Cut
                            | egui::Event::Paste(_)
                    ) {
                        use std::io::Write;
                        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
                            let _ = writeln!(f, "{ev:?} typing={typing}");
                        }
                    }
                }
            });
        }
        ctx.input(|i| {
            use egui::Key::*;
            esc = i.key_pressed(Escape);
            if typing || self.ask.is_some() || self.wall.is_some() || self.printdlg.is_some() {
                return;
            }
            let (shift, ctrl) = (i.modifiers.shift, i.modifiers.command);
            enter = i.key_pressed(Enter);
            if !tool_open {
                if i.key_pressed(Space) && is_video {
                    play_toggle = true;
                } else if i.key_pressed(ArrowRight) || i.key_pressed(PageDown) || i.key_pressed(Space) {
                    nav = Some(Nav::By(1));
                }
                if i.key_pressed(ArrowLeft) || i.key_pressed(PageUp) || i.key_pressed(Backspace) {
                    nav = Some(Nav::By(-1));
                }
                if i.key_pressed(Home) {
                    nav = Some(Nav::First);
                }
                if i.key_pressed(End) {
                    nav = Some(Nav::Last);
                }
            }
            // Plain digits zoom; with Ctrl they give stars.
            let plain = |key| crate::pressed(i, key).is_some_and(|m| !m.command && !m.alt);
            fit = plain(Num0) || plain(B);
            real = plain(Num1) || plain(A);
            for (k, n) in
                [(Num2, 2.0), (Num3, 3.0), (Num4, 4.0), (Num5, 5.0), (Num6, 6.0), (Num7, 7.0), (Num8, 8.0), (Num9, 9.0)]
            {
                if plain(k) {
                    to_pct = Some(n);
                }
            }
            for (k, n) in [(Num0, 0u8), (Num1, 1), (Num2, 2), (Num3, 3), (Num4, 4), (Num5, 5)] {
                if crate::pressed(i, k).is_some_and(|m| m.command) {
                    cmd.push(Cmd::Rate(n));
                }
            }
            if i.key_pressed(Plus) || i.key_pressed(Equals) {
                zoom_by = Some(1.25);
            }
            if i.key_pressed(Minus) {
                zoom_by = Some(0.8);
            }
            fit_w = shift && i.key_pressed(W);
            fit_h = shift && i.key_pressed(H);
            if i.key_pressed(I) {
                cmd.push(Cmd::Info);
            }
            // Editing keys. The modifiers come from each key press itself: Ctrl may already be up again when
            // the frame is handled (fast typing, remote tools).
            let _ = (ctrl, shift);
            let with_ctrl = |key| crate::pressed(i, key).is_some_and(|m| m.command);
            let k = |key| crate::pressed(i, key).is_some_and(|m| !m.command && !m.shift && !m.alt);
            if with_ctrl(Z) {
                cmd.push(Cmd::StepBack);
            } else if with_ctrl(S) {
                cmd.push(Cmd::Tool("save"));
            } else if with_ctrl(R) {
                cmd.push(Cmd::Tool("resize"));
            } else if with_ctrl(P) {
                cmd.push(Cmd::Print);
            } else if with_ctrl(H) {
                cmd.push(Cmd::Histo);
            } else if k(Q) {
                cmd.push(Cmd::Tag);
            } else if k(S) {
                cmd.push(Cmd::Slideshow);
            } else {
                for (key, c) in [
                    (L, Cmd::Edit(Edit::RotateLeft)),
                    (R, Cmd::Edit(Edit::RotateRight)),
                    (H, Cmd::Edit(Edit::FlipHorizontal)),
                    (V, Cmd::Edit(Edit::FlipVertical)),
                    (X, Cmd::Tool("crop")),
                    (T, Cmd::Tool("straighten")),
                    (C, Cmd::Tool("adjust")),
                    (E, Cmd::Tool("retouch")),
                    (F, Cmd::Tool("effects")),
                ] {
                    if k(key) {
                        cmd.push(c);
                    }
                }
            }
            // Wheel: next / previous (not while a tool is open). Ctrl+wheel arrives as zoom_delta instead.
            if !over_panel && !tool_open {
                for ev in &i.events {
                    if let egui::Event::MouseWheel { unit, delta, modifiers, .. } = ev
                        && !modifiers.ctrl
                        && !modifiers.command
                    {
                        wheel += match unit {
                            egui::MouseWheelUnit::Point => delta.y,
                            egui::MouseWheelUnit::Line => delta.y * 50.0,
                            egui::MouseWheelUnit::Page => delta.y * 500.0,
                        };
                    }
                }
                if wheel <= -WHEEL_STEP {
                    nav = Some(Nav::By(1));
                    wheel = 0.0;
                } else if wheel >= WHEEL_STEP {
                    nav = Some(Nav::By(-1));
                    wheel = 0.0;
                }
            }
        });
        self.wheel = wheel;
        if esc {
            if self.ask.is_some() {
                self.ask = None;
            } else if self.wall.is_some() || self.printdlg.is_some() {
                // The dialog closes itself.
            } else if tool_open {
                app.editor.cancel_tool();
                self.pinned = None;
            } else if self.pinned.is_some() {
                self.pinned = None;
            } else {
                cmd.push(Cmd::Close);
            }
        }
        if enter && !app.editor.enter() {
            cmd.push(Cmd::Close);
        } else if enter {
            self.pinned = None;
        }
        if let Some(n) = nav {
            cmd.push(Cmd::Nav(n));
        }

        // ---------- the current picture (the edit preview while editing) ----------
        let number = list.iter().position(|&i| i == self.index).map_or(0, |p| p + 1);
        let total = list.len();
        for p in self.neighbours(app.listing.as_ref()).into_iter().rev() {
            if p != entry.path {
                let _ = app.texture(&Key { path: p, size: FULL_SIDE, full: true });
            }
        }
        let preview_key = Key { path: entry.path.clone(), size: cs_thumb::PREVIEW_SIZE, full: false };
        let full_key = Key { path: entry.path.clone(), size: FULL_SIDE, full: true };
        let _ = app.texture(&preview_key);
        let _ = app.texture(&full_key);
        let editing = app.editor.on(&entry.path);
        let edit_tex = if editing { app.editor.tex.clone() } else { None };
        // A video plays: the player shows its frames (the cached poster until the first one is there).
        if is_video {
            if app.player.as_ref().is_none_or(|p| p.path != entry.path) {
                let side = (screen.width().max(screen.height()) * ctx.pixels_per_point()) as u32;
                let silent = std::env::var_os("CHRONOSORT_DRIVE").is_some();
                app.player = Some(crate::player::Player::open(&entry.path, side.clamp(640, 3840), silent));
            }
            if let Some(p) = &mut app.player {
                if play_toggle {
                    p.toggle();
                }
                p.update(&ctx);
            }
        } else {
            app.player = None;
        }
        let video_tex = app.player.as_ref().and_then(|p| p.tex.clone());
        let (tex, original, failed, sharp) = if let Some(t) = video_tex {
            let o = app.player.as_ref().and_then(|p| p.info.as_ref()).map(|i| (i.width, i.height));
            (Some(t), o, None, true)
        } else if let Some(t) = edit_tex {
            let o = app.editor.state.as_ref().map(|s| (s.out_width, s.out_height));
            (Some(t), o, None, true)
        } else {
            match (app.textures.get(&full_key), app.textures.get(&preview_key)) {
                (Some(Tex::Ready(t, o)), _) => (Some(t.clone()), *o, None, true),
                (_, Some(Tex::Ready(t, o))) => (Some(t.clone()), *o, None, false),
                (Some(Tex::Failed(e)), _) => (None, None, Some(e.clone()), false),
                _ => (None, None, None, false),
            }
        };

        ui.painter().rect_filled(screen, 0.0, Color32::BLACK);
        // While a tool is open its form sits on the left: the picture fits in the space beside it.
        let area = if tool_open {
            Rect::from_min_max(
                pos2(screen.min.x + Self::panel_rect(Side::Left, screen, true).width() + 8.0, screen.min.y),
                screen.max,
            )
        } else {
            screen
        };
        let resp = ui.allocate_rect(screen, Sense::click_and_drag());
        let ppp = ctx.pixels_per_point();
        let mut pct_text = String::from("…");
        let mut fit_scale_now = 1.0;
        let mut real_scale = 1.0 / ppp;
        let mut pic_rect = None;
        if let Some(t) = &tex {
            let size = t.size_vec2();
            // Real pixels per texture pixel (the texture can be a smaller preview).
            let px = original.map_or(1.0, |(w, _)| w as f32 / size.x).max(1e-3);
            real_scale = px / ppp;
            let fit_scale = (area.width() / size.x).min(area.height() / size.y).min(real_scale);
            fit_scale_now = fit_scale;
            if matches!(app.editor.tool, Tool::Crop(_) | Tool::Straighten { .. } | Tool::Retouch(_)) {
                self.scale = None;
                self.pan = Vec2::ZERO;
            }
            let mut scale = self.scale.unwrap_or(fit_scale);
            if fit {
                self.scale = None;
                self.pan = Vec2::ZERO;
                scale = fit_scale;
            }
            if real {
                scale = real_scale;
                self.scale = Some(scale);
                self.pan = Vec2::ZERO;
            }
            if let Some(n) = to_pct {
                scale = n * real_scale;
                self.scale = Some(scale);
            }
            if fit_w {
                scale = area.width() / size.x;
                self.scale = Some(scale);
                self.pan = vec2(0.0, ((size.y * scale - area.height()) / 2.0).max(0.0));
            }
            if fit_h {
                scale = area.height() / size.y;
                self.scale = Some(scale);
                self.pan = vec2(((size.x * scale - area.width()) / 2.0).max(0.0), 0.0);
            }
            let pinch = if over_panel || tool_open { 1.0 } else { ctx.input(|i| i.zoom_delta()) };
            let factor = zoom_by.unwrap_or(1.0) * pinch;
            if factor != 1.0 {
                let new = (scale * factor).clamp(fit_scale.min(0.05), 10.0 * real_scale);
                let anchor = if zoom_by.is_some() { area.center() } else { pointer.unwrap_or(area.center()) };
                let rel = anchor - area.center() - self.pan;
                self.pan += rel - rel * (new / scale);
                scale = new;
                self.scale = Some(new);
            }
            let cropping = matches!(app.editor.tool, Tool::Crop(_) | Tool::Retouch(_));
            // Fitted: holding the left button shows the magnifier instead of moving the photo.
            let magnifier = self.scale.is_none() && !tool_open && !is_video && !over_panel;
            if resp.dragged() && !over_panel && !cropping && !magnifier {
                self.pan += resp.drag_delta();
                self.scale.get_or_insert(scale);
            }
            if self.scale.is_none() {
                self.pan = Vec2::ZERO;
            }
            let rect = Rect::from_center_size(area.center() + self.pan, size * scale);
            let angle = app.editor.preview_angle();
            if angle == 0.0 {
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                ui.painter().image(t.id(), rect, uv, Color32::WHITE);
            } else {
                paint_turned(ui.painter(), t.id(), rect, angle.to_radians());
                paint_grid(ui.painter(), rect);
            }
            pic_rect = Some(rect);
            if magnifier
                && resp.is_pointer_button_down_on()
                && let Some(p) = pointer.filter(|p| rect.contains(*p))
            {
                // The original when it is loaded (sharper than the preview), else what is shown.
                let hi = match app.textures.get(&full_key) {
                    Some(Tex::Ready(f, _)) if !editing => f.id(),
                    _ => t.id(),
                };
                let f = (real_scale / scale).clamp(2.0, 8.0);
                paint_lens(ui.painter(), hi, rect, p, f);
            }
            pct_text = if sharp { format!("{:.0} %", scale / real_scale * 100.0) } else { "preview".into() };
        } else {
            let msg = failed.unwrap_or_else(|| "Loading…".into());
            ui.painter().text(area.center(), Align2::CENTER_CENTER, msg, FontId::proportional(20.0), colors::MUTED);
        }
        if let Some(r) = pic_rect
            && editing
        {
            app.editor.crop_overlay(ui, r, &resp);
            app.editor.retouch_overlay(ui, r, &resp);
        }
        if resp.double_clicked() && !over_panel && !tool_open {
            cmd.push(Cmd::Close);
        } else if resp.clicked()
            && is_video
            && !over_panel
            && let Some(p) = &mut app.player
        {
            p.toggle();
        }
        if is_video {
            self.video_bar(&ctx, app, screen);
        }

        // ---------- fly-outs: open at the edge, close when the mouse leaves them ----------
        if let Some(p) = pointer {
            if self.last_pointer != Some(p) {
                self.last_move = Instant::now();
                self.last_pointer = Some(p);
            }
            let dragging = ctx.input(|i| i.pointer.any_down());
            let at_edge = if p.x <= screen.min.x + EDGE {
                Some(Side::Left)
            } else if p.x >= screen.max.x - EDGE {
                Some(Side::Right)
            } else if p.y <= screen.min.y + EDGE {
                Some(Side::Top)
            } else if p.y >= screen.max.y - EDGE {
                Some(Side::Bottom)
            } else {
                None
            };
            match (at_edge, self.edge) {
                (Some(s), _) if !dragging => self.edge = Some(s),
                (None, Some(s)) if !Self::panel_rect(s, screen, wide_left).contains(p) => self.edge = None,
                _ => {}
            }
        }

        for side in [Side::Top, Side::Left, Side::Right, Side::Bottom] {
            let t = ctx.animate_bool_with_time(Id::unique(("fly", side as u8)), self.is_open(side), 0.16);
            if t <= 0.0 {
                continue;
            }
            let full = Self::panel_rect(side, screen, wide_left);
            let hide = 1.0 - t;
            let r = match side {
                Side::Top => full.translate(vec2(0.0, -full.height() * hide)),
                Side::Bottom => full.translate(vec2(0.0, full.height() * hide)),
                Side::Left => full.translate(vec2(-full.width() * hide, 0.0)),
                Side::Right => full.translate(vec2(full.width() * hide, 0.0)),
            };
            let radius = match side {
                Side::Left => egui::CornerRadius { ne: 10, se: 10, ..Default::default() },
                Side::Right => egui::CornerRadius { nw: 10, sw: 10, ..Default::default() },
                _ => egui::CornerRadius::ZERO,
            };
            egui::Area::new(Id::unique(("fly-area", side as u8))).fixed_pos(r.min).order(Order::Middle).show(
                &ctx,
                |ui| {
                    egui::Frame::new().fill(FLY).corner_radius(radius).inner_margin(10.0).show(ui, |ui| {
                        let inner = r.size() - vec2(20.0, 20.0);
                        ui.set_min_size(inner);
                        ui.set_max_size(inner);
                        match side {
                            Side::Top => self.top_strip(ui, app, &list, &entry.name, (number, total), &mut cmd),
                            Side::Left if tool_open => app.editor.tool_panel(ui),
                            Side::Left => self.left_menu(ui, app, &mut cmd),
                            Side::Right => self.info.ui(ui, &entry, original, (number, total)),
                            Side::Bottom => self.bottom_bar(ui, app, (number, total), &pct_text, &mut cmd),
                        }
                    });
                },
            );
        }
        if tool_open && app.editor.tool.is_none() {
            self.pinned = None; // the form closed itself (Cancel / Apply)
        }
        if self.info.editing() {
            self.pinned = Some(Side::Right);
        }
        if let Some(m) = self.info.saved.take() {
            self.say(m);
            app.forget_pictures(&entry.path);
            if let Some(i) = app.reload_listing(Some(&entry.path)) {
                self.index = i;
            }
        }

        // ---------- carry out the commands ----------
        for c in cmd {
            match c {
                Cmd::Nav(n) => {
                    if self.may_leave(app, After::Nav(n)) {
                        self.navigate(&list, n);
                    }
                }
                Cmd::Zoom(f) => {
                    let s = self.scale.unwrap_or(fit_scale_now);
                    self.scale = Some((s * f).clamp(0.05, 10.0 * real_scale));
                }
                Cmd::Fit => {
                    self.scale = None;
                    self.pan = Vec2::ZERO;
                }
                Cmd::Real => {
                    self.scale = Some(real_scale);
                    self.pan = Vec2::ZERO;
                }
                Cmd::Info => self.toggle_info(),
                Cmd::Explorer => {
                    if let Err(e) = cs_platform::show_file_in_file_manager(&entry.path) {
                        self.say(format!("Could not open Explorer: {e}"));
                    }
                }
                Cmd::Close => {
                    if self.may_leave(app, After::Close) {
                        return self.close(&ctx, app);
                    }
                }
                Cmd::Edit(e) if entry.kind != Kind::Video => {
                    app.editor.request(&entry.path, editor::Req::Apply(e));
                }
                Cmd::Tool(t) if entry.kind != Kind::Video => {
                    if t == "save" && !(editing && app.editor.dirty()) {
                        self.say("Nothing to save yet: this photo has no changes.");
                    } else {
                        self.scale = None;
                        self.pan = Vec2::ZERO;
                        app.editor.start(&entry.path, t);
                        self.pinned = Some(Side::Left);
                    }
                }
                Cmd::Edit(_) | Cmd::Tool(_) => self.say("Videos cannot be edited here."),
                Cmd::StepBack => {
                    if editing {
                        app.editor.step_back();
                    }
                }
                Cmd::Reset => {
                    if editing {
                        app.editor.reset();
                    }
                }
                Cmd::Wallpaper if entry.kind == Kind::Video => {
                    self.say("A video cannot be a wallpaper; choose a photo.")
                }
                Cmd::Wallpaper => self.wall = Some(WallDlg { style: "fill", job: None, error: None }),
                Cmd::Print if entry.kind == Kind::Video => self.say("Videos cannot be printed; choose a photo."),
                Cmd::Slideshow => {
                    app.player = None;
                    app.start_slideshow(self.index);
                }
                Cmd::Histo => self.show_hist = !self.show_hist,
                Cmd::Tag => {
                    let on = !app.marks.tagged(&entry.path);
                    match app.marks.set_tagged(std::slice::from_ref(&entry.path), on) {
                        Ok(()) => {
                            self.say(if on { "Tagged" } else { "Tag removed" });
                            app.refilter_pending = true;
                        }
                        Err(e) => self.say(format!("Could not save the tag list: {e}")),
                    }
                }
                Cmd::Rate(_) if !matches!(entry.kind, Kind::Image | Kind::Raw) => {
                    self.say("Stars are for photos only.")
                }
                Cmd::Rate(_) if editing && app.editor.dirty() => {
                    self.say("Save or leave the changes to this picture first, then give it stars.")
                }
                Cmd::Rate(_) if self.rate_job.is_some() => {}
                Cmd::Rate(n) => {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let (path, ctx2) = (entry.path.clone(), ctx.clone());
                    std::thread::spawn(move || {
                        let _ = tx.send(cs_app::rating::rate_files(std::slice::from_ref(&path), n, |_| {}));
                        ctx2.request_repaint();
                    });
                    self.rate_job = Some((entry.path.clone(), n, rx));
                }
                Cmd::Print => {
                    let all: Vec<PathBuf> = app
                        .listing
                        .as_ref()
                        .map(|l| {
                            l.entries
                                .iter()
                                .filter(|e| matches!(e.kind, Kind::Image | Kind::Raw))
                                .map(|e| e.path.clone())
                                .collect()
                        })
                        .unwrap_or_default();
                    self.printdlg = Some(crate::printdlg::PrintDlg::new(vec![entry.path.clone()], all));
                }
            }
        }

        // ---------- the print dialog ----------
        if let Some(d) = &mut self.printdlg
            && let crate::printdlg::Outcome::Closed(msg) = d.show(&ctx)
        {
            self.printdlg = None;
            if let Some(m) = msg {
                self.say(m);
                if let Some(i) = app.reload_listing(Some(&entry.path)) {
                    self.index = i;
                }
            }
        }

        // ---------- the wallpaper dialog ----------
        if let Some(w) = &mut self.wall {
            let mut close = false;
            let mut done = None;
            if let Some(r) = w.job.as_ref().and_then(|rx| rx.try_recv().ok()) {
                w.job = None;
                match r {
                    Ok(name) => {
                        done = Some(format!(
                            "“{name}” is now your desktop wallpaper. Ctrl+Z in the browser puts the previous one back."
                        ))
                    }
                    Err(e) => w.error = Some(e),
                }
            }
            egui::Modal::new(Id::unique("wallpaper")).show(&ctx, |ui| {
                ui.set_width(460.0);
                ui.label(RichText::new(format!("{}  Set as desktop wallpaper", icons::WALLPAPER)).font(bold(18.0)));
                ui.label(RichText::new(&entry.name).color(colors::MUTED));
                ui.add_space(8.0);
                for (k, t, hint) in WALL_STYLES {
                    ui.radio_value(&mut w.style, k, RichText::new(format!("{t}  —  {hint}")));
                }
                if let Some(e) = &w.error {
                    ui.colored_label(colors::RED, e);
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let busy = w.job.is_some();
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
                        let (src, style, data, ctx2) =
                            (entry.path.clone(), w.style, app.data_root.clone(), ctx.clone());
                        std::thread::spawn(move || {
                            let _ = tx.send(cs_app::wallpaper::set_wallpaper(&src, style, &data));
                            ctx2.request_repaint();
                        });
                        w.job = Some(rx);
                        w.error = None;
                    }
                });
            });
            if esc && w.job.is_none() {
                close = true;
            }
            if let Some(m) = done {
                self.say(m);
                close = true;
            }
            if close {
                self.wall = None;
            }
        }

        // ---------- the "unsaved changes" question ----------
        if let Some(after) = self.ask {
            let mut answer = None;
            egui::Modal::new(Id::unique("unsaved")).show(&ctx, |ui| {
                ui.set_width(440.0);
                ui.label(RichText::new("This photo has changes that are not saved").font(bold(18.0)));
                ui.add_space(6.0);
                ui.label("Do you want to save them first?");
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new(RichText::new("Save…").color(Color32::WHITE)).fill(colors::ACCENT))
                        .clicked()
                    {
                        answer = Some(0);
                    }
                    if ui.button("Throw the changes away").clicked() {
                        answer = Some(1);
                    }
                    if ui.button("Keep editing").clicked() {
                        answer = Some(2);
                    }
                });
            });
            match answer {
                Some(0) => {
                    self.ask = None;
                    app.editor.start(&entry.path, "save");
                    self.pinned = Some(Side::Left);
                }
                Some(1) => {
                    self.ask = None;
                    app.editor.close();
                    match after {
                        After::Nav(n) => self.navigate(&list, n),
                        After::Close => return self.close(&ctx, app),
                    }
                }
                Some(_) => self.ask = None,
                None => {}
            }
        }

        // ---------- stars being written ----------
        if let Some((path, n, rx)) = &self.rate_job
            && let Ok(r) = rx.try_recv()
        {
            let (path, n) = (path.clone(), *n);
            self.rate_job = None;
            match r {
                Ok(rep) if rep.failed.is_empty() => {
                    app.marks.set_stars(&path, n);
                    app.refilter_pending = true;
                    let msg = if rep.unchanged > 0 {
                        "This photo already had these stars.".to_string()
                    } else if n == 0 {
                        "Stars removed (Ctrl+Z in the browser undoes)".to_string()
                    } else {
                        format!("{} saved in the photo (Ctrl+Z in the browser undoes)", crate::marks::star_text5(n))
                    };
                    self.say(msg);
                }
                Ok(rep) => self.say(format!(
                    "Could not give stars: {}",
                    rep.failed.first().map(|f| f.1.clone()).unwrap_or_default()
                )),
                Err(e) => self.say(format!("Could not give stars: {e}")),
            }
        }

        // ---------- the histogram (Ctrl+H) ----------
        if self.show_hist && !is_video {
            let r = Rect::from_min_size(pos2(screen.max.x - 306.0, screen.min.y + 56.0), vec2(290.0, 190.0));
            match app.hists.get(&entry.path) {
                Some(h) => h.paint(ui.painter(), r, editing.then_some("before the changes")),
                None => {
                    ui.painter().rect_filled(r, 8.0, Color32::from_rgba_premultiplied(20, 22, 26, 235));
                    ui.painter().text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        "…",
                        FontId::proportional(20.0),
                        colors::MUTED,
                    );
                }
            }
        }

        // ---------- always-visible bits ----------
        let any_open = [Side::Top, Side::Left, Side::Right, Side::Bottom].into_iter().any(|s| self.is_open(s));
        if !self.is_open(Side::Top) && !self.is_open(Side::Right) {
            let close_rect = Rect::from_min_size(pos2(screen.max.x - 130.0, screen.min.y + 12.0), vec2(116.0, 34.0));
            let b = ui.put(
                close_rect,
                egui::Button::new(RichText::new(format!("{}  Close", icons::CLOSE)).color(Color32::WHITE))
                    .fill(Color32::from_black_alpha(150)),
            );
            if b.on_hover_text("Back to the folder (Esc)").clicked() && self.may_leave(app, After::Close) {
                return self.close(&ctx, app);
            }
        }
        if editing && (app.editor.busy() || app.editor.dirty()) {
            let text = if app.editor.busy() {
                "Working…".to_string()
            } else {
                let n = app.editor.changes();
                format!(
                    "{}  Edited ({n} change{}) — not saved yet · Ctrl+S saves · Ctrl+Z undoes",
                    icons::EDIT,
                    if n == 1 { "" } else { "s" }
                )
            };
            let galley = ui.painter().layout_no_wrap(text, FontId::proportional(14.0), Color32::WHITE);
            let r =
                Rect::from_center_size(pos2(screen.center().x, screen.min.y + 22.0), galley.size() + vec2(24.0, 12.0));
            ui.painter().rect_filled(r, 6.0, Color32::from_rgba_premultiplied(120, 80, 10, 230));
            ui.painter().galley(r.min + vec2(12.0, 6.0), galley, Color32::WHITE);
        }
        if !self.is_open(Side::Bottom) {
            let dims = original.map(|(w, h)| format!(" · {w} × {h}")).unwrap_or_default();
            let video = match &app.player {
                Some(p) if is_video => {
                    format!(" · {} / {}", crate::player::clock_text(p.shown), crate::player::clock_text(p.duration()))
                }
                _ => String::new(),
            };
            let mut marks = String::new();
            if let Some(s) = app.marks.stars(&entry.path).filter(|&s| s > 0) {
                marks += &format!("  {}", crate::marks::star_text(s));
            }
            if app.marks.tagged(&entry.path) {
                marks += &format!("  {} tagged", icons::TAG_FILLED);
            }
            if self.rate_job.is_some() {
                marks += "  (saving the stars…)";
            }
            let text = format!(
                "{}{marks}   ({number} of {total}) · {}{dims} · {pct_text}{video}",
                entry.name,
                browse::human_size(entry.size)
            );
            let galley = ui.painter().layout_no_wrap(text, FontId::proportional(14.0), Color32::WHITE);
            let r = Rect::from_min_size(pos2(screen.min.x, screen.max.y - 30.0), vec2(galley.size().x + 28.0, 30.0));
            ui.painter().rect_filled(
                r,
                egui::CornerRadius { ne: 8, ..Default::default() },
                Color32::from_black_alpha(140),
            );
            ui.painter().galley(r.left_center() + vec2(14.0, -galley.size().y / 2.0), galley, Color32::WHITE);
        }
        if let Some((text, at)) = &self.note {
            if at.elapsed() < Duration::from_secs(6) {
                let galley = ui.painter().layout(text.clone(), FontId::proportional(16.0), Color32::WHITE, 700.0);
                let r = Rect::from_center_size(
                    pos2(screen.center().x, screen.min.y + 90.0),
                    galley.size() + vec2(32.0, 20.0),
                );
                ui.painter().rect_filled(r, 8.0, Color32::from_rgba_premultiplied(30, 60, 120, 235));
                ui.painter().galley(r.min + vec2(16.0, 10.0), galley, Color32::WHITE);
                ctx.request_repaint_after(Duration::from_millis(500));
            } else {
                self.note = None;
            }
        }
        // Hide the mouse pointer after 2 s without movement (no panel open).
        if !any_open && self.ask.is_none() && self.last_move.elapsed() > Duration::from_secs(2) {
            ctx.set_cursor_icon(CursorIcon::None);
        } else {
            ctx.request_repaint_after(Duration::from_millis(2100));
        }
        true
    }

    /// Play / pause, the time, a slider to jump, sound on / off and volume. Shown while the mouse moves or when paused.
    fn video_bar(&mut self, ctx: &egui::Context, app: &mut App, screen: Rect) {
        let Some(p) = &mut app.player else { return };
        let recent = self.last_move.elapsed() < Duration::from_secs(3);
        if !(recent || p.paused || p.ended) || self.is_open(Side::Bottom) {
            return;
        }
        let w = (screen.width() - 40.0).min(820.0);
        let at = pos2(screen.center().x - w / 2.0, screen.max.y - 104.0);
        egui::Area::new(Id::unique("video-bar")).fixed_pos(at).order(Order::Middle).show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_premultiplied(20, 22, 26, 225))
                .corner_radius(10)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.set_width(w - 20.0);
                    ui.horizontal(|ui| {
                        let icon = if p.ended {
                            icons::REFRESH
                        } else if p.paused {
                            icons::PLAY
                        } else {
                            icons::PAUSE
                        };
                        let tip = if p.ended {
                            "Play again (Space)"
                        } else if p.paused {
                            "Play (Space or click)"
                        } else {
                            "Pause (Space or click)"
                        };
                        if ui
                            .add(egui::Button::new(RichText::new(icon).size(22.0)).min_size(vec2(44.0, 38.0)))
                            .on_hover_text(tip)
                            .clicked()
                        {
                            p.toggle();
                        }
                        let dur = p.duration().max(0.01);
                        ui.label(
                            RichText::new(format!(
                                "{} / {}",
                                crate::player::clock_text(p.shown),
                                crate::player::clock_text(dur)
                            ))
                            .size(15.0)
                            .color(Color32::WHITE),
                        );
                        let mut t = p.shown.min(dur);
                        ui.spacing_mut().slider_width = (w - 420.0).max(120.0);
                        let r = ui
                            .add(egui::Slider::new(&mut t, 0.0..=dur).show_value(false))
                            .on_hover_text("Drag to go to another moment");
                        if r.drag_stopped() || (r.changed() && !r.dragged()) {
                            p.seek(t);
                        }
                        let (mut vol, mut muted) = (p.volume, p.muted);
                        let mute_icon = if muted || vol == 0.0 { icons::VOLUME_OFF } else { icons::VOLUME };
                        if ui
                            .add(egui::Button::new(RichText::new(mute_icon).size(20.0)).min_size(vec2(38.0, 38.0)))
                            .on_hover_text("Sound on / off")
                            .clicked()
                        {
                            muted = !muted;
                        }
                        ui.spacing_mut().slider_width = 90.0;
                        ui.add(egui::Slider::new(&mut vol, 0.0..=1.0).show_value(false)).on_hover_text("Volume");
                        if vol != p.volume || muted != p.muted {
                            p.set_volume(vol, muted);
                        }
                    });
                    if let Some(n) = &p.note {
                        ui.label(RichText::new(n).size(12.5).color(colors::MUTED));
                    }
                });
        });
    }

    fn top_strip(
        &mut self,
        ui: &mut egui::Ui,
        app: &mut App,
        list: &[usize],
        name: &str,
        (number, total): (usize, usize),
        cmd: &mut Vec<Cmd>,
    ) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(name).font(bold(15.0)).color(Color32::WHITE));
            ui.label(
                RichText::new(format!(
                    "{number} of {total} photos and videos in this folder · click one to show it · the wheel scrolls the strip"
                ))
                .size(13.0)
                .color(colors::MUTED),
            );
        });
        ui.add_space(4.0);
        let entries: Vec<(usize, PathBuf, String, Kind)> = app
            .listing
            .as_ref()
            .map(|l| {
                list.iter()
                    .map(|&i| (i, l.entries[i].path.clone(), l.entries[i].name.clone(), l.entries[i].kind))
                    .collect()
            })
            .unwrap_or_default();
        egui::ScrollArea::horizontal().id_salt("strip").auto_shrink([false, true]).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for (n, (i, path, name, kind)) in entries.into_iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(vec2(96.0, 108.0), Sense::click());
                    let on = i == self.index;
                    if on && self.strip_follow {
                        resp.scroll_to_me(Some(Align::Center));
                        self.strip_follow = false;
                    }
                    if ui.is_rect_visible(r) {
                        let bg = if on {
                            colors::ACCENT.gamma_multiply(0.45)
                        } else if resp.hovered() {
                            colors::CELL_HOVER
                        } else {
                            colors::CELL
                        };
                        ui.painter().rect_filled(r, 4.0, bg);
                        let pic = Rect::from_min_size(r.min + vec2(4.0, 4.0), vec2(88.0, 80.0));
                        let key = Key { path, size: cs_thumb::DEFAULT_SIZE, full: false };
                        if let Some(Tex::Ready(t, _)) = app.texture(&key) {
                            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                            ui.painter().image(t.id(), crate::fit(t.size_vec2(), pic), uv, Color32::WHITE);
                        }
                        if kind == Kind::Video {
                            let at = pic.left_bottom() + vec2(4.0, -4.0);
                            ui.painter().text(
                                at,
                                Align2::LEFT_BOTTOM,
                                icons::PLAY,
                                FontId::proportional(14.0),
                                Color32::WHITE,
                            );
                        }
                        let mut job = egui::text::LayoutJob::simple_singleline(
                            name.clone(),
                            FontId::proportional(11.5),
                            colors::TEXT,
                        );
                        job.wrap = egui::text::TextWrapping::truncate_at_width(88.0);
                        let g = ui.painter().layout_job(job);
                        ui.painter().galley(pos2(r.center().x - g.size().x / 2.0, r.max.y - 20.0), g, colors::TEXT);
                    }
                    if resp.on_hover_text(&name).clicked() {
                        cmd.push(Cmd::Nav(Nav::To(n)));
                    }
                }
            });
        });
    }

    fn left_menu(&mut self, ui: &mut egui::Ui, app: &App, cmd: &mut Vec<Cmd>) {
        let dirty = app.editor.dirty();
        let edits = app.editor.state.as_ref().is_some_and(|s| s.edits > 0);
        egui::ScrollArea::vertical().id_salt("left").show(ui, |ui| {
            let item = |ui: &mut egui::Ui, icon: &str, text: &str, key: &str, works: bool, tip: &str| -> bool {
                let b = egui::Button::new(RichText::new(format!("{icon}   {text}")).size(15.0))
                    .shortcut_text(RichText::new(key).size(12.0).color(colors::MUTED))
                    .frame(false)
                    .min_size(vec2(ui.available_width(), 30.0));
                let r = ui.add_enabled(works, b);
                if works {
                    r.clicked()
                } else {
                    r.on_disabled_hover_text(tip);
                    false
                }
            };
            let head = |ui: &mut egui::Ui, t: &str| {
                ui.add_space(6.0);
                ui.label(RichText::new(t.to_uppercase()).font(bold(12.0)).color(colors::MUTED));
            };
            head(ui, "Picture");
            let picture: [(&str, &str, &str, Option<Cmd>); 12] = [
                (icons::ROTATE_LEFT, "Rotate left", "L", Some(Cmd::Edit(Edit::RotateLeft))),
                (icons::ROTATE_RIGHT, "Rotate right", "R", Some(Cmd::Edit(Edit::RotateRight))),
                (icons::FLIP_H, "Flip left–right", "H", Some(Cmd::Edit(Edit::FlipHorizontal))),
                (icons::FLIP_V, "Flip upside down", "V", Some(Cmd::Edit(Edit::FlipVertical))),
                (icons::CROP, "Crop…", "X", Some(Cmd::Tool("crop"))),
                (icons::RESIZE, "Resize…", "Ctrl+R", Some(Cmd::Tool("resize"))),
                (icons::STRAIGHTEN, "Straighten…", "T", Some(Cmd::Tool("straighten"))),
                (icons::CROP, "Canvas size / border…", "", Some(Cmd::Tool("canvas"))),
                (icons::DETAILS, "Print size (DPI)…", "", Some(Cmd::Tool("dpi"))),
                (icons::ADJUST, "Colour and light…", "C", Some(Cmd::Tool("adjust"))),
                (icons::BRUSH, "Red eye, heal, clone, draw…", "E", Some(Cmd::Tool("retouch"))),
                (icons::WAND, "Frame and watermark…", "F", Some(Cmd::Tool("effects"))),
            ];
            for (icon, text, key, c) in picture {
                let works = c.is_some();
                if item(ui, icon, text, key, works, SOON)
                    && let Some(c) = c
                {
                    cmd.push(c);
                }
            }
            head(ui, "Save");
            if item(ui, icons::SAVE, "Save as…", "Ctrl+S", dirty, "Nothing to save yet: this photo has no changes.") {
                cmd.push(Cmd::Tool("save"));
            }
            if item(ui, icons::UNDO, "Undo last change", "Ctrl+Z", edits, "No change to undo.") {
                cmd.push(Cmd::StepBack);
            }
            if item(ui, icons::CLOSE, "Throw away changes", "", dirty, "No changes.") {
                cmd.push(Cmd::Reset);
            }
            head(ui, "File");
            if item(ui, icons::EXPLORER, "Show in Explorer", "", true, "") {
                cmd.push(Cmd::Explorer);
            }
            if item(ui, icons::PRINT, "Print…", "Ctrl+P", true, "") {
                cmd.push(Cmd::Print);
            }
            if item(ui, icons::WALLPAPER, "Set as desktop wallpaper…", "", true, "") {
                cmd.push(Cmd::Wallpaper);
            }
            if item(ui, icons::INFO, "Information (EXIF)", "I", true, "") {
                cmd.push(Cmd::Info);
            }
            if item(ui, icons::CLOSE, "Back to the browser", "Esc", true, "") {
                cmd.push(Cmd::Close);
            }
        });
    }

    fn bottom_bar(
        &mut self,
        ui: &mut egui::Ui,
        app: &App,
        (number, total): (usize, usize),
        pct: &str,
        cmd: &mut Vec<Cmd>,
    ) {
        let dirty = app.editor.dirty();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let b = |ui: &mut egui::Ui, text: &str, tip: &str, works: bool| -> bool {
                let r =
                    ui.add_enabled(works, egui::Button::new(RichText::new(text).size(18.0)).min_size(vec2(38.0, 36.0)));
                if works {
                    r.on_hover_text(tip).clicked()
                } else {
                    r.on_disabled_hover_text(tip);
                    false
                }
            };
            let sep = |ui: &mut egui::Ui| {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
            };
            if b(ui, icons::BACK, "Previous (←, PageUp, wheel up)", true) {
                cmd.push(Cmd::Nav(Nav::By(-1)));
            }
            ui.label(RichText::new(format!("{number} / {total}")).size(15.0));
            if b(ui, icons::FORWARD, "Next (→, PageDown, wheel down)", true) {
                cmd.push(Cmd::Nav(Nav::By(1)));
            }
            sep(ui);
            if b(ui, icons::ZOOM_OUT, "Zoom out (−, Ctrl+wheel)", true) {
                cmd.push(Cmd::Zoom(0.8));
            }
            ui.label(RichText::new(pct).size(15.0)).on_hover_text("Zoom · keys 2–9 = 200–900 %");
            if b(ui, icons::ZOOM_IN, "Zoom in (+, Ctrl+wheel)", true) {
                cmd.push(Cmd::Zoom(1.25));
            }
            if b(ui, "1:1", "Actual size (1)", true) {
                cmd.push(Cmd::Real);
            }
            if b(ui, icons::FULLSCREEN, "Fit to screen (0)", true) {
                cmd.push(Cmd::Fit);
            }
            sep(ui);
            if b(ui, icons::ROTATE_LEFT, "Rotate left (L)", true) {
                cmd.push(Cmd::Edit(Edit::RotateLeft));
            }
            if b(ui, icons::ROTATE_RIGHT, "Rotate right (R)", true) {
                cmd.push(Cmd::Edit(Edit::RotateRight));
            }
            if b(ui, icons::CROP, "Crop (X)", true) {
                cmd.push(Cmd::Tool("crop"));
            }
            if b(ui, icons::RESIZE, "Resize (Ctrl+R)", true) {
                cmd.push(Cmd::Tool("resize"));
            }
            if b(ui, icons::SAVE, if dirty { "Save as (Ctrl+S)" } else { "Save as (Ctrl+S) — no changes yet" }, dirty)
            {
                cmd.push(Cmd::Tool("save"));
            }
            sep(ui);
            if b(ui, icons::INFO, "Information (I)", true) {
                cmd.push(Cmd::Info);
            }
            if b(ui, icons::EXPLORER, "Show in Explorer", true) {
                cmd.push(Cmd::Explorer);
            }
            if b(ui, icons::CLOSE, "Back to the browser (Esc)", true) {
                cmd.push(Cmd::Close);
            }
        });
    }
}

/// The picture turned by `angle` (radians) about its centre: the straighten preview.
/// The magnifier: a square lens around `at` showing the photo `f` times bigger than on screen.
fn paint_lens(p: &egui::Painter, tex: egui::TextureId, shown: Rect, at: Pos2, f: f32) {
    const SIDE: f32 = 300.0;
    let lens = Rect::from_center_size(at, vec2(SIDE, SIDE));
    let (u, v) = ((at.x - shown.min.x) / shown.width(), (at.y - shown.min.y) / shown.height());
    let (hu, hv) = (SIDE / 2.0 / (shown.width() * f), SIDE / 2.0 / (shown.height() * f));
    // Only the part that is inside the photo (no stretched edge pixels).
    let (u0, u1, v0, v1) = ((u - hu).max(0.0), (u + hu).min(1.0), (v - hv).max(0.0), (v + hv).min(1.0));
    let to_x = |x: f32| lens.min.x + (x - (u - hu)) / (2.0 * hu) * SIDE;
    let to_y = |y: f32| lens.min.y + (y - (v - hv)) / (2.0 * hv) * SIDE;
    p.rect_filled(lens.expand(3.0), 4.0, Color32::BLACK);
    p.image(
        tex,
        Rect::from_min_max(pos2(to_x(u0), to_y(v0)), pos2(to_x(u1), to_y(v1))),
        Rect::from_min_max(pos2(u0, v0), pos2(u1, v1)),
        Color32::WHITE,
    );
    p.rect_stroke(lens.expand(3.0), 4.0, egui::Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Outside);
    p.text(
        lens.left_bottom() + vec2(8.0, -8.0),
        Align2::LEFT_BOTTOM,
        format!("{f:.0}×"),
        FontId::proportional(13.0),
        Color32::WHITE,
    );
}

fn paint_turned(p: &egui::Painter, tex: egui::TextureId, rect: Rect, angle: f32) {
    let mut mesh = egui::Mesh::with_texture(tex);
    mesh.add_rect_with_uv(rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    let (s, c) = angle.sin_cos();
    let o = rect.center();
    for v in &mut mesh.vertices {
        let d = v.pos - o;
        v.pos = o + vec2(d.x * c - d.y * s, d.x * s + d.y * c);
    }
    p.add(egui::Shape::mesh(mesh));
}

/// A grid to line the horizon up with while straightening.
fn paint_grid(p: &egui::Painter, r: Rect) {
    let st = egui::Stroke::new(1.0, Color32::from_white_alpha(70));
    for i in 1..8 {
        let t = i as f32 / 8.0;
        p.line_segment([pos2(r.min.x + r.width() * t, r.min.y), pos2(r.min.x + r.width() * t, r.max.y)], st);
        p.line_segment([pos2(r.min.x, r.min.y + r.height() * t), pos2(r.max.x, r.min.y + r.height() * t)], st);
    }
}
