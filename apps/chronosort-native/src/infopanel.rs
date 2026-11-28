//! The viewer's information fly-out: File facts, then three tabs like the Tauri app — EXIF (a summary of the
//! useful tags), All tags (searchable), Edit (date taken, title, description, keywords, author, copyright,
//! JPEG comment, GPS position). Reading and writing go through `cs_app::meta` (the ExifTool that ships with
//! ChronoSort) on a worker thread; a write keeps a verified backup, so Ctrl+Z in the browser puts it back.

use crate::{bold, browse, colors, icons};
use cs_app::meta::{MetaChanges, MetaInfo};
use egui::{Color32, RichText};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

const ROWS: &[(&str, &[&str])] = &[
    ("Camera", &["Make", "Model"]),
    ("Lens", &["LensModel", "Lens", "LensID"]),
    ("Date taken", &["DateTimeOriginal", "CreateDate"]),
    ("Exposure", &["ExposureTime"]),
    ("Aperture", &["FNumber"]),
    ("ISO", &["ISO"]),
    ("Focal length", &["FocalLength"]),
    ("35 mm equivalent", &["FocalLengthIn35mmFormat"]),
    ("Flash", &["Flash"]),
    ("Program", &["ExposureProgram"]),
    ("Metering", &["MeteringMode"]),
    ("Exposure bias", &["ExposureCompensation"]),
    ("Picture size", &["ImageSize"]),
    ("Title", &["Title", "XPTitle", "ObjectName"]),
    ("Description", &["Description", "ImageDescription", "Caption-Abstract"]),
    ("Keywords", &["Subject", "Keywords", "XPKeywords"]),
    ("Author", &["Creator", "Artist", "By-line"]),
    ("Copyright", &["Rights", "Copyright", "CopyrightNotice"]),
    ("Place", &["City", "State", "Country"]),
    ("Software", &["Software"]),
];

#[derive(PartialEq, Eq, Clone, Copy)]
enum Tab {
    Summary,
    All,
    Edit,
}

#[derive(Default, Clone)]
struct Form {
    date: String,
    title: String,
    description: String,
    keywords: String,
    author: String,
    copyright: String,
    comment: String,
    lat: String,
    lon: String,
}

pub struct InfoPanel {
    path: Option<PathBuf>,
    info: Option<MetaInfo>,
    error: Option<String>,
    rx: Option<Receiver<(PathBuf, Result<MetaInfo, String>)>>,
    tab: Tab,
    filter: String,
    form: Option<Form>,
    original: Form,
    saving: bool,
    /// Something was written: the viewer reloads the picture and the browser its listing.
    pub saved: Option<String>,
}

impl InfoPanel {
    pub fn new() -> Self {
        Self {
            path: None,
            info: None,
            error: None,
            rx: None,
            tab: Tab::Summary,
            filter: String::new(),
            form: None,
            original: Form::default(),
            saving: false,
            saved: None,
        }
    }

    /// Editing right now? (The fly-out stays pinned while the form is open.)
    pub fn editing(&self) -> bool {
        self.form.is_some()
    }

    fn load(&mut self, path: &Path, ctx: &egui::Context) {
        self.path = Some(path.to_path_buf());
        self.info = None;
        self.error = None;
        self.form = None;
        if self.tab == Tab::Edit {
            self.tab = Tab::Summary;
        }
        let (tx, rx) = channel();
        let (p, ctx) = (path.to_path_buf(), ctx.clone());
        std::thread::spawn(move || {
            let r = cs_app::meta::read(&p);
            let _ = tx.send((p, r));
            ctx.request_repaint();
        });
        self.rx = Some(rx);
    }

    fn find(&self, name: &str) -> String {
        self.info
            .as_ref()
            .and_then(|i| i.tags.iter().find(|t| t.name == name))
            .map(|t| t.value.clone())
            .unwrap_or_default()
    }

