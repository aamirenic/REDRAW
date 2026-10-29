//! Small original vector widgets shared by the REDRAW panels.

use std::f32::consts::PI;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2,
};

use redraw_shared::ParamSpec;

use crate::theme::Palette;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Play,
    Stop,
    Record,
    Loop,
    Metronome,
    Rewind,
    Forward,
    Plus,
    Minus,
    Save,
    Folder,
    Wave,
    Piano,
    Mixer,
    Rack,
    Auto,
    Search,
    ChevronDown,
    ChevronRight,
    Speaker,
    Headphones,
    Close,
}

pub fn icon_button(
    ui: &mut Ui,
    palette: &Palette,
    icon: Icon,
    size: f32,
    active: bool,
    tooltip: &str,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let hovered = response.hovered();
    let painter = ui.painter();
    if active || hovered {
        painter.rect_filled(
            rect,
            CornerRadius::same(3),
            if active {
                palette.accent.gamma_multiply(0.35)
            } else {
                palette.raised
            },
        );
    }
    let color = if active {
        palette.accent
    } else if hovered {
        palette.text
    } else {
        palette.text_dim
    };
    paint_icon(painter, rect, icon, color);
    if !tooltip.is_empty() {
        response.clone().on_hover_text(tooltip);
    }
    response
}

pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let s = rect.width().min(rect.height()) * 0.32;
    let stroke = Stroke::new(1.6_f32, color);
    match icon {
        Icon::Play => {
            painter.add(Shape::convex_polygon(
                vec![
                    Pos2::new(c.x - s * 0.55, c.y - s),
                    Pos2::new(c.x - s * 0.55, c.y + s),
                    Pos2::new(c.x + s * 0.85, c.y),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Stop => {
            painter.rect_filled(
                Rect::from_center_size(c, Vec2::splat(s * 1.55)),
                CornerRadius::same(1),
                color,
            );
        }
        Icon::Record => {
            painter.circle_filled(c, s * 0.82, color);
        }
        Icon::Loop => {
            let r = s * 0.9;
            let pts: Vec<Pos2> = (0..=18)
                .map(|i| {
                    let a = -PI * 0.15 + i as f32 / 18.0 * PI * 1.65;
                    Pos2::new(c.x + a.cos() * r, c.y + a.sin() * r * 0.62)
                })
                .collect();
            painter.add(Shape::line(pts, stroke));
            painter.add(Shape::convex_polygon(
                vec![
                    Pos2::new(c.x + r * 0.95, c.y - r * 0.15),
                    Pos2::new(c.x + r * 0.35, c.y - r * 0.55),
                    Pos2::new(c.x + r * 0.4, c.y + r * 0.25),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Metronome => {
            painter.add(Shape::convex_polygon(
                vec![
                    Pos2::new(c.x - s * 0.55, c.y + s),
                    Pos2::new(c.x + s * 0.55, c.y + s),
                    Pos2::new(c.x + s * 0.12, c.y - s),
                    Pos2::new(c.x - s * 0.12, c.y - s),
                ],
                Color32::TRANSPARENT,
                stroke,
            ));
            painter.line_segment(
                [
                    Pos2::new(c.x, c.y + s * 0.8),
                    Pos2::new(c.x + s * 0.35, c.y - s * 0.7),
                ],
                stroke,
            );
        }
        Icon::Rewind => {
            for dx in [-0.45, 0.3] {
                painter.add(Shape::convex_polygon(
                    vec![
                        Pos2::new(c.x + s * (dx + 0.55), c.y - s * 0.8),
                        Pos2::new(c.x + s * (dx + 0.55), c.y + s * 0.8),
                        Pos2::new(c.x + s * (dx - 0.35), c.y),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
        }
        Icon::Forward => {
            for dx in [-0.3, 0.45] {
                painter.add(Shape::convex_polygon(
                    vec![
                        Pos2::new(c.x + s * (dx - 0.55), c.y - s * 0.8),
                        Pos2::new(c.x + s * (dx - 0.55), c.y + s * 0.8),
                        Pos2::new(c.x + s * (dx + 0.35), c.y),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
        }
        Icon::Plus => {
            painter.line_segment(
                [
                    Pos2::new(c.x - s * 0.75, c.y),
                    Pos2::new(c.x + s * 0.75, c.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(c.x, c.y - s * 0.75),
                    Pos2::new(c.x, c.y + s * 0.75),
                ],
                stroke,
            );
        }
        Icon::Minus => {
            painter.line_segment(
                [
                    Pos2::new(c.x - s * 0.75, c.y),
                    Pos2::new(c.x + s * 0.75, c.y),
                ],
                stroke,
            );
        }
        Icon::Save => {
            painter.rect_stroke(
                Rect::from_center_size(c, Vec2::splat(s * 1.8)),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Inside,
            );
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(c.x - s * 0.6, c.y - s * 0.85),
                    Vec2::new(s * 1.2, s * 0.6),
                ),
                CornerRadius::same(1),
                color,
            );
        }
        Icon::Folder => {
            painter.rect_stroke(
                Rect::from_min_size(
                    Pos2::new(c.x - s, c.y - s * 0.55),
                    Vec2::new(s * 2.0, s * 1.25),
                ),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Inside,
            );
            painter.line_segment(
                [
                    Pos2::new(c.x - s * 0.85, c.y - s * 0.55),
                    Pos2::new(c.x - s * 0.2, c.y - s * 0.9),
                ],
                stroke,
            );
        }
        Icon::Wave => {
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let x = c.x - s + i as f32 / 24.0 * s * 2.0;
                    let y = c.y + (i as f32 * 1.7).sin() * s * 0.55;
                    Pos2::new(x, y)
                })
                .collect();
            painter.add(Shape::line(pts, stroke));
        }
        Icon::Piano => {
            let r = Rect::from_center_size(c, Vec2::new(s * 2.0, s * 1.35));
            painter.rect_stroke(r, CornerRadius::same(1), stroke, StrokeKind::Inside);
            for i in 1..7 {
                let x = r.left() + r.width() * i as f32 / 7.0;
                painter.line_segment([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())], stroke);
            }
        }
        Icon::Mixer => {
            for i in 0..4 {
                let x = c.x - s + i as f32 * s * 0.66;
                painter.line_segment(
                    [
                        Pos2::new(x, c.y - s * 0.9),
                        Pos2::new(x, c.y + s * 0.9),
                    ],
                    stroke,
                );
                let y = c.y - s * 0.3 + (i as f32 * 0.55).sin() * s * 0.6;
                painter.rect_filled(
                    Rect::from_center_size(Pos2::new(x, y), Vec2::new(s * 0.4, s * 0.22)),
                    CornerRadius::same(1),
                    color,
                );
            }
        }
        Icon::Rack => {
            for i in 0..4 {
                let y = c.y - s * 0.85 + i as f32 * s * 0.55;
                painter.rect_stroke(
                    Rect::from_min_size(
                        Pos2::new(c.x - s, y),
                        Vec2::new(s * 2.0, s * 0.35),
                    ),
                    CornerRadius::same(1),
                    stroke,
                    StrokeKind::Inside,
                );
            }
        }
        Icon::Auto => {
            let pts: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let x = c.x - s + i as f32 / 16.0 * s * 2.0;
                    let y = c.y + s * 0.7 - (i as f32 / 16.0 * PI * 1.4).sin() * s * 1.2;
                    Pos2::new(x, y)
                })
                .collect();
            painter.add(Shape::line(pts, stroke));
        }
        Icon::Search => {
            painter.circle_stroke(
                Pos2::new(c.x - s * 0.2, c.y - s * 0.2),
                s * 0.65,
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(c.x + s * 0.3, c.y + s * 0.3),
                    Pos2::new(c.x + s * 0.85, c.y + s * 0.85),
                ],
                stroke,
            );
        }
        Icon::ChevronDown => {
            painter.add(Shape::line(
                vec![
                    Pos2::new(c.x - s * 0.6, c.y - s * 0.3),
                    Pos2::new(c.x, c.y + s * 0.35),
                    Pos2::new(c.x + s * 0.6, c.y - s * 0.3),
                ],
                stroke,
            ));
        }
        Icon::ChevronRight => {
            painter.add(Shape::line(
                vec![
                    Pos2::new(c.x - s * 0.3, c.y - s * 0.6),
                    Pos2::new(c.x + s * 0.35, c.y),
                    Pos2::new(c.x - s * 0.3, c.y + s * 0.6),
                ],
                stroke,
            ));
        }
        Icon::Speaker => {
            painter.add(Shape::convex_polygon(
                vec![
                    Pos2::new(c.x - s * 0.8, c.y - s * 0.35),
                    Pos2::new(c.x - s * 0.25, c.y - s * 0.35),
                    Pos2::new(c.x + s * 0.25, c.y - s * 0.8),
                    Pos2::new(c.x + s * 0.25, c.y + s * 0.8),
                    Pos2::new(c.x - s * 0.25, c.y + s * 0.35),
                    Pos2::new(c.x - s * 0.8, c.y + s * 0.35),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Headphones => {
            painter.circle_stroke(c, s * 0.85, stroke);
            painter.line_segment(
                [
                    Pos2::new(c.x - s * 0.85, c.y),
                    Pos2::new(c.x - s * 0.85, c.y + s * 0.6),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(c.x + s * 0.85, c.y),
                    Pos2::new(c.x + s * 0.85, c.y + s * 0.6),
                ],
                stroke,
            );
        }
        Icon::Close => {
            painter.line_segment(
                [
                    Pos2::new(c.x - s * 0.7, c.y - s * 0.7),
                    Pos2::new(c.x + s * 0.7, c.y + s * 0.7),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(c.x + s * 0.7, c.y - s * 0.7),
                    Pos2::new(c.x - s * 0.7, c.y + s * 0.7),
                ],
                stroke,
            );
        }
    }
}

pub fn meter(
    ui: &mut Ui,
    palette: &Palette,
    peak: f32,
    width: f32,
    height: f32,
    horizontal: bool,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(1), palette.bg);
    let level = peak.clamp(0.0, 1.0).sqrt();
    if horizontal {
        let fill = Rect::from_min_size(
            rect.min,
            Vec2::new(rect.width() * level, rect.height()),
        );
        ui.painter().rect_filled(
            fill,
            CornerRadius::same(1),
            crate::theme::meter_color(palette, peak),
        );
    } else {
        let h = rect.height() * level;
        let fill = Rect::from_min_max(
            Pos2::new(rect.left(), rect.bottom() - h),
            rect.max,
        );
        ui.painter().rect_filled(
            fill,
            CornerRadius::same(1),
            crate::theme::meter_color(palette, peak),
        );
    }
    response
}

pub struct KnobResponse {
    pub changed: bool,
}

/// A normalized knob. `value` is always in 0..=1; the caller maps it through
/// `ParamSpec::denormalize` when a plugin parameter is involved.
pub fn knob_norm(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    value: &mut f32,
    default: f32,
    label: &str,
    value_text: &str,
    accent: Color32,
    palette: &Palette,
) -> KnobResponse {
    let desired = Vec2::new(44.0, 49.0);
    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
    let response = ui.interact(rect, Id::new(id), Sense::click_and_drag());
    let center = Pos2::new(rect.center().x, rect.top() + 18.0);
    let radius = 13.0;
    let painter = ui.painter();

    if response.hovered() || response.dragged() {
        painter.circle_filled(center, radius + 3.0, palette.raised);
    }
    painter.circle_filled(center, radius, palette.bg);
    painter.circle_stroke(center, radius, Stroke::new(1.0_f32, palette.stroke));

    let start = PI * 0.75;
    let end = PI * 2.25;
    let steps = 28;
    let mut pts = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let a = start + (end - start) * t;
        pts.push(Pos2::new(
            center.x + a.cos() * (radius - 2.0),
            center.y + a.sin() * (radius - 2.0),
        ));
    }
    painter.add(Shape::line(pts, Stroke::new(1.4_f32, palette.grid)));
    let mut active = Vec::with_capacity(steps + 1);
    let count = ((value.clamp(0.0, 1.0) * steps as f32).round() as usize).max(1);
    for i in 0..=count {
        let t = i as f32 / steps as f32;
        let a = start + (end - start) * t;
        active.push(Pos2::new(
            center.x + a.cos() * (radius - 2.0),
            center.y + a.sin() * (radius - 2.0),
        ));
    }
    if active.len() > 1 {
        painter.add(Shape::line(active, Stroke::new(2.4_f32, accent)));
    }
    let a = start + (end - start) * value.clamp(0.0, 1.0);
    painter.line_segment(
        [
            Pos2::new(
                center.x + a.cos() * 4.0,
                center.y + a.sin() * 4.0,
            ),
            Pos2::new(
                center.x + a.cos() * (radius - 4.0),
                center.y + a.sin() * (radius - 4.0),
            ),
        ],
        Stroke::new(1.5_f32, palette.text),
    );
    painter.text(
        Pos2::new(rect.center().x, rect.top() + 36.0),
        Align2::CENTER_CENTER,
        value_text,
        FontId::proportional(9.5),
        palette.text,
    );
    // Knobs live in a wrapped grid; a long parameter name must never bleed into
    // its neighbour. Keep the full name in the hover text and clip the caption.
    let short = short_knob_label(label, 9);
    painter.with_clip_rect(rect).text(
        Pos2::new(rect.center().x, rect.bottom() - 2.0),
        Align2::CENTER_CENTER,
        short,
        FontId::proportional(9.0),
        palette.text_dim,
    );

    let mut changed = false;
    if response.double_clicked() {
        *value = default.clamp(0.0, 1.0);
        changed = true;
    } else if response.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let delta = response.drag_delta().y / if fine { 900.0 } else { 150.0 };
        if delta != 0.0 {
            *value = (*value - delta).clamp(0.0, 1.0);
            changed = true;
        }
    }
    if response.hovered() {
        response.clone().on_hover_text(format!(
            "{label}: {value_text}\nDrag to adjust. Double-click resets. Shift = fine."
        ));
    }
    KnobResponse { changed }
}

pub fn param_knob(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    spec: &ParamSpec,
    value: &mut f32,
    accent: Color32,
    palette: &Palette,
) -> KnobResponse {
    let mut norm = spec.normalize(*value);
    let default = spec.normalize(spec.default);
    let text = format_param_value(spec, *value);
    let r = knob_norm(
        ui,
        id,
        &mut norm,
        default,
        spec.name,
        &text,
        accent,
        palette,
    );
    if r.changed {
        *value = spec.denormalize(norm);
    }
    r
}

fn short_knob_label(label: &str, max_chars: usize) -> String {
    if label.chars().count() <= max_chars {
        label.to_owned()
    } else {
        let mut text: String = label.chars().take(max_chars.saturating_sub(1)).collect();
        text.push('…');
        text
    }
}

pub fn format_param_value(spec: &ParamSpec, value: f32) -> String {
    let abs = value.abs();
    let text = if abs >= 1000.0 {
        format!("{value:.0}")
    } else if abs >= 100.0 {
        format!("{value:.1}")
    } else if abs >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    };
    if spec.unit.is_empty() {
        text
    } else {
        format!("{text}{}", spec.unit)
    }
}

pub fn section_title(ui: &mut Ui, palette: &Palette, title: &str) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title)
                .size(11.0)
                .strong()
                .color(palette.text_dim),
        );
        let rect = ui.available_rect_before_wrap();
        let y = ui.cursor().top() + 7.0;
        ui.painter().line_segment(
            [Pos2::new(rect.left() + 4.0, y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0_f32, palette.grid),
        );
        ui.allocate_space(Vec2::new(0.0, 10.0));
    });
}

pub fn format_db(db: f32) -> String {
    if db <= -89.9 {
        "-inf".into()
    } else {
        format!("{db:+.1} dB")
    }
}

pub fn format_pan(pan: f32) -> String {
    if pan.abs() < 0.005 {
        "C".into()
    } else if pan < 0.0 {
        format!("L{:.0}", (-pan * 100.0).round())
    } else {
        format!("R{:.0}", (pan * 100.0).round())
    }
}

pub fn note_name(key: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let key = key.min(127);
    format!("{}{}", NAMES[(key % 12) as usize], key as i32 / 12 - 1)
}

pub fn is_black_key(key: u8) -> bool {
    matches!(key % 12, 1 | 3 | 6 | 8 | 10)
}

/// Draw a compact timeline grid inside `rect`. `px_per_tick` is horizontal
/// scale; `bar_ticks` and `grid_ticks` are musical spacing.
pub fn draw_timeline_grid(
    painter: &egui::Painter,
    rect: Rect,
    scroll_x: f32,
    px_per_tick: f32,
    bar_ticks: u64,
    grid_ticks: u64,
    palette: &Palette,
) {
    if bar_ticks == 0 || grid_ticks == 0 {
        return;
    }
    let first_tick = (scroll_x / px_per_tick).max(0.0) as u64;
    let last_tick = ((scroll_x + rect.width()) / px_per_tick).max(0.0) as u64;
    let first_grid = first_tick / grid_ticks * grid_ticks;
    let mut tick = first_grid;
    while tick <= last_tick.saturating_add(grid_ticks) {
        let x = rect.left() + tick as f32 * px_per_tick - scroll_x;
        let is_bar = tick % bar_ticks == 0;
        let color = if is_bar {
            palette.grid_strong
        } else {
            palette.grid
        };
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(if is_bar { 1.2_f32 } else { 0.7_f32 }, color),
        );
        tick = tick.saturating_add(grid_ticks);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names_are_midi_standard() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(69), "A4");
        assert_eq!(note_name(0), "C-1");
        assert!(is_black_key(61));
        assert!(!is_black_key(60));
    }

    #[test]
    fn value_formatters_are_stable() {
        assert_eq!(format_db(-90.0), "-inf");
        assert_eq!(format_db(0.0), "+0.0 dB");
        assert_eq!(format_pan(0.0), "C");
        assert_eq!(format_pan(-0.5), "L50");
        assert_eq!(format_pan(0.5), "R50");
    }

    #[test]
    fn knob_labels_are_bounded() {
        assert_eq!(short_knob_label("Cutoff", 9), "Cutoff");
        let long = short_knob_label("Osc1 Crossfade", 9);
        assert_eq!(long.chars().count(), 9);
        assert!(long.ends_with('…'));
    }
}
