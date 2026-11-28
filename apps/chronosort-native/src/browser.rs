//! The File browser, like the Tauri app's: the folder tree (favourites, places, drives — or "only this
//! folder"), the folder's files as Thumbnails S / M / L, Tiles, List or Details (sortable columns), several
//! picked at once, right-click menus and housekeeping (new folder, rename, copy / move / paste, Recycle Bin or
//! delete permanently, remove photo information) with a progress card and Stop, and the old shortcut keys.
//! The view, sort, favourites and the last folder are remembered in `ChronoSortData\settings\browser.json`.
//!
//! Keys: arrows / Home / End (Shift extends) · Enter open · Backspace or Alt+↑ up · Alt+← / Alt+→ back /
//! forward · Ctrl+A all · F2 rename · Del Recycle Bin · Shift+Del delete permanently · F8 or Ctrl+Shift+N new
//! folder · Ctrl+C / Ctrl+X / Ctrl+V copy, cut, paste · F5 refresh · Ctrl+D favourite · Ctrl+P print ·
//! S slideshow · Ctrl+Z undo · 1–5 stars (0 removes them; written into the photo) · Q tag · P compare 2–4
//! side by side.

use crate::browse::{self, Entry, Kind};
use crate::loader::Key as TexKey;
use crate::{App, Tex, ThumbSize, bold, colors, icons};
use cs_app::fileops::{self, Mode, OpReport};
use egui::{Align2, Color32, FontId, Id, Key, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum View {
    #[default]
    Thumbs,
    Tiles,
    List,
    Details,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortBy {
    #[default]
    Name,
    Date,
    Size,
    Type,
    Stars,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Prefs {
    pub view: View,
    pub size: ThumbSize,
    pub sort: SortBy,
    pub desc: bool,
    pub hidden: bool,
    pub favs: Vec<PathBuf>,
    /// "Show only this folder in the tree".
    pub root: Option<PathBuf>,
    pub last_folder: Option<PathBuf>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            view: View::Thumbs,
            size: ThumbSize::M,
            sort: SortBy::Name,
            desc: false,
            hidden: false,
            favs: Vec::new(),
            root: None,
            last_folder: None,
        }
    }
}

/// A file job running on a worker thread.
pub struct Job {
    label: String,
    rx: Receiver<JobMsg>,
    progress: Option<fileops::Progress>,
    stoppable: bool,
    /// Files may look different afterwards (information removed): load every picture fresh.
    fresh_pictures: bool,
}

enum JobMsg {
    Progress(fileops::Progress),
    Done(Result<String, String>),
}

pub enum Dialog {
    NewFolder(String),
    Rename(PathBuf, String),
    Delete { items: Vec<PathBuf>, permanent: bool },
    Strip { items: Vec<PathBuf>, what: &'static str },
    Wall(crate::walldlg::WallDlg),
    Print(crate::printdlg::PrintDlg),
    Batch(crate::batchdlg::BatchDlg),
    Create(crate::createdlg::CreateDlg),
}

/// What a right-click menu asked for (carried out after the menu closed).
enum Act {
    Open(usize),
    Navigate(PathBuf),
    Explorer(PathBuf),
    Reveal(PathBuf),
    Slideshow(usize),
    Clip(Mode),
    Paste,
    To(Mode),
    Rename,
    NewFolder,
    Delete(bool),
    Strip(&'static str),
    Wallpaper(PathBuf),
    Print,
    Refresh,
    SelectAll,
    Hidden,
    Fav(PathBuf),
    FavMove(PathBuf, isize),
    Root(Option<PathBuf>),
    Batch(crate::batchdlg::Tab),
    Create(crate::createdlg::Tab),
    Rate(u8),
    Tag,
    Compare,
}

/// Work a dialog asks for, done after it is drawn (it needs the whole app).
type Deferred = Box<dyn FnOnce(&mut App, &egui::Context)>;

const LABEL_H: f32 = 22.0;
const GAP: f32 = 8.0;

impl App {
    // ------------------------------------------------------------------ the folder and its order

    pub(crate) fn sort_listing(&mut self) {
        let (by, desc) = (self.prefs.sort, self.prefs.desc);
        let keep_sel = self.selected.and_then(|i| self.listing.as_ref()?.entries.get(i).map(|e| e.path.clone()));
        let keep_picked: Vec<PathBuf> =
            self.picked.iter().filter_map(|&i| self.listing.as_ref()?.entries.get(i).map(|e| e.path.clone())).collect();
        let Some(l) = &mut self.listing else { return };
        let (marks, filter) = (&self.marks, self.star_filter);
        let name = self.name_filter.trim().to_lowercase();
        l.entries = l
            .all
            .iter()
            .filter(|e| {
                marks.pass(filter, &e.path, e.kind == Kind::Folder)
                    && (name.is_empty() || e.name.to_lowercase().contains(&name))
            })
            .cloned()
            .collect();
        l.entries.sort_by(|a, b| {
            let folders = (b.kind == Kind::Folder).cmp(&(a.kind == Kind::Folder));
            let o = match by {
                SortBy::Name => std::cmp::Ordering::Equal,
                SortBy::Date => a.modified.cmp(&b.modified),
                SortBy::Size => a.size.cmp(&b.size),
                SortBy::Type => a.ext.cmp(&b.ext),
                SortBy::Stars => marks.stars(&a.path).unwrap_or(0).cmp(&marks.stars(&b.path).unwrap_or(0)),
            }
            .then_with(|| browse::natural_cmp(&a.name, &b.name));
            folders.then(if desc { o.reverse() } else { o })
        });
        let pos = |p: &PathBuf| l.entries.iter().position(|e| &e.path == p);
        let n = l.entries.len();
        self.selected = keep_sel.as_ref().and_then(pos).or(self.selected.filter(|&i| i < n)).or((n > 0).then_some(0));
        self.picked = keep_picked.iter().filter_map(pos).collect();
        self.anchor = self.anchor.filter(|&i| i < n);
    }

    fn save_prefs(&self) {
        crate::settings::save(&self.data_root, "browser", &self.prefs);
    }

    /// The picked items (or the one with the cursor).
    pub(crate) fn picked_entries(&self) -> Vec<Entry> {
        let Some(l) = &self.listing else { return Vec::new() };
        if self.picked.is_empty() {
            return self.selected.and_then(|i| l.entries.get(i)).cloned().into_iter().collect();
        }
        self.picked.iter().filter_map(|&i| l.entries.get(i)).cloned().collect()
    }

    fn select_one(&mut self, i: usize) {
        self.selected = Some(i);
        self.anchor = Some(i);
        self.picked = BTreeSet::from([i]);
    }

    fn click(&mut self, i: usize, m: egui::Modifiers) {
        if m.command {
            if !self.picked.remove(&i) {
                self.picked.insert(i);
            }
            self.selected = Some(i);
            self.anchor = Some(i);
        } else if m.shift {
            let a = self.anchor.unwrap_or(i);
            self.picked = (a.min(i)..=a.max(i)).collect();
            self.selected = Some(i);
        } else {
            self.select_one(i);
        }
    }

    pub(crate) fn go_back(&mut self) {
        if let Some(p) = self.back.pop() {
            if let Some(cur) = &self.listing {
                self.forward.push(cur.path.clone());
            }
            self.open_folder(&p, false);
        }
    }

    fn go_forward(&mut self) {
        if let Some(p) = self.forward.pop() {
            if let Some(cur) = &self.listing {
                self.back.push(cur.path.clone());
            }
            self.open_folder(&p, false);
        }
    }

    fn parent(&self) -> Option<PathBuf> {
        self.listing.as_ref().and_then(|l| l.path.parent().map(Path::to_path_buf))
    }

    // ------------------------------------------------------------------ the whole browser

    pub(crate) fn browser_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.take_samples();
        self.poll_job(ctx);
        let mut acts: Vec<Act> = Vec::new();
        if self.dialog.is_none() && !ctx.text_edit_focused() {
            self.browser_keys(ctx, &mut acts);
        }
        egui::Panel::top("top").frame(egui::Frame::new().fill(colors::PANEL).inner_margin(8.0)).show(ui, |ui| {
            self.top_bar(ui, &mut acts);
            if self.listing.is_some() {
                ui.add_space(6.0);
                self.filter_bar(ui, &mut acts);
            }
        });
        egui::Panel::bottom("status")
            .frame(egui::Frame::new().fill(colors::PANEL).inner_margin(6.0))
            .show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("tree")
            .resizable(true)
            .default_size(270.0)
            .frame(egui::Frame::new().fill(colors::PANEL).inner_margin(8.0))
            .show(ui, |ui| self.tree_panel(ui, &mut acts));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(colors::BG).inner_margin(10.0)).show(ui, |ui| {
            if let Some(e) = self.error.clone() {
                ui.colored_label(colors::RED, e);
            }
            self.files(ui, &mut acts);
        });
        self.job_card(ctx);
        self.dialogs(ctx);
        for a in acts {
            self.act(a, ctx);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if ui
                .add_enabled(!self.back.is_empty(), egui::Button::new(format!("{}  Back", icons::BACK)))
                .on_hover_text("Alt+←")
                .clicked()
            {
                self.go_back();
            }
            if ui
                .add_enabled(!self.forward.is_empty(), egui::Button::new(icons::FORWARD))
                .on_hover_text("Forward (Alt+→)")
                .clicked()
            {
                self.go_forward();
            }
            if ui
                .add_enabled(self.parent().is_some(), egui::Button::new(format!("{}  Up", icons::UP)))
                .on_hover_text("Backspace")
                .clicked()
                && let Some(p) = self.parent()
            {
                acts.push(Act::Navigate(p));
            }
            if ui
                .add_enabled(self.listing.is_some(), egui::Button::new(icons::REFRESH))
                .on_hover_text("Refresh (F5)")
                .clicked()
            {
                acts.push(Act::Refresh);
            }
            if let Some(l) = &self.listing {
                let fav = self.prefs.favs.contains(&l.path);
                let star = if fav { icons::STAR_FILLED } else { icons::STAR };
                if ui
                    .button(star)
                    .on_hover_text(if fav { "Remove from favourites (Ctrl+D)" } else { "Add to favourites (Ctrl+D)" })
                    .clicked()
                {
                    acts.push(Act::Fav(l.path.clone()));
                }
            }
            let path = self
                .listing
                .as_ref()
                .map(|l| browse::shown(&l.path))
                .unwrap_or_else(|| "Choose a folder on the left".into());
            ui.label(RichText::new(path).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let exit = egui::Button::new(
                    RichText::new(format!(" {}  Exit ", icons::EXIT)).font(bold(16.0)).color(Color32::WHITE),
                )
                .fill(colors::RED);
                if ui.add(exit).on_hover_text("Close ChronoSort").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                ui.add_space(12.0);
                // Sort.
                let arrow = if self.prefs.desc { icons::SORT_DESC } else { icons::SORT_ASC };
                if ui.button(arrow).on_hover_text("Reverse the order").clicked() {
                    self.prefs.desc = !self.prefs.desc;
                    self.sort_listing();
                    self.save_prefs();
                }
                let mut by = self.prefs.sort;
                egui::ComboBox::from_id_salt("sort-by").selected_text(sort_name(by)).width(110.0).show_ui(ui, |ui| {
                    for s in [SortBy::Name, SortBy::Date, SortBy::Size, SortBy::Type, SortBy::Stars] {
                        ui.selectable_value(&mut by, s, sort_name(s));
                    }
                });
                if by != self.prefs.sort {
                    self.prefs.sort = by;
                    self.prefs.desc = matches!(by, SortBy::Date | SortBy::Size | SortBy::Stars);
                    self.sort_listing();
                    self.save_prefs();
                }
                ui.add_space(10.0);
                // Views.
                let mut changed = false;
                for (v, label, tip) in [
                    (View::Details, icons::DETAILS, "Details"),
                    (View::List, icons::LIST, "List"),
                    (View::Tiles, icons::TILES, "Tiles"),
                ] {
                    if ui.selectable_label(self.prefs.view == v, label).on_hover_text(tip).clicked() {
                        self.prefs.view = v;
                        changed = true;
                    }
                }
                for (s, label) in [(ThumbSize::L, "L"), (ThumbSize::M, "M"), (ThumbSize::S, "S")] {
                    let on = self.prefs.view == View::Thumbs && self.prefs.size == s;
                    if ui.selectable_label(on, label).on_hover_text("Thumbnail size").clicked() {
                        self.prefs.view = View::Thumbs;
                        self.prefs.size = s;
                        changed = true;
                    }
                }
                if changed {
                    self.scroll_to = self.selected;
                    self.save_prefs();
                }
            });
        });
    }

    /// Second row: which files to show (stars, tag, part of the name), and the buttons for picked photos.
    fn filter_bar(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        use crate::marks::{StarFilter, star_text};
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(format!("{}  Show", icons::FILTER)).color(colors::MUTED));
            let mut f = self.star_filter;
            egui::ComboBox::from_id_salt("star-filter").selected_text(f.label()).width(150.0).height(400.0).show_ui(
                ui,
                |ui| {
                    for x in StarFilter::ALL {
                        ui.selectable_value(&mut f, x, x.label());
                    }
                },
            );
            let mut changed = f != self.star_filter;
            self.star_filter = f;
            let edit = egui::TextEdit::singleline(&mut self.name_filter)
                .hint_text(format!("{}  Names containing…", icons::SEARCH))
                .desired_width(220.0);
            let r = ui.add(edit).on_hover_text("Show only names containing this text");
            if r.changed() {
                changed = true;
            }
            let active = self.star_filter != StarFilter::Any || !self.name_filter.trim().is_empty();
            if active && ui.button(format!("{}  Show all", icons::CLOSE)).clicked() {
                self.star_filter = StarFilter::Any;
                self.name_filter.clear();
                changed = true;
            }
            if changed {
                self.sort_listing();
                self.scroll_to = self.selected;
            }
            if self.marks.loading && (self.star_filter != StarFilter::Any || self.prefs.sort == SortBy::Stars) {
                ui.spinner();
                ui.label(RichText::new("Reading the stars…").color(colors::MUTED));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let media = self.picked_entries().iter().filter(|e| e.kind.viewable()).count();
                if ui
                    .add_enabled((2..=4).contains(&media), egui::Button::new(format!("{}  Compare", icons::COMPARE)))
                    .on_hover_text("Compare the picked photos side by side (P)")
                    .on_disabled_hover_text("Pick 2 to 4 photos (Ctrl+click), then compare them side by side (P)")
                    .clicked()
                {
                    acts.push(Act::Compare);
                }
                if ui
                    .button(format!("{}  Tag", icons::TAG))
                    .on_hover_text("Tag or untag the picked files (Q)")
                    .clicked()
                {
                    acts.push(Act::Tag);
                }
                ui.add_space(8.0);
                if ui
                    .add(egui::Button::new(RichText::new(icons::STAR_OFF).color(colors::MUTED)).frame(false))
                    .on_hover_text("Remove the stars from the picked photos (0)")
                    .clicked()
                {
                    acts.push(Act::Rate(0));
                }
                for n in (1..=5u8).rev() {
                    let b = egui::Button::new(RichText::new(icons::STAR_FILLED).size(18.0).color(colors::GOLD))
                        .frame(false);
                    if ui
                        .add(b)
                        .on_hover_text(format!("Give {} to the picked photos (key {n})", star_text(n)))
                        .clicked()
                    {
                        acts.push(Act::Rate(n));
                    }
                }
                ui.label(RichText::new("Stars 1–5:").color(colors::MUTED));
            });
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(l) = &self.listing {
                let folders = l.entries.iter().filter(|e| e.kind == Kind::Folder).count();
                let files = l.entries.len() - folders;
                let mut s = format!("{folders} folders, {files} files");
                if l.hidden_count > 0 {
                    s += &format!(" · {} hidden", l.hidden_count);
                }
                if l.unreadable > 0 {
                    s += &format!(" · {} could not be read", l.unreadable);
                }
                let filtered = l.all.len() - l.entries.len();
                if filtered > 0 {
                    s += &format!(" · {filtered} not shown (filter)");
                }
                let picked = self.picked_entries();
                if picked.len() > 1 {
                    let bytes: u64 = picked.iter().map(|e| e.size).sum();
                    s += &format!("   |   {} picked · {}", picked.len(), browse::human_size(bytes));
                } else if let Some(e) = picked.first() {
                    s += &format!("   |   {}", e.name);
                    if e.kind != Kind::Folder {
                        s += &format!(" · {}", browse::human_size(e.size));
                    }
                    if e.modified > 0 {
                        let (y, mo, d, ..) = cs_model::civil::from_unix(e.modified);
                        s += &format!(" · changed {y}-{mo:02}-{d:02}");
                    }
                }
                ui.label(s);
            }
            if let Some((msg, at)) = &self.note
                && at.elapsed() < std::time::Duration::from_secs(8)
            {
                ui.label(RichText::new(format!("   |   {msg}")).color(colors::GREEN));
                ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                return;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new("Photos stay on this PC").color(colors::MUTED)).on_hover_text(format!(
                    "Everything ChronoSort keeps is in its own folder:\n{}",
                    browse::shown(&self.data_root)
                ));
            });
        });
    }

    // ------------------------------------------------------------------ the tree

    fn tree_panel(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if let Some(root) = self.prefs.root.clone() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Only this folder").color(colors::MUTED));
                    if ui.small_button("Show all").clicked() {
                        acts.push(Act::Root(None));
                    }
                });
                let name =
                    root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| browse::shown(&root));
                self.tree_node(ui, icons::FOLDER, &name, &root, acts, true);
                return;
            }
            if !self.prefs.favs.is_empty() {
                ui.label(RichText::new("Favourites").color(colors::MUTED));
                for p in self.prefs.favs.clone() {
                    let name =
                        p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| browse::shown(&p));
                    self.tree_node(ui, icons::STAR_FILLED, &name, &p, acts, false);
                }
                ui.add_space(10.0);
            }
            ui.label(RichText::new("Places").color(colors::MUTED));
            for (icon, name, path) in self.places.clone() {
                self.tree_node(ui, icon, &name, &path, acts, false);
            }
            ui.add_space(10.0);
            ui.label(RichText::new("This PC").color(colors::MUTED));
            for (icon, name, path) in self.drives.clone() {
                self.tree_node(ui, icon, &name, &path, acts, false);
            }
        });
        self.tree_reveal = None;
    }

    fn tree_node(
        &mut self,
        ui: &mut egui::Ui,
        icon: &str,
        name: &str,
        path: &Path,
        acts: &mut Vec<Act>,
        open_first: bool,
    ) {
        let current = self.listing.as_ref().is_some_and(|l| l.path == path);
        let id = Id::unique(("tree", path));
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open_first);
        // Open the tree down to the shown folder (once, after navigating).
        if self.tree_reveal.as_ref().is_some_and(|r| r != path && r.starts_with(path)) {
            state.set_open(true);
        }
        let open = state.is_open();
        let (_, header, _) = state
            .show_header(ui, |ui| {
                let r = ui.selectable_label(current, format!("{icon}  {name}"));
                if current && self.tree_reveal.is_some() {
                    r.scroll_to_me(None);
                }
                r
            })
            .body(|ui| {
                if open && !self.tree.contains_key(path) {
                    self.tree.insert(path.to_path_buf(), browse::subfolders(path));
                }
                let kids = self.tree.get(path).cloned().unwrap_or_default();
                if kids.is_empty() {
                    ui.label(RichText::new("(no folders)").color(colors::MUTED));
                }
                for (n, p) in kids {
                    self.tree_node(ui, icons::FOLDER, &n, &p, acts, false);
                }
            });
        let r = header.inner;
        if r.clicked() {
            acts.push(Act::Navigate(path.to_path_buf()));
        }
        let fav = self.prefs.favs.iter().position(|f| f == path);
        let root_on = self.prefs.root.as_deref() == Some(path);
        r.context_menu(|ui| {
            if ui.button(format!("{}  Open", icons::OPEN)).clicked() {
                acts.push(Act::Navigate(path.to_path_buf()));
            }
            if ui.button(format!("{}  Open in Explorer", icons::EXPLORER)).clicked() {
                acts.push(Act::Explorer(path.to_path_buf()));
            }
            ui.separator();
            let fav_label = if fav.is_some() { "Remove from favourites" } else { "Add to favourites" };
            if ui.button(format!("{}  {fav_label}", icons::STAR)).clicked() {
                acts.push(Act::Fav(path.to_path_buf()));
            }
            if let Some(i) = fav {
                if ui.add_enabled(i > 0, egui::Button::new("Move up in favourites")).clicked() {
                    acts.push(Act::FavMove(path.to_path_buf(), -1));
                }
                if ui.add_enabled(i + 1 < self.prefs.favs.len(), egui::Button::new("Move down in favourites")).clicked()
                {
                    acts.push(Act::FavMove(path.to_path_buf(), 1));
                }
            }
            if root_on {
                if ui.button(format!("{}  Show all folders in the tree", icons::PC)).clicked() {
                    acts.push(Act::Root(None));
                }
            } else if ui.button(format!("{}  Show only this folder in the tree", icons::SIDEBAR)).clicked() {
                acts.push(Act::Root(Some(path.to_path_buf())));
            }
        });
    }

    // ------------------------------------------------------------------ the files

    fn files(&mut self, ui: &mut egui::Ui, acts: &mut Vec<Act>) {
        let Some(count) = self.listing.as_ref().map(|l| l.entries.len()) else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Choose a folder on the left.").size(22.0).color(colors::MUTED))
            });
            return;
        };
        let (cols, row_h) = match self.prefs.view {
            View::Thumbs => {
                let cell = self.prefs.size.cell();
                (((ui.available_width() - 12.0 + GAP) / (cell + GAP)).floor().max(1.0) as usize, cell + LABEL_H + GAP)
            }
            View::Tiles => (((ui.available_width() - 12.0 + GAP) / (320.0 + GAP)).floor().max(1.0) as usize, 104.0),
            View::List | View::Details => (1, 30.0),
        };
        if count == 0 {
            let r = ui
                .centered_and_justified(|ui| {
                    let text = if self.listing.as_ref().is_some_and(|l| !l.all.is_empty()) {
                        "Nothing here matches the filter (“Show all” at the top shows everything again)."
                    } else {
                        "This folder is empty."
                    };
                    ui.label(RichText::new(text).size(22.0).color(colors::MUTED))
                })
                .response;
            let r = ui.interact(r.rect, Id::unique("empty-bg"), Sense::click());
            self.background_menu(&r, acts);
            return;
        }
        self.cols = cols;
        if self.prefs.view == View::Details {
            self.details_header(ui);
        }
        let rows = count.div_ceil(cols);
        let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if let Some(i) = self.scroll_to.take() {
            let (offset, view_h) = self.last_scroll;
            let top = (i / cols) as f32 * row_h;
            if top < offset {
                area = area.vertical_scroll_offset(top);
            } else if top + row_h > offset + view_h {
                area = area.vertical_scroll_offset(top + row_h - view_h);
            }
        }
        // The whole file area answers clicks where no file is (files lie on top of it).
        let bg = ui.interact(ui.available_rect_before_wrap(), Id::unique("files-bg"), Sense::click());
        let item_gap = ui.spacing().item_spacing.y;
        let out = area.show_rows(ui, row_h - item_gap, rows, |ui, range| {
            for row in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for i in row * cols..((row + 1) * cols).min(count) {
                        let resp = match self.prefs.view {
                            View::Thumbs => self.cell(ui, i),
                            View::Tiles => self.tile(ui, i),
                            View::List | View::Details => self.row(ui, i),
                        };
                        self.item_events(&resp, i, acts);
                    }
                });
            }
        });
        self.last_scroll = (out.state.clamped_offset().y, out.inner_rect.height());
        if bg.clicked() {
            self.picked.clear();
        }
        self.background_menu(&bg, acts);
    }

    fn item_events(&mut self, resp: &egui::Response, i: usize, acts: &mut Vec<Act>) {
        let m = resp.ctx.input(|inp| inp.modifiers);
        if resp.clicked() {
            self.click(i, m);
        }
        if resp.double_clicked() {
            acts.push(Act::Open(i));
        }
        if resp.secondary_clicked() && !self.picked.contains(&i) {
            self.select_one(i);
        }
        resp.context_menu(|ui| self.item_menu(ui, i, acts));
    }

    fn details_header(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width() - 12.0;
        ui.horizontal(|ui| {
            for (by, label, width) in [
                (SortBy::Name, "Name", w - 520.0),
                (SortBy::Stars, "Stars", 100.0),
                (SortBy::Date, "Date changed", 170.0),
                (SortBy::Type, "Type", 110.0),
                (SortBy::Size, "Size", 120.0),
            ] {
                let arrow = if self.prefs.sort == by { if self.prefs.desc { " ▼" } else { " ▲" } } else { "" };
                let (r, resp) = ui.allocate_exact_size(vec2(width.max(80.0), 24.0), Sense::click());
                ui.painter().text(
                    r.left_center() + vec2(6.0, 0.0),
                    Align2::LEFT_CENTER,
                    format!("{label}{arrow}"),
                    bold(14.0),
                    colors::MUTED,
                );
                if resp.on_hover_text("Sort by this").clicked() {
                    if self.prefs.sort == by {
                        self.prefs.desc = !self.prefs.desc;
                    } else {
                        self.prefs.sort = by;
                        self.prefs.desc = matches!(by, SortBy::Date | SortBy::Size | SortBy::Stars);
                    }
                    self.sort_listing();
                    self.save_prefs();
                }
            }
        });
        ui.separator();
    }

    fn background(&self, ui: &egui::Ui, rect: Rect, i: usize, hovered: bool) {
        let picked = self.picked.contains(&i) || (self.picked.is_empty() && self.selected == Some(i));
        let cursor = self.selected == Some(i);
        let bg = if picked {
            colors::ACCENT.gamma_multiply(0.35)
        } else if hovered {
            colors::CELL_HOVER
        } else {
            colors::CELL
        };
        ui.painter().rect_filled(rect, 4.0, bg);
        if cursor {
            ui.painter().rect_stroke(rect, 4.0, Stroke::new(2.0, colors::ACCENT), StrokeKind::Inside);
        }
    }

    /// The picture of an entry inside `inner`: its thumbnail, a folder (with sample photos), or a badge.
    fn picture(&mut self, ui: &egui::Ui, e: &Entry, inner: Rect, px: u32) {
        match e.kind {
            Kind::Folder => {
                paint_folder(ui.painter(), inner);
                let samples = self.samples_of(&e.path);
                if !samples.is_empty() {
                    let w = inner.width().min(inner.height()) * 0.7;
                    let body = Rect::from_center_size(inner.center() + vec2(0.0, w * 0.06), vec2(w, w * 0.68))
                        .shrink(w * 0.06);
                    let n = samples.len().min(4);
                    let (c, r) = if n == 1 { (1, 1) } else { (2, n.div_ceil(2)) };
                    let (cw, ch) = (body.width() / c as f32, body.height() / r as f32);
                    for (k, s) in samples.into_iter().take(4).enumerate() {
                        let cell = Rect::from_min_size(
                            body.min + vec2((k % c) as f32 * cw, (k / c) as f32 * ch),
                            vec2(cw, ch),
                        )
                        .shrink(1.5);
                        if let Some(Tex::Ready(t, _)) =
                            self.texture(&TexKey { path: s, size: cs_thumb::DEFAULT_SIZE, full: false })
                        {
                            let uv = cover_uv(t.size_vec2(), cell);
                            ui.painter().image(t.id(), cell, uv, Color32::WHITE);
                        }
                    }
                }
            }
            k if k.has_thumbnail() => {
                let key = TexKey { path: e.path.clone(), size: px, full: false };
                match self.texture(&key) {
                    Some(Tex::Ready(t, _)) => {
                        let r = crate::fit(t.size_vec2(), inner);
                        ui.painter().image(
                            t.id(),
                            r,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    Some(Tex::Failed(_)) => paint_badge(ui.painter(), inner, &e.ext, colors::MUTED),
                    None => {
                        ui.painter().text(
                            inner.center(),
                            Align2::CENTER_CENTER,
                            "…",
                            FontId::proportional(20.0),
                            colors::MUTED,
                        );
                    }
                }
                if k == Kind::Video && inner.width() > 40.0 {
                    let b = Rect::from_min_size(inner.left_bottom() + vec2(4.0, -26.0), vec2(30.0, 22.0));
                    ui.painter().rect_filled(b, 3.0, Color32::from_black_alpha(170));
                    ui.painter().text(
                        b.center(),
                        Align2::CENTER_CENTER,
                        icons::PLAY,
                        FontId::proportional(14.0),
                        Color32::WHITE,
                    );
                }
            }
            _ => paint_badge(ui.painter(), inner, &e.ext, colors::MUTED),
        }
    }

    fn cell(&mut self, ui: &mut egui::Ui, i: usize) -> egui::Response {
        let cell = self.prefs.size.cell();
        let (rect, resp) = ui.allocate_exact_size(vec2(cell, cell + LABEL_H), Sense::click());
        if !ui.is_rect_visible(rect) {
            return resp;
        }
        let Some(e) = self.listing.as_ref().and_then(|l| l.entries.get(i)).cloned() else { return resp };
        self.background(ui, rect, i, resp.hovered());
        let pic = Rect::from_min_size(rect.min, vec2(cell, cell));
        self.picture(ui, &e, pic.shrink(6.0), self.prefs.size.pixels());
        self.paint_marks(ui, &e, pic.shrink(6.0));
        let name_rect = Rect::from_min_size(pos2(rect.min.x + 4.0, pic.max.y), vec2(cell - 8.0, LABEL_H));
        text_line(ui, &e.name, name_rect, 13.0, colors::TEXT, true);
        resp.on_hover_text(&e.name)
    }

    fn tile(&mut self, ui: &mut egui::Ui, i: usize) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(320.0, 96.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return resp;
        }
        let Some(e) = self.listing.as_ref().and_then(|l| l.entries.get(i)).cloned() else { return resp };
        self.background(ui, rect, i, resp.hovered());
        let pic = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(84.0, 84.0));
        self.picture(ui, &e, pic, cs_thumb::DEFAULT_SIZE);
        let x = pic.max.x + 10.0;
        self.marks_text(ui, &e, Rect::from_min_size(pos2(rect.max.x - 100.0, rect.min.y + 62.0), vec2(94.0, 20.0)));
        let w = rect.max.x - x - 6.0;
        text_line(
            ui,
            &e.name,
            Rect::from_min_size(pos2(x, rect.min.y + 14.0), vec2(w, 22.0)),
            15.0,
            colors::TEXT,
            false,
        );
        let kind = if e.kind == Kind::Folder {
            "Folder".to_string()
        } else {
            format!("{} · {}", e.ext.to_uppercase(), browse::human_size(e.size))
        };
        text_line(
            ui,
            &kind,
            Rect::from_min_size(pos2(x, rect.min.y + 40.0), vec2(w, 20.0)),
            13.0,
            colors::MUTED,
            false,
        );
        text_line(
            ui,
            &date_text(e.modified),
            Rect::from_min_size(pos2(x, rect.min.y + 62.0), vec2(w, 20.0)),
            13.0,
            colors::MUTED,
            false,
        );
        resp.on_hover_text(&e.name)
    }

    fn row(&mut self, ui: &mut egui::Ui, i: usize) -> egui::Response {
        let w = ui.available_width() - 12.0;
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return resp;
        }
        let Some(e) = self.listing.as_ref().and_then(|l| l.entries.get(i)).cloned() else { return resp };
        self.background(ui, rect, i, resp.hovered());
        let icon_r = Rect::from_min_size(rect.min + vec2(4.0, 2.0), vec2(24.0, 24.0));
        let small = matches!(e.kind, Kind::Image | Kind::Raw | Kind::Video);
        if small {
            self.picture(ui, &e, icon_r, cs_thumb::DEFAULT_SIZE);
        } else {
            let icon = match e.kind {
                Kind::Folder => icons::FOLDER_FILLED,
                Kind::Audio => icons::AUDIO,
                _ => icons::DOCUMENT,
            };
            let c = if e.kind == Kind::Folder { colors::FOLDER } else { colors::MUTED };
            ui.painter().text(icon_r.center(), Align2::CENTER_CENTER, icon, FontId::proportional(18.0), c);
        }
        let x = icon_r.max.x + 8.0;
        if self.prefs.view == View::Details {
            let name_w = (w - 520.0).max(80.0) - 36.0;
            text_line(
                ui,
                &e.name,
                Rect::from_min_size(pos2(x, rect.min.y + 4.0), vec2(name_w, 20.0)),
                14.0,
                colors::TEXT,
                false,
            );
            let mut cx = rect.min.x + (w - 520.0).max(80.0) + 6.0;
            self.marks_text(ui, &e, Rect::from_min_size(pos2(cx, rect.min.y + 4.0), vec2(92.0, 20.0)));
            cx += 100.0;
            for (t, cw) in [
                (date_text(e.modified), 170.0),
                (if e.kind == Kind::Folder { "Folder".into() } else { e.ext.to_uppercase() }, 110.0),
                (if e.kind == Kind::Folder { String::new() } else { browse::human_size(e.size) }, 120.0),
            ] {
                text_line(
                    ui,
                    &t,
                    Rect::from_min_size(pos2(cx, rect.min.y + 4.0), vec2(cw - 8.0, 20.0)),
                    13.5,
                    colors::MUTED,
                    false,
                );
                cx += cw;
            }
        } else {
            text_line(
                ui,
                &e.name,
                Rect::from_min_size(pos2(x, rect.min.y + 4.0), vec2(rect.max.x - x - 106.0, 20.0)),
                14.0,
                colors::TEXT,
                false,
            );
            self.marks_text(ui, &e, Rect::from_min_size(pos2(rect.max.x - 100.0, rect.min.y + 4.0), vec2(94.0, 20.0)));
        }
        resp
    }

    /// Stars (bottom left) and the tag (top right) on a thumbnail.
    fn paint_marks(&self, ui: &egui::Ui, e: &Entry, r: Rect) {
        if e.kind == Kind::Folder {
            return;
        }
        let p = ui.painter();
        if let Some(n) = self.marks.stars(&e.path).filter(|&n| n > 0) {
            let size = if r.width() < 110.0 { 11.0 } else { 14.0 };
            let g = p.layout_no_wrap(crate::marks::star_text(n), FontId::proportional(size), colors::GOLD);
            let b = Rect::from_min_size(r.left_bottom() + vec2(3.0, -g.size().y - 7.0), g.size() + vec2(8.0, 4.0));
            p.rect_filled(b, 3.0, Color32::from_black_alpha(170));
            p.galley(b.min + vec2(4.0, 2.0), g, colors::GOLD);
        }
        if self.marks.tagged(&e.path) {
            let b = Rect::from_min_size(r.right_top() + vec2(-25.0, 3.0), vec2(22.0, 22.0));
            p.rect_filled(b, 11.0, Color32::from_black_alpha(170));
            p.text(b.center(), Align2::CENTER_CENTER, icons::TAG_FILLED, FontId::proportional(14.0), colors::TAG);
        }
    }

    /// "★★★" and the tag icon in one line (tiles, list, details).
    fn marks_text(&self, ui: &egui::Ui, e: &Entry, r: Rect) {
        if e.kind == Kind::Folder {
            return;
        }
        let mut x = r.min.x;
        if let Some(n) = self.marks.stars(&e.path).filter(|&n| n > 0) {
            let g = ui.painter().layout_no_wrap(crate::marks::star_text(n), FontId::proportional(13.0), colors::GOLD);
            let w = g.size().x;
            ui.painter().galley(pos2(x, r.center().y - g.size().y / 2.0), g, colors::GOLD);
            x += w + 6.0;
        }
        if self.marks.tagged(&e.path) {
            ui.painter().text(
                pos2(x, r.center().y),
                Align2::LEFT_CENTER,
                icons::TAG_FILLED,
                FontId::proportional(14.0),
                colors::TAG,
            );
        }
    }

    // ------------------------------------------------------------------ folder samples

    /// Up to four photos inside a folder, for its icon (looked up on a worker thread, once).
    fn samples_of(&mut self, folder: &Path) -> Vec<PathBuf> {
        if let Some(s) = self.samples.get(folder) {
            return s.clone();
        }
        self.samples.insert(folder.to_path_buf(), Vec::new());
        let (tx, f, ctx) = (self.samples_tx.clone(), folder.to_path_buf(), self.ctx.clone());
        std::thread::spawn(move || {
            let mut v: Vec<PathBuf> = std::fs::read_dir(&f)
                .map(|rd| {
                    rd.flatten()
                        .map(|d| d.path())
                        .filter(|p| {
                            p.extension()
                                .map(|x| x.to_string_lossy().to_lowercase())
                                .is_some_and(|x| matches!(browse::kind_of(&x), Kind::Image))
                        })
                        .take(200)
                        .collect()
                })
                .unwrap_or_default();
            v.sort_by(|a, b| browse::natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
            v.truncate(4);
            let _ = tx.send((f, v));
            ctx.request_repaint();
        });
        Vec::new()
    }

    fn take_samples(&mut self) {
        while let Ok((f, v)) = self.samples_rx.try_recv() {
            self.samples.insert(f, v);
        }
    }

    // ------------------------------------------------------------------ menus

    fn item_menu(&mut self, ui: &mut egui::Ui, i: usize, acts: &mut Vec<Act>) {
        let picked = self.picked_entries();
        let n = picked.len();
        let Some(e) = self.listing.as_ref().and_then(|l| l.entries.get(i)).cloned() else { return };
        let media = matches!(e.kind, Kind::Image | Kind::Raw | Kind::Video);
        let photos = picked.iter().filter(|x| matches!(x.kind, Kind::Image | Kind::Raw)).count();
        let mut more: Vec<Act> = Vec::new();
        let mut item = |ui: &mut egui::Ui, text: String, key: &str, a: Act| {
            if ui.add(egui::Button::new(text).shortcut_text(RichText::new(key).color(colors::MUTED))).clicked() {
                acts.push(a);
            }
        };
        if n <= 1 {
            if e.kind == Kind::Folder {
                item(ui, format!("{}  Open", icons::OPEN), "Enter", Act::Open(i));
                item(ui, format!("{}  Open in Explorer", icons::EXPLORER), "", Act::Explorer(e.path.clone()));
            } else {
                let open = if media { "View full screen" } else { "Open" };
                item(ui, format!("{}  {open}", icons::FULLSCREEN), "Enter", Act::Open(i));
                item(ui, format!("{}  Show in Explorer", icons::EXPLORER), "", Act::Reveal(e.path.clone()));
            }
        }
        if media {
            item(ui, format!("{}  Slideshow from here", icons::SLIDESHOW), "S", Act::Slideshow(i));
        }
        ui.separator();
        item(ui, "Copy".into(), "Ctrl+C", Act::Clip(Mode::Copy));
        item(ui, "Cut".into(), "Ctrl+X", Act::Clip(Mode::Move));
        if self.clip.is_some() {
            item(ui, "Paste here".into(), "Ctrl+V", Act::Paste);
        }
        item(ui, format!("{}  Copy to folder…", icons::COPY_TO), "", Act::To(Mode::Copy));
        item(ui, format!("{}  Move to folder…", icons::MOVE_TO), "", Act::To(Mode::Move));
        if n == 1 {
            item(ui, format!("{}  Rename", icons::RENAME), "F2", Act::Rename);
        }
        let media_n = picked.iter().filter(|x| x.kind.viewable()).count();
        if media_n >= 2 {
            let label = if media_n <= 4 {
                format!("Compare {media_n} side by side")
            } else {
                "Compare side by side (2 to 4)".into()
            };
            let b = egui::Button::new(format!("{}  {label}", icons::COMPARE))
                .shortcut_text(RichText::new("P").color(colors::MUTED));
            if ui.add_enabled(media_n <= 4, b).clicked() {
                more.push(Act::Compare);
            }
        }
        let files: Vec<&Entry> = picked.iter().filter(|x| x.kind != Kind::Folder).collect();
        if photos > 0 {
            ui.separator();
            let same = {
                let s: Vec<u8> = files.iter().filter_map(|x| self.marks.stars(&x.path)).collect();
                s.first().copied().filter(|f| s.iter().all(|x| x == f))
            };
            ui.horizontal(|ui| {
                ui.label(if photos > 1 { format!("Stars for {photos}:") } else { "Stars:".into() });
                for k in 1..=5u8 {
                    let on = same.is_some_and(|s| s >= k);
                    let star = if on { icons::STAR_FILLED } else { icons::STAR };
                    let b = egui::Button::new(RichText::new(star).size(18.0).color(colors::GOLD)).frame(false);
                    if ui.add(b).on_hover_text(format!("{} (key {k})", crate::marks::star_text(k))).clicked() {
                        more.push(Act::Rate(k));
                        ui.close();
                    }
                }
                let b = egui::Button::new(RichText::new(icons::STAR_OFF).size(18.0).color(colors::MUTED)).frame(false);
                if ui.add(b).on_hover_text("Remove the stars (key 0)").clicked() {
                    more.push(Act::Rate(0));
                    ui.close();
                }
            });
        }
        if !files.is_empty() {
            let all_tagged = files.iter().all(|x| self.marks.tagged(&x.path));
            let label = if all_tagged { "Remove the tag" } else { "Tag" };
            item(ui, format!("{}  {label}", icons::TAG), "Q", Act::Tag);
        }
        if photos > 0 {
            ui.separator();
            item(ui, format!("{}  Convert / resize…", icons::RESIZE), "F3", Act::Batch(crate::batchdlg::Tab::Convert));
            item(
                ui,
                format!(
                    "{}  {}",
                    icons::LIST,
                    if n > 1 { format!("Rename these {n}…") } else { "Batch rename…".into() }
                ),
                "F4",
                Act::Batch(crate::batchdlg::Tab::Rename),
            );
            item(
                ui,
                format!("{}  Contact sheet, PDF or ZIP…", icons::THUMBS),
                "",
                Act::Create(crate::createdlg::Tab::Sheet),
            );
            item(ui, format!("{}  Print…", icons::PRINT), "Ctrl+P", Act::Print);
            if n == 1 && e.kind != Kind::Video {
                item(
                    ui,
                    format!("{}  Set as desktop wallpaper…", icons::WALLPAPER),
                    "",
                    Act::Wallpaper(e.path.clone()),
                );
            }
            item(ui, format!("{}  Remove the place (GPS)…", icons::PIN), "", Act::Strip("place"));
            item(ui, format!("{}  Remove all photo information…", icons::CLOSE), "", Act::Strip("all"));
        }
        ui.separator();
        item(ui, format!("{}  Delete (Recycle Bin)", icons::TRASH), "Del", Act::Delete(false));
        item(ui, format!("{}  Delete permanently…", icons::TRASH), "Shift+Del", Act::Delete(true));
        acts.append(&mut more);
    }

    fn background_menu(&mut self, r: &egui::Response, acts: &mut Vec<Act>) {
        let has_clip = self.clip.is_some();
        let hidden = self.prefs.hidden;
        r.context_menu(|ui| {
            let mut item = |ui: &mut egui::Ui, text: String, key: &str, a: Act| {
                if ui.add(egui::Button::new(text).shortcut_text(RichText::new(key).color(colors::MUTED))).clicked() {
                    acts.push(a);
                }
            };
            item(ui, format!("{}  Slideshow of this folder", icons::SLIDESHOW), "S", Act::Slideshow(0));
            item(
                ui,
                format!("{}  Convert / resize all…", icons::RESIZE),
                "F3",
                Act::Batch(crate::batchdlg::Tab::Convert),
            );
            item(ui, format!("{}  Rename all…", icons::LIST), "F4", Act::Batch(crate::batchdlg::Tab::Rename));
            item(
                ui,
                format!("{}  Contact sheet, PDF or ZIP of this folder…", icons::THUMBS),
                "",
                Act::Create(crate::createdlg::Tab::Sheet),
            );
            ui.separator();
            item(ui, format!("{}  New folder", icons::FOLDER), "F8", Act::NewFolder);
            if has_clip {
                item(ui, "Paste here".into(), "Ctrl+V", Act::Paste);
            }
            item(ui, format!("{}  Refresh", icons::REFRESH), "F5", Act::Refresh);
            item(ui, "Select all".into(), "Ctrl+A", Act::SelectAll);
            item(
                ui,
                format!("{}  {} hidden files", icons::PREVIEW, if hidden { "Hide" } else { "Show" }),
                "",
                Act::Hidden,
            );
            if let Some(l) = &self.listing {
                item(ui, format!("{}  Open in Explorer", icons::EXPLORER), "", Act::Explorer(l.path.clone()));
            }
        });
    }

    // ------------------------------------------------------------------ keys

    fn browser_keys(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let Some(count) = self.listing.as_ref().map(|l| l.entries.len()) else { return };
        let cols = self.cols.max(1);
        let p = |k: Key| ctx.input(|i| crate::pressed(i, k));
        let plain = |k: Key| p(k).is_some_and(|m| m.is_none() || m == egui::Modifiers::SHIFT);
        let mut moved: Option<isize> = None;
        let mut shift = false;
        if count > 0 {
            let horizontal = self.prefs.view != View::Details && self.prefs.view != View::List;
            for (k, d) in [
                (Key::ArrowRight, 1isize),
                (Key::ArrowLeft, -1),
                (Key::ArrowDown, cols as isize),
                (Key::ArrowUp, -(cols as isize)),
            ] {
                if let Some(m) = p(k).filter(|m| !m.alt && !m.command)
                    && (horizontal || matches!(k, Key::ArrowDown | Key::ArrowUp))
                {
                    moved = Some(d);
                    shift = m.shift;
                }
            }
            if plain(Key::Home) {
                moved = Some(-(count as isize));
                shift = p(Key::Home).is_some_and(|m| m.shift);
            }
            if plain(Key::End) {
                moved = Some(count as isize);
                shift = p(Key::End).is_some_and(|m| m.shift);
            }
        }
        if let Some(d) = moved {
            let to = (self.selected.unwrap_or(0) as isize + d).clamp(0, count as isize - 1) as usize;
            if shift {
                let a = self.anchor.unwrap_or(self.selected.unwrap_or(0));
                self.picked = (a.min(to)..=a.max(to)).collect();
                self.selected = Some(to);
            } else {
                self.select_one(to);
            }
            self.scroll_to = Some(to);
        }
        if p(Key::Enter).is_some_and(|m| m.is_none())
            && let Some(i) = self.selected
        {
            acts.push(Act::Open(i));
        }
        if (p(Key::Backspace).is_some_and(|m| m.is_none()) || p(Key::ArrowUp).is_some_and(|m| m.alt))
            && let Some(par) = self.parent()
        {
            acts.push(Act::Navigate(par));
        }
        if p(Key::ArrowLeft).is_some_and(|m| m.alt)
            || ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1))
        {
            self.go_back();
        }
        if p(Key::ArrowRight).is_some_and(|m| m.alt)
            || ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra2))
        {
            self.go_forward();
        }
        if p(Key::A).is_some_and(|m| m.command) {
            acts.push(Act::SelectAll);
        }
        if p(Key::F2).is_some() {
            acts.push(Act::Rename);
        }
        if let Some(m) = p(Key::Delete) {
            acts.push(Act::Delete(m.shift));
        }
        if p(Key::F8).is_some() || p(Key::N).is_some_and(|m| m.command && m.shift) {
            acts.push(Act::NewFolder);
        }
        if p(Key::C).is_some_and(|m| m.command) {
            acts.push(Act::Clip(Mode::Copy));
        }
        if p(Key::X).is_some_and(|m| m.command) {
            acts.push(Act::Clip(Mode::Move));
        }
        if p(Key::V).is_some_and(|m| m.command) {
            acts.push(Act::Paste);
        }
        if p(Key::F3).is_some() {
            acts.push(Act::Batch(crate::batchdlg::Tab::Convert));
        }
        if p(Key::F4).is_some() {
            acts.push(Act::Batch(crate::batchdlg::Tab::Rename));
        }
        if p(Key::F5).is_some() {
            acts.push(Act::Refresh);
        }
        if p(Key::D).is_some_and(|m| m.command)
            && let Some(l) = &self.listing
        {
            acts.push(Act::Fav(l.path.clone()));
        }
        if p(Key::P).is_some_and(|m| m.command) {
            acts.push(Act::Print);
        }
        if p(Key::S).is_some_and(|m| m.is_none()) {
            acts.push(Act::Slideshow(self.selected.unwrap_or(0)));
        }
        // Stars: 0–5 (also with Ctrl, like in the viewer).
        for (k, n) in [(Key::Num0, 0u8), (Key::Num1, 1), (Key::Num2, 2), (Key::Num3, 3), (Key::Num4, 4), (Key::Num5, 5)]
        {
            if p(k).is_some_and(|m| !m.alt && !m.shift) {
                acts.push(Act::Rate(n));
            }
        }
        if p(Key::Q).is_some_and(|m| m.is_none()) {
            acts.push(Act::Tag);
        }
        if p(Key::P).is_some_and(|m| m.is_none()) {
            acts.push(Act::Compare);
        }
    }

    // ------------------------------------------------------------------ doing it

    fn act(&mut self, a: Act, ctx: &egui::Context) {
        let say = |app: &mut App, m: String| app.note = Some((m, Instant::now()));
        match a {
            Act::Open(i) => self.open_entry(i),
            Act::Navigate(p) => self.nav = Some(p),
            Act::Explorer(p) => {
                if let Err(e) = cs_platform::open_in_file_manager(&p) {
                    say(self, format!("Could not open Explorer: {e}"));
                }
            }
            Act::Reveal(p) => {
                if let Err(e) = cs_platform::show_file_in_file_manager(&p) {
                    say(self, format!("Could not open Explorer: {e}"));
                }
            }
            Act::Slideshow(i) => self.start_slideshow(i),
            Act::Clip(mode) => {
                let items: Vec<PathBuf> = self.picked_entries().into_iter().map(|e| e.path).collect();
                if !items.is_empty() {
                    let what = if mode == Mode::Copy { "copy" } else { "move" };
                    say(
                        self,
                        format!("{} item(s) ready to {what}: open the target folder and press Ctrl+V.", items.len()),
                    );
                    self.clip = Some((items, mode));
                }
            }
            Act::Paste => {
                if let (Some((items, mode)), Some(l)) = (self.clip.clone(), &self.listing) {
                    let dest = l.path.clone();
                    if mode == Mode::Move {
                        self.clip = None;
                    }
                    self.start_transfer(items, dest, mode, ctx);
                }
            }
            Act::To(mode) => {
                let items: Vec<PathBuf> = self.picked_entries().into_iter().map(|e| e.path).collect();
                let title = if mode == Mode::Copy { "Copy to which folder?" } else { "Move to which folder?" };
                if !items.is_empty()
                    && let Some(dest) = rfd::FileDialog::new().set_title(title).pick_folder()
                {
                    self.start_transfer(items, dest, mode, ctx);
                }
            }
            Act::Rename => {
                let p = self.picked_entries();
                if p.len() == 1 {
                    self.dialog = Some(Dialog::Rename(p[0].path.clone(), p[0].name.clone()));
                }
            }
            Act::NewFolder => {
                if self.listing.is_some() {
                    self.dialog = Some(Dialog::NewFolder("New folder".into()));
                }
            }
            Act::Delete(permanent) => {
                let items: Vec<PathBuf> = self.picked_entries().into_iter().map(|e| e.path).collect();
                if !items.is_empty() {
                    let bin = cs_platform::recycle_bin(&items[0]);
                    self.dialog = Some(Dialog::Delete { items, permanent: permanent || !bin.available });
                }
            }
            Act::Strip(what) => {
                let items: Vec<PathBuf> = self
                    .picked_entries()
                    .into_iter()
                    .filter(|e| matches!(e.kind, Kind::Image | Kind::Raw | Kind::Video))
                    .map(|e| e.path)
                    .collect();
                if !items.is_empty() {
                    self.dialog = Some(Dialog::Strip { items, what });
                }
            }
            Act::Wallpaper(p) => self.dialog = Some(Dialog::Wall(crate::walldlg::WallDlg::new(&p))),
            Act::Print => {
                let one: Vec<PathBuf> = self
                    .picked_entries()
                    .into_iter()
                    .filter(|e| matches!(e.kind, Kind::Image | Kind::Raw))
                    .map(|e| e.path)
                    .collect();
                let all: Vec<PathBuf> = self
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
                let one = if one.is_empty() { all.clone() } else { one };
                if one.is_empty() {
                    say(self, "There are no photos here to print.".into());
                } else {
                    self.dialog = Some(Dialog::Print(crate::printdlg::PrintDlg::new(one, all)));
                }
            }
            Act::Refresh => {
                let keep = self.selected.and_then(|i| self.listing.as_ref()?.entries.get(i).map(|e| e.path.clone()));
                self.samples.clear();
                self.tree.clear();
                self.reload_listing(keep.as_deref());
            }
            Act::SelectAll => {
                if let Some(l) = &self.listing {
                    self.picked = (0..l.entries.len()).collect();
                }
            }
            Act::Hidden => {
                self.prefs.hidden = !self.prefs.hidden;
                self.save_prefs();
                let keep = self.selected.and_then(|i| self.listing.as_ref()?.entries.get(i).map(|e| e.path.clone()));
                self.reload_listing(keep.as_deref());
            }
            Act::Fav(p) => {
                if let Some(i) = self.prefs.favs.iter().position(|f| f == &p) {
                    self.prefs.favs.remove(i);
                    say(self, "Removed from favourites.".into());
                } else {
                    self.prefs.favs.push(p);
                    say(self, "Added to favourites (top of the tree).".into());
                }
                self.save_prefs();
            }
            Act::FavMove(p, d) => {
                if let Some(i) = self.prefs.favs.iter().position(|f| f == &p) {
                    let j = (i as isize + d).clamp(0, self.prefs.favs.len() as isize - 1) as usize;
                    self.prefs.favs.swap(i, j);
                    self.save_prefs();
                }
            }
            Act::Create(tab) => {
                let picked: Vec<Entry> = self.picked_entries().into_iter().filter(|e| e.kind != Kind::Folder).collect();
                let all: Vec<Entry> = self
                    .listing
                    .as_ref()
                    .map(|l| l.entries.iter().filter(|e| e.kind != Kind::Folder).cloned().collect())
                    .unwrap_or_default();
                let list = if picked.len() > 1 { picked } else { all };
                let media: Vec<(PathBuf, i64)> = list
                    .iter()
                    .filter(|e| matches!(e.kind, Kind::Image | Kind::Raw | Kind::Video))
                    .map(|e| (e.path.clone(), e.modified))
                    .collect();
                let folder = self
                    .listing
                    .as_ref()
                    .and_then(|l| l.path.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if list.is_empty() {
                    self.note = Some(("There are no files here.".into(), Instant::now()));
                } else {
                    self.dialog = Some(Dialog::Create(crate::createdlg::CreateDlg::new(
                        tab,
                        list.into_iter().map(|e| e.path).collect(),
                        media,
                        &folder,
                    )));
                }
            }
            Act::Batch(tab) => {
                // The picked files, or every file of the folder when only one (or none) is picked.
                let picked: Vec<PathBuf> =
                    self.picked_entries().into_iter().filter(|e| e.kind != Kind::Folder).map(|e| e.path).collect();
                let files = if picked.len() > 1 {
                    picked
                } else {
                    self.listing
                        .as_ref()
                        .map(|l| l.entries.iter().filter(|e| e.kind != Kind::Folder).map(|e| e.path.clone()).collect())
                        .unwrap_or_default()
                };
                if files.is_empty() {
                    self.note = Some(("There are no files here.".into(), Instant::now()));
                } else {
                    self.dialog = Some(Dialog::Batch(crate::batchdlg::BatchDlg::new(tab, files)));
                }
            }
            Act::Root(r) => {
                self.prefs.root = r;
                self.save_prefs();
            }
            Act::Rate(n) => self.rate(n, ctx),
            Act::Tag => self.toggle_tag(),
            Act::Compare => {
                let media: Vec<Entry> = self.picked_entries().into_iter().filter(|e| e.kind.viewable()).collect();
                if (2..=4).contains(&media.len()) {
                    self.compare = Some(crate::compare::Compare::new(media));
                } else {
                    say(
                        self,
                        "Pick 2 to 4 photos to compare them side by side (Ctrl+click to add one), then press P.".into(),
                    );
                }
            }
        }
    }

    /// Q: tag the picked files, or untag them when all of them are tagged already.
    fn toggle_tag(&mut self) {
        let files: Vec<PathBuf> =
            self.picked_entries().into_iter().filter(|e| e.kind != Kind::Folder).map(|e| e.path).collect();
        if files.is_empty() {
            self.note = Some(("Pick one or more files to tag them (Q).".into(), Instant::now()));
            return;
        }
        let on = !files.iter().all(|p| self.marks.tagged(p));
        let msg = match self.marks.set_tagged(&files, on) {
            Ok(()) => {
                let n = files.len();
                let what = if n == 1 { "1 file".to_string() } else { format!("{n} files") };
                if self.star_filter == crate::marks::StarFilter::Tagged {
                    self.sort_listing();
                }
                if on {
                    format!("{what} tagged. To see only tagged files, choose “Tagged” under Show.")
                } else {
                    format!("Tag removed from {what}.")
                }
            }
            Err(e) => format!("Could not save the tag list: {e}"),
        };
        self.note = Some((msg, Instant::now()));
    }

    /// Give `n` stars (0 = none) to the picked photos: written into each photo, with a backup and one Undo.
    fn rate(&mut self, n: u8, ctx: &egui::Context) {
        if self.job.is_some() {
            self.note = Some(("Another file job is still running.".into(), Instant::now()));
            return;
        }
        let picked: Vec<Entry> = self.picked_entries().into_iter().filter(|e| e.kind != Kind::Folder).collect();
        let photos: Vec<PathBuf> = picked
            .iter()
            .filter(|e| matches!(e.kind, Kind::Image | Kind::Raw) && cs_app::meta::WRITABLE.contains(&e.ext.as_str()))
            .map(|e| e.path.clone())
            .collect();
        if photos.is_empty() {
            let msg = "Pick one or more photos to give them stars (keys 1–5, 0 removes them). \
                       Videos and other files cannot have stars.";
            self.note = Some((msg.into(), Instant::now()));
            return;
        }
        let skipped = picked.len() - photos.len();
        let label = if n == 0 {
            format!("Removing the stars from {} photo(s)…", photos.len())
        } else {
            format!("Giving {} to {} photo(s)…", crate::marks::star_text(n), photos.len())
        };
        self.spawn_job(label, false, ctx, move |tx| {
            let r = cs_app::rating::rate_files(&photos, n, |p| {
                let _ = tx.send(JobMsg::Progress(p));
            })?;
            Ok(rating_report(n, &r, skipped))
        });
        if let Some(j) = &mut self.job {
            j.fresh_pictures = false;
        }
    }

    pub(crate) fn open_entry(&mut self, i: usize) {
        let Some(e) = self.listing.as_ref().and_then(|l| l.entries.get(i)).cloned() else { return };
        if e.kind == Kind::Folder {
            self.nav = Some(e.path);
        } else if e.kind.viewable() {
            self.viewer = Some(crate::viewer::Viewer::new(i));
        } else if let Err(err) = cs_platform::show_file_in_file_manager(&e.path) {
            self.note = Some((format!("ChronoSort does not open this kind of file: {err}"), Instant::now()));
        }
    }

    fn start_transfer(&mut self, items: Vec<PathBuf>, dest: PathBuf, mode: Mode, ctx: &egui::Context) {
        if self.job.is_some() {
            self.note = Some(("Another file job is still running.".into(), Instant::now()));
            return;
        }
        let what = if mode == Mode::Copy { "Copying" } else { "Moving" };
        let label = format!(
            "{what} {} item(s) to “{}”…",
            items.len(),
            dest.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
        );
        let done_word = if mode == Mode::Copy { "Copied" } else { "Moved" };
        self.spawn_job(label, true, ctx, move |tx| {
            let mut send = |p: &fileops::Progress| {
                let _ = tx.send(JobMsg::Progress(p.clone()));
            };
            fileops::transfer_items(&items, &dest, mode, &mut send).map(|r| report(done_word, &r))
        });
    }

    fn spawn_job(
        &mut self,
        label: String,
        stoppable: bool,
        ctx: &egui::Context,
        work: impl FnOnce(&std::sync::mpsc::Sender<JobMsg>) -> Result<String, String> + Send + 'static,
    ) {
        let (tx, rx) = channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&tx)))
                .unwrap_or_else(|_| Err("The job stopped unexpectedly.".into()));
            let _ = tx.send(JobMsg::Done(r));
            c.request_repaint();
        });
        self.job = Some(Job { label, rx, progress: None, stoppable, fresh_pictures: true });
    }

    fn poll_job(&mut self, ctx: &egui::Context) {
        let Some(j) = &mut self.job else { return };
        let mut done = None;
        while let Ok(m) = j.rx.try_recv() {
            match m {
                JobMsg::Progress(p) => j.progress = Some(p),
                JobMsg::Done(r) => done = Some(r),
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
        if let Some(r) = done {
            let fresh = self.job.take().is_some_and(|j| j.fresh_pictures);
            let msg = r.unwrap_or_else(|e| e);
            self.note = Some((msg, Instant::now()));
            if fresh {
                self.picked.clear();
                self.samples.clear();
                self.tree.clear();
                self.textures.clear(); // files may have changed (info removed): load every picture fresh
                self.tex_order.clear();
            }
            let keep = self.selected.and_then(|i| self.listing.as_ref()?.entries.get(i).map(|e| e.path.clone()));
            self.reload_listing(keep.as_deref());
        }
    }

    fn job_card(&mut self, ctx: &egui::Context) {
        let Some(j) = &self.job else { return };
        let mut stop = false;
        egui::Area::new(Id::unique("job-card"))
            .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -48.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(12.0).show(ui, |ui| {
                    ui.set_width(380.0);
                    ui.label(RichText::new(&j.label).font(bold(15.0)));
                    match &j.progress {
                        Some(p) if p.files_total > 0 => {
                            let f = if p.bytes_total > 0 {
                                p.bytes_done as f32 / p.bytes_total as f32
                            } else {
                                p.files_done as f32 / p.files_total as f32
                            };
                            ui.add(egui::ProgressBar::new(f.clamp(0.0, 1.0)).show_percentage());
                            ui.label(
                                RichText::new(format!("{} of {} files · {}", p.files_done, p.files_total, p.current))
                                    .size(13.0)
                                    .color(colors::MUTED),
                            );
                        }
                        _ => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Working…").color(colors::MUTED));
                            });
                        }
                    }
                    if j.stoppable
                        && ui
                            .button("Stop")
                            .on_hover_text("Stops after the file being copied; nothing half-done is left behind")
                            .clicked()
                    {
                        stop = true;
                    }
                });
            });
        if stop {
            fileops::cancel();
        }
    }

    // ------------------------------------------------------------------ dialogs

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.dialog else { return };
        let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
        let enter = ctx.input(|i| i.key_pressed(Key::Enter));
        let dir = self.listing.as_ref().map(|l| l.path.clone());
        let mut run: Option<Deferred> = None;
        match d {
            Dialog::NewFolder(_) | Dialog::Rename(..) => {
                let renaming = matches!(d, Dialog::Rename(..));
                let mut ok = enter;
                let (title, button) = if renaming { ("Rename", "Rename") } else { ("New folder", "Make the folder") };
                let mut name_now = String::new();
                egui::Modal::new(Id::unique("name-dlg")).show(ctx, |ui| {
                    ui.set_width(420.0);
                    let (Dialog::NewFolder(name) | Dialog::Rename(_, name)) = &mut *d else { return };
                    ui.label(RichText::new(title).font(bold(18.0)));
                    ui.add_space(6.0);
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                    if !r.has_focus() && !r.lost_focus() {
                        r.request_focus();
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                        if ui
                            .add(egui::Button::new(RichText::new(button).color(Color32::WHITE)).fill(colors::ACCENT))
                            .clicked()
                        {
                            ok = true;
                        }
                    });
                    name_now = name.clone();
                });
                if ok {
                    let target = match &*d {
                        Dialog::Rename(p, _) => Some(p.clone()),
                        _ => None,
                    };
                    run = Some(Box::new(move |app: &mut App, _ctx| {
                        let r = match &target {
                            Some(p) => fileops::rename(p, &name_now)
                                .map(|to| (to, format!("Renamed to “{name_now}”. Ctrl+Z puts the old name back."))),
                            None => match &dir {
                                Some(dir) => fileops::new_folder(dir, &name_now)
                                    .map(|to| (to, format!("Folder “{name_now}” made."))),
                                None => Err("No folder is open.".into()),
                            },
                        };
                        match r {
                            Ok((to, msg)) => {
                                app.note = Some((msg, Instant::now()));
                                app.tree.clear();
                                if let Some(i) = app.reload_listing(Some(&to)) {
                                    app.select_one(i);
                                    app.scroll_to = Some(i);
                                }
                                app.dialog = None;
                            }
                            Err(e) => app.note = Some((e, Instant::now())),
                        }
                    }));
                }
            }
            Dialog::Delete { items, permanent } => {
                let (items, permanent) = (items.clone(), *permanent);
                let mut ok = false;
                egui::Modal::new(Id::unique("delete-dlg")).show(ctx, |ui| {
                    ui.set_width(460.0);
                    let n = items.len();
                    let what = if n == 1 {
                        format!("“{}”", items[0].file_name().map(|f| f.to_string_lossy()).unwrap_or_default())
                    } else {
                        format!("these {n} items")
                    };
                    if permanent {
                        ui.label(
                            RichText::new(format!("{}  Delete {what} permanently?", icons::WARNING))
                                .font(bold(18.0))
                                .color(colors::RED),
                        );
                        ui.label("They do not go to the Recycle Bin and cannot be brought back, not even with Undo.");
                        let bin = cs_platform::recycle_bin(&items[0]);
                        if !bin.available {
                            ui.label(
                                RichText::new(bin.reason.unwrap_or_else(|| "This drive has no Recycle Bin.".into()))
                                    .color(colors::MUTED),
                            );
                        }
                    } else {
                        ui.label(
                            RichText::new(format!("{}  Move {what} to the Recycle Bin?", icons::TRASH))
                                .font(bold(18.0)),
                        );
                        ui.label(RichText::new("Ctrl+Z (or the Recycle Bin) brings them back.").color(colors::MUTED));
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                        let (label, fill) = if permanent {
                            ("Delete permanently", colors::RED)
                        } else {
                            ("Move to the Recycle Bin", colors::ACCENT)
                        };
                        if ui.add(egui::Button::new(RichText::new(label).color(Color32::WHITE)).fill(fill)).clicked() {
                            ok = true;
                        }
                    });
                });
                if ok {
                    run = Some(Box::new(move |app: &mut App, ctx| {
                        app.dialog = None;
                        let n = items.len();
                        let label = if permanent {
                            format!("Deleting {n} item(s) permanently…")
                        } else {
                            format!("Moving {n} item(s) to the Recycle Bin…")
                        };
                        app.spawn_job(label, false, ctx, move |_tx| {
                            fileops::delete(&items, permanent)
                                .map(|r| report(if permanent { "Deleted" } else { "Moved to the Recycle Bin:" }, &r))
                        });
                    }));
                }
            }
            Dialog::Strip { items, what } => {
                let (items, what) = (items.clone(), *what);
                let mut ok = false;
                egui::Modal::new(Id::unique("strip-dlg")).show(ctx, |ui| {
                    ui.set_width(480.0);
                    let n = items.len();
                    if what == "place" {
                        ui.label(RichText::new(format!("{}  Remove the place from {n} photo(s)?", icons::PIN)).font(bold(18.0)));
                        ui.label("The GPS position and the place names are taken out; everything else stays.");
                    } else {
                        ui.label(RichText::new(format!("{}  Remove all information from {n} photo(s)?", icons::WARNING)).font(bold(18.0)));
                        ui.label("Camera, date, place, names and comments are taken out (the way the photo is turned and its colours stay). Good before sharing a photo.");
                    }
                    ui.label(RichText::new("A safety copy is kept first; Ctrl+Z puts everything back.").color(colors::MUTED));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                        if ui.add(egui::Button::new(RichText::new("Remove").color(Color32::WHITE)).fill(colors::ACCENT)).clicked() {
                            ok = true;
                        }
                    });
                });
                if ok {
                    run = Some(Box::new(move |app: &mut App, ctx| {
                        app.dialog = None;
                        let n = items.len();
                        app.spawn_job(format!("Removing information from {n} photo(s)…"), false, ctx, move |tx| {
                            let r = cs_app::privacy::strip_files(&items, what, |p| {
                                let _ = tx.send(JobMsg::Progress(p));
                            })?;
                            let mut s = format!("Information removed from {} photo(s).", r.done);
                            if r.unchanged > 0 {
                                s += &format!(" {} had nothing to remove.", r.unchanged);
                            }
                            if !r.failed.is_empty() {
                                s += &format!(" {} could not be changed: {}", r.failed.len(), r.failed[0].1);
                            }
                            Ok(s + " Ctrl+Z puts it back.")
                        });
                    }));
                }
            }
            Dialog::Wall(w) => {
                if let crate::walldlg::Outcome::Closed(msg) = w.show(ctx, &self.data_root) {
                    if let Some(m) = msg {
                        self.note = Some((m, Instant::now()));
                    }
                    self.dialog = None;
                }
                return;
            }
            Dialog::Create(c) => {
                if let crate::createdlg::Outcome::Closed { changed } = c.show(ctx) {
                    self.dialog = None;
                    if changed {
                        let keep =
                            self.selected.and_then(|i| self.listing.as_ref()?.entries.get(i).map(|e| e.path.clone()));
                        self.reload_listing(keep.as_deref());
                    }
                }
                return;
            }
            Dialog::Batch(b) => {
                if let crate::batchdlg::Outcome::Closed { changed, open } = b.show(ctx) {
                    self.dialog = None;
                    if changed {
                        self.textures.clear();
                        self.tex_order.clear();
                        let keep =
                            self.selected.and_then(|i| self.listing.as_ref()?.entries.get(i).map(|e| e.path.clone()));
                        self.reload_listing(keep.as_deref());
                    }
                    if let Some(f) = open {
                        self.nav = Some(f);
                    }
                }
                return;
            }
            Dialog::Print(p) => {
                if let crate::printdlg::Outcome::Closed(msg) = p.show(ctx) {
                    if let Some(m) = msg {
                        self.note = Some((m, Instant::now()));
                        let keep =
                            self.selected.and_then(|i| self.listing.as_ref()?.entries.get(i).map(|e| e.path.clone()));
                        self.reload_listing(keep.as_deref());
                    }
                    self.dialog = None;
                }
                return;
            }
        }
        if let Some(f) = run {
            f(self, ctx);
        } else if close {
            self.dialog = None;
        }
    }
}

