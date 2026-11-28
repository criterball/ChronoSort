//! ChronoSort with a native Rust UI (egui): no WebView2, no browser engine.
//!
//! Prototype step 1 (2026-11-27): File browser (folder tree + thumbnail grid S/M/L) and viewer
//! (fit / zoom / pan / next / previous). Everything ChronoSort writes stays in `ChronoSortData`
//! beside the program. Start with a folder: `ChronoSortNative.exe D:\Photos`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod batchdlg;
mod browse;
mod browser;
mod compare;
mod createdlg;
mod data;
mod drive;
mod editor;
mod histo;
mod icons;
mod infopanel;
mod loader;
mod marks;
mod player;
mod printdlg;
mod raster;
mod settings;
mod sheet;
mod slideshow;
mod viewer;
mod walldlg;

use browse::{Kind, Listing};
use egui::{Color32, FontId, Key, Rect, Vec2, vec2};
use loader::Loader;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

/// Most thumbnails kept on the graphics card at once (older ones are dropped and reloaded from the cache).
const MAX_THUMBS: usize = 1500;

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThumbSize {
    S,
    M,
    L,
}

impl ThumbSize {
    /// Cell side in points.
    fn cell(self) -> f32 {
        match self {
            ThumbSize::S => 120.0,
            ThumbSize::M => 180.0,
            ThumbSize::L => 270.0,
        }
    }
    /// Cached thumbnail size: the Tauri app's sizes, so both share the cache.
    fn pixels(self) -> u32 {
        match self {
            ThumbSize::S | ThumbSize::M => cs_thumb::DEFAULT_SIZE,
            ThumbSize::L => 640,
        }
    }
}

pub enum Tex {
    Ready(egui::TextureHandle, Option<(u32, u32)>),
    Failed(String),
}

pub mod colors {
    use egui::Color32;
    pub const BG: Color32 = Color32::from_rgb(0x1f, 0x21, 0x24);
    pub const PANEL: Color32 = Color32::from_rgb(0x2a, 0x2d, 0x31);
    pub const CELL: Color32 = Color32::from_rgb(0x33, 0x36, 0x3b);
    pub const CELL_HOVER: Color32 = Color32::from_rgb(0x3d, 0x41, 0x47);
    pub const ACCENT: Color32 = Color32::from_rgb(0x3b, 0x82, 0xf6);
    pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xea, 0xed);
    pub const MUTED: Color32 = Color32::from_rgb(0x9a, 0xa0, 0xa6);
    pub const RED: Color32 = Color32::from_rgb(0xc6, 0x28, 0x28);
    pub const FOLDER: Color32 = Color32::from_rgb(0xe8, 0xb5, 0x3c);
    pub const GREEN: Color32 = Color32::from_rgb(0x4a, 0xc2, 0x7a);
    /// Stars.
    pub const GOLD: Color32 = Color32::from_rgb(0xff, 0xc8, 0x3d);
    /// The tag mark.
    pub const TAG: Color32 = Color32::from_rgb(0x8a, 0xb4, 0xff);
}

