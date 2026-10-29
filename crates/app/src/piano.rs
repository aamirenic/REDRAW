//! Piano roll editor: note drawing, selection, resize, velocity and CC lanes.

use eframe::egui::{
    self, Align2, FontId, PointerButton, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind,
    Vec2,
};

use crate::app::{NoteDrag, NoteDragMode};
use crate::widgets::{draw_timeline_grid, is_black_key, note_name};
use crate::RedrawApp;

const KEY_W: f32 = 54.0;
const RULER_H: f32 = 21.0;

struct PianoGeometry {
    rect: Rect,
    timeline_left: f32,
    timeline_top: f32,
    lane_top: f32,
    lane_height: f32,
    px_per_tick: f32,
    row_height: f32,
    grid: u64,
}

impl PianoGeometry {
    fn tick_at(&self, x: f32) -> u64 {
        let tick = ((x - self.timeline_left) / self.px_per_tick.max(0.0001)).max(0.0) as u64;
        let grid = self.grid.max(1);
        (tick + grid / 2) / grid * grid
    }

    fn raw_tick_at(&self, x: f32) -> u64 {
        ((x - self.timeline_left) / self.px_per_tick.max(0.0001)).max(0.0) as u64
    }

    fn key_at(&self, y: f32) -> u8 {
        let row = ((y - self.timeline_top) / self.row_height.max(1.0)).floor() as i32;
        (127 - row).clamp(0, 127) as u8
    }

    fn x_for_tick(&self, tick: u64) -> f32 {
        self.timeline_left + tick as f32 * self.px_per_tick
    }

    fn y_for_key(&self, key: u8) -> f32 {
        self.timeline_top + (127 - key.min(127)) as f32 * self.row_height
    }

    fn note_rect(&self, note: &redraw_shared::Note) -> Rect {
        let left = self.x_for_tick(note.start);
        let right = self.x_for_tick(note.start.saturating_add(note.length)).max(left + 2.0);
        let top = self.y_for_key(note.key);
        Rect::from_min_max(
            Pos2::new(left, top + 1.0),
            Pos2::new(right, top + self.row_height - 1.0),
        )
    }
}

