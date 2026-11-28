//! Compare 2–4 pictures side by side (P in the browser), like the Tauri app's `Compare.svelte`.
//!
//! Each pane keeps its own view: `z` is relative to "fit" (1 = the whole picture fits its pane) and (cx, cy) is
//! the point of the picture in the middle of the pane, 0..1 of its width / height. So "together" shows the SAME
//! part of every version, even when one is a smaller copy. The original is decoded once the preview's own
//! pixels are not enough.
//!
//! Mouse: wheel = zoom at the pointer · drag = move · double-click = 100 % / fit. Keys: Esc back · 0 or B fit ·
//! 1 or A 100 % · + / − zoom · S zoom and move together on / off · Tab next picture.

use crate::browse::{self, Entry};
use crate::loader::Key;
use crate::{App, Tex, bold, colors, icons};
use egui::{Align2, Color32, FontId, Id, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use std::path::PathBuf;

const FULL_SIDE: u32 = 4096;

#[derive(Clone, Copy, PartialEq, Debug)]
struct View {
    z: f32,
    cx: f32,
    cy: f32,
}

const FIT: View = View { z: 1.0, cx: 0.5, cy: 0.5 };

pub struct Compare {
    items: Vec<Entry>,
    /// The real size from the file header (stored, not turned).
    header: Vec<Option<(u32, u32)>>,
    views: Vec<View>,
    together: bool,
    active: usize,
    /// Pane rectangles of the last frame (keys act on the active one).
    panes: Vec<Rect>,
}

/// Where the picture sits in its pane for view `v` (picture aspect `nat`).
struct Box_ {
    rect: Rect,
    cx: f32,
    cy: f32,
}

/// The middle point, kept so the picture never leaves an empty edge when it is larger than the pane.
fn eff(c: f32, disp: f32, pane: f32) -> f32 {
    if disp <= pane {
        return 0.5;
    }
    let m = pane / 2.0 / disp;
    c.clamp(m, 1.0 - m)
}

fn fit_w(nat: Vec2, pane: Rect) -> f32 {
    if nat.x <= 0.0 || nat.y <= 0.0 {
        return 0.0;
    }
    (pane.width() / nat.x).min(pane.height() / nat.y) * nat.x
}

fn place(v: View, nat: Vec2, pane: Rect) -> Box_ {
    let w = fit_w(nat, pane) * v.z;
    let h = if nat.x > 0.0 { w * nat.y / nat.x } else { 0.0 };
    let (cx, cy) = (eff(v.cx, w, pane.width()), eff(v.cy, h, pane.height()));
    let min = pane.center() - vec2(cx * w, cy * h);
    Box_ { rect: Rect::from_min_size(min, vec2(w, h)), cx, cy }
}

/// The new view after zooming to `z` about point `m` (screen) of the pane.
fn zoom_at(v: View, z: f32, m: egui::Pos2, nat: Vec2, pane: Rect) -> View {
    let z = z.clamp(0.25, 80.0);
    let b = place(v, nat, pane);
    if b.rect.width() <= 0.0 {
        return v;
    }
    let d = m - pane.center();
    let (u, w) = (b.cx + d.x / b.rect.width(), b.cy + d.y / b.rect.height());
    let w2 = fit_w(nat, pane) * z;
    let h2 = w2 * nat.y / nat.x;
    View { z, cx: u - d.x / w2, cy: w - d.y / h2 }
}

impl Compare {
    pub fn new(items: Vec<Entry>) -> Self {
        let header = items.iter().map(|e| crate::loader::image_size(&e.path)).collect();
        let n = items.len();
        Self { items, header, views: vec![FIT; n], together: true, active: 0, panes: vec![Rect::NOTHING; n] }
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.items.iter().map(|e| e.path.clone()).collect()
    }

    pub fn state_text(&self) -> String {
        let v: Vec<String> = self.views.iter().map(|v| format!("{:.2}/{:.2}/{:.2}", v.z, v.cx, v.cy)).collect();
        format!("compare={} together={} active={} views={}", self.items.len(), self.together, self.active, v.join(" "))
    }

    fn set_view(&mut self, i: usize, v: View) {
        if self.together {
            self.views.iter_mut().for_each(|x| *x = v);
        } else {
            self.views[i] = v;
        }
    }

    /// The real picture's size, upright like the shown texture (an EXIF turn swaps the stored size).
    fn real(&self, i: usize, nat: Vec2) -> Option<(f32, f32)> {
        let (w, h) = self.header[i]?;
        let (w, h) = (w as f32, h as f32);
        let swapped = (nat.x > nat.y) != (w > h) && (w - h).abs() > 0.5;
        Some(if swapped { (h, w) } else { (w, h) })
    }

    /// The best picture of item `i` ready now (original when needed and loaded, else the preview).
    fn texture(&self, app: &mut App, i: usize, want_full: bool) -> Option<egui::TextureHandle> {
        let path = self.items[i].path.clone();
        let preview = Key { path: path.clone(), size: cs_thumb::PREVIEW_SIZE, full: false };
        let full = Key { path, size: FULL_SIDE, full: true };
        let _ = app.texture(&preview);
        if want_full {
            let _ = app.texture(&full);
            if let Some(Tex::Ready(t, _)) = app.textures.get(&full) {
                return Some(t.clone());
            }
        }
        match app.textures.get(&preview) {
            Some(Tex::Ready(t, _)) => Some(t.clone()),
            _ => None,
        }
    }

    fn failed(app: &App, e: &Entry) -> bool {
        let k = Key { path: e.path.clone(), size: cs_thumb::PREVIEW_SIZE, full: false };
        matches!(app.textures.get(&k), Some(Tex::Failed(_)))
    }

    /// Draws the screen; `false` = back to the browser.
    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut App) -> bool {
        let ctx = ui.ctx().clone();
        let screen = ui.max_rect();
        let ppp = ctx.pixels_per_point();
        ui.painter().rect_filled(screen, 0.0, colors::BG);
        let n = self.items.len();
        let mut close = false;
        let mut fit_all = false;
        let mut actual: Option<usize> = None;
        let mut zoom_by: Option<f32> = None;

        // ---------- keys ----------
        ctx.input(|i| {
            use egui::Key::*;
            let k = |key| crate::pressed(i, key).is_some_and(|m| !m.command && !m.alt);
            close = i.key_pressed(Escape);
            fit_all = k(Num0) || k(B);
            if k(Num1) || k(A) {
                actual = Some(self.active);
            }
            if k(Plus) || k(Equals) {
                zoom_by = Some(1.25);
            }
            if k(Minus) {
                zoom_by = Some(0.8);
            }
            if k(S) {
                self.together = !self.together;
                if self.together {
                    let v = self.views[self.active];
                    self.views.iter_mut().for_each(|x| *x = v);
                }
            }
            if let Some(m) = crate::pressed(i, Tab) {
                self.active = (self.active + if m.shift { n - 1 } else { 1 }) % n;
            }
        });

        // ---------- header ----------
        let head = Rect::from_min_size(screen.min, vec2(screen.width(), 50.0));
        ui.painter().rect_filled(head, 0.0, colors::PANEL);
        let mut hui = ui.new_child(egui::UiBuilder::new().max_rect(head.shrink2(vec2(12.0, 8.0))));
        hui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(format!("{}  Compare {n} pictures", icons::COMPARE)).font(bold(17.0)));
            ui.add_space(14.0);
            if ui.button(icons::ZOOM_OUT).on_hover_text("Zoom out (−)").clicked() {
                zoom_by = Some(0.8);
            }
            if ui.button(icons::ZOOM_IN).on_hover_text("Zoom in (+)").clicked() {
                zoom_by = Some(1.25);
            }
            if ui.button("Fit").on_hover_text("Whole pictures (0)").clicked() {
                fit_all = true;
            }
            if ui.button("100 %").on_hover_text("Real pixels of the highlighted picture (1)").clicked() {
                actual = Some(self.active);
            }
            let mut t = self.together;
            if ui.checkbox(&mut t, "Zoom and move together").on_hover_text("S").changed() {
                self.together = t;
                if t {
                    let v = self.views[self.active];
                    self.views.iter_mut().for_each(|x| *x = v);
                }
            }
            ui.label(
                RichText::new("Wheel = zoom at the pointer · drag = move · double-click = 100 % / fit")
                    .size(13.0)
                    .color(colors::MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let b = egui::Button::new(RichText::new(format!("{}  Close", icons::CLOSE)).color(Color32::WHITE))
                    .fill(colors::RED);
                if ui.add(b).on_hover_text("Back to the folder (Esc)").clicked() {
                    close = true;
                }
            });
        });

        // ---------- the panes ----------
        let (cols, rows) = if n == 4 { (2, 2) } else { (n, 1) };
        let grid = Rect::from_min_max(pos2(screen.min.x + 8.0, head.max.y + 8.0), screen.max - vec2(8.0, 8.0));
        let (cw, ch) = (grid.width() / cols as f32, grid.height() / rows as f32);
        const CAP: f32 = 52.0;
        // Hints: which version has the most pixels / the biggest file (only when one clearly wins).
        let unique_best = |v: Vec<u64>| -> Option<usize> {
            let best = *v.iter().max()?;
            (best > 0 && v.iter().filter(|&&x| x == best).count() == 1).then(|| v.iter().position(|&x| x == best))?
        };
        let most_px =
            unique_best(self.header.iter().map(|h| h.map_or(0, |(w, h)| u64::from(w) * u64::from(h))).collect());
        let biggest = unique_best(self.items.iter().map(|e| e.size).collect());

        for i in 0..n {
            let cell =
                Rect::from_min_size(grid.min + vec2((i % cols) as f32 * cw, (i / cols) as f32 * ch), vec2(cw, ch))
                    .shrink(4.0);
            let pane = Rect::from_min_max(cell.min, pos2(cell.max.x, cell.max.y - CAP));
            self.panes[i] = pane;
            let resp = ui.interact(pane, Id::unique(("compare-pane", i)), Sense::click_and_drag());
            if resp.hovered() {
                self.active = i;
            }
            ui.painter().rect_filled(pane, 4.0, Color32::BLACK);
            let e = self.items[i].clone();
            // Which texture: the preview first; the original once more pixels are shown than it has.
            let pre = self.texture(app, i, false);
            let nat = pre.as_ref().map_or(Vec2::ZERO, |t| t.size_vec2());
            let real = self.real(i, nat);
            let shown_px = place(self.views[i], nat, pane).rect.width() * ppp;
            let want_full = real.is_some_and(|(rw, _)| rw > nat.x * 1.01) && shown_px > nat.x * 1.01;
            let tex = if want_full { self.texture(app, i, true) } else { pre };

            // Mouse.
            if nat.x > 0.0 {
                if resp.hovered() {
                    let (scroll, zoom) = ctx.input(|inp| {
                        let s: f32 = inp
                            .events
                            .iter()
                            .filter_map(|ev| match ev {
                                egui::Event::MouseWheel { delta, modifiers, .. } if !modifiers.command => Some(delta.y),
                                _ => None,
                            })
                            .sum();
                        (s, inp.zoom_delta())
                    });
                    let f = if zoom != 1.0 {
                        zoom
                    } else if scroll != 0.0 {
                        if scroll > 0.0 { 1.2 } else { 1.0 / 1.2 }
                    } else {
                        1.0
                    };
                    if f != 1.0
                        && let Some(m) = resp.hover_pos()
                    {
                        let v = zoom_at(self.views[i], self.views[i].z * f, m, nat, pane);
                        self.set_view(i, v);
                    }
                }
                if resp.dragged() {
                    let b = place(self.views[i], nat, pane);
                    let d = resp.drag_delta();
                    let v = View {
                        z: self.views[i].z,
                        cx: eff(b.cx - d.x / b.rect.width(), b.rect.width(), pane.width()),
                        cy: eff(b.cy - d.y / b.rect.height(), b.rect.height(), pane.height()),
                    };
                    self.set_view(i, v);
                }
                if resp.double_clicked() {
                    if self.views[i].z > 1.01 {
                        self.set_view(i, FIT);
                    } else if let Some(m) = resp.interact_pointer_pos() {
                        let z = self.real_zoom(i, nat, pane, ppp);
                        let v = zoom_at(self.views[i], z, m, nat, pane);
                        self.set_view(i, v);
                    }
                }
            }

            // Picture.
            if let Some(t) = &tex {
                let b = place(self.views[i], nat, pane);
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                ui.painter().with_clip_rect(pane).image(t.id(), b.rect, uv, Color32::WHITE);
                if let Some((rw, _)) = real {
                    let pct = b.rect.width() * ppp / rw * 100.0;
                    let g =
                        ui.painter().layout_no_wrap(format!("{pct:.0} %"), FontId::proportional(13.0), Color32::WHITE);
                    let r = Rect::from_min_size(
                        pane.right_top() + vec2(-g.size().x - 18.0, 6.0),
                        g.size() + vec2(12.0, 6.0),
                    );
                    ui.painter().rect_filled(r, 4.0, Color32::from_black_alpha(160));
                    ui.painter().galley(r.min + vec2(6.0, 3.0), g, Color32::WHITE);
                }
            } else {
                let msg = if Self::failed(app, &e) { "No preview for this file" } else { "Loading…" };
                ui.painter().text(pane.center(), Align2::CENTER_CENTER, msg, FontId::proportional(18.0), colors::MUTED);
            }
            if self.active == i && n > 1 {
                ui.painter().rect_stroke(pane, 4.0, Stroke::new(2.0, colors::ACCENT), StrokeKind::Inside);
            }

            // Caption: name, size, date, stars, hints, Show in Explorer.
            let cap = Rect::from_min_max(pos2(cell.min.x, pane.max.y + 4.0), cell.max);
            let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(cap));
            cui.horizontal(|ui| {
                ui.label(RichText::new(&e.name).font(bold(14.5)));
                if let Some(s) = app.marks.stars(&e.path).filter(|&s| s > 0) {
                    ui.label(RichText::new(crate::marks::star_text(s)).color(colors::GOLD));
                }
                if app.marks.tagged(&e.path) {
                    ui.label(RichText::new(icons::TAG_FILLED).color(colors::TAG));
                }
                if most_px == Some(i) {
                    ui.label(RichText::new("most pixels").size(12.5).color(colors::GREEN));
                }
                if biggest == Some(i) {
                    ui.label(RichText::new("biggest file").size(12.5).color(colors::GREEN));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(icons::EXPLORER).on_hover_text("Show in Explorer").clicked() {
                        let _ = cs_platform::show_file_in_file_manager(&e.path);
                    }
                });
            });
            let mut info: Vec<String> = Vec::new();
            if let Some((w, h)) = real {
                info.push(format!("{w:.0} × {h:.0}"));
            }
            info.push(browse::human_size(e.size));
            if e.modified > 0 {
                let (y, mo, d, h, mi, _) = cs_model::civil::from_unix(e.modified);
                info.push(format!("changed {y}-{mo:02}-{d:02} {h:02}:{mi:02}"));
            }
            if let Some(p) = e.path.parent() {
                info.push(browse::shown(p));
            }
            cui.label(RichText::new(info.join(" · ")).size(13.0).color(colors::MUTED));
        }

        // ---------- buttons and keys that act on the panes ----------
        if fit_all {
            self.views.iter_mut().for_each(|v| *v = FIT);
        }
        let a = self.active;
        let nat_of = |app: &App, i: usize| -> Vec2 {
            let k = Key { path: self.items[i].path.clone(), size: cs_thumb::PREVIEW_SIZE, full: false };
            match app.textures.get(&k) {
                Some(Tex::Ready(t, _)) => t.size_vec2(),
                _ => Vec2::ZERO,
            }
        };
        let nat = nat_of(app, a);
        if nat.x > 0.0 {
            let pane = self.panes[a];
            if let Some(f) = zoom_by {
                let v = zoom_at(self.views[a], self.views[a].z * f, pane.center(), nat, pane);
                self.set_view(a, v);
            }
            if actual.is_some() {
                let z = self.real_zoom(a, nat, pane, ppp);
                let v = zoom_at(self.views[a], z, pane.center(), nat, pane);
                self.set_view(a, v);
            }
        }
        !close
    }

    /// The zoom (relative to fit) at which pane `i` shows one real pixel per screen pixel.
    fn real_zoom(&self, i: usize, nat: Vec2, pane: Rect, ppp: f32) -> f32 {
        let f = fit_w(nat, pane);
        if f <= 0.0 {
            return 1.0;
        }
        let w = self.real(i, nat).map_or(nat.x, |(w, _)| w);
        w / ppp / f
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_the_point_under_the_mouse() {
        let pane = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        let nat = vec2(800.0, 600.0);
        let m = pos2(300.0, 100.0);
        let before = place(FIT, nat, pane);
        let u = (m.x - before.rect.min.x) / before.rect.width();
        let v = zoom_at(FIT, 4.0, m, nat, pane);
        let after = place(v, nat, pane);
        let u2 = (m.x - after.rect.min.x) / after.rect.width();
        assert!((u - u2).abs() < 1e-4, "{u} vs {u2}");
        assert_eq!(after.rect.width(), 1600.0);
    }

    #[test]
    fn no_empty_edges() {
        let pane = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        let b = place(View { z: 2.0, cx: 0.0, cy: 0.0 }, vec2(800.0, 600.0), pane);
        assert_eq!(b.rect.min, pos2(0.0, 0.0));
    }
}
