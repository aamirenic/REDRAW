//! REDRAW visual language.
//!
//! Every colour, spacing value and shape here is original. The layout borrows
//! only general DAW conventions (dense panels, a dark working surface, an
//! accent for transport and selection); no third-party theme or artwork is used.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

/// Selectable palettes. `Custom` starts from the dark token map and is edited
/// in Preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeKind {
    Dark,
    Light,
    Midnight,
    Custom,
}

impl ThemeKind {
    pub const ALL: [Self; 4] = [Self::Dark, Self::Light, Self::Midnight, Self::Custom];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Dark => "Graphite",
            Self::Light => "Daylight",
            Self::Midnight => "Midnight",
            Self::Custom => "Custom",
        }
    }

    pub const fn is_light(self) -> bool {
        matches!(self, Self::Light)
    }
}

/// Complete token map used by custom painting and egui's style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub panel_alt: Color32,
    pub raised: Color32,
    pub stroke: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub accent2: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub rec: Color32,
    pub grid: Color32,
    pub grid_strong: Color32,
    pub meter_low: Color32,
    pub meter_mid: Color32,
    pub meter_high: Color32,
}

impl Palette {
    pub const fn dark() -> Self {
        Self {
            bg: Color32::from_rgb(0x1E, 0x22, 0x2A),
            panel: Color32::from_rgb(0x26, 0x2B, 0x33),
            panel_alt: Color32::from_rgb(0x2E, 0x34, 0x3E),
            raised: Color32::from_rgb(0x35, 0x3C, 0x47),
            stroke: Color32::from_rgb(0x12, 0x15, 0x1A),
            text: Color32::from_rgb(0xD9, 0xDF, 0xE9),
            text_dim: Color32::from_rgb(0x8C, 0x96, 0xA6),
            accent: Color32::from_rgb(0x4E, 0xA1, 0xE8),
            accent2: Color32::from_rgb(0xE8, 0x87, 0x3A),
            ok: Color32::from_rgb(0x63, 0xC3, 0x6B),
            warn: Color32::from_rgb(0xE6, 0xB4, 0x50),
            rec: Color32::from_rgb(0xE0, 0x5A, 0x5A),
            grid: Color32::from_rgb(0x3A, 0x42, 0x4E),
            grid_strong: Color32::from_rgb(0x4A, 0x54, 0x64),
            meter_low: Color32::from_rgb(0x4F, 0xAE, 0x68),
            meter_mid: Color32::from_rgb(0xD9, 0xB4, 0x4A),
            meter_high: Color32::from_rgb(0xD9, 0x58, 0x4A),
        }
    }

    pub const fn light() -> Self {
        Self {
            bg: Color32::from_rgb(0xE9, 0xEC, 0xF1),
            panel: Color32::from_rgb(0xF6, 0xF7, 0xF9),
            panel_alt: Color32::from_rgb(0xE1, 0xE5, 0xEB),
            raised: Color32::from_rgb(0xD2, 0xD8, 0xE1),
            stroke: Color32::from_rgb(0xB4, 0xBC, 0xC8),
            text: Color32::from_rgb(0x22, 0x27, 0x30),
            text_dim: Color32::from_rgb(0x61, 0x6B, 0x79),
            accent: Color32::from_rgb(0x1D, 0x6F, 0xBB),
            accent2: Color32::from_rgb(0xB8, 0x5B, 0x16),
            ok: Color32::from_rgb(0x2D, 0x89, 0x44),
            warn: Color32::from_rgb(0xA5, 0x72, 0x0D),
            rec: Color32::from_rgb(0xB8, 0x35, 0x35),
            grid: Color32::from_rgb(0xC7, 0xCE, 0xD8),
            grid_strong: Color32::from_rgb(0xA8, 0xB2, 0xC0),
            meter_low: Color32::from_rgb(0x2E, 0x9B, 0x54),
            meter_mid: Color32::from_rgb(0xBE, 0x8F, 0x1A),
            meter_high: Color32::from_rgb(0xC0, 0x3E, 0x3E),
        }
    }

    pub const fn midnight() -> Self {
        Self {
            bg: Color32::from_rgb(0x0C, 0x10, 0x18),
            panel: Color32::from_rgb(0x13, 0x19, 0x23),
            panel_alt: Color32::from_rgb(0x1B, 0x23, 0x30),
            raised: Color32::from_rgb(0x25, 0x2F, 0x3E),
            stroke: Color32::from_rgb(0x05, 0x08, 0x0D),
            text: Color32::from_rgb(0xD8, 0xE3, 0xF2),
            text_dim: Color32::from_rgb(0x7F, 0x8F, 0xA6),
            accent: Color32::from_rgb(0x65, 0xC8, 0xF0),
            accent2: Color32::from_rgb(0xF0, 0x9A, 0x4B),
            ok: Color32::from_rgb(0x72, 0xD2, 0x84),
            warn: Color32::from_rgb(0xE8, 0xC2, 0x62),
            rec: Color32::from_rgb(0xF0, 0x6A, 0x6A),
            grid: Color32::from_rgb(0x26, 0x31, 0x42),
            grid_strong: Color32::from_rgb(0x38, 0x47, 0x5D),
            meter_low: Color32::from_rgb(0x55, 0xC2, 0x74),
            meter_mid: Color32::from_rgb(0xE4, 0xBF, 0x55),
            meter_high: Color32::from_rgb(0xE8, 0x62, 0x62),
        }
    }

    pub const fn for_kind(kind: ThemeKind) -> Self {
        match kind {
            ThemeKind::Dark | ThemeKind::Custom => Self::dark(),
            ThemeKind::Light => Self::light(),
            ThemeKind::Midnight => Self::midnight(),
        }
    }