/// "Copied 3 item(s). 1 could not: …"
fn report(done_word: &str, r: &OpReport) -> String {
    let mut s = format!("{done_word} {} item(s).", r.done);
    if r.stopped {
        s += " Stopped.";
    }
    if !r.failed.is_empty() {
        s += &format!(" {} could not: {} ({})", r.failed.len(), r.failed[0].0, r.failed[0].1);
    }
    for w in r.warnings.iter().take(1) {
        s += &format!(" {w}");
    }
    s + " Ctrl+Z undoes it."
}

/// What a star job did, in one line for the status bar (the viewer uses it too).
pub fn rating_report(n: u8, r: &cs_app::rating::RatingReport, skipped: usize) -> String {
    let star = crate::marks::star_text(n);
    let mut msg = Vec::new();
    if r.done > 0 {
        msg.push(if n == 0 {
            format!("Stars removed from {} photo(s)", r.done)
        } else {
            format!("{star} saved in {} photo(s)", r.done)
        });
    }
    if r.unchanged > 0 {
        msg.push(format!("{} already had {}", r.unchanged, if n == 0 { "no stars" } else { "these stars" }));
    }
    if skipped > 0 {
        msg.push(format!("{skipped} left out (stars are for photos only)"));
    }
    if let Some((name, e)) = r.failed.first() {
        msg.push(format!("{} could not be changed ({name}: {e})", r.failed.len()));
    }
    if r.done > 0 {
        msg.push("Ctrl+Z undoes".into());
    }
    msg.join(" · ")
}

