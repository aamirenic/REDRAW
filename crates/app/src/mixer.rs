//! Mixer: 64 channel strips, four buses and the master strip.

use eframe::egui::{self, Align, Layout, RichText, ScrollArea, Sense, Stroke, Vec2};

use crate::app::FxStrip;
use crate::widgets::{format_db, format_pan};
use crate::RedrawApp;

const STRIP_W: f32 = 76.0;

impl RedrawApp {
    pub fn mixer_ui(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Mixer").strong().size(12.0));
            ui.label(
                RichText::new("64 channels / 4 buses / master")
                    .small()
                    .color(palette.text_dim),
            );
            ui.separator();
            if ui.button("Add effect to selected").clicked() {
                self.open_add_effect(FxStrip::Channel(self.selection.strip));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new("Meters are post-insert, pre-master")
                        .small()
                        .color(palette.text_dim),
                );
            });
        });
        ui.separator();
        let height = (ui.available_height() - 4.0).max(240.0);
        ui.horizontal_top(|ui| {
            let master_w = STRIP_W + 8.0;
            let strip_area = (ui.available_width() - master_w).max(180.0);
            ui.allocate_ui_with_layout(
                Vec2::new(strip_area, height),
                Layout::top_down(Align::Min),
                |ui| {
                    ScrollArea::horizontal()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.horizontal_top(|ui| {
                                for strip in 0..redraw_shared::MAX_CHANNELS {
                                    self.channel_strip(ui, strip, height);
                                }
                                for bus in 0..4 {
                                    self.bus_strip(ui, bus, height);
                                }
                            });
                        });
                },
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                Vec2::new(master_w, height),
                Layout::top_down(Align::Min),
                |ui| {
                    self.master_strip(ui, height);
                },
            );
        });
    }

    fn channel_strip(&mut self, ui: &mut egui::Ui, strip: usize, height: f32) {
        let palette = self.palette;
        let Some(state) = self.project.mixer.strips.get(strip).cloned() else {
            return;
        };
        let channel = self
            .project
            .channels
            .iter()
            .find(|c| c.mixer_index == strip)
            .cloned();
        let selected = self.selection.strip == strip;
        let name = channel
            .as_ref()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| state.name.clone());
        let color = channel
            .as_ref()
            .map(|c| c.color)
            .unwrap_or([96, 104, 112]);
        let mut gain = state.volume_db;
        let mut pan = state.pan;
        let mut mute = state.mute;
        let mut solo = state.solo;
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(STRIP_W, height.min(ui.available_height()).max(220.0)),
            Sense::click(),
        );
        let painter = ui.painter_at(rect);
        let bg = if selected {
            palette.accent.gamma_multiply(0.12)
        } else if response.hovered() {
            palette.raised
        } else {
            palette.panel_alt
        };
        painter.rect_filled(rect, egui::CornerRadius::same(2), bg);
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, Vec2::new(STRIP_W, 3.0)),
            egui::CornerRadius::same(1),
            crate::theme::color(color),
        );
        painter.text(
            egui::Pos2::new(rect.center().x, rect.top() + 7.0),
            egui::Align2::CENTER_TOP,
            trim_name(&name, 11),
            egui::FontId::proportional(9.5),
            palette.text,
        );
        if response.clicked() {
            if let Some(channel) = &channel {
                self.set_channel(channel.id);
            }
        }

        let content = rect.shrink2(Vec2::new(4.0, 6.0));
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(content).layout(Layout::top_down(Align::Center)),
            |ui| {
                ui.add_space(17.0);
                let inserts = state.inserts.clone();
                self.insert_grid(ui, FxStrip::Channel(strip), &inserts, 4);
                ui.add_space(2.0);
                let previous_pan = pan;
                pan_slider(ui, &mut pan, &palette);
                if pan != previous_pan {
                    self.set_strip_pan(strip, pan);
                }
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    let fader_h = (ui.available_height() - 66.0).clamp(72.0, 128.0);
                    let fader = ui.add_sized(
                        [16.0, fader_h],
                        egui::Slider::new(&mut gain, -90.0..=12.0)
                            .vertical()
                            .show_value(false),
                    );
                    if fader.changed() {
                        self.set_strip_gain(strip, gain);
                    }
                    let peak = self.audio.strip_peak(strip);
                    crate::widgets::meter(ui, &palette, peak, 7.0, fader_h, false);
                });
                ui.add_space(1.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_sized(
                            [28.0, 15.0],
                            egui::Button::new(RichText::new("M").size(8.5))
                                .selected(mute)
                                .frame(true),
                        )
                        .clicked()
                    {
                        mute = !mute;
                        self.set_strip_mute(strip, mute);
                    }
                    if ui
                        .add_sized(
                            [28.0, 15.0],
                            egui::Button::new(RichText::new("S").size(8.5))
                                .selected(solo)
                                .frame(true),
                        )
                        .clicked()
                    {
                        solo = !solo;
                        self.set_strip_solo(strip, solo);
                    }
                });
                ui.label(
                    RichText::new(format_db(gain))
                        .monospace()
                        .size(8.0)
                        .color(palette.text_dim),
                );
            },
        );
    }

    fn bus_strip(&mut self, ui: &mut egui::Ui, bus: usize, height: f32) {
        let palette = self.palette;
        let name = self
            .project
            .mixer
            .bus_names
            .get(bus)
            .cloned()
            .unwrap_or_else(|| format!("Bus {}", bus + 1));
        let inserts = self
            .project
            .mixer
            .bus_inserts
            .get(bus)
            .cloned()
            .unwrap_or_default();
        let mut volume = self
            .project
            .mixer
            .bus_volumes_db
            .get(bus)
            .copied()
            .unwrap_or(0.0);
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(STRIP_W, height.min(ui.available_height()).max(220.0)),
            Sense::click(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(
            rect,
            egui::CornerRadius::same(2),
            if response.hovered() {
                palette.raised
            } else {
                palette.panel_alt
            },
        );
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, Vec2::new(STRIP_W, 3.0)),
            egui::CornerRadius::same(1),
            palette.warn.gamma_multiply(0.65),
        );
        painter.text(
            egui::Pos2::new(rect.center().x, rect.top() + 7.0),
            egui::Align2::CENTER_TOP,
            trim_name(&name, 11),
            egui::FontId::proportional(9.5),
            palette.text,
        );
        let content = rect.shrink2(Vec2::new(4.0, 6.0));
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(content).layout(Layout::top_down(Align::Center)),
            |ui| {
                ui.add_space(17.0);
                self.insert_grid(ui, FxStrip::Bus(bus), &inserts, 4);
                ui.add_space(4.0);
                let fader_h = (ui.available_height() - 44.0).clamp(72.0, 128.0);
                let fader = ui.add_sized(
                    [16.0, fader_h],
                    egui::Slider::new(&mut volume, -90.0..=12.0)
                        .vertical()
                        .show_value(false),
                );
                if fader.changed() {
                    if let Some(target) = self.project.mixer.bus_volumes_db.get_mut(bus) {
                        *target = volume;
                    }
                    self.dirty = true;
                }
                ui.label(
                    RichText::new(format_db(volume))
                        .monospace()
                        .size(8.0)
                        .color(palette.text_dim),
                );
            },
        );
    }

    fn master_strip(&mut self, ui: &mut egui::Ui, height: f32) {
        let palette = self.palette;
        let state = self.project.mixer.master.clone();
        let mut gain = state.volume_db;
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(STRIP_W, height.min(ui.available_height()).max(220.0)),
            Sense::hover(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, egui::CornerRadius::same(2), palette.raised);
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, Vec2::new(STRIP_W, 3.0)),
            egui::CornerRadius::same(1),
            palette.accent,
        );
        painter.text(
            egui::Pos2::new(rect.center().x, rect.top() + 7.0),
            egui::Align2::CENTER_TOP,
            "MASTER",
            egui::FontId::proportional(9.5),
            palette.text,
        );
        let content = rect.shrink2(Vec2::new(4.0, 6.0));
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(content).layout(Layout::top_down(Align::Center)),
            |ui| {
                ui.add_space(17.0);
                self.insert_grid(ui, FxStrip::Master, &state.inserts, 4);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let fader_h = (ui.available_height() - 50.0).clamp(80.0, 150.0);
                    let fader = ui.add_sized(
                        [18.0, fader_h],
                        egui::Slider::new(&mut gain, -90.0..=12.0)
                            .vertical()
                            .show_value(false),
                    );
                    if fader.changed() {
                        self.update_master_gain(gain);
                    }
                    let (left, right) = self.audio.master_peak();
                    let peak = left.max(right);
                    crate::widgets::meter(ui, &palette, peak, 8.0, fader_h, false);
                });
                ui.label(
                    RichText::new(format_db(gain))
                        .monospace()
                        .size(8.0)
                        .color(palette.text_dim),
                );
            },
        );
    }

    fn insert_grid(
        &mut self,
        ui: &mut egui::Ui,
        strip: FxStrip,
        inserts: &[Option<redraw_shared::FxSlotState>],
        columns: usize,
    ) {
        let palette = self.palette;
        let rows = inserts.len().div_ceil(columns.max(1));
        for row in 0..rows {
            ui.horizontal(|ui| {
                for col in 0..columns {
                    let slot = row * columns + col;
                    let Some(effect) = inserts.get(slot) else {
                        continue;
                    };
                    let label = format!("{}", slot + 1);
                    let color = if effect.is_some() {
                        crate::theme::color([90, 150, 190])
                    } else {
                        palette.panel
                    };
                    let response = ui
                        .add_sized(
                            [29.0, 13.0],
                            egui::Button::new(
                                RichText::new(label).size(7.5).color(if effect.is_some() {
                                    palette.text
                                } else {
                                    palette.text_dim
                                }),
                            )
                            .fill(color)
                            .frame(false),
                        );
                    let response = if let Some(effect) = effect {
                        response.on_hover_text(format!(
                            "{} ({})",
                            effect.effect_type,
                            if effect.bypass { "bypassed" } else { "active" }
                        ))
                    } else {
                        response.on_hover_text("Empty insert slot")
                    };
                    if response.clicked() {
                        if effect.is_some() {
                            self.open_plugin_for_fx(strip, slot);
                        } else {
                            self.pending_fx = Some((strip, slot));
                            self.dialog = Some(crate::app::DialogKind::AddEffect);
                        }
                    }
                }
            });
        }
    }
}

fn trim_name(name: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if out.chars().count() >= max {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn pan_slider(ui: &mut egui::Ui, pan: &mut f32, palette: &crate::theme::Palette) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(58.0, 12.0), Sense::click_and_drag());
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius::same(2), palette.bg);
    let center = rect.center().x;
    painter.line_segment(
        [egui::Pos2::new(rect.left() + 4.0, rect.center().y), egui::Pos2::new(rect.right() - 4.0, rect.center().y)],
        Stroke::new(1.0_f32, palette.stroke),
    );
    painter.line_segment(
        [egui::Pos2::new(center, rect.top() + 2.0), egui::Pos2::new(center, rect.bottom() - 2.0)],
        Stroke::new(1.0_f32, palette.grid_strong),
    );
    if response.dragged() {
        *pan = (*pan + response.drag_delta().x / 40.0).clamp(-1.0, 1.0);
    }
    if response.double_clicked() {
        *pan = 0.0;
    }
    let x = rect.left() + 4.0 + (rect.width() - 8.0) * (*pan + 1.0) * 0.5;
    painter.circle_filled(
        egui::Pos2::new(x, rect.center().y),
        3.2,
        palette.accent,
    );
    response.on_hover_text(format_pan(*pan))
}