impl RedrawApp {
    pub fn piano_roll_ui(&mut self, ui: &mut egui::Ui) {
        // Release an on-screen keyboard preview once the mouse is up.
        if let Some(key) = self.piano_preview {
            if !ui.input(|i| i.pointer.primary_down()) {
                self.preview_note_off(key);
                self.piano_preview = None;
            }
        }
        let palette = self.palette;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Tools").small().color(palette.text_dim));
            for (tool, label) in [
                (redraw_piano_roll::Tool::Draw, "Draw"),
                (redraw_piano_roll::Tool::Select, "Select"),
                (redraw_piano_roll::Tool::Erase, "Erase"),
                (redraw_piano_roll::Tool::Split, "Split"),
                (redraw_piano_roll::Tool::Merge, "Merge"),
            ] {
                if ui
                    .selectable_label(self.piano.tool == tool, label)
                    .clicked()
                {
                    self.piano.tool = tool;
                }
            }
            ui.separator();
            ui.label(RichText::new("Grid").small().color(palette.text_dim));
            egui::ComboBox::from_id_salt("piano_grid")
                .selected_text(
                    redraw_piano_roll::GRIDS
                        .iter()
                        .find(|(value, _)| *value == self.piano.grid)
                        .map(|(_, label)| *label)
                        .unwrap_or("1/16"),
                )
                .width(66.0)
                .show_ui(ui, |ui| {
                    for (value, label) in redraw_piano_roll::GRIDS {
                        if ui
                            .selectable_label(self.piano.grid == value, label)
                            .clicked()
                        {
                            self.piano.set_grid(value);
                        }
                    }
                });
            ui.separator();
            ui.label(RichText::new("Scale").small().color(palette.text_dim));
            egui::ComboBox::from_id_salt("piano_root")
                .selected_text(note_name(self.piano.scale_root + 60))
                .width(52.0)
                .show_ui(ui, |ui| {
                    for root in 0..12u8 {
                        if ui
                            .selectable_label(
                                self.piano.scale_root == root,
                                note_name(root + 60),
                            )
                            .clicked()
                        {
                            self.piano.scale_root = root;
                        }
                    }
                });
            egui::ComboBox::from_id_salt("piano_scale")
                .selected_text(self.piano.scale.name())
                .width(106.0)
                .show_ui(ui, |ui| {
                    for scale in redraw_piano_roll::Scale::ALL {
                        if ui
                            .selectable_label(self.piano.scale == scale, scale.name())
                            .clicked()
                        {
                            self.piano.scale = scale;
                        }
                    }
                });
            ui.separator();
            ui.label(RichText::new("Velocity").small().color(palette.text_dim));
            let mut velocity = self.piano.default_velocity as i32;
            if ui
                .add(egui::DragValue::new(&mut velocity).range(1..=127))
                .changed()
            {
                self.piano.default_velocity = velocity.clamp(1, 127) as u8;
            }
            ui.label(RichText::new("Pan").small().color(palette.text_dim));
            let mut pan = self.piano.default_pan as i32;
            if ui
                .add(egui::DragValue::new(&mut pan).range(-64..=63))
                .changed()
            {
                self.piano.default_pan = pan.clamp(-64, 63) as i8;
            }
            ui.separator();
            if ui.small_button("Quantize").clicked() {
                self.dialog = Some(crate::app::DialogKind::Quantize);
            }
            if ui.small_button("Humanize").clicked() {
                self.dialog = Some(crate::app::DialogKind::Humanize);
            }
            if ui.small_button("Transpose").clicked() {
                self.dialog = Some(crate::app::DialogKind::Transpose);
            }
            if ui.small_button("Legato").clicked() {
                self.begin_edit();
                if let Some(pattern) = self.project.pattern_mut() {
                    self.piano.legato(pattern, 0);
                }
                self.commit_edit();
                self.schedule_dirty = true;
            }
            ui.separator();
            ui.label(RichText::new("Zoom").small().color(palette.text_dim));
            ui.add(
                egui::Slider::new(&mut self.piano_view.px_per_tick, 0.02..=0.5)
                    .logarithmic(true)
                    .show_value(false),
            );
            if ui.small_button("Fit").clicked() {
                let ticks = self.pattern().map(|p| p.length_ticks()).unwrap_or(1).max(1);
                let width = (ui.available_width() - KEY_W - 30.0).max(180.0);
                self.piano_view.px_per_tick = (width / ticks as f32).clamp(0.02, 0.5);
            }
        });
        ui.separator();
        let Some(pattern) = self.pattern() else {
            ui.label(RichText::new("No pattern").color(palette.text_dim));
            return;
        };
        let pattern_len = pattern.length_ticks().max(self.bar_ticks().saturating_mul(4)).max(1);
        let notes = pattern
            .channel(self.selection.channel_id)
            .map(|c| c.notes.clone())
            .unwrap_or_default();
        let cc = pattern
            .channel(self.selection.channel_id)
            .map(|c| c.cc.clone())
            .unwrap_or_default();
        let ghosts = self.piano.ghost_notes(&self.project);
        let px = self.piano_view.px_per_tick;
        let row_h = self.piano_view.row_height;
        let lane_h = self.piano_view.lane_height;
        let width = KEY_W + pattern_len as f32 * px + 80.0;
        let height = RULER_H + 128.0 * row_h + lane_h + 12.0;
        ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(width.max(360.0), height.max(220.0)),
                    Sense::click_and_drag(),
                );
                let timeline_top = rect.top() + RULER_H;
                let lane_top = timeline_top + 128.0 * row_h;
                let g = PianoGeometry {
                    rect,
                    timeline_left: rect.left() + KEY_W,
                    timeline_top,
                    lane_top,
                    lane_height: lane_h,
                    px_per_tick: px,
                    row_height: row_h,
                    grid: self.piano.grid,
                };
                self.draw_piano(ui, &g, &response, &notes, &cc, &ghosts, pattern_len);
            });
    }

    fn draw_piano(
        &mut self,
        ui: &mut egui::Ui,
        g: &PianoGeometry,
        response: &egui::Response,
        notes: &[redraw_shared::Note],
        cc: &[redraw_shared::CcEvent],
        ghosts: &[redraw_piano_roll::GhostNote],
        pattern_len: u64,
    ) {
        let palette = self.palette;
        let painter = ui.painter_at(g.rect);
        painter.rect_filled(g.rect, egui::CornerRadius::ZERO, palette.bg);
        let lane_rect = Rect::from_min_max(
            Pos2::new(g.timeline_left, g.lane_top),
            Pos2::new(g.rect.right(), g.lane_top + g.lane_height),
        );
        let timeline_rect = Rect::from_min_max(
            Pos2::new(g.timeline_left, g.timeline_top),
            Pos2::new(g.rect.right(), g.lane_top),
        );

        // Key rows.
        for key in 0..128u8 {
            let y = g.y_for_key(key);
            let row = Rect::from_min_max(
                Pos2::new(g.rect.left(), y),
                Pos2::new(g.rect.right(), y + g.row_height),
            );
            let black = is_black_key(key);
            let fill = if black {
                palette.bg.gamma_multiply(0.82)
            } else if key % 12 == 0 {
                palette.panel_alt.gamma_multiply(0.72)
            } else {
                palette.bg
            };
            painter.rect_filled(row, egui::CornerRadius::ZERO, fill);
            painter.line_segment(
                [Pos2::new(row.left(), row.bottom()), Pos2::new(row.right(), row.bottom())],
                Stroke::new(0.4_f32, palette.grid.gamma_multiply(0.55)),
            );
        }
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(g.rect.left(), g.timeline_top),
                Pos2::new(g.timeline_left, g.lane_top),
            ),
            egui::CornerRadius::ZERO,
            palette.panel,
        );
        for key in 0..128u8 {
            let y = g.y_for_key(key);
            let rect = Rect::from_min_max(
                Pos2::new(g.rect.left(), y),
                Pos2::new(g.timeline_left - 1.0, y + g.row_height),
            );
            let black = is_black_key(key);
            painter.rect_filled(
                rect,
                egui::CornerRadius::ZERO,
                if black { palette.bg } else { palette.panel_alt },
            );
            if !black {
                painter.line_segment(
                    [Pos2::new(rect.left(), rect.bottom()), Pos2::new(rect.right(), rect.bottom())],
                    Stroke::new(0.5_f32, palette.grid_strong),
                );
            }
            if key % 12 == 0 {
                painter.text(
                    Pos2::new(rect.right() - 4.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    note_name(key),
                    FontId::monospace(7.5),
                    if black { palette.text_dim } else { palette.text },
                );
            }
        }

        // Timeline grid and ruler.
        draw_timeline_grid(
            &painter,
            timeline_rect,
            0.0,
            g.px_per_tick,
            self.bar_ticks().max(1),
            self.piano.grid.max(1),
            &palette,
        );
        let ruler = Rect::from_min_max(
            Pos2::new(g.timeline_left, g.rect.top()),
            Pos2::new(g.rect.right(), g.timeline_top),
        );
        painter.rect_filled(ruler, egui::CornerRadius::ZERO, palette.panel_alt);
        painter.line_segment(
            [Pos2::new(ruler.left(), ruler.bottom()), Pos2::new(ruler.right(), ruler.bottom())],
            Stroke::new(1.0_f32, palette.stroke),
        );
        let bar = self.bar_ticks().max(1);
        let mut tick = 0u64;
        while tick <= pattern_len + bar {
            let x = g.x_for_tick(tick);
            if x > ruler.right() {
                break;
            }
            painter.line_segment(
                [Pos2::new(x, ruler.bottom() - 5.0), Pos2::new(x, ruler.bottom())],
                Stroke::new(1.0_f32, palette.grid_strong),
            );
            painter.text(
                Pos2::new(x + 3.0, ruler.top() + 2.0),
                Align2::LEFT_TOP,
                format!("{}", tick / bar + 1),
                FontId::monospace(8.5),
                palette.text_dim,
            );
            tick = tick.saturating_add(bar);
        }

        // Ghost notes from other patterns.
        for ghost in ghosts {
            let rect = g.note_rect(&ghost.note);
            if rect.intersects(timeline_rect) {
                painter.rect_filled(
                    rect,
                    egui::CornerRadius::same(1),
                    palette.text_dim.gamma_multiply(0.16),
                );
            }
        }

        // CC lane.
        painter.rect_filled(lane_rect, egui::CornerRadius::ZERO, palette.panel_alt);
        painter.line_segment(
            [Pos2::new(lane_rect.left(), lane_rect.top()), Pos2::new(lane_rect.right(), lane_rect.top())],
            Stroke::new(1.0_f32, palette.stroke),
        );
        let cc_points: Vec<Pos2> = cc
            .iter()
            .filter(|c| c.controller == self.piano_view.active_cc)
            .map(|c| {
                Pos2::new(
                    g.x_for_tick(c.tick),
                    lane_rect.bottom() - lane_rect.height() * (c.value as f32 / 127.0),
                )
            })
            .collect();
        if cc_points.len() > 1 {
            painter.add(egui::Shape::line(
                cc_points.clone(),
                Stroke::new(1.4_f32, palette.accent),
            ));
        }
        for point in &cc_points {
            painter.circle_filled(*point, 2.2, palette.accent);
        }
        painter.text(
            Pos2::new(lane_rect.left() + 4.0, lane_rect.top() + 2.0),
            Align2::LEFT_TOP,
            format!("CC {}", self.piano_view.active_cc),
            FontId::monospace(8.0),
            palette.text_dim,
        );

        // Notes.
        let accent = self
            .selected_channel()
            .map(|c| crate::theme::color(c.color))
            .unwrap_or(palette.accent);
        for note in notes {
            let rect = g.note_rect(note);
            if !rect.intersects(timeline_rect) {
                continue;
            }
            let selected = self.piano.is_selected(note.id);
            let velocity = (note.velocity as f32 / 127.0).clamp(0.25, 1.0);
            painter.rect_filled(
                rect,
                egui::CornerRadius::same(2),
                accent.gamma_multiply(0.55 + 0.35 * velocity),
            );
            if selected {
                painter.rect_stroke(
                    rect,
                    egui::CornerRadius::same(2),
                    Stroke::new(1.6_f32, palette.text),
                    StrokeKind::Inside,
                );
            }
            if rect.width() > 26.0 && rect.height() > 8.0 {
                painter.text(
                    Pos2::new(rect.left() + 3.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    note_name(note.key),
                    FontId::monospace(7.5),
                    palette.text.gamma_multiply(0.85),
                );
            }
        }

        // Playhead.
        let play_x = g.x_for_tick(self.current_tick());
        if play_x >= timeline_rect.left() && play_x <= timeline_rect.right() {
            painter.line_segment(
                [Pos2::new(play_x, g.rect.top()), Pos2::new(play_x, lane_rect.bottom())],
                Stroke::new(1.3_f32, palette.rec),
            );
        }

        self.piano_interactions(ui, g, response, notes, cc);
    }

    fn piano_interactions(
        &mut self,
        ui: &mut egui::Ui,
        g: &PianoGeometry,
        response: &egui::Response,
        notes: &[redraw_shared::Note],
        cc: &[redraw_shared::CcEvent],
    ) {
        let Some(pos) = response.interact_pointer_pos() else {
            return;
        };
        let in_timeline = pos.x >= g.timeline_left && pos.y >= g.timeline_top && pos.y < g.lane_top;
        let in_keyboard = pos.x < g.timeline_left && pos.y >= g.timeline_top && pos.y < g.lane_top;
        let in_ruler = pos.y < g.timeline_top && pos.x >= g.timeline_left;
        let in_lane = pos.y >= g.lane_top && pos.x >= g.timeline_left;
        let tick = g.tick_at(pos.x);
        let raw_tick = g.raw_tick_at(pos.x);
        let key = g.key_at(pos.y);

        if in_keyboard && (response.clicked() || response.drag_started_by(PointerButton::Primary)) {
            self.preview_note_on(key);
            self.piano_preview = Some(key);
        }
        if in_ruler && response.clicked() {
            self.audio.seek(tick);
            return;
        }
        if in_lane && response.clicked() {
            let value = ((g.lane_top + g.lane_height - pos.y) / g.lane_height)
                .clamp(0.0, 1.0);
            self.begin_edit();
            let controller = self.piano_view.active_cc;
            if let Some(pattern) = self.project.pattern_mut() {
                self.piano.add_cc(pattern, tick, controller, (value * 127.0).round() as u8);
            }
            self.commit_edit();
            self.schedule_dirty = true;
            return;
        }
        if in_timeline && response.drag_started_by(PointerButton::Primary) {
            let hit = notes.iter().find(|n| {
                n.key == key && n.start <= tick && n.start.saturating_add(n.length) > tick
            });
            let shift = ui.input(|i| i.modifiers.shift);
            match self.piano.tool {
                redraw_piano_roll::Tool::Erase => {
                    if let Some(note) = hit {
                        self.begin_edit();
                        self.piano.select([note.id]);
                        if let Some(pattern) = self.project.pattern_mut() {
                            self.piano.delete_selected(pattern);
                        }
                        self.commit_edit();
                        self.schedule_dirty = true;
                    }
                    return;
                }
                redraw_piano_roll::Tool::Split => {
                    if let Some(note) = hit {
                        self.begin_edit();
                        if let Some(pattern) = self.project.pattern_mut() {
                            self.piano.split(pattern, note.id, raw_tick);
                        }
                        self.commit_edit();
                        self.schedule_dirty = true;
                    }
                    return;
                }
                redraw_piano_roll::Tool::Merge => {
                    self.begin_edit();
                    if let Some(pattern) = self.project.pattern_mut() {
                        self.piano.merge_selected(pattern);
                    }
                    self.commit_edit();
                    self.schedule_dirty = true;
                    return;
                }
                redraw_piano_roll::Tool::Select | redraw_piano_roll::Tool::Draw => {}
            }
            if let Some(note) = hit {
                if !self.piano.selection.contains(&note.id) {
                    if shift {
                        self.piano.toggle_note(note.id);
                    } else {
                        self.piano.select([note.id]);
                    }
                } else if shift {
                    self.piano.deselect_note(note.id);
                }
                let originals = self
                    .pattern()
                    .map(|p| self.piano.selected_notes(p))
                    .unwrap_or_default();
                if originals.is_empty() {
                    return;
                }
                let rect = g.note_rect(note);
                let mode = if (rect.right() - pos.x).abs() <= 7.0 {
                    NoteDragMode::Resize
                } else {
                    NoteDragMode::Move
                };
                self.begin_edit();
                self.note_drag = Some(NoteDrag {
                    ids: originals.iter().map(|n| n.id).collect(),
                    mode,
                    start_pointer_tick: tick,
                    start_pointer_key: key,
                    originals,
                    changed: false,
                });
            } else if self.piano.tool == redraw_piano_roll::Tool::Draw {
                self.begin_edit();
                let length = self.piano.last_length.max(self.piano.grid).max(1);
                let note = if let Some(pattern) = self.project.pattern_mut() {
                    self.piano.draw(pattern, tick, key, Some(length))
                } else {
                    self.history.cancel();
                    return;
                };
                self.note_drag = Some(NoteDrag {
                    ids: vec![note.id],
                    mode: NoteDragMode::Resize,
                    start_pointer_tick: tick,
                    start_pointer_key: key,
                    originals: vec![note],
                    changed: true,
                });
            } else if !shift {
                self.piano.clear_selection();
            }
        }

        if response.dragged_by(PointerButton::Primary) {
            if let Some(mut drag) = self.note_drag.take() {
                let delta_tick = tick as i64 - drag.start_pointer_tick as i64;
                let delta_key = key as i32 - drag.start_pointer_key as i32;
                if delta_tick != 0 || delta_key != 0 {
                    let channel_id = self.piano.channel_id;
                    let grid = self.piano.grid.max(1) as i64;
                    if let Some(pattern) = self.project.pattern_mut() {
                        if let Some(pc) = pattern
                            .channels
                            .iter_mut()
                            .find(|c| c.channel_id == channel_id)
                        {
                            for original in &drag.originals {
                                if let Some(note) = pc.notes.iter_mut().find(|n| n.id == original.id) {
                                    match drag.mode {
                                        NoteDragMode::Move => {
                                            note.start = (original.start as i64 + delta_tick).max(0) as u64;
                                            note.key = (original.key as i32 + delta_key).clamp(0, 127) as u8;
                                        }
                                        NoteDragMode::Resize => {
                                            note.length = (original.length as i64 + delta_tick)
                                                .max(grid)
                                                .max(1) as u64;
                                        }
                                        NoteDragMode::Velocity => {
                                            note.velocity = (original.velocity as i32 + delta_key)
                                                .clamp(1, 127) as u8;
                                        }
                                        NoteDragMode::Pan => {
                                            note.pan = (original.pan as i32 + delta_key)
                                                .clamp(-64, 63) as i8;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    drag.changed = true;
                    self.schedule_dirty = true;
                }
                self.note_drag = Some(drag);
            }
        }
        if response.drag_stopped_by(PointerButton::Primary) {
            if let Some(drag) = self.note_drag.take() {
                if drag.changed {
                    self.commit_edit();
                    self.sync_schedules();
                } else {
                    self.history.cancel();
                }
            }
            if let Some(key) = self.piano_preview.take() {
                self.preview_note_off(key);
            }
        }

        if response.clicked() && in_timeline {
            let hit = notes.iter().find(|n| {
                n.key == key && n.start <= tick && n.start.saturating_add(n.length) > tick
            });
            let shift = ui.input(|i| i.modifiers.shift);
            match self.piano.tool {
                redraw_piano_roll::Tool::Draw => {
                    if let Some(note) = hit {
                        if shift {
                            self.piano.toggle_note(note.id);
                        } else {
                            self.piano.select([note.id]);
                        }
                    } else {
                        self.begin_edit();
                        let length = self.piano.last_length.max(self.piano.grid).max(1);
                        if let Some(pattern) = self.project.pattern_mut() {
                            self.piano.draw(pattern, tick, key, Some(length));
                        }
                        self.commit_edit();
                        self.schedule_dirty = true;
                    }
                }
                redraw_piano_roll::Tool::Select => {
                    if let Some(note) = hit {
                        if shift {
                            self.piano.toggle_note(note.id);
                        } else {
                            self.piano.select([note.id]);
                        }
                    } else if !shift {
                        self.piano.clear_selection();
                    }
                }
                redraw_piano_roll::Tool::Erase => {
                    if let Some(note) = hit {
                        self.begin_edit();
                        self.piano.select([note.id]);
                        if let Some(pattern) = self.project.pattern_mut() {
                            self.piano.delete_selected(pattern);
                        }
                        self.commit_edit();
                        self.schedule_dirty = true;
                    }
                }
                redraw_piano_roll::Tool::Split => {
                    if let Some(note) = hit {
                        self.begin_edit();
                        if let Some(pattern) = self.project.pattern_mut() {
                            self.piano.split(pattern, note.id, raw_tick);
                        }
                        self.commit_edit();
                        self.schedule_dirty = true;
                    }
                }
                redraw_piano_roll::Tool::Merge => {
                    self.begin_edit();
                    if let Some(pattern) = self.project.pattern_mut() {
                        self.piano.merge_selected(pattern);
                    }
                    self.commit_edit();
                    self.schedule_dirty = true;
                }
            }
        }

        let _ = cc;
    }
}