fn sort_name(s: SortBy) -> &'static str {
    match s {
        SortBy::Name => "Name",
        SortBy::Date => "Date",
        SortBy::Size => "Size",
        SortBy::Type => "Type",
        SortBy::Stars => "Stars",
    }
}

fn date_text(t: i64) -> String {
    if t <= 0 {
        return String::new();
    }
    let (y, mo, d, h, mi, _) = cs_model::civil::from_unix(t);
    format!("{y}-{mo:02}-{d:02} {h:02}:{mi:02}")
}

/// One line of text in `r`, cut with "…" when too long; centred or left.
fn text_line(ui: &egui::Ui, text: &str, r: Rect, size: f32, color: Color32, centred: bool) {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), FontId::proportional(size), color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(r.width().max(4.0));
    let g = ui.painter().layout_job(job);
    let x = if centred { r.center().x - g.size().x / 2.0 } else { r.min.x };
    ui.painter().galley(pos2(x, r.min.y + (r.height() - g.size().y) / 2.0), g, color);
}

/// The part of a picture that covers `cell` without bars (centre cut).
fn cover_uv(size: egui::Vec2, cell: Rect) -> Rect {
    let (pa, ca) = (size.x / size.y.max(1.0), cell.width() / cell.height().max(1.0));
    if pa > ca {
        let w = ca / pa;
        Rect::from_min_max(pos2(0.5 - w / 2.0, 0.0), pos2(0.5 + w / 2.0, 1.0))
    } else {
        let h = pa / ca;
        Rect::from_min_max(pos2(0.0, 0.5 - h / 2.0), pos2(1.0, 0.5 + h / 2.0))
    }
}

pub fn paint_folder(p: &egui::Painter, r: Rect) {
    let w = r.width().min(r.height()) * 0.7;
    let body = Rect::from_center_size(r.center() + vec2(0.0, w * 0.06), vec2(w, w * 0.68));
    let tab = Rect::from_min_size(body.min - vec2(0.0, w * 0.12), vec2(w * 0.42, w * 0.16));
    p.rect_filled(tab, 3.0, colors::FOLDER.gamma_multiply(0.8));
    p.rect_filled(body, 4.0, colors::FOLDER);
}

pub fn paint_badge(p: &egui::Painter, r: Rect, ext: &str, c: Color32) {
    let b = Rect::from_center_size(r.center(), vec2(r.width() * 0.5, r.width() * 0.62));
    p.rect_stroke(b, 4.0, Stroke::new(2.0, c), StrokeKind::Inside);
    let t = if ext.is_empty() { "file".to_string() } else { ext.to_uppercase() };
    p.text(b.center(), Align2::CENTER_CENTER, t, FontId::proportional((r.width() / 8.0).clamp(9.0, 15.0)), c);
}
