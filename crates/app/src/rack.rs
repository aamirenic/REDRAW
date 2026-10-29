//! Channel rack and step sequencer.

use eframe::egui::{self, Align2, FontId, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, Vec2};

use crate::app::LowerTab;
use crate::widgets::{format_db, format_pan};
use crate::RedrawApp;

impl RedrawApp {
    pub fn rack_ui(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(
                    self.pattern()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "Pattern".into()),
                )
                .strong(),
            );
            ui.label(
                RichText::new(format!(
                    "{} steps",
                    self.pattern().map(|p| p.length_steps).unwrap_or(16)
                ))
                .small()
                .color(palette.text_dim),
            );
            ui.separator();
            if ui.button("+ Instrument").clicked() {
                self.dialog = Some(crate::app::DialogKind::AddChannel);
            }
            if ui.button("+ Pattern").clicked() {
                self.add_pattern();
            }
            if ui.button("Clone Pattern").clicked() {
                self.clone_pattern();
            }
            if ui.button("Clear steps").clicked() {
                self.clear_pattern();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Piano roll").clicked() {
                    self.set_lower_tab(LowerTab::PianoRoll);
                }
            });
        });
        ui.separator();
        ui.horizontal_top(|ui| {
            let list_width = 304.0_f32.min(ui.available_width() * 0.45);
            ui.allocate_ui_with_layout(
                Vec2::new(list_width, ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let channels = self.project.channels.clone();
                            for channel in channels {
                                self.channel_row(ui, channel.id);
                            }
                            ui.add_space(6.0);
                            if ui
                                .add_sized(
                                    [ui.available_width(), 24.0],
                                    egui::Button::new("+ Add instrument"),
                                )
                                .clicked()
                            {
                                self.dialog = Some(crate::app::DialogKind::AddChannel);
                            }
                        });
                },
            );
            ui.separator();
            let selected = self.selection.channel_id;
            if self.project.channel(selected).is_none() {
                ui.label(RichText::new("Select a channel to edit steps.").color(palette.text_dim));
                return;
            }
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    self.step_editor(ui, selected);
                },
            );
        });
    }

    fn channel_row(&mut self, ui: &mut egui::Ui, channel_id: u32) {
        let palette = self.palette;
        let Some(channel) = self.project.channel(channel_id) else {
            return;
        };
        let selected = self.selection.channel_id == channel_id;
        let color = channel.color;
        let name = channel.name.clone();
        let mute = channel.mute;
        let solo = channel.solo;
        let volume = channel.volume_db;
        let pan = channel.pan;
        let synth = channel.synth_type.clone();
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), 31.0),
            Sense::click(),
        );
        let bg = if selected {
            palette.accent.gamma_multiply(0.22)
        } else if response.hovered() {
            palette.raised
        } else {
            palette.panel_alt
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(2), bg);
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(4.0, rect.height())),
            egui::CornerRadius::same(1),
            crate::theme::color(color),
        );
        ui.painter().text(
            Pos2::new(rect.left() + 10.0, rect.center().y - 1.0),
            Align2::LEFT_CENTER,
            &name,
            FontId::proportional(11.5),
            palette.text,
        );
        ui.painter().text(
            Pos2::new(rect.left() + 10.0, rect.center().y + 11.0),
            Align2::LEFT_TOP,
            format!("{} | {} | {}", synth, format_db(volume), format_pan(pan)),
            FontId::proportional(8.5),
            palette.text_dim,
        );
        let m = Rect::from_min_size(
            Pos2::new(rect.right() - 48.0, rect.top() + 7.0),
            Vec2::new(18.0, 17.0),
        );
        let s = Rect::from_min_size(
            Pos2::new(rect.right() - 26.0, rect.top() + 7.0),
            Vec2::new(18.0, 17.0),
        );
        ui.painter().rect_filled(
            m,
            egui::CornerRadius::same(2),
            if mute { palette.rec.gamma_multiply(0.65) } else { palette.panel },
        );
        ui.painter().rect_filled(
            s,
            egui::CornerRadius::same(2),
            if solo { palette.warn.gamma_multiply(0.65) } else { palette.panel },
        );
        ui.painter().text(m.center(), Align2::CENTER_CENTER, "M", FontId::proportional(9.5), palette.text);
        ui.painter().text(s.center(), Align2::CENTER_CENTER, "S", FontId::proportional(9.5), palette.text);
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                if m.contains(pos) {
                    self.update_channel_mute(channel_id, !mute);
                } else if s.contains(pos) {
                    self.update_channel_solo(channel_id, !solo);
                } else {
                    self.set_channel(channel_id);
                }
            }
        }
        if response.double_clicked() {
            self.open_plugin_for_channel(channel_id);
        }
        response.on_hover_text("Click to select. Double-click for the instrument.");
    }

    fn step_editor(&mut self, ui: &mut egui::Ui, channel_id: u32) {
        let palette = self.palette;
        let Some(channel) = self.project.channel(channel_id) else {
            return;
        };
        let channel_name = channel.name.clone();
        let synth_type = channel.synth_type.clone();
        let color = channel.color;
        let steps_len = self
            .pattern()
            .map(|p| p.length_steps as usize)
            .unwrap_or(16)
            .max(1);
        let active_steps = self
            .pattern()
            .and_then(|p| p.channel(channel_id))
            .map(|c| c.steps.clone())
            .unwrap_or_default();
        let step_ticks = redraw_shared::PPQ as u64 / 4;
        let play_step = if self.audio.is_playing() {
            ((self.current_tick() / step_ticks) as usize) % steps_len
        } else {
            usize::MAX
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(&channel_name).strong());
            ui.label(
                RichText::new(format!("{synth_type} | {steps_len} steps"))
                    .small()
                    .color(palette.text_dim),
            );
            if ui.small_button("Instrument").clicked() {
                self.open_plugin_for_channel(channel_id);
            }
            if ui.small_button("Piano roll").clicked() {
                self.set_lower_tab(LowerTab::PianoRoll);
            }
        });
        ui.add_space(3.0);
        let per_row = 16usize.min(steps_len);
        let cell = Vec2::new(26.0, 34.0);
        let accent = crate::theme::color(color);
        let mut toggled = None;
        let mut paint = None;
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for row_start in (0..steps_len).step_by(per_row.max(1)) {
                    ui.horizontal(|ui| {
                        for index in row_start..(row_start + per_row).min(steps_len) {
                            let (rect, response) = ui.allocate_exact_size(cell, Sense::click_and_drag());
                            let state = active_steps.get(index).copied().unwrap_or_default();
                            let is_play = index == play_step;
                            let fill = if state.active {
                                accent.gamma_multiply(0.75)
                            } else if response.hovered() {
                                palette.raised
                            } else {
                                palette.panel_alt
                            };
                            ui.painter().rect_filled(rect, egui::CornerRadius::same(2), fill);
                            if is_play {
                                ui.painter().rect_stroke(
                                    rect,
                                    egui::CornerRadius::same(2),
                                    Stroke::new(1.4_f32, palette.rec),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if state.active {
                                let h = rect.height() * (state.velocity as f32 / 127.0).clamp(0.15, 1.0);
                                ui.painter().rect_filled(
                                    Rect::from_min_size(
                                        Pos2::new(rect.left() + 3.0, rect.bottom() - h - 3.0),
                                        Vec2::new(rect.width() - 6.0, h),
                                    ),
                                    egui::CornerRadius::same(1),
                                    palette.text.gamma_multiply(0.22),
                                );
                            }
                            ui.painter().text(
                                Pos2::new(rect.center().x, rect.top() + 4.0),
                                Align2::CENTER_TOP,
                                format!("{}", index + 1),
                                FontId::monospace(8.5),
                                if state.active { palette.text } else { palette.text_dim },
                            );
                            if response.drag_started() {
                                self.begin_edit();
                                let active = !state.active;
                                self.step_paint = Some(crate::app::StepPaint {
                                    channel_id,
                                    step: index as u32,
                                    active,
                                });
                                self.paint_step(channel_id, index, active);
                            } else if response.dragged() {
                                if let Some(paint) = self.step_paint {
                                    self.paint_step(paint.channel_id, index, paint.active);
                                }
                            } else if response.clicked() {
                                toggled = Some(index);
                            } else if response.secondary_clicked() {
                                if state.active {
                                    self.set_step_velocity(channel_id, index, 64);
                                }
                            }
                            if response.dragged() {
                                paint = Some(());
                            }
                        }
                    });
                }
                if steps_len > per_row {
                    ui.label(
                        RichText::new("Rows wrap every 16 steps.")
                            .small()
                            .color(palette.text_dim),
                    );
                }
            });
        let _ = paint;
        if let Some(index) = toggled {
            self.toggle_step(channel_id, index);
        }
        if ui.input(|i| i.pointer.any_released()) && self.step_paint.is_some() {
            self.step_paint = None;
            self.commit_edit();
            self.sync_schedules();
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Steps are 16th notes at 960 PPQ. Current pattern tick {} / {}.", self.current_tick(), self.pattern().map(|p| p.length_ticks()).unwrap_or(0)))
                    .small()
                    .color(palette.text_dim),
            );
        });
    }
}
