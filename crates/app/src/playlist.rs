//! Playlist timeline: pattern clips, loop range, markers and drop targets.

use eframe::egui::{
    self, Align2, FontId, PointerButton, Pos2, Rect, RichText, ScrollArea, Sense, Stroke,
    StrokeKind, Vec2,
};

use crate::app::{ClipDrag, ClipDragMode};
use crate::browser::{BrowserDrag, BrowserItemKind};
use crate::widgets::draw_timeline_grid;
use crate::RedrawApp;

const HEADER_W: f32 = 128.0;
const RULER_H: f32 = 25.0;
const TRACK_H: f32 = 62.0;

struct Geometry {
    rect: Rect,
    timeline_left: f32,
    timeline_top: f32,
    px_per_tick: f32,
    snap: u64,
}

impl Geometry {
    fn tick_at(&self, x: f32) -> u64 {
        let tick = ((x - self.timeline_left) / self.px_per_tick.max(0.0001)).max(0.0) as u64;
        self.snap_tick(tick)
    }

    fn snap_tick(&self, tick: u64) -> u64 {
        let snap = self.snap.max(1);
        (tick + snap / 2) / snap * snap
    }

    fn x_for_tick(&self, tick: u64) -> f32 {
        self.timeline_left + tick as f32 * self.px_per_tick
    }

    fn track_at(&self, y: f32) -> usize {
        (((y - self.timeline_top) / TRACK_H).floor()).max(0.0) as usize
    }

    fn track_rect(&self, track: usize, tracks: usize) -> Rect {
        let top = self.timeline_top + track as f32 * TRACK_H;
        Rect::from_min_max(
            Pos2::new(self.rect.left(), top),
            Pos2::new(
                self.rect.right(),
                (top + TRACK_H).min(self.timeline_top + tracks as f32 * TRACK_H),
            ),
        )
    }
}