    pub fn color(self, rgb: [u8; 3]) -> Color32 {
        Color32::from_rgb(rgb[0], rgb[1], rgb[2])
    }

    pub fn with_alpha(self, color: Color32, alpha: u8) -> Color32 {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
    }

    pub fn apply(self, ctx: &egui::Context, kind: ThemeKind, scale: f32, show_tooltips: bool) {
        ctx.set_theme(if kind.is_light() {
            egui::ThemePreference::Light
        } else {
            egui::ThemePreference::Dark
        });

        let mut style = (*ctx.style()).clone();
        style.text_styles.insert(
            TextStyle::Body,
            FontId::new(12.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(12.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(10.5, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(16.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(11.5, FontFamily::Monospace),
        );
        style.spacing.item_spacing = egui::vec2(4.0, 4.0);
        style.spacing.button_padding = egui::vec2(7.0, 3.0);
        style.spacing.interact_size = egui::vec2(22.0, 18.0);
        style.spacing.slider_width = 110.0;
        style.spacing.combo_width = 120.0;
        style.spacing.indent = 12.0;

        let mut visuals = if kind.is_light() {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        visuals.dark_mode = !kind.is_light();
        visuals.override_text_color = Some(self.text);
        visuals.panel_fill = self.panel;
        visuals.window_fill = self.panel;
        visuals.window_stroke = Stroke::new(1.0_f32, self.stroke);
        visuals.window_corner_radius = CornerRadius::same(8);
        visuals.menu_corner_radius = CornerRadius::same(5);
        visuals.faint_bg_color = self.panel_alt;
        visuals.extreme_bg_color = self.bg;
        visuals.code_bg_color = self.panel_alt;
        visuals.warn_fg_color = self.warn;
        visuals.error_fg_color = self.rec;
        visuals.hyperlink_color = self.accent;
        visuals.selection.bg_fill = self.accent.gamma_multiply(0.38);
        visuals.selection.stroke = Stroke::new(1.0_f32, self.accent);
        visuals.slider_trailing_fill = true;
        visuals.striped = true;
        visuals.button_frame = true;

        visuals.widgets.noninteractive.bg_fill = self.panel;
        visuals.widgets.noninteractive.weak_bg_fill = self.panel;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, self.stroke);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, self.text_dim);
        visuals.widgets.noninteractive.corner_radius = CornerRadius::same(3);

        visuals.widgets.inactive.bg_fill = self.panel_alt;
        visuals.widgets.inactive.weak_bg_fill = self.panel_alt;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, self.stroke);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, self.text);
        visuals.widgets.inactive.corner_radius = CornerRadius::same(3);

        visuals.widgets.hovered.bg_fill = self.raised;
        visuals.widgets.hovered.weak_bg_fill = self.raised;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, self.accent.gamma_multiply(0.75));
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.2_f32, self.text);
        visuals.widgets.hovered.corner_radius = CornerRadius::same(3);

        visuals.widgets.active.bg_fill = self.accent.gamma_multiply(0.55);
        visuals.widgets.active.weak_bg_fill = self.accent.gamma_multiply(0.35);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, self.accent);
        visuals.widgets.active.fg_stroke = Stroke::new(1.2_f32, self.text);
        visuals.widgets.active.corner_radius = CornerRadius::same(3);

        visuals.widgets.open.bg_fill = self.raised;
        visuals.widgets.open.weak_bg_fill = self.raised;
        visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, self.accent);
        visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, self.text);
        visuals.widgets.open.corner_radius = CornerRadius::same(3);

        style.visuals = visuals;
        ctx.set_style(style);
        ctx.set_zoom_factor(scale.clamp(0.75, 1.75));
        ctx.style_mut(|style| {
            style.interaction.show_tooltips_only_when_still = !show_tooltips;
        });
    }
}

/// Pattern/clip colour wheel. Values are deliberately generic and original.
pub const PATTERN_COLORS: [[u8; 3]; 16] = [
    [95, 179, 108],
    [91, 141, 201],
    [214, 160, 92],
    [156, 110, 180],
    [180, 170, 90],
    [90, 170, 180],
    [190, 95, 110],
    [120, 140, 170],
    [126, 190, 156],
    [112, 160, 214],
    [222, 142, 92],
    [174, 132, 202],
    [204, 188, 102],
    [96, 188, 190],
    [210, 122, 138],
    [146, 158, 186],
];

pub fn color(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// A compact meter colour for a peak value.
pub fn meter_color(palette: &Palette, peak: f32) -> Color32 {
    if peak >= 0.98 {
        palette.meter_high
    } else if peak >= 0.72 {
        palette.meter_mid
    } else {
        palette.meter_low
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_have_contrast_and_distinct_accents() {
        for kind in ThemeKind::ALL {
            let p = Palette::for_kind(kind);
            let text = p.text.r() as i32 + p.text.g() as i32 + p.text.b() as i32;
            let bg = p.bg.r() as i32 + p.bg.g() as i32 + p.bg.b() as i32;
            assert!((text - bg).abs() > 300, "{kind:?} contrast");
            assert_ne!(p.accent, p.rec);
            assert_ne!(p.panel, p.panel_alt);
        }
    }

    #[test]
    fn pattern_colors_are_original_and_visible() {
        assert_eq!(PATTERN_COLORS.len(), 16);
        for c in PATTERN_COLORS {
            let sum = c[0] as u32 + c[1] as u32 + c[2] as u32;
            assert!((120..=600).contains(&sum));
        }
    }
}
