//! `ChronoSortNative.exe --icons`: every icon of the ChronoSort UI font with its name, plus sample
//! text, to check the font at a glance (and later the start of the Help page's icon list).

use crate::{bold, colors, icons};
use egui::{Color32, FontId, RichText};

pub struct Sheet;

impl eframe::App for Sheet {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(colors::BG).inner_margin(16.0)).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new("ChronoSort UI font").font(bold(26.0)));
                ui.label(
                    RichText::new(format!("Inter + Tabler Icons · {} icons", icons::ALL.len())).color(colors::MUTED),
                );
                ui.add_space(10.0);
                for size in [16.0, 22.0] {
                    ui.label(RichText::new(format!(
                        "{} Back   {} Up   {} Play   {} Exit   ✓ Done   ★★★☆☆   ▶ Video   ⚠ Careful   ⚙ Settings   🔒 Locked   ♪ Music   📍 Paris   ← → ↑ ↓   … · — “quoted” × −",
                        icons::BACK, icons::UP, icons::PLAY, icons::EXIT
                    )).size(size));
                }
                ui.label(RichText::new("Grüße aus Köln · Ελληνικά · Русский текст · Tiếng Việt · Żółć · Ærø · Ñandú").size(18.0));
                ui.label(RichText::new("SemiBold: Sort photos into years — 2005 · 2006 · 2007").font(bold(18.0)));
                ui.add_space(14.0);
                let cols = ((ui.available_width() + 8.0) / 150.0).floor().max(1.0) as usize;
                egui::Grid::new("icons").spacing([8.0, 10.0]).show(ui, |ui| {
                    for (i, (name, glyph)) in icons::ALL.iter().enumerate() {
                        ui.vertical_centered(|ui| {
                            ui.set_width(142.0);
                            ui.label(RichText::new(*glyph).font(FontId::proportional(30.0)).color(Color32::WHITE));
                            ui.label(RichText::new(*name).size(12.0).color(colors::MUTED));
                        });
                        if (i + 1) % cols == 0 {
                            ui.end_row();
                        }
                    }
                });
            });
        });
    }
}