    fn row(&self, names: &[&str]) -> String {
        match names[0] {
            "Make" => {
                let make = self.find("Make");
                let model = self.find("Model").replace(&make, "").trim().to_string();
                [make, model].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
            }
            "City" => names.iter().map(|n| self.find(n)).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", "),
            _ => names.iter().map(|n| self.find(n)).find(|s| !s.is_empty()).unwrap_or_default(),
        }
    }

    fn start_edit(&mut self) {
        let dt = [self.find("DateTimeOriginal"), self.find("CreateDate")]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or_default();
        // "2004:05:06 14:30:00" -> "2004-05-06 14:30:00"
        let date =
            if dt.len() >= 10 { format!("{}{}", dt[..10].replace(':', "-"), &dt[10..dt.len().min(19)]) } else { dt };
        let (lat, lon) = self
            .info
            .as_ref()
            .and_then(|i| i.gps)
            .map_or((String::new(), String::new()), |(a, b)| (format!("{a:.6}"), format!("{b:.6}")));
        let f = Form {
            date,
            title: self.row(&["Title", "XPTitle", "ObjectName"]),
            description: self.row(&["Description", "ImageDescription", "Caption-Abstract"]),
            keywords: self.row(&["Subject", "Keywords", "XPKeywords"]).replace(';', ","),
            author: self.row(&["Creator", "Artist", "By-line"]),
            copyright: self.row(&["Rights", "Copyright", "CopyrightNotice"]),
            comment: self.find("Comment"),
            lat,
            lon,
        };
        self.original = f.clone();
        self.form = Some(f);
        self.tab = Tab::Edit;
    }

    /// Only what changed is written (an untouched field is left as it is in the photo).
    fn changes(&self) -> Result<MetaChanges, String> {
        let f = self.form.as_ref().ok_or("nothing to save")?;
        let o = &self.original;
        let changed = |a: &String, b: &String| (a.trim() != b.trim()).then(|| a.trim().to_string());
        let mut c = MetaChanges {
            date_taken: changed(&f.date, &o.date),
            title: changed(&f.title, &o.title),
            description: changed(&f.description, &o.description),
            keywords: changed(&f.keywords, &o.keywords)
                .map(|k| k.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()),
            author: changed(&f.author, &o.author),
            copyright: changed(&f.copyright, &o.copyright),
            comment: changed(&f.comment, &o.comment),
            ..Default::default()
        };
        if f.lat.trim() != o.lat.trim() || f.lon.trim() != o.lon.trim() {
            if f.lat.trim().is_empty() && f.lon.trim().is_empty() {
                c.clear_gps = true;
            } else {
                let lat: f64 = f.lat.trim().parse().map_err(|_| "The latitude must be a number like 51.5072.")?;
                let lon: f64 = f.lon.trim().parse().map_err(|_| "The longitude must be a number like -0.1276.")?;
                if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                    return Err("Latitude goes from -90 to 90 and longitude from -180 to 180.".into());
                }
                c.gps = Some((lat, lon));
            }
        }
        Ok(c)
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        entry: &browse::Entry,
        original: Option<(u32, u32)>,
        (number, total): (usize, usize),
    ) {
        let ctx = ui.ctx().clone();
        if self.path.as_deref() != Some(entry.path.as_path()) && !self.saving {
            self.load(&entry.path, &ctx);
        }
        if let Some((p, r)) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.rx = None;
            if self.path.as_deref() == Some(p.as_path()) {
                match r {
                    Ok(i) => {
                        if self.saving {
                            self.saving = false;
                            self.form = None;
                            self.tab = Tab::Summary;
                            self.saved =
                                Some("Information saved. Ctrl+Z in the browser puts the old information back.".into());
                        }
                        self.info = Some(i);
                    }
                    Err(e) => {
                        self.saving = false;
                        self.error = Some(e);
                    }
                }
            }
        }

        ui.label(RichText::new("FILE").font(bold(12.0)).color(colors::MUTED));
        let changed = (entry.modified > 0).then(|| {
            let (y, mo, d, h, mi, _) = cs_model::civil::from_unix(entry.modified);
            format!("{y}-{mo:02}-{d:02} {h:02}:{mi:02} (UTC)")
        });
        egui::Grid::new("file-grid").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
            let mut r = |k: &str, v: String| {
                ui.label(RichText::new(k).color(colors::MUTED));
                ui.add(egui::Label::new(RichText::new(v).color(Color32::WHITE)).wrap());
                ui.end_row();
            };
            r("Name", format!("{}  [{number} / {total}]", entry.name));
            let size = original.map(|(w, h)| format!(" · {w} × {h}")).unwrap_or_default();
            r("Type", format!("{}{size}", entry.ext.to_uppercase()));
            r("Size", browse::human_size(entry.size));
            if let Some(c) = changed {
                r("Modified", c);
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let editing = self.form.is_some();
            for (t, label) in [(Tab::Summary, "EXIF"), (Tab::All, "All tags")] {
                if ui.add_enabled(!editing, egui::Button::selectable(self.tab == t, label)).clicked() {
                    self.tab = t;
                }
            }
            if self.info.as_ref().is_some_and(|i| i.writable)
                && ui.selectable_label(self.tab == Tab::Edit, format!("{}  Edit", icons::EDIT)).clicked()
                && !editing
            {
                self.start_edit();
            }
        });
        ui.separator();
        if let Some(e) = &self.error {
            ui.colored_label(colors::RED, e);
        }
        if self.info.is_none() && self.error.is_none() {
            ui.label(RichText::new("Reading…").color(colors::MUTED));
            return;
        }
        egui::ScrollArea::vertical().id_salt("info-scroll").auto_shrink([false, false]).show(ui, |ui| match self.tab {
            Tab::Summary => {
                egui::Grid::new("exif-grid").num_columns(2).spacing([12.0, 4.0]).striped(true).show(ui, |ui| {
                    for (label, names) in ROWS {
                        ui.label(RichText::new(*label).color(colors::MUTED));
                        ui.add(egui::Label::new(RichText::new(self.row(names)).color(Color32::WHITE)).wrap());
                        ui.end_row();
                    }
                    ui.label(RichText::new("GPS").color(colors::MUTED));
                    let g = self
                        .info
                        .as_ref()
                        .and_then(|i| i.gps)
                        .map(|(a, b)| format!("{a:.5}, {b:.5}"))
                        .unwrap_or_default();
                    ui.label(RichText::new(g).color(Color32::WHITE));
                    ui.end_row();
                });
            }
            Tab::All => {
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter).hint_text("Find a tag…").desired_width(f32::INFINITY),
                );
                let f = self.filter.to_lowercase();
                egui::Grid::new("all-grid").num_columns(2).spacing([12.0, 3.0]).striped(true).show(ui, |ui| {
                    if let Some(i) = &self.info {
                        for t in i.tags.iter().filter(|t| {
                            f.is_empty() || format!("{}{}{}", t.group, t.name, t.value).to_lowercase().contains(&f)
                        }) {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&t.name).color(colors::MUTED));
                                ui.label(RichText::new(&t.group).size(11.0).color(colors::MUTED));
                            });
                            ui.add(egui::Label::new(RichText::new(&t.value).color(Color32::WHITE)).wrap());
                            ui.end_row();
                        }
                    }
                });
            }
            Tab::Edit => {
                let jpeg = matches!(entry.ext.as_str(), "jpg" | "jpeg" | "jfif");
                let mut cancel = false;
                let mut save = false;
                if let Some(f) = &mut self.form {
                    let field = |ui: &mut egui::Ui, label: &str, v: &mut String, hint: &str, multi: bool| {
                        ui.label(RichText::new(label).color(colors::MUTED));
                        let te = if multi {
                            egui::TextEdit::multiline(v).desired_rows(2)
                        } else {
                            egui::TextEdit::singleline(v)
                        };
                        ui.add(te.hint_text(hint).desired_width(f32::INFINITY));
                    };
                    field(ui, "Date taken", &mut f.date, "YYYY-MM-DD HH:MM:SS", false);
                    field(ui, "Title", &mut f.title, "", false);
                    field(ui, "Description", &mut f.description, "", true);
                    field(ui, "Keywords", &mut f.keywords, "beach, family, 2004", false);
                    field(ui, "Author", &mut f.author, "", false);
                    field(ui, "Copyright", &mut f.copyright, "", false);
                    if jpeg {
                        field(ui, "JPEG comment", &mut f.comment, "", false);
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new("Place (GPS)").color(colors::MUTED));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut f.lat).hint_text("Latitude").desired_width(120.0));
                        ui.add(egui::TextEdit::singleline(&mut f.lon).hint_text("Longitude").desired_width(120.0));
                        if ui.button(icons::CLOSE).on_hover_text("Remove the position").clicked() {
                            f.lat.clear();
                            f.lon.clear();
                        }
                    });
                    ui.label(
                        RichText::new("The town search moves over with the Places screen; numbers work now.")
                            .size(12.0)
                            .color(colors::MUTED),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "A safety copy is kept first; Ctrl+Z in the browser puts the old information back.",
                        )
                        .size(12.5)
                        .color(colors::MUTED),
                    );
                    ui.horizontal(|ui| {
                        cancel = ui.add_enabled(!self.saving, egui::Button::new("Cancel")).clicked();
                        let label = if self.saving { "Saving…" } else { "Save information" };
                        save = ui
                            .add_enabled(
                                !self.saving,
                                egui::Button::new(RichText::new(label).color(Color32::WHITE)).fill(colors::ACCENT),
                            )
                            .clicked();
                    });
                }
                if cancel {
                    self.form = None;
                    self.tab = Tab::Summary;
                    self.error = None;
                }
                if save {
                    match self.changes() {
                        Ok(c) => {
                            self.error = None;
                            self.saving = true;
                            let (tx, rx) = channel();
                            let (p, ctx) = (entry.path.clone(), ctx.clone());
                            std::thread::spawn(move || {
                                let r = cs_app::meta::write(&p, &c);
                                let _ = tx.send((p, r));
                                ctx.request_repaint();
                            });
                            self.rx = Some(rx);
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
        });
    }
}