struct App {
    data_root: PathBuf,
    loader: Loader,
    textures: HashMap<loader::Key, Tex>,
    tex_order: VecDeque<loader::Key>,
    listing: Option<Listing>,
    error: Option<String>,
    back: Vec<PathBuf>,
    prefs: browser::Prefs,
    selected: Option<usize>,
    /// The picked items (several with Ctrl / Shift), and where a Shift range starts.
    picked: std::collections::BTreeSet<usize>,
    anchor: Option<usize>,
    forward: Vec<PathBuf>,
    /// Copied or cut items, waiting for Ctrl+V.
    clip: Option<(Vec<PathBuf>, cs_app::fileops::Mode)>,
    job: Option<browser::Job>,
    dialog: Option<browser::Dialog>,
    /// Sample photos for folder icons, and their worker channel.
    samples: HashMap<PathBuf, Vec<PathBuf>>,
    samples_tx: std::sync::mpsc::Sender<(PathBuf, Vec<PathBuf>)>,
    samples_rx: std::sync::mpsc::Receiver<(PathBuf, Vec<PathBuf>)>,
    /// Columns of the current view (for the arrow keys).
    cols: usize,
    ctx: egui::Context,
    /// Open the tree down to this folder (after navigating).
    tree_reveal: Option<PathBuf>,
    scroll_to: Option<usize>,
    last_scroll: (f32, f32),
    tree: HashMap<PathBuf, Vec<(String, PathBuf)>>,
    places: Vec<(&'static str, String, PathBuf)>,
    drives: Vec<(&'static str, String, PathBuf)>,
    nav: Option<PathBuf>,
    viewer: Option<viewer::Viewer>,
    editor: editor::Editor,
    /// A message in the status bar (Undo results), with when it was shown.
    note: Option<(String, std::time::Instant)>,
    /// A running Undo (file operations block, so they run on a thread).
    undo_rx: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    /// Test driver (only with CHRONOSORT_DRIVE set).
    driver: Option<drive::Driver>,
    /// The video playing in the viewer.
    player: Option<player::Player>,
    slideshow: Option<slideshow::Slideshow>,
    /// Compare 2–4 pictures side by side (P).
    compare: Option<compare::Compare>,
    /// Stars and tags of the shown folder, and the browser's filters (this session only).
    marks: marks::Marks,
    star_filter: marks::StarFilter,
    name_filter: String,
    /// Stars changed while the viewer was open: filter and sort again once it closes.
    refilter_pending: bool,
    /// Histograms of viewer previews (Ctrl+H), and whether the box is shown.
    hists: HashMap<PathBuf, Box<histo::Histo>>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, data: data::DataDirs, start: Option<PathBuf>) -> Self {
        style(&cc.egui_ctx);
        let places = cs_platform::known_folders()
            .into_iter()
            .map(|k| {
                let (icon, name) = match k.id {
                    "pictures" => (icons::PICTURES, "Pictures"),
                    "desktop" => (icons::DESKTOP, "Desktop"),
                    "downloads" => (icons::DOWNLOADS, "Downloads"),
                    "videos" => (icons::VIDEOS, "Videos"),
                    "documents" => (icons::DOCUMENTS, "Documents"),
                    "home" => (icons::HOME, "My folder"),
                    other => (icons::FOLDER, other),
                };
                (icon, name.to_string(), k.path)
            })
            .collect();
        let drives = cs_platform::volumes()
            .into_iter()
            .map(|v| {
                let root = browse::shown(&v.path);
                let name = if v.label.is_empty() {
                    root.clone()
                } else {
                    format!("{} ({})", v.label, root.trim_end_matches('\\'))
                };
                let icon = match v.kind {
                    cs_platform::VolumeKind::Removable => icons::USB,
                    cs_platform::VolumeKind::Network => icons::NETWORK,
                    cs_platform::VolumeKind::Optical => icons::DISC,
                    _ => icons::DRIVE,
                };
                (icon, name, v.path)
            })
            .collect();
        let (samples_tx, samples_rx) = std::sync::mpsc::channel();
        let data_root = data.root.clone();
        let mut app = Self {
            data_root,
            loader: Loader::new(data.cache, cc.egui_ctx.clone()),
            textures: HashMap::new(),
            tex_order: VecDeque::new(),
            listing: None,
            error: None,
            back: Vec::new(),
            prefs: settings::load(&data.root, "browser"),
            selected: None,
            picked: Default::default(),
            anchor: None,
            forward: Vec::new(),
            clip: None,
            job: None,
            dialog: None,
            samples: HashMap::new(),
            samples_tx,
            samples_rx,
            cols: 1,
            ctx: cc.egui_ctx.clone(),
            tree_reveal: None,
            scroll_to: None,
            last_scroll: (0.0, 0.0),
            tree: HashMap::new(),
            places,
            drives,
            nav: None,
            viewer: None,
            editor: editor::Editor::new(cc.egui_ctx.clone()),
            note: None,
            undo_rx: None,
            driver: drive::Driver::from_env(),
            player: None,
            slideshow: None,
            compare: None,
            marks: marks::Marks::new(&data.root),
            star_filter: marks::StarFilter::Any,
            name_filter: String::new(),
            refilter_pending: false,
            hists: HashMap::new(),
        };
        // The folder named on the command line, else the one open last time (not in test mode).
        let last =
            app.prefs.last_folder.clone().filter(|p| p.is_dir() && std::env::var_os("CHRONOSORT_DRIVE").is_none());
        if let Some(p) = start.or(last) {
            app.open_folder(&p, false);
        }
        app
    }

    fn open_folder(&mut self, p: &Path, remember: bool) {
        match browse::list(p, self.prefs.hidden) {
            Ok(l) => {
                if remember && let Some(cur) = &self.listing {
                    self.back.push(cur.path.clone());
                    self.forward.clear();
                }
                self.loader.new_generation();
                self.tree_reveal = Some(l.path.clone());
                self.prefs.last_folder = Some(l.path.clone());
                crate::settings::save(&self.data_root, "browser", &self.prefs);
                self.listing = Some(l);
                self.name_filter.clear();
                self.picked.clear();
                self.sort_listing();
                self.load_marks();
                let empty = self.listing.as_ref().is_none_or(|l| l.entries.is_empty());
                self.selected = if empty { None } else { Some(0) };
                self.anchor = self.selected;
                self.scroll_to = Some(0);
                self.error = None;
                self.viewer = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// A file changed on disk (saved over, undone): drop its pictures so they load fresh.
    pub fn forget_pictures(&mut self, p: &Path) {
        self.textures.retain(|k, _| k.path != p);
        self.tex_order.retain(|k| k.path != p);
    }

    /// List the shown folder again (files were added or changed); returns the new index of `keep`.
    pub fn reload_listing(&mut self, keep: Option<&Path>) -> Option<usize> {
        let path = self.listing.as_ref()?.path.clone();
        let l = browse::list(&path, self.prefs.hidden).ok()?;
        let sel = self.selected.and_then(|i| self.listing.as_ref()?.entries.get(i).map(|e| e.path.clone()));
        self.listing = Some(l);
        self.picked.clear();
        self.selected = None;
        self.sort_listing();
        self.load_marks();
        let empty = self.listing.as_ref().is_none_or(|l| l.entries.is_empty());
        let find = |p: &Path| self.listing.as_ref()?.entries.iter().position(|e| e.path == p);
        self.selected = sel.as_deref().and_then(find).or(if empty { None } else { Some(0) });
        self.anchor = self.selected;
        keep.and_then(find)
    }

    /// Read the stars of the shown folder's photos (worker thread).
    fn load_marks(&mut self) {
        let Some(l) = &self.listing else { return };
        let files: Vec<PathBuf> =
            l.all.iter().filter(|e| matches!(e.kind, Kind::Image | Kind::Raw)).map(|e| e.path.clone()).collect();
        let folder = l.path.clone();
        self.marks.load(&folder, files, &self.ctx);
    }

    /// The slideshow of the shown folder's photos and videos, starting at entry `start`.
    pub fn start_slideshow(&mut self, start: usize) {
        let Some(l) = &self.listing else { return };
        let items: Vec<browse::Entry> =
            l.entries.iter().filter(|e| matches!(e.kind, Kind::Image | Kind::Raw | Kind::Video)).cloned().collect();
        if items.is_empty() {
            self.note = Some(("No photos or videos here for a slideshow.".into(), std::time::Instant::now()));
            return;
        }
        let first = l.entries.get(start).and_then(|e| items.iter().position(|x| x.path == e.path)).unwrap_or(0);
        self.slideshow = Some(slideshow::Slideshow::new(items, first, settings::load(&self.data_root, "slideshow")));
    }

    /// For the test driver: what the app shows right now.
    fn write_state(&self) {
        let Some(d) = &self.driver else { return };
        let mut lines: Vec<String> = Vec::new();
        if let Some(l) = &self.listing {
            lines.push(format!("folder={}", browse::shown(&l.path)));
            lines.push(format!("entries={}", l.entries.len()));
            if let Some(e) = self.selected.and_then(|i| l.entries.get(i)) {
                lines.push(format!("selected={}", e.name));
            }
            lines.push(format!("picked={}", self.picked_entries().len()));
            lines.push(format!("view={:?} sort={:?} desc={}", self.prefs.view, self.prefs.sort, self.prefs.desc));
            lines.push(format!("dialog={}", self.dialog.is_some()));
            lines.push(format!("all={} filter={:?} name_filter={}", l.all.len(), self.star_filter, self.name_filter));
            if let Some(e) = self.selected.and_then(|i| l.entries.get(i)) {
                lines.push(format!(
                    "selected.stars={:?} selected.tagged={}",
                    self.marks.stars(&e.path),
                    self.marks.tagged(&e.path)
                ));
            }
            let rated: Vec<String> = l
                .entries
                .iter()
                .filter_map(|e| self.marks.stars(&e.path).filter(|&n| n > 0).map(|n| format!("{}:{n}", e.name)))
                .collect();
            lines.push(format!("rated={}", rated.join(",")));
            lines.push(format!("marks.loading={}", self.marks.loading));
            lines.push(format!("job={}", self.job.is_some()));
        }
        match &self.viewer {
            Some(v) => {
                let name = self
                    .listing
                    .as_ref()
                    .and_then(|l| l.entries.get(v.index))
                    .map(|e| e.name.clone())
                    .unwrap_or_default();
                lines.push("viewer=open".into());
                lines.push(format!("showing={name}"));
            }
            None => lines.push("viewer=closed".into()),
        }
        if let Some(st) = &self.editor.state {
            lines.push(format!("edit.size={}x{}", st.width, st.height));
            lines.push(format!("edit.out={}x{}", st.out_width, st.out_height));
            lines.push(format!("edit.edits={}", st.edits));
            lines.push(format!("edit.dirty={}", self.editor.dirty()));
            lines.push(format!("edit.lossless={}", st.lossless));
            lines.push(format!("edit.effects={}", st.has_effects));
            lines.push(format!("edit.watermark={:?}", st.watermark));
        }
        lines.push(format!("edit.tool={}", self.editor.tool_name()));
        lines.push(format!("edit.busy={}", self.editor.busy()));
        if let Some(p) = &self.player {
            lines.push(format!("video.time={:.2}", p.shown));
            lines.push(format!("video.duration={:.2}", p.duration()));
            lines.push(format!("video.paused={}", p.paused));
            lines.push(format!("video.ended={}", p.ended));
            lines.push(format!("video.frame={:?}", p.tex.as_ref().map(|t| t.size())));
        }
        if let Some(s) = &self.slideshow {
            lines.push(s.state_text());
        }
        if let Some(c) = &self.compare {
            lines.push(c.state_text());
        }
        if let Some((n, _)) = &self.note {
            lines.push(format!("note={n}"));
        }
        let _ = std::fs::write(d.out_file(), lines.join("\n") + "\n");
    }

    /// Ctrl+Z in the browser: undo the newest file operation (a save, a copy …) on a worker thread.
    fn start_undo(&mut self, ctx: &egui::Context) {
        if self.undo_rx.is_some() {
            return;
        }
        let Some(label) = cs_app::fileops::last_undo() else {
            self.note = Some(("Nothing to undo.".into(), std::time::Instant::now()));
            return;
        };
        self.note = Some((format!("Undoing: {label}…"), std::time::Instant::now()));
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(cs_app::fileops::undo());
            ctx.request_repaint();
        });
        self.undo_rx = Some(rx);
    }

    /// Picture for `key` if loaded; asks the loader otherwise.
    pub fn texture(&mut self, key: &loader::Key) -> Option<&Tex> {
        if !self.textures.contains_key(key) {
            self.loader.request(key.clone());
            return None;
        }
        self.textures.get(key)
    }

    fn take_finished(&mut self, ctx: &egui::Context) {
        for d in self.loader.finished() {
            let tex = match d.result {
                Ok(mut p) => {
                    if let Some(h) = p.hist.take() {
                        if self.hists.len() > 64 {
                            self.hists.clear();
                        }
                        self.hists.insert(d.key.path.clone(), h);
                    }
                    let img = egui::ColorImage::from_rgba_unmultiplied([p.width, p.height], &p.rgba);
                    let name = format!("{}@{}", d.key.path.display(), d.key.size);
                    Tex::Ready(ctx.load_texture(name, img, egui::TextureOptions::LINEAR), p.original)
                }
                Err(e) => Tex::Failed(e),
            };
            if !d.key.full {
                self.tex_order.push_back(d.key.clone());
            }
            self.textures.insert(d.key, tex);
        }
        while self.tex_order.len() > MAX_THUMBS {
            if let Some(k) = self.tex_order.pop_front() {
                self.textures.remove(&k);
            }
        }
        // Keep at most a few full-size pictures (the one shown and its neighbours).
        let mut keep: Vec<PathBuf> =
            self.viewer.as_ref().map(|v| v.neighbours(self.listing.as_ref())).unwrap_or_default();
        if let Some(c) = &self.compare {
            keep.extend(c.paths());
        }
        self.textures.retain(|k, _| !k.full || keep.contains(&k.path));
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(d) = &mut self.driver {
            // Keep running while covered by other windows (the user keeps working during a test).
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
            d.feed(raw, ctx);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.driver.as_ref().is_some_and(|d| d.want_state) {
            self.write_state();
        }
        self.take_finished(&ctx);
        if self.marks.poll() {
            self.refilter_pending = true;
        }
        if self.refilter_pending && self.viewer.is_none() && self.slideshow.is_none() {
            self.refilter_pending = false;
            if self.star_filter != marks::StarFilter::Any || self.prefs.sort == browser::SortBy::Stars {
                self.sort_listing();
            }
        }
        if let Some(p) = self.nav.take() {
            self.open_folder(&p, true);
        }
        // The slideshow takes the whole window while it runs.
        if self.slideshow.is_some() {
            let mut s = self.slideshow.take();
            if let Some(slideshow::Outcome::Closed(last)) = s.as_mut().map(|s| s.show(ui, self)) {
                // Show where it ended: in the viewer if that is open, else selected in the browser.
                if let Some(i) = last.and_then(|p| self.listing.as_ref()?.entries.iter().position(|e| e.path == p)) {
                    match &mut self.viewer {
                        Some(v) => v.index = i,
                        None => {
                            self.selected = Some(i);
                            self.scroll_to = Some(i);
                        }
                    }
                }
            } else {
                self.slideshow = s;
            }
            return;
        }
        if let Some(mut c) = self.compare.take() {
            if c.show(ui, self) {
                self.compare = Some(c);
            }
            return;
        }
        if self.viewer.is_some() {
            let mut v = self.viewer.take();
            let keep = v.as_mut().is_some_and(|v| v.show(ui, self));
            if keep {
                self.viewer = v;
            } else if let Some(v) = v {
                self.selected = Some(v.index);
                self.scroll_to = Some(v.index);
            }
            return;
        }
        if self.dialog.is_none()
            && !ctx.text_edit_focused()
            && ui.input(|i| pressed(i, Key::Z).is_some_and(|m| m.command))
        {
            self.start_undo(&ctx);
        }
        if let Some(r) = self.undo_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.undo_rx = None;
            let msg = r.unwrap_or_else(|e| format!("Could not undo: {e}"));
            self.note = Some((msg, std::time::Instant::now()));
            let keep = self.selected.and_then(|i| self.listing.as_ref()?.entries.get(i).map(|e| e.path.clone()));
            if let Some(p) = &keep {
                self.forget_pictures(p);
            }
            self.reload_listing(keep.as_deref());
            self.textures.clear(); // files may have changed back: load every picture fresh
            self.tex_order.clear();
        }
        self.browser_ui(ui, &ctx);
    }
}

/// `size` scaled to fill `area` as far as it goes while staying whole (small photos are enlarged), centred.
pub fn fit_fill(size: Vec2, area: Rect) -> Rect {
    let s = (area.width() / size.x).min(area.height() / size.y);
    Rect::from_center_size(area.center(), size * s)
}

/// `size` scaled to fit inside `area`, centred (never enlarged).
pub fn fit(size: Vec2, area: Rect) -> Rect {
    let s = (area.width() / size.x).min(area.height() / size.y).min(1.0);
    Rect::from_center_size(area.center(), size * s)
}

/// The ChronoSort UI font (Inter + Tabler icons, built by fonts/build_font.py) first; egui's own fonts
/// stay behind it as a fallback. Embedded in the exe: nothing to install, nothing read from the PC.
fn fonts(ctx: &egui::Context) {
    use egui::{FontData, FontFamily};
    use std::sync::Arc;
    let mut f = egui::FontDefinitions::default();
    f.font_data
        .insert("cs-ui".into(), Arc::new(FontData::from_static(include_bytes!("../fonts/ChronoSortUI-Regular.ttf"))));
    f.font_data.insert(
        "cs-ui-semibold".into(),
        Arc::new(FontData::from_static(include_bytes!("../fonts/ChronoSortUI-SemiBold.ttf"))),
    );
    let fallback = f.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(p) = f.families.get_mut(&FontFamily::Proportional) {
        p.insert(0, "cs-ui".into());
    }
    let mut bold = vec!["cs-ui-semibold".to_string()];
    bold.extend(fallback);
    f.families.insert(FontFamily::Name(BOLD.into()), bold);
    ctx.set_fonts(f);
}

/// Font family name of the SemiBold weight (headings, buttons that matter).
pub const BOLD: &str = "bold";

pub fn bold(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name(BOLD.into()))
}

fn style(ctx: &egui::Context) {
    fonts(ctx);
    ctx.set_visuals(egui::Visuals::dark());
    ctx.all_styles_mut(|s| {
        use egui::TextStyle::*;
        s.text_styles.insert(Body, FontId::proportional(16.0));
        s.text_styles.insert(Button, FontId::proportional(16.0));
        s.text_styles.insert(Small, FontId::proportional(13.0));
        s.text_styles.insert(Heading, bold(22.0));
        s.spacing.button_padding = vec2(10.0, 5.0);
        s.spacing.interact_size.y = 28.0;
        s.visuals.panel_fill = colors::PANEL;
        s.visuals.selection.bg_fill = colors::ACCENT;
        s.visuals.override_text_color = Some(colors::TEXT);
    });
}

fn main() -> eframe::Result {
    // Started as the video-thumbnail helper (a short-lived copy of this program; a damaged video cannot
    // crash the app)? Then do that and end; otherwise register this program as the helper.
    if let Some(code) = cs_thumb::video_helper_main() {
        std::process::exit(code);
    }
    let start = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ChronoSort")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    if start.as_deref() == Some(Path::new("--icons")) {
        return eframe::run_native(
            "ChronoSort",
            options,
            Box::new(|cc| {
                style(&cc.egui_ctx);
                Ok(Box::new(sheet::Sheet))
            }),
        );
    }
    let data = data::data_dirs();
    match data {
        Ok(d) => eframe::run_native("ChronoSort", options, Box::new(move |cc| Ok(Box::new(App::new(cc, d, start))))),
        Err(msg) => eframe::run_native(
            "ChronoSort",
            options,
            Box::new(move |cc| {
                style(&cc.egui_ctx);
                Ok(Box::new(Fatal(msg)))
            }),
        ),
    }
}

/// Shown instead of the app when the ChronoSort folder cannot be written.
struct Fatal(String);

impl eframe::App for Fatal {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(40.0);
            ui.heading("ChronoSort cannot start here");
            ui.add_space(12.0);
            ui.label(&self.0);
            ui.add_space(20.0);
            if ui
                .add(egui::Button::new(egui::RichText::new("  Exit  ").color(Color32::WHITE)).fill(colors::RED))
                .clicked()
            {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }
}

/// Was `key` pressed this frame, and with which modifiers (taken from the key event itself, not from the
/// modifier state at the end of the frame, which can differ when keys come fast)?
pub fn pressed(i: &egui::InputState, key: Key) -> Option<egui::Modifiers> {
    i.events.iter().find_map(|e| match e {
        egui::Event::Key { key: k, pressed: true, modifiers, .. } if *k == key => Some(*modifiers),
        _ => None,
    })
}