impl RedrawApp {
    pub fn playlist_ui(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Playlist").strong().size(12.0));
            ui.separator();
            if ui
                .button("+ Clip")
                .on_hover_text("Place the current pattern at the playhead")
                .clicked()
            {
                let track = self.playlist_view.selected_track;
                let tick = self.current_tick();
                self.add_pattern_clip_at(track, tick);
            }
            if ui.button("+ Track").clicked() {
                self.add_playlist_track();
            }
            if ui.button("- Track").clicked() {
                self.remove_playlist_track();
            }
            ui.separator();
            ui.label(RichText::new("Snap").small().color(palette.text_dim));
            let snap_options = [
                (redraw_piano_roll::GRID_1_32, "1/32"),
                (redraw_piano_roll::GRID_1_16, "1/16"),
                (redraw_piano_roll::GRID_1_8, "1/8"),
                (redraw_piano_roll::GRID_QUARTER, "1/4"),
                (self.bar_ticks().max(1), "Bar"),
            ];
            egui::ComboBox::from_id_salt("playlist_snap")
                .selected_text(
                    snap_options
                        .iter()
                        .find(|(value, _)| *value == self.playlist_view.snap)
                        .map(|(_, label)| *label)
                        .unwrap_or("1/16"),
                )
                .width(70.0)
                .show_ui(ui, |ui| {
                    for (value, label) in snap_options {
                        if ui
                            .selectable_label(self.playlist_view.snap == value, label)
                            .clicked()
                        {
                            self.playlist_view.snap = value.max(1);
                        }
                    }
                });
            ui.separator();
            if ui
                .selectable_label(self.project.playlist.loop_enabled, "Loop")
                .clicked()
            {
                self.toggle_loop();
            }
            let mut loop_start = self.project.playlist.loop_start;
            let mut loop_end = self.project.playlist.loop_end;
            if ui
                .add(
                    egui::DragValue::new(&mut loop_start)
                        .speed(10.0)
                        .prefix("from ")
                        .suffix("t"),
                )
                .changed()
            {
                self.project.playlist.loop_start = loop_start;
                self.audio.set_loop(
                    self.project.playlist.loop_enabled,
                    self.project.playlist.loop_start,
                    self.project.playlist.loop_end,
                );
                self.dirty = true;
            }
            if ui
                .add(
                    egui::DragValue::new(&mut loop_end)
                        .speed(10.0)
                        .prefix("to ")
                        .suffix("t"),
                )
                .changed()
            {
                self.project.playlist.loop_end = loop_end.max(loop_start + 1);
                self.audio.set_loop(
                    self.project.playlist.loop_enabled,
                    self.project.playlist.loop_start,
                    self.project.playlist.loop_end,
                );
                self.dirty = true;
            }
            ui.separator();
            ui.label(RichText::new("Zoom").small().color(palette.text_dim));
            ui.add(
                egui::Slider::new(&mut self.playlist_view.px_per_tick, 0.008..=0.24)
                    .logarithmic(true)
                    .show_value(false),
            );
            if ui.small_button("Fit").clicked() {
                let total = self.total_timeline_ticks();
                let width = (ui.available_width() - HEADER_W - 20.0).max(160.0);
                self.playlist_view.px_per_tick =
                    (width / total.max(1) as f32).clamp(0.008, 0.24);
            }
        });
        ui.add_space(2.0);
        let total_ticks = self.total_timeline_ticks();
        let track_count = self.project.playlist.tracks.len().max(1);
        let width = HEADER_W + total_ticks as f32 * self.playlist_view.px_per_tick + 80.0;
        let height = RULER_H + track_count as f32 * TRACK_H + 18.0;
        ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(width.max(320.0), height.max(140.0)), Sense::click_and_drag());
                let geometry = Geometry {
                    rect,
                    timeline_left: rect.left() + HEADER_W,
                    timeline_top: rect.top() + RULER_H,
                    px_per_tick: self.playlist_view.px_per_tick,
                    snap: self.playlist_view.snap,
                };
                self.draw_playlist(ui, &geometry, &response);
            });
    }

    fn total_timeline_ticks(&self) -> u64 {
        let song = redraw_arrangement::playlist_length(&self.project);
        let pattern = self
            .pattern()
            .map(|p| p.length_ticks())
            .unwrap_or(self.bar_ticks());
        let loop_end = if self.project.playlist.loop_enabled {
            self.project.playlist.loop_end
        } else {
            0
        };
        let current = self.current_tick().saturating_add(self.bar_ticks().saturating_mul(8));
        song.max(pattern.saturating_mul(4))
            .max(loop_end)
            .max(current)
            .max(self.bar_ticks().saturating_mul(16))
            .min(self.bar_ticks().saturating_mul(512))
    }

    fn draw_playlist(&mut self, ui: &mut egui::Ui, g: &Geometry, response: &egui::Response) {
        let palette = self.palette;
        let painter = ui.painter_at(g.rect);
        painter.rect_filled(g.rect, egui::CornerRadius::ZERO, palette.bg);
        let track_count = self.project.playlist.tracks.len().max(1);
        let tracks_bottom = g.timeline_top + track_count as f32 * TRACK_H;

        // Rows and headers.
        for index in 0..track_count {
            let row = g.track_rect(index, track_count);
            let selected_track = index == self.playlist_view.selected_track;
            painter.rect_filled(
                row,
                egui::CornerRadius::ZERO,
                if selected_track {
                    palette.panel_alt
                } else if index % 2 == 0 {
                    palette.bg
                } else {
                    palette.panel.gamma_multiply(0.55)
                },
            );
            painter.line_segment(
                [Pos2::new(row.left(), row.bottom()), Pos2::new(row.right(), row.bottom())],
                Stroke::new(1.0_f32, palette.stroke),
            );
            let header = Rect::from_min_max(
                Pos2::new(row.left(), row.top()),
                Pos2::new(g.timeline_left, row.bottom()),
            );
            painter.rect_filled(header, egui::CornerRadius::ZERO, palette.panel);
            painter.line_segment(
                [Pos2::new(header.right(), header.top()), Pos2::new(header.right(), header.bottom())],
                Stroke::new(1.0_f32, palette.stroke),
            );
            if let Some(track) = self.project.playlist.tracks.get(index) {
                let color = crate::theme::color(track.color);
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(header.left(), header.top() + 6.0),
                        Vec2::new(4.0, header.height() - 12.0),
                    ),
                    egui::CornerRadius::same(1),
                    color,
                );
                painter.text(
                    Pos2::new(header.left() + 12.0, header.top() + 10.0),
                    Align2::LEFT_TOP,
                    &track.name,
                    FontId::proportional(11.5),
                    if selected_track { palette.text } else { palette.text_dim },
                );
                painter.text(
                    Pos2::new(header.left() + 12.0, header.top() + 27.0),
                    Align2::LEFT_TOP,
                    format!("{:?}", track.kind),
                    FontId::proportional(9.0),
                    palette.text_dim,
                );
                let m = Rect::from_min_size(
                    Pos2::new(header.right() - 44.0, header.top() + 8.0),
                    Vec2::new(18.0, 16.0),
                );
                let s = Rect::from_min_size(
                    Pos2::new(header.right() - 22.0, header.top() + 8.0),
                    Vec2::new(18.0, 16.0),
                );
                painter.rect_filled(
                    m,
                    egui::CornerRadius::same(2),
                    if track.mute { palette.rec.gamma_multiply(0.6) } else { palette.panel_alt },
                );
                painter.rect_filled(
                    s,
                    egui::CornerRadius::same(2),
                    if track.solo { palette.warn.gamma_multiply(0.6) } else { palette.panel_alt },
                );
                painter.text(m.center(), Align2::CENTER_CENTER, "M", FontId::proportional(9.5), palette.text);
                painter.text(s.center(), Align2::CENTER_CENTER, "S", FontId::proportional(9.5), palette.text);
            }
        }

        // Timeline surface and grid.
        let timeline = Rect::from_min_max(
            Pos2::new(g.timeline_left, g.timeline_top),
            Pos2::new(g.rect.right(), tracks_bottom),
        );
        painter.rect_filled(timeline, egui::CornerRadius::ZERO, palette.bg);
        draw_timeline_grid(
            &painter,
            timeline,
            0.0,
            g.px_per_tick,
            self.bar_ticks().max(1),
            self.playlist_view.snap.max(1),
            &palette,
        );
        for index in 0..track_count {
            let row = g.track_rect(index, track_count);
            painter.line_segment(
                [Pos2::new(g.timeline_left, row.bottom()), Pos2::new(row.right(), row.bottom())],
                Stroke::new(1.0_f32, palette.grid),
            );
        }

        // Loop region.
        if self.project.playlist.loop_enabled {
            let x0 = g.x_for_tick(self.project.playlist.loop_start);
            let x1 = g.x_for_tick(self.project.playlist.loop_end);
            let loop_rect = Rect::from_min_max(
                Pos2::new(x0, g.timeline_top),
                Pos2::new(x1, tracks_bottom),
            );
            painter.rect_filled(loop_rect, egui::CornerRadius::ZERO, palette.accent.gamma_multiply(0.06));
            painter.line_segment(
                [Pos2::new(x0, g.timeline_top), Pos2::new(x0, tracks_bottom)],
                Stroke::new(1.5_f32, palette.accent),
            );
            painter.line_segment(
                [Pos2::new(x1, g.timeline_top), Pos2::new(x1, tracks_bottom)],
                Stroke::new(1.5_f32, palette.accent),
            );
        }

        // Clips.
        for clip in &self.project.playlist.clips {
            if clip.track >= track_count {
                continue;
            }
            let row = g.track_rect(clip.track, track_count);
            let left = g.x_for_tick(clip.start);
            let right = g.x_for_tick(clip.end());
            if right < timeline.left() || left > timeline.right() {
                continue;
            }
            let clip_rect = Rect::from_min_max(
                Pos2::new(left.max(timeline.left()) + 1.0, row.top() + 4.0),
                Pos2::new(right.min(timeline.right()) - 1.0, row.bottom() - 5.0),
            );
            if clip_rect.width() <= 0.5 {
                continue;
            }
            let color = crate::theme::color(clip.color);
            let selected = self.selection.clips.contains(&clip.id);
            painter.rect_filled(clip_rect, egui::CornerRadius::same(3), color.gamma_multiply(0.45));
            painter.rect_filled(
                Rect::from_min_size(clip_rect.min, Vec2::new(clip_rect.width(), 3.0)),
                egui::CornerRadius::same(1),
                color,
            );
            if selected {
                painter.rect_stroke(
                    clip_rect,
                    egui::CornerRadius::same(3),
                    Stroke::new(1.8_f32, palette.accent),
                    StrokeKind::Inside,
                );
            }
            if clip.kind == redraw_shared::ClipKind::Pattern {
                if let Some(pattern) = self.project.patterns.iter().find(|p| p.id == clip.source_id) {
                    let pattern_ticks = pattern.length_ticks().max(1);
                    let repeats = if clip.looping {
                        (clip.length / pattern_ticks).max(1).min(32)
                    } else {
                        1
                    };
                    for repeat in 0..repeats {
                        for pc in &pattern.channels {
                            for (step, state) in pc.steps.iter().enumerate() {
                                if !state.active {
                                    continue;
                                }
                                let tick = repeat * pattern_ticks
                                    + step as u64 * (redraw_shared::PPQ as u64 / 4);
                                if tick >= clip.length {
                                    break;
                                }
                                let x = left + tick as f32 * g.px_per_tick;
                                let height = (clip_rect.height() - 18.0).max(2.0);
                                let y = clip_rect.bottom() - 4.0 - height;
                                painter.rect_filled(
                                    Rect::from_min_size(
                                        Pos2::new(x + 1.0, y),
                                        Vec2::new((g.px_per_tick * 180.0).max(1.5), height),
                                    ),
                                    egui::CornerRadius::same(1),
                                    color.gamma_multiply(0.75),
                                );
                            }
                        }
                    }
                }
                painter.text(
                    Pos2::new(clip_rect.left() + 5.0, clip_rect.top() + 5.0),
                    Align2::LEFT_TOP,
                    &clip.name,
                    FontId::proportional(10.5),
                    palette.text,
                );
                if clip.looping {
                    painter.text(
                        Pos2::new(clip_rect.right() - 5.0, clip_rect.top() + 4.0),
                        Align2::RIGHT_TOP,
                        "loop",
                        FontId::proportional(8.5),
                        palette.text_dim,
                    );
                }
            } else {
                painter.text(
                    Pos2::new(clip_rect.left() + 5.0, clip_rect.top() + 5.0),
                    Align2::LEFT_TOP,
                    &clip.name,
                    FontId::proportional(10.5),
                    palette.text,
                );
                painter.text(
                    Pos2::new(clip_rect.left() + 5.0, clip_rect.bottom() - 4.0),
                    Align2::LEFT_BOTTOM,
                    "placeholder",
                    FontId::proportional(8.5),
                    palette.text_dim,
                );
            }
        }

        // Ruler and markers.
        let ruler = Rect::from_min_max(
            Pos2::new(g.timeline_left, g.rect.top()),
            Pos2::new(g.rect.right(), g.timeline_top),
        );
        painter.rect_filled(
            Rect::from_min_max(g.rect.min, Pos2::new(g.timeline_left, g.timeline_top)),
            egui::CornerRadius::ZERO,
            palette.panel_alt,
        );
        painter.rect_filled(ruler, egui::CornerRadius::ZERO, palette.panel_alt);
        painter.line_segment(
            [Pos2::new(ruler.left(), ruler.bottom()), Pos2::new(ruler.right(), ruler.bottom())],
            Stroke::new(1.0_f32, palette.stroke),
        );
        let bar = self.bar_ticks().max(1);
        let first_bar = 0u64;
        let last_bar = (ruler.right() - g.timeline_left).max(0.0) as f32 / g.px_per_tick / bar as f32;
        let label_every = if last_bar > 80.0 { 8 } else if last_bar > 40.0 { 4 } else { 1 };
        let mut b = first_bar;
        while b as f32 <= last_bar {
            let tick = b * bar;
            let x = g.x_for_tick(tick);
            painter.line_segment(
                [Pos2::new(x, ruler.bottom() - 6.0), Pos2::new(x, ruler.bottom())],
                Stroke::new(1.0_f32, palette.grid_strong),
            );
            if b % label_every == 0 {
                painter.text(
                    Pos2::new(x + 3.0, ruler.top() + 3.0),
                    Align2::LEFT_TOP,
                    format!("{}", b + 1),
                    FontId::monospace(9.0),
                    palette.text_dim,
                );
            }
            b += 1;
        }
        for marker in &self.project.playlist.markers {
            let x = g.x_for_tick(marker.pos);
            let color = crate::theme::color(marker.color);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(x, g.rect.top() + 2.0),
                    Pos2::new(x + 8.0, g.rect.top() + 2.0),
                    Pos2::new(x + 4.0, g.rect.top() + 8.0),
                ],
                color,
                Stroke::NONE,
            ));
            painter.text(
                Pos2::new(x + 8.0, g.rect.top() + 1.0),
                Align2::LEFT_TOP,
                &marker.name,
                FontId::proportional(8.5),
                color,
            );
        }

        // Playhead.
        let play_x = g.x_for_tick(self.current_tick());
        if play_x >= timeline.left() && play_x <= timeline.right() {
            painter.line_segment(
                [Pos2::new(play_x, g.rect.top()), Pos2::new(play_x, tracks_bottom)],
                Stroke::new(1.4_f32, palette.rec),
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(play_x - 4.0, g.rect.top()),
                    Pos2::new(play_x + 4.0, g.rect.top()),
                    Pos2::new(play_x, g.rect.top() + 6.0),
                ],
                palette.rec,
                Stroke::NONE,
            ));
        }

        self.playlist_interactions(ui, g, response, track_count);
    }

    fn playlist_interactions(
        &mut self,
        ui: &mut egui::Ui,
        g: &Geometry,
        response: &egui::Response,
        track_count: usize,
    ) {
        let pointer = response.interact_pointer_pos();
        let Some(pos) = pointer else {
            self.finish_clip_drag();
            return;
        };
        let in_timeline = pos.x >= g.timeline_left && pos.y >= g.timeline_top;
        let tick = g.tick_at(pos.x);
        let track = g.track_at(pos.y).min(track_count.saturating_sub(1));

        if response.drag_started_by(PointerButton::Primary) && in_timeline {
            if let Some((id, mode)) = self.clip_hit(pos, g, track_count) {
                self.begin_edit();
                let Some(clip) = redraw_arrangement::find_clip(&self.project, id).cloned() else {
                    self.history.cancel();
                    return;
                };
                self.selection.clips = vec![id];
                self.playlist_view.drag = Some(ClipDrag {
                    id,
                    mode,
                    start_pointer_tick: tick,
                    original_start: clip.start,
                    original_track: clip.track,
                    original_length: clip.length,
                    changed: false,
                });
            }
        }
        if response.dragged_by(PointerButton::Primary) {
            if let Some(mut drag) = self.playlist_view.drag.take() {
                let delta = tick as i64 - drag.start_pointer_tick as i64;
                match drag.mode {
                    ClipDragMode::Move => {
                        let new_start = (drag.original_start as i64 + delta).max(0) as u64;
                        let new_track = track.min(track_count.saturating_sub(1));
                        if new_start != drag.original_start || new_track != drag.original_track {
                            if redraw_arrangement::move_clip(&mut self.project, drag.id, new_track, new_start) {
                                drag.changed = true;
                                self.schedule_dirty = true;
                            }
                        }
                    }
                    ClipDragMode::Resize => {
                        let length = (tick as i64 - drag.original_start as i64).max(1) as u64;
                        if length != drag.original_length {
                            if redraw_arrangement::resize_clip(&mut self.project, drag.id, length) {
                                drag.changed = true;
                                self.schedule_dirty = true;
                            }
                        }
                    }
                }
                self.playlist_view.drag = Some(drag);
            }
        }
        if response.drag_stopped_by(PointerButton::Primary) {
            self.finish_clip_drag();
        }

        if response.clicked() {
            if pos.y < g.timeline_top && pos.x >= g.timeline_left {
                let seek = tick;
                self.audio.seek(seek);
                if response.double_clicked() {
                    self.marker_text = format!(
                        "Marker {}",
                        self.project.playlist.markers.len() + 1
                    );
                    self.dialog = Some(crate::app::DialogKind::AddMarker);
                }
            } else if pos.x < g.timeline_left && pos.y >= g.timeline_top {
                let header = g.track_rect(track, track_count);
                let m = Rect::from_min_size(
                    Pos2::new(header.right() - 44.0, header.top() + 8.0),
                    Vec2::new(18.0, 16.0),
                );
                let s = Rect::from_min_size(
                    Pos2::new(header.right() - 22.0, header.top() + 8.0),
                    Vec2::new(18.0, 16.0),
                );
                if m.contains(pos) {
                    if let Some(track) = self.project.playlist.tracks.get_mut(track) {
                        track.mute = !track.mute;
                    }
                    self.dirty = true;
                    self.sync_schedules();
                } else if s.contains(pos) {
                    if let Some(track) = self.project.playlist.tracks.get_mut(track) {
                        track.solo = !track.solo;
                    }
                    self.dirty = true;
                    self.sync_schedules();
                } else {
                    self.playlist_view.selected_track = track;
                }
            } else if in_timeline {
                if let Some((id, _)) = self.clip_hit(pos, g, track_count) {
                    if ui.input(|i| i.modifiers.shift) {
                        if self.selection.clips.contains(&id) {
                            self.selection.clips.retain(|c| *c != id);
                        } else {
                            self.selection.clips.push(id);
                        }
                    } else {
                        self.selection.clips = vec![id];
                    }
                } else {
                    self.selection.clips.clear();
                    self.playlist_view.selected_track = track;
                    if response.double_clicked() {
                        self.add_pattern_clip_at(track, tick);
                    }
                }
            }
        }

        // Browser drop target.
        if let Some(payload) = response.dnd_release_payload::<BrowserDrag>() {
            match &payload.kind {
                BrowserItemKind::Pattern(index) => {
                    if *index < self.project.patterns.len() {
                        self.project.current_pattern = *index;
                        self.add_pattern_clip_at(track, tick);
                    }
                }
                BrowserItemKind::Instrument(name) => {
                    self.add_channel_by_name(name);
                }
                BrowserItemKind::Sample(path) => {
                    self.add_audio_clip(path, track, tick);
                }
                BrowserItemKind::Project(path) => {
                    self.open_path(path);
                }
                BrowserItemKind::Effect(name) => {
                    let strip = crate::app::FxStrip::Channel(self.selection.strip);
                    self.add_effect_by_name(strip, name);
                }
                BrowserItemKind::Plugin => {}
            }
        }
    }

    fn clip_hit(&self, pos: Pos2, g: &Geometry, track_count: usize) -> Option<(u64, ClipDragMode)> {
        let track = g.track_at(pos.y);
        if track >= track_count {
            return None;
        }
        let tick = g.tick_at(pos.x);
        for clip in &self.project.playlist.clips {
            if clip.track != track {
                continue;
            }
            if tick >= clip.start && tick < clip.end() {
                let right = g.x_for_tick(clip.end());
                let mode = if (right - pos.x).abs() <= 7.0 {
                    ClipDragMode::Resize
                } else {
                    ClipDragMode::Move
                };
                return Some((clip.id, mode));
            }
        }
        None
    }

    fn finish_clip_drag(&mut self) {
        if let Some(drag) = self.playlist_view.drag.take() {
            if drag.changed {
                self.commit_edit();
                self.sync_schedules();
            } else {
                self.history.cancel();
            }
        }
    }
}
