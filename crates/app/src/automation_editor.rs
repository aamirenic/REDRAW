//! Automation editor: normalized envelopes for tempo, mixer, sends and plugin parameters.

use eframe::egui::{
    self, Align2, FontId, PointerButton, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, Vec2,
};

use redraw_shared::{AutoTarget, Interp};

use crate::app::{AutoDrag, LowerTab};
use crate::RedrawApp;

impl RedrawApp {
    pub fn automation_ui(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette;
        let clips: Vec<(u64, String)> = self
            .project
            .automation
            .iter()
            .map(|clip| (clip.id, clip.name.clone()))
            .collect();
        let targets = self.automation_targets();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Automation").strong().size(12.0));
            ui.separator();
            let current_label = self
                .selection
                .automation
                .and_then(|id| clips.iter().find(|(clip_id, _)| *clip_id == id))
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| "Select lane".into());
            egui::ComboBox::from_id_salt("automation_clip")
                .selected_text(current_label)
                .width(190.0)
                .show_ui(ui, |ui| {
                    for (id, name) in &clips {
                        if ui
                            .selectable_label(self.selection.automation == Some(*id), name)
                            .clicked()
                        {
                            self.selection.automation = Some(*id);
                            self.automation_view.selected_point = None;
                            self.layout.lower_tab = LowerTab::Automation;
                        }
                    }
                });
            if ui.small_button("+ Lane").clicked() {
                self.add_automation_lane();
            }
            let target_index = self
                .automation_target_index
                .min(targets.len().saturating_sub(1));
            egui::ComboBox::from_id_salt("new_automation_target")
                .selected_text(
                    targets
                        .get(target_index)
                        .map(|(label, _)| label.clone())
                        .unwrap_or_else(|| "Target".into()),
                )
                .width(160.0)
                .show_ui(ui, |ui| {
                    for (index, (label, _)) in targets.iter().enumerate() {
                        if ui
                            .selectable_label(target_index == index, label)
                            .clicked()
                        {
                            self.automation_target_index = index;
                        }
                    }
                });
            if ui.small_button("Delete").clicked() {
                self.delete_selection();
            }
            ui.separator();
            if ui.small_button("Add point").clicked() {
                self.add_automation_point_at_playhead();
            }
            if ui.small_button("Clear points").clicked() {
                self.clear_automation_points();
            }
            ui.separator();
            ui.label(RichText::new("Zoom").small().color(palette.text_dim));
            ui.add(
                egui::Slider::new(&mut self.automation_view.px_per_tick, 0.02..=0.6)
                    .logarithmic(true)
                    .show_value(false),
            );
        });
        ui.separator();
        let Some(id) = self.selection.automation else {
            ui.label(
                RichText::new(
                    "No automation lane selected. Add a lane to shape tempo, volume, pan, sends or a plugin parameter.",
                )
                .color(palette.text_dim),
            );
            return;
        };
        let Some(clip) = redraw_automation::find_clip(&self.project, id).cloned() else {
            self.selection.automation = None;
            return;
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Target").small().color(palette.text_dim));
            let current = redraw_automation::target_label(&clip.target);
            egui::ComboBox::from_id_salt("automation_target")
                .selected_text(current)
                .width(220.0)
                .show_ui(ui, |ui| {
                    for (label, target) in &targets {
                        if ui
                            .selectable_label(clip.target == *target, label)
                            .clicked()
                        {
                            if let Some(target_clip) =
                                redraw_automation::find_clip_mut(&mut self.project, id)
                            {
                                redraw_automation::set_clip_target(target_clip, target.clone());
                            }
                            self.dirty = true;
                            self.sync_schedules();
                        }
                    }
                });
            ui.label(RichText::new("Curve").small().color(palette.text_dim));
            let mut interp = clip.interp;
            egui::ComboBox::from_id_salt("automation_interp")
                .selected_text(format!("{interp:?}"))
                .width(86.0)
                .show_ui(ui, |ui| {
                    for value in [Interp::Linear, Interp::Smooth, Interp::Step] {
                        if ui
                            .selectable_label(interp == value, format!("{value:?}"))
                            .clicked()
                        {
                            interp = value;
                        }
                    }
                });
            if interp != clip.interp {
                if let Some(target_clip) = redraw_automation::find_clip_mut(&mut self.project, id) {
                    redraw_automation::set_clip_interp(target_clip, interp);
                }
                self.dirty = true;
                self.sync_schedules();
            }
            let mut enabled = clip.enabled;
            if ui.checkbox(&mut enabled, "Enabled").changed() {
                if let Some(target_clip) = redraw_automation::find_clip_mut(&mut self.project, id) {
                    redraw_automation::set_clip_enabled(target_clip, enabled);
                }
                self.dirty = true;
                self.sync_schedules();
            }
            ui.label(RichText::new("Length").small().color(palette.text_dim));
            let mut length = clip.length;
            if ui
                .add(
                    egui::DragValue::new(&mut length)
                        .range(1..=u64::MAX)
                        .speed(10.0)
                        .suffix(" t"),
                )
                .changed()
            {
                self.set_automation_length(id, length);
            }
            ui.label(
                RichText::new(format!("{} points", clip.points.len()))
                    .small()
                    .color(palette.text_dim),
            );
        });
        ui.add_space(2.0);
        let width = clip.length as f32 * self.automation_view.px_per_tick + 50.0;
        let height = (ui.available_height() - 4.0).max(220.0);
        ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(width.max(340.0), height),
                    Sense::click_and_drag(),
                );
                self.draw_automation(ui, rect, &response, &clip);
            });
    }

    fn draw_automation(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        response: &egui::Response,
        clip: &redraw_shared::AutomationClipState,
    ) {
        let palette = self.palette;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, egui::CornerRadius::ZERO, palette.bg);
        let left = rect.left() + 12.0;
        let right = rect.right() - 12.0;
        let top = rect.top() + 20.0;
        let bottom = rect.bottom() - 22.0;
        let plot = Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom));
        let px = self.automation_view.px_per_tick.max(0.001);
        let x_for = |tick: f64| left + (tick as f32 * px).min(plot.width());
        let y_for = |value: f32| bottom - value.clamp(0.0, 1.0) * plot.height();
        let tick_at = |x: f32| ((x - left) / px).max(0.0) as f64;
        let value_at = |y: f32| ((bottom - y) / plot.height()).clamp(0.0, 1.0);

        painter.rect_filled(plot, egui::CornerRadius::ZERO, palette.panel_alt);
        let bar = self.bar_ticks().max(1) as f64;
        let mut tick = 0.0f64;
        while tick <= clip.length as f64 {
            let x = x_for(tick);
            if x > plot.right() {
                break;
            }
            painter.line_segment(
                [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
                Stroke::new(1.0_f32, palette.grid),
            );
            painter.text(
                Pos2::new(x + 3.0, plot.top() + 2.0),
                Align2::LEFT_TOP,
                format!("{}", (tick / bar) as u64 + 1),
                FontId::monospace(8.0),
                palette.text_dim,
            );
            tick += bar;
        }
        for value in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
            let y = y_for(value);
            painter.line_segment(
                [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                Stroke::new(if (value - 0.5).abs() < 0.01 { 1.0_f32 } else { 0.6_f32 }, palette.grid),
            );
            painter.text(
                Pos2::new(plot.left() + 2.0, y - 1.0),
                Align2::LEFT_BOTTOM,
                format_automation_value(&clip.target, value),
                FontId::monospace(7.5),
                palette.text_dim,
            );
        }

        // Envelope polyline, sampled so Smooth curves render correctly.
        let samples = 256usize;
        let mut points = Vec::with_capacity(samples + 1);
        for index in 0..=samples {
            let t = clip.length as f64 * index as f64 / samples as f64;
            points.push(Pos2::new(x_for(t), y_for(clip.value_at(t))));
        }
        if points.len() > 1 {
            painter.add(egui::Shape::line(
                points,
                Stroke::new(1.7_f32, crate::theme::color(clip.color)),
            ));
        }

        // Points.
        for (index, point) in clip.points.iter().enumerate() {
            let pos = Pos2::new(x_for(point.time), y_for(point.value));
            let selected = self.automation_view.selected_point == Some(index);
            painter.circle_filled(
                pos,
                if selected { 5.2 } else { 4.0 },
                if selected { palette.text } else { crate::theme::color(clip.color) },
            );
            if selected {
                painter.circle_stroke(pos, 7.0, Stroke::new(1.0_f32, palette.accent));
            }
        }

        // Playhead.
        let play = x_for(self.current_tick() as f64);
        if play >= plot.left() && play <= plot.right() {
            painter.line_segment(
                [Pos2::new(play, plot.top()), Pos2::new(play, plot.bottom())],
                Stroke::new(1.2_f32, palette.rec),
            );
        }

        let Some(pos) = response.interact_pointer_pos() else {
            return;
        };
        let hit = clip.points.iter().enumerate().find(|(_, point)| {
            let point_pos = Pos2::new(x_for(point.time), y_for(point.value));
            point_pos.distance(pos) <= 8.0
        }).map(|(index, _)| index);

        if response.clicked_by(PointerButton::Secondary) {
            if let Some(index) = hit {
                self.begin_edit();
                if let Some(target) = redraw_automation::find_clip_mut(&mut self.project, clip.id) {
                    redraw_automation::remove_point(target, index);
                }
                self.commit_edit();
                self.automation_view.selected_point = None;
                self.sync_schedules();
            }
            return;
        }

        if response.drag_started_by(PointerButton::Primary) {
            if let Some(index) = hit {
                self.automation_view.selected_point = Some(index);
                self.begin_edit();
                self.automation_view.drag = Some(AutoDrag {
                    clip_id: clip.id,
                    point: index,
                    changed: false,
                });
            }
        }
        if response.dragged_by(PointerButton::Primary) {
            if let Some(mut drag) = self.automation_view.drag.take() {
                let time = tick_at(pos.x);
                let value = value_at(pos.y);
                if let Some(target) = redraw_automation::find_clip_mut(&mut self.project, drag.clip_id) {
                    if redraw_automation::move_point(target, drag.point, time, value).is_some() {
                        drag.changed = true;
                        self.schedule_dirty = true;
                    }
                }
                self.automation_view.drag = Some(drag);
            }
        }
        if response.drag_stopped_by(PointerButton::Primary) {
            if let Some(drag) = self.automation_view.drag.take() {
                if drag.changed {
                    self.commit_edit();
                    self.sync_schedules();
                } else {
                    self.history.cancel();
                }
            }
        }

        if response.clicked() {
            if let Some(index) = hit {
                self.automation_view.selected_point = Some(index);
            } else {
                let time = tick_at(pos.x);
                let value = value_at(pos.y);
                self.begin_edit();
                let added = if let Some(target) =
                    redraw_automation::find_clip_mut(&mut self.project, clip.id)
                {
                    redraw_automation::add_point(target, time, value, 0.0)
                } else {
                    None
                };
                if added.is_some() {
                    self.commit_edit();
                    self.automation_view.selected_point = added;
                    self.sync_schedules();
                } else {
                    self.history.cancel();
                }
            }
        }
    }

    fn automation_targets(&self) -> Vec<(String, AutoTarget)> {
        let channel_id = self.selection.channel_id;
        vec![
            ("Tempo".into(), AutoTarget::Tempo),
            ("Master volume".into(), AutoTarget::MasterVolume),
            (
                "Selected channel volume".into(),
                AutoTarget::ChannelVolume { channel_id },
            ),
            (
                "Selected channel pan".into(),
                AutoTarget::ChannelPan { channel_id },
            ),
            (
                "Selected channel parameter 1".into(),
                AutoTarget::ChannelParam {
                    channel_id,
                    param: 0,
                },
            ),
            (
                "Selected channel insert 1 parameter 1".into(),
                AutoTarget::FxParam {
                    channel_id,
                    slot: 0,
                    param: 0,
                },
            ),
            (
                "Selected channel send 1".into(),
                AutoTarget::SendLevel { channel_id, send: 0 },
            ),
        ]
    }

    fn initial_value_for_target(&self, target: &AutoTarget) -> f32 {
        match target {
            AutoTarget::Tempo => redraw_automation::tempo_normalized(self.project.bpm),
            AutoTarget::MasterVolume => {
                redraw_automation::volume_normalized(self.project.mixer.master.volume_db)
            }
            AutoTarget::ChannelVolume { channel_id } => self
                .project
                .channel(*channel_id)
                .map(|c| redraw_automation::volume_normalized(c.volume_db))
                .unwrap_or(0.5),
            AutoTarget::ChannelPan { channel_id } => self
                .project
                .channel(*channel_id)
                .map(|c| redraw_automation::pan_normalized(c.pan))
                .unwrap_or(0.5),
            AutoTarget::ChannelParam { .. }
            | AutoTarget::FxParam { .. }
            | AutoTarget::SendLevel { .. } => 0.5,
        }
    }

    fn add_automation_lane(&mut self) {
        let targets = self.automation_targets();
        let index = self.automation_target_index.min(targets.len().saturating_sub(1));
        let (label, target) = targets[index].clone();
        let initial = self.initial_value_for_target(&target);
        let length = redraw_arrangement::playlist_length(&self.project)
            .max(self.bar_ticks().saturating_mul(16))
            .max(1);
        let color = crate::theme::PATTERN_COLORS[self.project.automation.len() % 16];
        self.begin_edit();
        if let Some(id) = redraw_automation::add_named_clip(
            &mut self.project,
            format!("{label} Automation"),
            target,
            length,
            initial,
            color,
        ) {
            self.selection.automation = Some(id);
            self.automation_view.selected_point = None;
            self.layout.lower_tab = LowerTab::Automation;
            self.commit_edit();
            self.sync_schedules();
            self.set_status(format!("Added automation lane: {label}"));
        } else {
            self.history.cancel();
            self.set_error("Automation lane limit reached");
        }
    }

    fn add_automation_point_at_playhead(&mut self) {
        let Some(id) = self.selection.automation else {
            return;
        };
        let tick = self.current_tick() as f64;
        let value = redraw_automation::find_clip(&self.project, id)
            .map(|clip| clip.value_at(tick))
            .unwrap_or(0.5);
        self.begin_edit();
        let added = if let Some(clip) = redraw_automation::find_clip_mut(&mut self.project, id) {
            redraw_automation::add_point(clip, tick, value, 0.0)
        } else {
            None
        };
        if added.is_some() {
            self.commit_edit();
            self.automation_view.selected_point = added;
            self.sync_schedules();
        } else {
            self.history.cancel();
        }
    }

    fn clear_automation_points(&mut self) {
        let Some(id) = self.selection.automation else {
            return;
        };
        self.begin_edit();
        if let Some(clip) = redraw_automation::find_clip_mut(&mut self.project, id) {
            redraw_automation::clear_points(clip);
        }
        self.commit_edit();
        self.automation_view.selected_point = None;
        self.sync_schedules();
    }
}

fn format_automation_value(target: &AutoTarget, normalized: f32) -> String {
    if let Some(value) = redraw_automation::denormalize_target(target, normalized) {
        if let Some(range) = redraw_automation::fixed_range(target) {
            if range.unit.is_empty() {
                format!("{value:.2}")
            } else {
                format!("{value:.1}{}", range.unit)
            }
        } else {
            format!("{value:.2}")
        }
    } else {
        format!("{:.0}%", normalized * 100.0)
    }
}
