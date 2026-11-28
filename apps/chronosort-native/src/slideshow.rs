//! The slideshow (key S in the browser and the viewer), like the Tauri app's: a settings card first, then the
//! photos (and videos) full screen with a transition, optional music, captions, and controls that appear
//! when the mouse moves. The screen is kept awake while it plays. Nothing is changed on disk; the options
//! are remembered in `ChronoSortData\settings\slideshow.json`.
//!
//! Keys while it plays: Space pause · ← → back / next · + − faster / slower · M music on / off ·
//! Home from the start · Esc stop.

use crate::browse::{Entry, Kind};
use crate::loader::Key;
use crate::player::{Music, Player};
use crate::{App, Tex, bold, colors, icons};
use egui::{Color32, FontId, Id, Order, Rect, RichText, Vec2, pos2, vec2};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const TRANSITION: f32 = 0.9;
const FULL_SIDE: u32 = 4096;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Transition {
    Fade,
    Slide,
    Zoom,
    Random,
    None,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Videos {
    Play,
    Still,
    Skip,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Caption {
    None,
    Name,
    Date,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Options {
    pub seconds: u32,
    pub transition: Transition,
    pub shuffle: bool,
    pub repeat: bool,
    pub videos: Videos,
    pub sound: bool,
    pub caption: Caption,
    pub music: Vec<PathBuf>,
    pub volume: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            seconds: 5,
            transition: Transition::Fade,
            shuffle: false,
            repeat: true,
            videos: Videos::Play,
            sound: true,
            caption: Caption::None,
            music: Vec::new(),
            volume: 70,
        }
    }
}

#[derive(PartialEq, Eq)]
enum Phase {
    Setup,
    Play,
    End,
}

pub struct Slideshow {
    items: Vec<Entry>,
    start: usize,
    opt: Options,
    phase: Phase,
    order: Vec<usize>,
    pos: usize,
    paused: bool,
    /// When the current slide's picture appeared (its time runs from then).
    shown_at: Option<Instant>,
    paused_left: Option<Duration>,
    /// The slide before, fading out, and when the change started.
    prev: Option<(egui::TextureHandle, Instant, Transition)>,
    kind: Transition,
    last_tex: Option<egui::TextureHandle>,
    last_move: Instant,
    last_pointer: Option<egui::Pos2>,
    started_full: bool,
    was_fullscreen: bool,
    music: Option<Music>,
    player: Option<Player>,
    note: String,
    last_frame: Instant,
    // What the keys and buttons asked for this frame (carried out in one place).
    paused_toggle_wanted: bool,
    step_wanted: isize,
    speed_wanted: f32,
    music_toggle_wanted: bool,
    home_wanted: bool,
    close_wanted: bool,
}

pub enum Outcome {
    Running,
    /// Closed; the item shown last (to select it in the browser).
    Closed(Option<PathBuf>),
}

impl Slideshow {
    pub fn new(items: Vec<Entry>, start: usize, opt: Options) -> Self {
        let mut opt = opt;
        opt.music.retain(|p| p.is_file()); // songs from last time that are gone
        Self {
            items,
            start,
            opt,
            phase: Phase::Setup,
            order: Vec::new(),
            pos: 0,
            paused: false,
            shown_at: None,
            paused_left: None,
            prev: None,
            kind: Transition::Fade,
            last_tex: None,
            last_move: Instant::now(),
            last_pointer: None,
            started_full: false,
            was_fullscreen: false,
            music: None,
            player: None,
            note: String::new(),
            last_frame: Instant::now(),
            paused_toggle_wanted: false,
            step_wanted: 0,
            speed_wanted: 0.0,
            music_toggle_wanted: false,
            home_wanted: false,
            close_wanted: false,
        }
    }

    fn shown(&self) -> Vec<usize> {
        (0..self.items.len())
            .filter(|&i| !(self.opt.videos == Videos::Skip && self.items[i].kind == Kind::Video))
            .collect()
    }

    fn count_text(&self) -> String {
        let v = self.items.iter().filter(|e| e.kind == Kind::Video).count();
        let p = self.items.len() - v;
        let ps = format!("{p} photo{}", if p == 1 { "" } else { "s" });
        let vs = format!("{v} video{}", if v == 1 { "" } else { "s" });
        match (v, self.opt.videos) {
            (0, _) => ps,
            (_, Videos::Skip) => format!("{ps} ({vs} left out)"),
            _ if p == 0 => vs,
            _ => format!("{ps} and {vs}"),
        }
    }

    fn cur(&self) -> Option<&Entry> {
        self.order.get(self.pos).and_then(|&i| self.items.get(i))
    }

    fn plays_video(&self) -> bool {
        self.cur().is_some_and(|e| e.kind == Kind::Video) && self.opt.videos == Videos::Play
    }

    fn begin(&mut self, app: &App, ctx: &egui::Context) {
        let shown = self.shown();
        if shown.is_empty() {
            self.note = "There is nothing to show here.".into();
            return;
        }
        crate::settings::save(&app.data_root, "slideshow", &self.opt);
        let first = shown.iter().position(|&i| i == self.start).unwrap_or(0);
        let mut order = shown.clone();
        if self.opt.shuffle {
            // A simple shuffle (xorshift from the clock); the chosen photo stays first.
            let mut seed =
                Instant::now().elapsed().as_nanos() as u64 ^ 0x9E37_79B9_7F4A_7C15 ^ (shown.len() as u64) << 17;
            for i in (1..order.len()).rev() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                order.swap(i, (seed % (i as u64 + 1)) as usize);
            }
            let f = shown[first];
            order.retain(|&x| x != f);
            order.insert(0, f);
        } else {
            order.rotate_left(first);
        }
        self.order = order;
        self.pos = 0;
        self.paused = false;
        self.phase = Phase::Play;
        self.pick_kind();
        self.enter_slide();
        let test = std::env::var_os("CHRONOSORT_DRIVE").is_some();
        if !test {
            self.was_fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            self.started_full = true;
        }
        cs_platform::keep_awake(true);
        self.music = (!self.opt.music.is_empty())
            .then(|| Music::new(self.opt.music.clone(), self.opt.volume as f32 / 100.0, test));
    }

    /// For the test driver.
    pub fn state_text(&self) -> String {
        let phase = match self.phase {
            Phase::Setup => "setup",
            Phase::Play => "play",
            Phase::End => "end",
        };
        let name = self.cur().map(|e| e.name.clone()).unwrap_or_default();
        format!(
            "show.phase={phase}
show.pos={}
show.count={}
show.showing={name}
show.paused={}
show.seconds={}",
            self.pos + 1,
            self.order.len(),
            self.paused,
            self.opt.seconds
        )
    }

    fn pick_kind(&mut self) {
        self.kind = match self.opt.transition {
            Transition::Random => [Transition::Fade, Transition::Slide, Transition::Zoom]
                [(Instant::now().elapsed().subsec_nanos() % 3) as usize],
            t => t,
        };
    }

    /// A new slide is on: its time starts when its picture is there.
    fn enter_slide(&mut self) {
        self.shown_at = None;
        self.paused_left = None;
        self.player = None;
    }

    fn step(&mut self, d: isize) {
        if self.order.is_empty() {
            return;
        }
        let mut p = self.pos as isize + d;
        if p >= self.order.len() as isize {
            if !self.opt.repeat {
                self.finish();
                return;
            }
            p = 0;
        }
        if p < 0 {
            p = self.order.len() as isize - 1;
        }
        if let Some(t) = self.last_tex.clone() {
            self.pick_kind();
            self.prev = Some((t, Instant::now(), self.kind));
        }
        self.pos = p as usize;
        self.enter_slide();
    }

    fn finish(&mut self) {
        self.phase = Phase::End;
        self.player = None;
        if let Some(m) = &mut self.music {
            m.fade_out();
        }
        cs_platform::keep_awake(false);
    }

    fn toggle_pause(&mut self) {
        if self.phase != Phase::Play {
            return;
        }
        self.paused = !self.paused;
        if let Some(p) = &mut self.player {
            p.set_paused(self.paused);
        }
        if self.paused {
            self.paused_left =
                self.shown_at.map(|t| Duration::from_secs(u64::from(self.opt.seconds)).saturating_sub(t.elapsed()));
        } else if let Some(left) = self.paused_left.take() {
            // Continue with the time that was left.
            self.shown_at =
                Some(Instant::now() - Duration::from_secs(u64::from(self.opt.seconds)).saturating_sub(left));
        }
    }

    fn speed(&mut self, f: f32) {
        self.opt.seconds = ((self.opt.seconds as f32 * f).round() as u32).clamp(1, 60);
    }

    fn close(&mut self, app: &App, ctx: &egui::Context) -> Outcome {
        cs_platform::keep_awake(false);
        if self.started_full && !self.was_fullscreen {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        crate::settings::save(&app.data_root, "slideshow", &self.opt);
        Outcome::Closed(self.cur().map(|e| e.path.clone()))
    }

    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut App) -> Outcome {
        let ctx = ui.ctx().clone();
        if self.phase == Phase::Setup {
            return self.setup(&ctx, app);
        }
        let dt = self.last_frame.elapsed().as_secs_f32().min(0.2);
        self.last_frame = Instant::now();
        let screen = ui.max_rect();
        ui.painter().rect_filled(screen, 0.0, Color32::BLACK);
        let resp = ui.allocate_rect(screen, egui::Sense::click());

        // ---------- keys ----------
        let mut out = None;
        ctx.input(|i| {
            use egui::Key::*;
            if i.key_pressed(Escape) {
                out = Some(());
            }
            if i.key_pressed(Space) || i.key_pressed(K) {
                self.paused_toggle_wanted = true;
            }
            if i.key_pressed(ArrowRight) || i.key_pressed(PageDown) {
                self.step_wanted = 1;
            }
            if i.key_pressed(ArrowLeft) || i.key_pressed(PageUp) {
                self.step_wanted = -1;
            }
            if i.key_pressed(Plus) || i.key_pressed(Equals) {
                self.speed_wanted = 0.75;
            }
            if i.key_pressed(Minus) {
                self.speed_wanted = 1.35;
            }
            if i.key_pressed(M) {
                self.music_toggle_wanted = true;
            }
            if i.key_pressed(Home) {
                self.home_wanted = true;
            }
            if let Some(p) = i.pointer.hover_pos()
                && self.last_pointer != Some(p)
            {
                self.last_pointer = Some(p);
                self.last_move = Instant::now();
            }
        });
        if out.is_some() {
            return self.close(app, &ctx);
        }
        self.apply_wants();

        // ---------- the current slide ----------
        let Some(entry) = self.cur().cloned() else { return self.close(app, &ctx) };
        let is_video = entry.kind == Kind::Video;
        let mut tex: Option<egui::TextureHandle> = None;
        if self.phase == Phase::Play && is_video && self.opt.videos == Videos::Play {
            if self.player.as_ref().is_none_or(|p| p.path != entry.path) {
                let side = (screen.width().max(screen.height()) * ctx.pixels_per_point()) as u32;
                let silent = std::env::var_os("CHRONOSORT_DRIVE").is_some() || !self.opt.sound;
                let mut p = Player::open(&entry.path, side.clamp(640, 3840), silent);
                p.set_paused(self.paused);
                self.player = Some(p);
            }
            if let Some(p) = &mut self.player {
                p.update(&ctx);
                tex = p.tex.clone();
                if p.ended && !self.paused {
                    self.step(1);
                }
            }
        }
        if tex.is_none() {
            let full = Key { path: entry.path.clone(), size: FULL_SIDE, full: true };
            let preview = Key { path: entry.path.clone(), size: cs_thumb::PREVIEW_SIZE, full: false };
            let key = if is_video { &preview } else { &full };
            match app.texture(key) {
                Some(Tex::Ready(t, _)) => tex = Some(t.clone()),
                Some(Tex::Failed(_)) => {
                    // No picture: still give it its time (the next slide comes).
                    if self.shown_at.is_none() {
                        self.shown_at = Some(Instant::now());
                    }
                }
                None => {
                    let _ = app.texture(&preview);
                    if let Some(Tex::Ready(t, _)) = app.textures.get(&preview) {
                        tex = Some(t.clone());
                    }
                }
            }
            // Decode the next photo ahead.
            if let Some(next) =
                self.order.get((self.pos + 1) % self.order.len().max(1)).and_then(|&i| self.items.get(i))
                && next.kind != Kind::Video
            {
                let _ = app.texture(&Key { path: next.path.clone(), size: FULL_SIDE, full: true });
            }
        }
        if tex.is_some() && self.shown_at.is_none() && !self.paused {
            self.shown_at = Some(Instant::now());
        }
        // Time's up (photos, stills; a playing video moves on when it ends).
        let plays = self.plays_video() && self.player.is_some();
        if self.phase == Phase::Play
            && !self.paused
            && !plays
            && self.shown_at.is_some_and(|t| t.elapsed() >= Duration::from_secs(u64::from(self.opt.seconds)))
        {
            self.step(1);
        }

        // ---------- draw: the slide going out, then the one coming in ----------
        let since = |t: Instant| (t.elapsed().as_secs_f32() / TRANSITION).min(1.0);
        let mut t_in = 1.0;
        if let Some((old, at, kind)) = &self.prev {
            let t = since(*at);
            t_in = t;
            if t < 1.0 {
                let rect = crate::fit_fill(old.size_vec2(), screen);
                let (r, alpha) = match kind {
                    Transition::Slide => (rect.translate(vec2(-screen.width() * ease(t), 0.0)), 1.0),
                    Transition::None => (rect, 0.0),
                    _ => (rect, 1.0 - t),
                };
                paint(ui.painter(), old, r, alpha);
            }
        }
        if self.prev.as_ref().is_some_and(|(_, at, _)| since(*at) >= 1.0) {
            self.prev = None;
        }
        if let Some(t) = &tex {
            let mut rect = crate::fit_fill(t.size_vec2(), screen);
            let kind = self.prev.as_ref().map_or(self.kind, |p| p.2);
            let alpha = match kind {
                Transition::None => 1.0,
                Transition::Slide => 1.0,
                _ => t_in,
            };
            if kind == Transition::Slide && t_in < 1.0 {
                rect = rect.translate(vec2(screen.width() * (1.0 - ease(t_in)), 0.0));
            }
            // Zoom: comes in slightly bigger, then drifts slowly (Ken Burns) while it is shown.
            if kind == Transition::Zoom || self.opt.transition == Transition::Zoom {
                let drift = self
                    .shown_at
                    .map_or(0.0, |s| (s.elapsed().as_secs_f32() / (self.opt.seconds as f32 + 1.0)).min(1.0));
                let s = if t_in < 1.0 { 1.15 - 0.15 * ease(t_in) } else { 1.0 } + 0.07 * drift;
                rect = Rect::from_center_size(rect.center(), rect.size() * s);
            }
            paint(ui.painter(), t, rect, alpha);
            self.last_tex = Some(t.clone());
        }

        // ---------- caption ----------
        let controls =
            self.paused || self.phase == Phase::End || self.last_move.elapsed() < Duration::from_millis(2500);
        if self.opt.caption != Caption::None {
            let mut text = entry.name.clone();
            if self.opt.caption == Caption::Date && entry.modified > 0 {
                let (y, m, d, ..) = cs_model::civil::from_unix(entry.modified);
                const MONTHS: [&str; 12] = [
                    "January",
                    "February",
                    "March",
                    "April",
                    "May",
                    "June",
                    "July",
                    "August",
                    "September",
                    "October",
                    "November",
                    "December",
                ];
                text += &format!("  ·  {d} {} {y}", MONTHS[usize::from(m.clamp(1, 12)) - 1]);
            }
            let y = if controls { screen.max.y - 110.0 } else { screen.max.y - 40.0 };
            for (off, c) in [(vec2(1.5, 1.5), Color32::from_black_alpha(220)), (Vec2::ZERO, Color32::WHITE)] {
                ui.painter().text(
                    pos2(screen.center().x, y) + off,
                    egui::Align2::CENTER_CENTER,
                    &text,
                    FontId::proportional(22.0),
                    c,
                );
            }
        }

        // ---------- music ----------
        let duck = self.plays_video() && self.opt.sound;
        if let Some(m) = &mut self.music {
            m.set(m.on, self.paused, duck);
            m.update(dt);
        }

        // ---------- the end card, the controls ----------
        let mut close = false;
        if self.phase == Phase::End {
            egui::Area::new(Id::unique("show-end"))
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .order(Order::Middle)
                .show(&ctx, |ui| {
                    egui::Frame::new()
                        .fill(Color32::from_rgba_premultiplied(20, 22, 27, 235))
                        .corner_radius(14)
                        .inner_margin(24.0)
                        .show(ui, |ui| {
                            ui.label(RichText::new("The end").font(bold(28.0)).color(Color32::WHITE));
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                if ui.button(RichText::new(format!("{}  Play again", icons::PLAY)).size(17.0)).clicked()
                                {
                                    self.home_wanted = true;
                                    self.phase = Phase::Play;
                                    cs_platform::keep_awake(true);
                                    if let Some(m) = &mut self.music {
                                        m.restart();
                                    }
                                }
                                if ui
                                    .add(
                                        egui::Button::new(RichText::new("Close").size(17.0).color(Color32::WHITE))
                                            .fill(colors::ACCENT),
                                    )
                                    .clicked()
                                {
                                    close = true;
                                }
                            });
                        });
                });
        }
        if controls {
            self.controls_bar(&ctx, screen);
        } else {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }
        if resp.clicked() && self.phase == Phase::Play {
            self.toggle_pause();
        }
        if close || self.close_wanted {
            return self.close(app, &ctx);
        }
        ctx.request_repaint_after(Duration::from_millis(
            if self.prev.is_some() || self.opt.transition == Transition::Zoom { 16 } else { 100 },
        ));
        Outcome::Running
    }

    fn apply_wants(&mut self) {
        if std::mem::take(&mut self.paused_toggle_wanted) {
            self.toggle_pause();
        }
        match std::mem::take(&mut self.step_wanted) {
            0 => {}
            d => {
                if self.phase == Phase::Play {
                    self.step(d);
                }
            }
        }
        let f = std::mem::take(&mut self.speed_wanted);
        if f > 0.0 {
            self.speed(f);
        }
        if std::mem::take(&mut self.music_toggle_wanted)
            && let Some(m) = &mut self.music
        {
            m.on = !m.on;
        }
        if std::mem::take(&mut self.home_wanted) {
            self.pos = 0;
            self.phase = Phase::Play;
            self.paused = false;
            self.prev = None;
            self.enter_slide();
        }
    }

    fn controls_bar(&mut self, ctx: &egui::Context, screen: Rect) {
        let w = 640.0f32.min(screen.width() - 40.0);
        egui::Area::new(Id::unique("show-controls"))
            .fixed_pos(pos2(screen.center().x - w / 2.0, screen.max.y - 76.0))
            .order(Order::Middle)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgba_premultiplied(20, 22, 27, 225))
                    .corner_radius(12)
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.set_width(w - 16.0);
                        ui.horizontal(|ui| {
                            let b = |ui: &mut egui::Ui, t: &str, tip: &str| {
                                ui.add(egui::Button::new(RichText::new(t).size(18.0)).min_size(vec2(40.0, 36.0)))
                                    .on_hover_text(tip)
                                    .clicked()
                            };
                            if b(ui, icons::BACK, "Back (←)") {
                                self.step_wanted = -1;
                            }
                            let (icon, tip) = if self.paused {
                                (icons::PLAY, "Play (Space)")
                            } else {
                                (icons::PAUSE, "Pause (Space)")
                            };
                            if b(ui, icon, tip) {
                                self.paused_toggle_wanted = true;
                            }
                            if b(ui, icons::FORWARD, "Next (→)") {
                                self.step_wanted = 1;
                            }
                            ui.label(
                                RichText::new(format!("{} / {}", self.pos + 1, self.order.len()))
                                    .size(15.0)
                                    .color(Color32::WHITE),
                            );
                            ui.add_space(8.0);
                            if b(ui, "−", "Slower (−)") {
                                self.speed_wanted = 1.35;
                            }
                            ui.label(RichText::new(format!("{} s", self.opt.seconds)).size(15.0).color(Color32::WHITE));
                            if b(ui, "+", "Faster (+)") {
                                self.speed_wanted = 0.75;
                            }
                            if let Some(m) = &self.music {
                                let label =
                                    if m.on { icons::MUSIC.to_string() } else { format!("{} off", icons::MUSIC) };
                                let tip = m.track_name().map_or("Music on / off (M)".to_string(), |n| {
                                    format!("Music on / off (M) — now: {n}")
                                });
                                if b(ui, &label, &tip) {
                                    self.music_toggle_wanted = true;
                                }
                            }
                            if self.paused {
                                ui.label(RichText::new("Paused").size(15.0).color(colors::MUTED));
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let stop = egui::Button::new(
                                    RichText::new(format!("{}  Stop", icons::CLOSE)).size(16.0).color(Color32::WHITE),
                                )
                                .fill(colors::RED);
                                if ui.add(stop).on_hover_text("Stop the slideshow (Esc)").clicked() {
                                    self.close_wanted = true;
                                }
                            });
                        });
                    });
            });
    }

    fn setup(&mut self, ctx: &egui::Context, app: &App) -> Outcome {
        let mut out = Outcome::Running;
        let mut start = ctx.input(|i| i.key_pressed(egui::Key::Enter)) && !ctx.text_edit_focused();
        let cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        egui::Modal::new(Id::unique("show-setup")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(RichText::new(format!("{}  Slideshow", icons::SLIDESHOW)).font(bold(20.0)));
            ui.label(RichText::new(self.count_text()).color(colors::MUTED));
            ui.add_space(8.0);
            ui.add(egui::Slider::new(&mut self.opt.seconds, 1..=60).suffix(" s").text("Show each photo for"));
            ui.horizontal(|ui| {
                ui.label("Change");
                for (t, l) in [
                    (Transition::Fade, "Fade"),
                    (Transition::Slide, "Slide"),
                    (Transition::Zoom, "Zoom"),
                    (Transition::Random, "Mixed"),
                    (Transition::None, "None"),
                ] {
                    ui.selectable_value(&mut self.opt.transition, t, l);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Videos");
                ui.selectable_value(&mut self.opt.videos, Videos::Play, "Play them");
                ui.selectable_value(&mut self.opt.videos, Videos::Still, "A still picture");
                ui.selectable_value(&mut self.opt.videos, Videos::Skip, "Leave them out");
            });
            if self.opt.videos == Videos::Play {
                ui.checkbox(&mut self.opt.sound, "Video sound on");
            }
            ui.horizontal(|ui| {
                ui.label("Caption");
                ui.selectable_value(&mut self.opt.caption, Caption::None, "None");
                ui.selectable_value(&mut self.opt.caption, Caption::Name, "File name");
                ui.selectable_value(&mut self.opt.caption, Caption::Date, "Name and date");
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.opt.shuffle, "Random order");
                ui.checkbox(&mut self.opt.repeat, "Start again at the end");
            });
            ui.add_space(6.0);
            ui.label(RichText::new("MUSIC").font(bold(12.0)).color(colors::MUTED));
            ui.horizontal_wrapped(|ui| {
                if self.opt.music.is_empty() {
                    ui.label(RichText::new("No music").color(colors::MUTED));
                } else {
                    let names: Vec<String> = self
                        .opt
                        .music
                        .iter()
                        .filter_map(|p| p.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                        .collect();
                    ui.label(format!("{} {}", icons::MUSIC, names.join(", ")));
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Choose music…").clicked()
                    && let Some(files) = rfd::FileDialog::new()
                        .set_title("Choose music for the slideshow (one or more songs)")
                        .add_filter("Music", &["mp3", "m4a", "aac", "wav", "wma", "flac", "ogg", "opus"])
                        .pick_files()
                {
                    self.opt.music = files;
                }
                if !self.opt.music.is_empty() && ui.button("No music").clicked() {
                    self.opt.music.clear();
                }
                ui.label(RichText::new("MP3, M4A, WAV, WMA or FLAC from this PC").size(12.5).color(colors::MUTED));
            });
            if !self.opt.music.is_empty() {
                ui.add(egui::Slider::new(&mut self.opt.volume, 0..=100).suffix(" %").text("Music volume"));
            }
            if !self.note.is_empty() {
                ui.colored_label(colors::RED, &self.note);
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "While it plays: Space pause · ← → back / next · + − faster / slower · M music on/off · Esc stop",
                )
                .size(12.5)
                .color(colors::MUTED),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    out = Outcome::Closed(None);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(format!("{}  Start  (Enter)", icons::PLAY)).color(Color32::WHITE),
                            )
                            .fill(colors::ACCENT),
                        )
                        .clicked()
                    {
                        start = true;
                    }
                });
            });
        });
        if cancel {
            out = Outcome::Closed(None);
        }
        if start && matches!(out, Outcome::Running) {
            self.begin(app, ctx);
        }
        out
    }
}

fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn paint(p: &egui::Painter, t: &egui::TextureHandle, r: Rect, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let tint = Color32::from_white_alpha((alpha.clamp(0.0, 1.0) * 255.0) as u8);
    p.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
}
