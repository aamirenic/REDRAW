//! Project browser: patterns, instruments, effects, samples, projects and plugins.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText, ScrollArea, Sense};

use crate::app::{draw_panel_header, FxStrip};
use crate::widgets::{paint_icon, Icon};
use redraw_shared::PluginFactory;

use crate::RedrawApp;

#[derive(Clone, Debug)]
pub enum BrowserItemKind {
    Pattern(usize),
    Instrument(String),
    Effect(String),
    Sample(PathBuf),
    Project(PathBuf),
    Plugin,
}

/// Payload accepted by the playlist, rack and mixer drop targets.
#[derive(Clone, Debug)]
pub struct BrowserDrag {
    pub kind: BrowserItemKind,
}

pub fn list_samples(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let dir = root.join("samples");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(extension.as_str(), "wav" | "aif" | "aiff") {
            out.push(path);
        }
    }
    out.sort();
    out
}

impl RedrawApp {
    pub fn browser_ui(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette;
        draw_panel_header(ui, &palette, "Browser", "double-click to load");
        ui.horizontal(|ui| {
            let icon_rect = ui
                .allocate_exact_size(egui::vec2(16.0, 18.0), Sense::hover())
                .0;
            paint_icon(ui.painter(), icon_rect, Icon::Search, palette.text_dim);
            ui.add(
                egui::TextEdit::singleline(&mut self.browser.search)
                    .hint_text("Search instruments, effects...")
                    .desired_width(f32::INFINITY),
            );
        });
        ui.add_space(2.0);
        let query = self.browser.search.trim().to_ascii_lowercase();
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.browser_patterns(ui, &query);
                self.browser_instruments(ui, &query);
                self.browser_effects(ui, &query);
                self.browser_samples(ui, &query);
                self.browser_projects(ui, &query);
                self.browser_plugins(ui, &query);
                ui.add_space(12.0);
                ui.label(
                    RichText::new("Drag items into the playlist, rack or mixer.")
                        .small()
                        .color(palette.text_dim),
                );
            });
    }

    fn browser_patterns(&mut self, ui: &mut egui::Ui, query: &str) {
        let palette = self.palette;
        egui::CollapsingHeader::new(
            RichText::new(format!("Patterns ({})", self.project.patterns.len()))
                .size(11.5)
                .strong(),
        )
        .default_open(true)
        .show(ui, |ui| {
            let patterns: Vec<(usize, String, [u8; 3])> = self
                .project
                .patterns
                .iter()
                .enumerate()
                .filter(|(_, p)| query.is_empty() || p.name.to_ascii_lowercase().contains(query))
                .map(|(i, p)| (i, p.name.clone(), p.color))
                .collect();
            if patterns.is_empty() {
                ui.label(RichText::new("No matching patterns").small().color(palette.text_dim));
            }
            for (index, name, color) in patterns {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(4.0, 16.0), Sense::hover());
                    ui.painter()
                        .rect_filled(rect, egui::CornerRadius::same(1), crate::theme::color(color));
                    let selected = self.project.current_pattern == index;
                    let response = ui
                        .selectable_label(selected, RichText::new(&name).size(11.5))
                        .on_hover_text("Double-click to select. Drag to the playlist.");
                    response.dnd_set_drag_payload(BrowserDrag {
                        kind: BrowserItemKind::Pattern(index),
                    });
                    if response.double_clicked() {
                        self.set_pattern_index(index);
                    }
                });
            }
            ui.horizontal(|ui| {
                if ui.small_button("+ Pattern").clicked() {
                    self.add_pattern();
                }
                if ui.small_button("Clone").clicked() {
                    self.clone_pattern();
                }
            });
        });
    }

    fn browser_instruments(&mut self, ui: &mut egui::Ui, query: &str) {
        let palette = self.palette;
        let names: Vec<&'static str> = self.factory.synth_names().to_vec();
        egui::CollapsingHeader::new(
            RichText::new(format!("Instruments ({})", names.len()))
                .size(11.5)
                .strong(),
        )
        .default_open(true)
        .show(ui, |ui| {
            let mut chosen = None;
            for name in names {
                if !query.is_empty() && !name.to_ascii_lowercase().contains(query) {
                    continue;
                }
                let response = ui
                    .selectable_label(false, RichText::new(name).size(11.5))
                    .on_hover_text("Double-click to add. Drag to the channel rack.");
                response.dnd_set_drag_payload(BrowserDrag {
                    kind: BrowserItemKind::Instrument(name.to_string()),
                });
                if response.double_clicked() {
                    chosen = Some(name);
                }
            }
            if let Some(name) = chosen {
                self.add_channel_by_name(name);
            }
            if query.is_empty() {
                ui.label(
                    RichText::new("Original REDRAW instruments")
                        .small()
                        .color(palette.text_dim),
                );
            }
        });
    }

    fn browser_effects(&mut self, ui: &mut egui::Ui, query: &str) {
        let names: Vec<&'static str> = self.factory.effect_names().to_vec();
        egui::CollapsingHeader::new(
            RichText::new(format!("Effects ({})", names.len()))
                .size(11.5)
                .strong(),
        )
        .default_open(false)
        .show(ui, |ui| {
            let mut chosen = None;
            for name in names {
                if !query.is_empty() && !name.to_ascii_lowercase().contains(query) {
                    continue;
                }
                let response = ui
                    .selectable_label(false, RichText::new(name).size(11.5))
                    .on_hover_text("Double-click to add to the selected insert chain.");
                response.dnd_set_drag_payload(BrowserDrag {
                    kind: BrowserItemKind::Effect(name.to_string()),
                });
                if response.double_clicked() {
                    chosen = Some(name);
                }
            }
            if let Some(name) = chosen {
                self.open_add_effect(FxStrip::Channel(self.selection.strip));
                self.browser.search = name.to_string();
            }
        });
    }

    fn browser_samples(&mut self, ui: &mut egui::Ui, query: &str) {
        let palette = self.palette;
        let samples = list_samples(&self.root);
        egui::CollapsingHeader::new(
            RichText::new(format!("Samples ({})", samples.len()))
                .size(11.5)
                .strong(),
        )
        .default_open(false)
        .show(ui, |ui| {
            if samples.is_empty() {
                ui.label(
                    RichText::new("Put your own WAV/AIFF files in samples/ to see them here.")
                        .small()
                        .color(palette.text_dim),
                );
                return;
            }
            let mut chosen = None;
            for path in samples {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                if !query.is_empty() && !name.to_ascii_lowercase().contains(query) {
                    continue;
                }
                let response = ui
                    .selectable_label(false, RichText::new(name).size(11.5))
                    .on_hover_text("Double-click to place an audio clip placeholder.");
                response.dnd_set_drag_payload(BrowserDrag {
                    kind: BrowserItemKind::Sample(path.clone()),
                });
                if response.double_clicked() {
                    chosen = Some(path.clone());
                }
            }
            if let Some(path) = chosen {
                let track = self.playlist_view.selected_track;
                let tick = self.current_tick();
                self.add_audio_clip(&path, track, tick);
            }
        });
    }

    fn browser_projects(&mut self, ui: &mut egui::Ui, query: &str) {
        let palette = self.palette;
        let recent = self.recent.clone();
        egui::CollapsingHeader::new(
            RichText::new(format!("Projects ({})", recent.len()))
                .size(11.5)
                .strong(),
        )
        .default_open(false)
        .show(ui, |ui| {
            if recent.is_empty() {
                ui.label(
                    RichText::new("Recent projects appear here.")
                        .small()
                        .color(palette.text_dim),
                );
            }
            for path in recent {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                if !query.is_empty() && !name.to_ascii_lowercase().contains(query) {
                    continue;
                }
                let response = ui
                    .selectable_label(false, RichText::new(name).size(11.5))
                    .on_hover_text(path.display().to_string());
                response.dnd_set_drag_payload(BrowserDrag {
                    kind: BrowserItemKind::Project(path.clone()),
                });
                if response.double_clicked() {
                    self.open_path(&path);
                }
            }
        });
    }

    fn browser_plugins(&mut self, ui: &mut egui::Ui, query: &str) {
        let palette = self.palette;
        let plugins = self.vst_plugins.clone();
        egui::CollapsingHeader::new(
            RichText::new(format!("Plugins ({})", plugins.len()))
                .size(11.5)
                .strong(),
        )
        .default_open(false)
        .show(ui, |ui| {
            if plugins.is_empty() {
                ui.label(
                    RichText::new("Run Tools > Scan VST Plugins to index VST2/VST3 modules.")
                        .small()
                        .color(palette.text_dim),
                );
            }
            for plugin in &plugins {
                if !query.is_empty() && !plugin.name.to_ascii_lowercase().contains(query) {
                    continue;
                }
                let response = ui
                    .selectable_label(false, RichText::new(&plugin.name).size(11.5))
                    .on_hover_text(format!("{:?} | {}", plugin.kind, plugin.path));
                response.dnd_set_drag_payload(BrowserDrag {
                    kind: BrowserItemKind::Plugin,
                });
            }
        });
    }
}

