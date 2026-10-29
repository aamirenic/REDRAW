//! Modal windows, preferences, diagnostics and the export dialog.

use eframe::egui::{self, Color32, RichText, ScrollArea};

use crate::app::{DialogKind, FxStrip};
use crate::widgets::section_title;
use redraw_shared::PluginFactory;

use crate::RedrawApp;

impl RedrawApp {
    pub fn dialogs_ui(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.clone() else {
            return;
        };
        let mut keep = true;
        match dialog {
            DialogKind::Preferences => self.preferences_window(ctx, &mut keep),
            DialogKind::MidiSettings => self.midi_window(ctx, &mut keep),
            DialogKind::AudioSettings => self.audio_window(ctx, &mut keep),
            DialogKind::ProjectInfo => self.project_info_window(ctx, &mut keep),
            DialogKind::Statistics => self.statistics_window(ctx, &mut keep),
            DialogKind::Shortcuts => self.shortcuts_window(ctx, &mut keep),
            DialogKind::About => self.about_window(ctx, &mut keep),
            DialogKind::Diagnostics => self.diagnostics_window(ctx, &mut keep),
            DialogKind::PatternLength => self.pattern_length_window(ctx, &mut keep),
            DialogKind::RenamePattern => self.rename_pattern_window(ctx, &mut keep),
            DialogKind::AddMarker => self.add_marker_window(ctx, &mut keep),
            DialogKind::Quantize => self.quantize_window(ctx, &mut keep),
            DialogKind::Humanize => self.humanize_window(ctx, &mut keep),
            DialogKind::Transpose => self.transpose_window(ctx, &mut keep),
            DialogKind::AddChannel => self.add_channel_window(ctx, &mut keep),
            DialogKind::ReplaceSynth => self.replace_synth_window(ctx, &mut keep),
            DialogKind::AddEffect => self.add_effect_window(ctx, &mut keep),
            DialogKind::ScanVst => self.scan_vst_window(ctx, &mut keep),
            DialogKind::CloseConfirm => self.close_confirm_window(ctx, &mut keep),
            DialogKind::Recovery => self.recovery_window(ctx, &mut keep),
        }
        if !keep {
            self.dialog = None;
        }
    }

    fn preferences_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        let mut changed = false;
        egui::Window::new("Preferences")
            .id(egui::Id::new("prefs_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(470.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Theme");
                    egui::ComboBox::from_id_salt("prefs_theme")
                        .selected_text(self.prefs.theme.label())
                        .show_ui(ui, |ui| {
                            for kind in crate::ThemeKind::ALL {
                                if ui
                                    .selectable_label(self.prefs.theme == kind, kind.label())
                                    .clicked()
                                {
                                    self.prefs.theme = kind;
                                    changed = true;
                                }
                            }
                        });
                });
                if self.prefs.theme == crate::ThemeKind::Custom {
                    let mut bg = [
                        self.prefs.custom.bg.r(),
                        self.prefs.custom.bg.g(),
                        self.prefs.custom.bg.b(),
                    ];
                    let mut panel = [
                        self.prefs.custom.panel.r(),
                        self.prefs.custom.panel.g(),
                        self.prefs.custom.panel.b(),
                    ];
                    let mut accent = [
                        self.prefs.custom.accent.r(),
                        self.prefs.custom.accent.g(),
                        self.prefs.custom.accent.b(),
                    ];
                    ui.horizontal(|ui| {
                        ui.label("Background");
                        if egui::color_picker::color_edit_button_srgb(ui, &mut bg).changed() {
                            self.prefs.custom.bg = Color32::from_rgb(bg[0], bg[1], bg[2]);
                            changed = true;
                        }
                        ui.label("Panel");
                        if egui::color_picker::color_edit_button_srgb(ui, &mut panel).changed() {
                            self.prefs.custom.panel =
                                Color32::from_rgb(panel[0], panel[1], panel[2]);
                            changed = true;
                        }
                        ui.label("Accent");
                        if egui::color_picker::color_edit_button_srgb(ui, &mut accent).changed() {
                            self.prefs.custom.accent =
                                Color32::from_rgb(accent[0], accent[1], accent[2]);
                            changed = true;
                        }
                    });
                }
                ui.add(
                    egui::Slider::new(&mut self.prefs.scale, 0.75..=1.75)
                        .text("Interface scale")
                        .fixed_decimals(2),
                );
                if ui
                    .checkbox(&mut self.prefs.tooltips, "Show tooltips")
                    .changed()
                {
                    changed = true;
                }
                ui.checkbox(&mut self.prefs.autosave, "Autosave every five minutes");
                ui.separator();
                ui.label(
                    RichText::new("Default export folder")
                        .small()
                        .color(self.palette.text_dim),
                );
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(self.prefs.export_dir.display().to_string())
                            .monospace()
                            .small()
                            .color(self.palette.text_dim),
                    );
                    if ui.button("Browse...").clicked() {
                        if let Some(path) = self.pick_folder("Choose export folder") {
                            self.prefs.export_dir = path;
                        }
                    }
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Files are kept inside {}.",
                        self.root.display()
                    ))
                    .small()
                    .color(self.palette.text_dim),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        self.apply_theme(ctx);
                        self.save_preferences();
                    }
                    if ui.button("Close").clicked() {
                        *keep = false;
                    }
                });
            });
        if changed {
            self.apply_theme(ctx);
            self.save_preferences();
        }
        if !open {
            *keep = false;
        }
    }

    fn midi_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        egui::Window::new("MIDI Settings")
            .id(egui::Id::new("midi_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Rescan ports").clicked() {
                        self.refresh_midi_ports();
                    }
                    if self.midi.is_connected() {
                        if ui.button("Disconnect").clicked() {
                            self.midi.disconnect();
                            self.midi_port = None;
                            self.set_status("MIDI disconnected");
                        }
                    }
                });
                ui.separator();
                if self.midi_ports.is_empty() {
                    ui.label(
                        RichText::new("No MIDI inputs found. Connect a controller and rescan.")
                            .color(self.palette.text_dim),
                    );
                } else {
                    let connected = self.midi.connected_port().map(str::to_string);
                    ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                        let ports = self.midi_ports.clone();
                        for port in ports {
                            ui.horizontal(|ui| {
                                let is_current = connected.as_deref() == Some(port.name.as_str());
                                if ui
                                    .selectable_label(is_current, format!("{}  ({})", port.name, port.index))
                                    .clicked()
                                {
                                    self.connect_midi(port.index);
                                }
                            });
                        }
                    });
                }
                ui.separator();
                section_title(ui, &self.palette, "Recording");
                ui.horizontal(|ui| {
                    ui.label("Input filter");
                    let mut filter = self.midi_recorder.channel_filter;
                    egui::ComboBox::from_id_salt("midi_filter")
                        .selected_text(
                            filter
                                .map(|c| format!("Channel {}", c + 1))
                                .unwrap_or_else(|| "All channels".into()),
                        )
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(filter.is_none(), "All channels").clicked() {
                                filter = None;
                            }
                            for channel in 0..16u8 {
                                if ui
                                    .selectable_label(
                                        filter == Some(channel),
                                        format!("Channel {}", channel + 1),
                                    )
                                    .clicked()
                                {
                                    filter = Some(channel);
                                }
                            }
                        });
                    self.midi_recorder.set_channel_filter(filter);
                });
                ui.horizontal(|ui| {
                    ui.label("Quantize note starts");
                    let mut quantize = self.midi_recorder.quantize;
                    egui::ComboBox::from_id_salt("midi_quantize")
                        .selected_text(match quantize {
                            Some(v) if v == redraw_piano_roll::GRID_QUARTER => String::from("1/4"),
                            Some(v) if v == redraw_piano_roll::GRID_1_8 => String::from("1/8"),
                            Some(v) if v == redraw_piano_roll::GRID_1_16 => String::from("1/16"),
                            Some(v) if v == redraw_piano_roll::GRID_1_32 => String::from("1/32"),
                            Some(_) => String::from("Custom"),
                            None => String::from("Off"),
                        })
                        .show_ui(ui, |ui| {
                            for (value, label) in [
                                (None, "Off"),
                                (Some(redraw_piano_roll::GRID_QUARTER), "1/4"),
                                (Some(redraw_piano_roll::GRID_1_8), "1/8"),
                                (Some(redraw_piano_roll::GRID_1_16), "1/16"),
                                (Some(redraw_piano_roll::GRID_1_32), "1/32"),
                            ] {
                                if ui
                                    .selectable_label(quantize == value, label)
                                    .clicked()
                                {
                                    quantize = value;
                                }
                            }
                        });
                    self.midi_recorder.set_quantize(quantize);
                });
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(self.record_armed, "Record armed")
                        .clicked()
                    {
                        self.toggle_record();
                    }
                    ui.label(
                        RichText::new(format!(
                            "{} note(s), {} CC event(s) recorded this session",
                            self.midi_recorder.recorded_notes, self.midi_recorder.recorded_cc
                        ))
                        .small()
                        .color(self.palette.text_dim),
                    );
                });
                ui.separator();
                if ui.button("Close").clicked() {
                    *keep = false;
                }
            });
        if !open {
            *keep = false;
        }
    }

    fn audio_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        egui::Window::new("Audio Settings")
            .id(egui::Id::new("audio_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(450.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Output device").strong());
                ui.label(
                    RichText::new(if self.audio.is_running() {
                        format!(
                            "{}  |  {} Hz  |  {} frames",
                            self.audio.device_name(),
                            self.audio.sample_rate(),
                            self.audio.block_size()
                        )
                    } else {
                        "No audio device is open".to_string()
                    })
                    .color(if self.audio.is_running() {
                        self.palette.ok
                    } else {
                        self.palette.rec
                    }),
                );
                if let Some(error) = &self.audio.error {
                    ui.label(RichText::new(error).small().color(self.palette.rec));
                }
                ui.separator();
                ui.label(
                    RichText::new(
                        "REDRAW requests 48 kHz stereo f32 with a 512-frame buffer. If that is not available the default device format is used.",
                    )
                    .small()
                    .color(self.palette.text_dim),
                );
                ui.horizontal(|ui| {
                    if ui.button("Retry audio").clicked() {
                        self.audio.restart(&self.project, &self.factory);
                        if self.audio.is_running() {
                            self.set_status(format!(
                                "Audio: {} @ {} Hz",
                                self.audio.device_name(),
                                self.audio.sample_rate()
                            ));
                        } else {
                            let error = self
                                .audio
                                .error
                                .clone()
                                .unwrap_or_else(|| "audio device unavailable".into());
                            self.set_error(format!("Audio unavailable: {error}"));
                        }
                    }
                    if ui.button("Close").clicked() {
                        *keep = false;
                    }
                });
            });
        if !open {
            *keep = false;
        }
    }

    fn project_info_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        egui::Window::new("Project Info")
            .id(egui::Id::new("project_info_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Title");
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut self.project.name)
                                .desired_width(300.0),
                        )
                        .changed()
                    {
                        self.dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Author");
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut self.project.author)
                                .desired_width(290.0),
                        )
                        .changed()
                    {
                        self.dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Tempo");
                    if ui
                        .add(
                            egui::DragValue::new(&mut self.project.bpm)
                                .range(10.0..=999.0)
                                .speed(0.25)
                                .suffix(" BPM"),
                        )
                        .changed()
                    {
                        self.audio.set_bpm(self.project.bpm);
                        self.dirty = true;
                    }
                    ui.label("Swing");
                    if ui
                        .add(
                            egui::Slider::new(&mut self.project.swing, 0.0..=0.75)
                                .fixed_decimals(2),
                        )
                        .changed()
                    {
                        self.audio.set_swing(self.project.swing);
                        self.dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Time signature");
                    let mut num = self.project.time_sig.0 as i32;
                    let mut den = self.project.time_sig.1 as i32;
                    if ui.add(egui::DragValue::new(&mut num).range(1..=16)).changed() {
                        self.project.time_sig.0 = num as u8;
                        self.dirty = true;
                    }
                    ui.label("/");
                    if ui.add(egui::DragValue::new(&mut den).range(1..=16)).changed() {
                        self.project.time_sig.1 = den as u8;
                        self.dirty = true;
                    }
                });
                ui.separator();
                ui.label(
                    RichText::new(format!(
                        "{} pattern(s), {} channel(s), {} playlist clip(s), {} automation lane(s)",
                        self.project.patterns.len(),
                        self.project.channels.len(),
                        self.project.playlist.clips.len(),
                        self.project.automation.len()
                    ))
                    .small()
                    .color(self.palette.text_dim),
                );
                ui.label(
                    RichText::new(format!(
                        "Project PPQ {} | engine sample rate {} Hz | native format .rdp",
                        redraw_shared::PPQ,
                        self.project.sample_rate
                    ))
                    .small()
                    .color(self.palette.text_dim),
                );
                ui.separator();
                if ui.button("Close").clicked() {
                    *keep = false;
                }
            });
        if !open {
            *keep = false;
        }
    }

    fn statistics_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        let notes: usize = self
            .project
            .patterns
            .iter()
            .flat_map(|p| p.channels.iter())
            .map(|c| c.notes.len())
            .sum();
        let steps: usize = self
            .project
            .patterns
            .iter()
            .flat_map(|p| p.channels.iter())
            .map(|c| c.steps.iter().filter(|s| s.active).count())
            .sum();
        let cc: usize = self
            .project
            .patterns
            .iter()
            .flat_map(|p| p.channels.iter())
            .map(|c| c.cc.len())
            .sum();
        let points: usize = self.project.automation.iter().map(|a| a.points.len()).sum();
        let song_ticks = redraw_arrangement::playlist_length(&self.project);
        let bars = if self.bar_ticks() > 0 {
            song_ticks as f64 / self.bar_ticks() as f64
        } else {
            0.0
        };
        egui::Window::new("Statistics")
            .id(egui::Id::new("statistics_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                egui::Grid::new("stats_grid")
                    .num_columns(2)
                    .spacing([24.0, 4.0])
                    .show(ui, |ui| {
                        for (label, value) in [
                            ("Patterns", self.project.patterns.len().to_string()),
                            ("Channels", self.project.channels.len().to_string()),
                            ("Notes", notes.to_string()),
                            ("Active steps", steps.to_string()),
                            ("CC events", cc.to_string()),
                            ("Playlist clips", self.project.playlist.clips.len().to_string()),
                            ("Markers", self.project.playlist.markers.len().to_string()),
                            ("Automation lanes", self.project.automation.len().to_string()),
                            ("Automation points", points.to_string()),
                            ("Song length", format!("{:.2} bars", bars)),
                            ("Song end", self.tick_text(song_ticks)),
                        ] {
                            ui.label(RichText::new(label).color(self.palette.text_dim));
                            ui.label(RichText::new(value).strong());
                            ui.end_row();
                        }
                    });
                ui.separator();
                if ui.button("Close").clicked() {
                    *keep = false;
                }
            });
        if !open {
            *keep = false;
        }
    }

    fn shortcuts_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        egui::Window::new("Keyboard Shortcuts")
            .id(egui::Id::new("shortcuts_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .default_height(440.0)
            .show(ctx, |ui| {
                ScrollArea::vertical().show(ui, |ui| {
                    let rows = [
                        ("Space", "Play / stop"),
                        ("Ctrl+Space", "Play from start"),
                        ("R", "Arm live MIDI recording"),
                        ("L", "Toggle playlist loop"),
                        ("Ctrl+N / Ctrl+O", "New / open project"),
                        ("Ctrl+S / Ctrl+Shift+S", "Save / save as"),
                        ("Ctrl+E", "Export audio or video"),
                        ("Ctrl+Z / Ctrl+Y", "Undo / redo"),
                        ("Ctrl+X / Ctrl+C / Ctrl+V", "Cut / copy / paste"),
                        ("Ctrl+D", "Duplicate"),
                        ("Delete", "Delete selection"),
                        ("Ctrl+A", "Select all"),
                        ("Esc", "Deselect / close window"),
                        ("F5", "Show or hide playlist"),
                        ("F6 / F7 / F9", "Channel rack / piano roll / mixer"),
                        ("F8", "Show or hide browser"),
                        ("F10", "Preferences"),
                        ("F11", "Full screen"),
                        ("1 2 3 4 5", "Draw / select / erase / split / merge (piano roll)"),
                        ("Q / H", "Quantize / humanize"),
                        ("Shift+drag", "Fine knob adjustment"),
                        ("Double-click knob", "Reset to default"),
                        ("Ctrl+wheel", "Zoom timeline or piano roll"),
                    ];
                    egui::Grid::new("shortcuts_grid")
                        .num_columns(2)
                        .spacing([22.0, 5.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for (keys, action) in rows {
                                ui.label(RichText::new(keys).monospace().color(self.palette.accent));
                                ui.label(action);
                                ui.end_row();
                            }
                        });
                });
                ui.separator();
                if ui.button("Close").clicked() {
                    *keep = false;
                }
            });
        if !open {
            *keep = false;
        }
    }

    fn about_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        egui::Window::new("About REDRAW")
            .id(egui::Id::new("about_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(430.0)
            .show(ctx, |ui| {
                ui.heading(redraw_shared::APP_NAME);
                ui.label(
                    RichText::new(redraw_shared::APP_TAGLINE)
                        .italics()
                        .color(self.palette.accent),
                );
                ui.add_space(6.0);
                ui.label("Version 0.1.0  |  Phase 16 release");
                ui.label(
                    RichText::new(
                        "A clean-room digital audio workstation written in Rust. The user interface, plugin registry, project format and DSP are original work.",
                    )
                    .small()
                    .color(self.palette.text_dim),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "No third-party code, samples, presets, fonts, artwork or trademarks are bundled. .rdp is the authoritative project format; .flp support is a partial compatibility layer.",
                    )
                    .small()
                    .color(self.palette.text_dim),
                );
                ui.separator();
                if ui.button("Close").clicked() {
                    *keep = false;
                }
            });
        if !open {
            *keep = false;
        }
    }

    fn diagnostics_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        let ffmpeg = redraw_export::find_ffmpeg()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not found".into());
        let log = self.log.clone();
        egui::Window::new("Diagnostics")
            .id(egui::Id::new("diagnostics_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .default_height(430.0)
            .show(ctx, |ui| {
                egui::Grid::new("diag_grid")
                    .num_columns(2)
                    .spacing([20.0, 3.0])
                    .show(ui, |ui| {
                        for (label, value) in [
                            ("Workspace", self.root.display().to_string()),
                            ("User data", self.user_data.display().to_string()),
                            ("Audio", self.audio.device_name().to_string()),
                            ("Audio error", self.audio.error.clone().unwrap_or_else(|| "-".into())),
                            ("MIDI", self.midi.connected_port().unwrap_or("-").to_string()),
                            ("ffmpeg", ffmpeg.clone()),
                            ("Autosave", self.autosave.autosave_dir().display().to_string()),
                        ] {
                            ui.label(RichText::new(label).color(self.palette.text_dim));
                            ui.label(RichText::new(value).monospace().small());
                            ui.end_row();
                        }
                    });
                ui.separator();
                ui.label(RichText::new("Log").strong());
                ScrollArea::vertical()
                    .max_height(220.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in log.iter().rev().take(160).rev() {
                            ui.label(RichText::new(line).monospace().small());
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Open log folder").clicked() {
                        self.open_log_folder();
                    }
                    if ui.button("Stress test").clicked() {
                        self.stress_test();
                    }
                    if ui.button("Close").clicked() {
                        *keep = false;
                    }
                });
            });
        if !open {
            *keep = false;
        }
    }

    fn pattern_length_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        egui::Window::new("Pattern Length")
            .id(egui::Id::new("pattern_length_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(340.0)
            .show(ctx, |ui| {
                ui.label("Steps (16th notes)");
                ui.add(
                    egui::DragValue::new(&mut self.pattern_length_steps)
                        .range(1..=4096)
                        .speed(1.0),
                );
                ui.horizontal(|ui| {
                    for steps in [16u32, 32, 64, 128] {
                        if ui.button(steps.to_string()).clicked() {
                            self.pattern_length_steps = steps;
                        }
                    }
                });
                ui.label(
                    RichText::new(format!(
                        "Current length: {} steps",
                        self.pattern().map(|p| p.length_steps).unwrap_or(0)
                    ))
                    .small()
                    .color(self.palette.text_dim),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        self.set_pattern_length(self.pattern_length_steps);
                        *keep = false;
                    }
                    if ui.button("Cancel").clicked() {
                        *keep = false;
                    }
                });
            });
        if !open {
            *keep = false;
        }
    }

    fn rename_pattern_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        egui::Window::new("Rename Pattern")
            .id(egui::Id::new("rename_pattern_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .show(ctx, |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.rename_text)
                        .desired_width(f32::INFINITY),
                );
                response.request_focus();
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Rename").clicked() {
                        self.set_pattern_name(&self.rename_text.clone());
                        *keep = false;
                    }
                    if ui.button("Cancel").clicked() {
                        *keep = false;
                    }
                });
            });
        if !open {
            *keep = false;
        }
    }

    fn add_marker_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        egui::Window::new("Add Marker")
            .id(egui::Id::new("add_marker_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.marker_text)
                            .desired_width(240.0),
                    );
                });
                ui.label(
                    RichText::new(format!("Position: {}", self.tick_text(self.current_tick())))
                        .small()
                        .color(self.palette.text_dim),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Add").clicked() {
                        let tick = self.current_tick();
                        redraw_arrangement::add_marker(
                            &mut self.project,
                            self.marker_text.trim(),
                            tick,
                        );
                        self.dirty = true;
                        self.set_status("Marker added");
                        *keep = false;
                    }
                    if ui.button("Cancel").clicked() {
                        *keep = false;
                    }
                });
            });
        if !open {
            *keep = false;
        }
    }

    fn quantize_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        egui::Window::new("Quantize")
            .id(egui::Id::new("quantize_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Grid");
                    egui::ComboBox::from_id_salt("quant_grid")
                        .selected_text(
                            redraw_piano_roll::GRIDS
                                .iter()
                                .find(|(value, _)| *value == self.quantize.grid)
                                .map(|(_, label)| *label)
                                .unwrap_or("1/16"),
                        )
                        .show_ui(ui, |ui| {
                            for (value, label) in redraw_piano_roll::GRIDS {
                                if ui
                                    .selectable_label(self.quantize.grid == value, label)
                                    .clicked()
                                {
                                    self.quantize.grid = value;
                                }
                            }
                        });
                });
                ui.add(
                    egui::Slider::new(&mut self.quantize.strength, 0.0..=1.0)
                        .text("Strength")
                        .fixed_decimals(2),
                );
                ui.add(
                    egui::Slider::new(&mut self.quantize.swing, 0.5..=0.75)
                        .text("Swing")
                        .fixed_decimals(2),
                );
                ui.checkbox(&mut self.quantize.lengths, "Quantize note lengths");
                ui.checkbox(&mut self.quantize.velocities, "Quantize velocities");
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        let options = self.quantize;
                        self.begin_edit();
                        let count = if let Some(pattern) = self.project.pattern_mut() {
                            self.piano.quantize(pattern, options)
                        } else {
                            0
                        };
                        self.commit_edit();
                        self.schedule_dirty = true;
                        self.set_status(format!("Quantized {count} note(s)"));
                        *keep = false;
                    }
                    if ui.button("Cancel").clicked() {
                        *keep = false;
                    }
                });
            });
        if !open {
            *keep = false;
        }
    }

    fn humanize_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        egui::Window::new("Humanize")
            .id(egui::Id::new("humanize_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.add(
                    egui::Slider::new(&mut self.humanize.time, 0..=120)
                        .text("Timing (ticks)"),
                );
                ui.add(
                    egui::Slider::new(&mut self.humanize.velocity, 0..=40)
                        .text("Velocity"),
                );
                ui.add(
                    egui::Slider::new(&mut self.humanize.pan, 0..=32).text("Pan"),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        let options = self.humanize;
                        self.begin_edit();
                        let count = if let Some(pattern) = self.project.pattern_mut() {
                            self.piano.humanize(pattern, options)
                        } else {
                            0
                        };
                        self.commit_edit();
                        self.schedule_dirty = true;
                        self.set_status(format!("Humanized {count} note(s)"));
                        *keep = false;
                    }
                    if ui.button("Cancel").clicked() {
                        *keep = false;
                    }
                });
            });
        if !open {
            *keep = false;
        }
    }

    fn transpose_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        egui::Window::new("Transpose")
            .id(egui::Id::new("transpose_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(340.0)
            .show(ctx, |ui| {
                ui.add(
                    egui::Slider::new(&mut self.transpose_semitones, -36..=36)
                        .text("Semitones"),
                );
                ui.horizontal(|ui| {
                    if ui.button("-12").clicked() {
                        self.transpose_semitones = -12;
                    }
                    if ui.button("+12").clicked() {
                        self.transpose_semitones = 12;
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        let semitones = self.transpose_semitones;
                        self.begin_edit();
                        if let Some(pattern) = self.project.pattern_mut() {
                            self.piano.nudge_selected(pattern, 0, semitones);
                        }
                        self.commit_edit();
                        self.schedule_dirty = true;
                        *keep = false;
                    }
                    if ui.button("Cancel").clicked() {
                        *keep = false;
                    }
                });
            });
        if !open {
            *keep = false;
        }
    }

    fn add_channel_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        let names: Vec<&'static str> = self.factory.synth_names().to_vec();
        let mut chosen: Option<&'static str> = None;
        egui::Window::new("Add Instrument")
            .id(egui::Id::new("add_channel_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(390.0)
            .default_height(460.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Search");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.browser.search)
                            .hint_text("instrument name")
                            .desired_width(240.0),
                    );
                });
                ui.separator();
                let query = self.browser.search.trim().to_ascii_lowercase();
                ScrollArea::vertical().show(ui, |ui| {
                    for name in names {
                        if !query.is_empty() && !name.to_ascii_lowercase().contains(&query) {
                            continue;
                        }
                        if ui
                            .add_sized(
                                [ui.available_width(), 22.0],
                                egui::Button::new(name).frame(false),
                            )
                            .clicked()
                        {
                            chosen = Some(name);
                        }
                    }
                });
                ui.separator();
                if ui.button("Cancel").clicked() {
                    *keep = false;
                }
            });
        if let Some(name) = chosen {
            self.add_channel_by_name(name);
            *keep = false;
        }
        if !open {
            *keep = false;
        }
    }

    fn replace_synth_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let Some(channel_id) = self.pending_channel else {
            *keep = false;
            return;
        };
        let mut open = true;
        let names: Vec<&'static str> = self.factory.synth_names().to_vec();
        let mut chosen: Option<&'static str> = None;
        egui::Window::new("Replace Instrument")
            .id(egui::Id::new("replace_synth_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(390.0)
            .default_height(460.0)
            .show(ctx, |ui| {
                let current = self
                    .project
                    .channel(channel_id)
                    .map(|c| c.synth_type.clone())
                    .unwrap_or_default();
                ui.label(
                    RichText::new(format!("Current: {current}"))
                        .small()
                        .color(self.palette.text_dim),
                );
                ScrollArea::vertical().show(ui, |ui| {
                    for name in names {
                        if ui
                            .selectable_label(name == current, name)
                            .clicked()
                        {
                            chosen = Some(name);
                        }
                    }
                });
                ui.separator();
                if ui.button("Cancel").clicked() {
                    *keep = false;
                }
            });
        if let Some(name) = chosen {
            self.replace_synth_by_name(channel_id, name);
            *keep = false;
        }
        if !open {
            *keep = false;
        }
    }

    fn add_effect_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let (strip, _) = self
            .pending_fx
            .unwrap_or((FxStrip::Channel(self.selection.strip), 0));
        let mut open = true;
        let names: Vec<&'static str> = self.factory.effect_names().to_vec();
        let mut chosen: Option<&'static str> = None;
        egui::Window::new("Add Effect")
            .id(egui::Id::new("add_effect_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(390.0)
            .default_height(460.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!("Insert on {}", strip.label()))
                        .small()
                        .color(self.palette.text_dim),
                );
                ui.horizontal(|ui| {
                    ui.label("Search");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.browser.search)
                            .hint_text("effect name")
                            .desired_width(240.0),
                    );
                });
                ui.separator();
                let query = self.browser.search.trim().to_ascii_lowercase();
                ScrollArea::vertical().show(ui, |ui| {
                    for name in names {
                        if !query.is_empty() && !name.to_ascii_lowercase().contains(&query) {
                            continue;
                        }
                        if ui
                            .add_sized(
                                [ui.available_width(), 22.0],
                                egui::Button::new(name).frame(false),
                            )
                            .clicked()
                        {
                            chosen = Some(name);
                        }
                    }
                });
                ui.separator();
                if ui.button("Cancel").clicked() {
                    *keep = false;
                }
            });
        if let Some(name) = chosen {
            self.add_effect_by_name(strip, name);
            *keep = false;
        }
        if !open {
            *keep = false;
        }
    }

    fn scan_vst_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        let plugins = self.vst_plugins.clone();
        egui::Window::new("VST Scan")
            .id(egui::Id::new("scan_vst_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(600.0)
            .default_height(460.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Rescan").clicked() {
                        self.scan_vst_plugins();
                    }
                    ui.label(
                        RichText::new(format!("{} module(s)", plugins.len()))
                            .small()
                            .color(self.palette.text_dim),
                    );
                });
                ui.separator();
                ScrollArea::vertical().show(ui, |ui| {
                    if plugins.is_empty() {
                        ui.label(
                            RichText::new(
                                "No VST2/VST3 modules found in the standard folders. REDRAW's built-in instruments and effects are always available.",
                            )
                            .color(self.palette.text_dim),
                        );
                    }
                    for plugin in &plugins {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&plugin.name).strong());
                            ui.label(
                                RichText::new(format!("{:?} / {:?}", plugin.format, plugin.kind))
                                    .small()
                                    .color(self.palette.text_dim),
                            );
                        });
                        ui.label(
                            RichText::new(&plugin.path)
                                .monospace()
                                .small()
                                .color(self.palette.text_dim),
                        );
                    }
                });
                ui.separator();
                if ui.button("Close").clicked() {
                    *keep = false;
                }
            });
        if !open {
            *keep = false;
        }
    }

    fn close_confirm_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        egui::Window::new("Unsaved Changes")
            .id(egui::Id::new("close_confirm_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(390.0)
            .show(ctx, |ui| {
                ui.label("Save the current project before closing?");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Save and close").clicked() {
                        if self.save() {
                            self.close_window(ctx);
                            *keep = false;
                        }
                    }
                    if ui.button("Discard").clicked() {
                        self.allow_close = true;
                        self.close_window(ctx);
                        *keep = false;
                    }
                    if ui.button("Cancel").clicked() {
                        *keep = false;
                    }
                });
            });
        if !open {
            *keep = false;
        }
    }

    fn recovery_window(&mut self, ctx: &egui::Context, keep: &mut bool) {
        let mut open = true;
        let latest = self
            .recovery
            .as_ref()
            .and_then(|offer| offer.latest.as_ref())
            .map(|entry| (entry.path.display().to_string(), entry.header.clone()));
        egui::Window::new("Recover Autosave")
            .id(egui::Id::new("recovery_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(450.0)
            .show(ctx, |ui| {
                ui.label("REDRAW found a snapshot from a session that did not close cleanly.");
                if let Some((path, header)) = &latest {
                    ui.add_space(4.0);
                    ui.label(RichText::new(path).monospace().small());
                    if let Some(header) = header {
                        ui.label(
                            RichText::new(format!(
                                "Project: {} | modified {}",
                                header.title, header.modified
                            ))
                            .small()
                            .color(self.palette.text_dim),
                        );
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Recover").clicked() {
                        self.recover_autosave();
                        *keep = false;
                    }
                    if ui.button("Discard snapshots").clicked() {
                        let _ = self.autosave.end_session_clean();
                        self.recovery = None;
                        *keep = false;
                    }
                    if ui.button("Keep for later").clicked() {
                        *keep = false;
                    }
                });
            });
        if !open {
            *keep = false;
        }
    }

    // ------------------------------------------------------------ export

    pub fn export_dialog_ui(&mut self, ctx: &egui::Context) {
        if !self.export.open {
            return;
        }
        let mut open = self.export.open;
        let mut browse = false;
        let mut export = false;
        let mut format_changed = false;
        let palette = self.palette;
        egui::Window::new("Export Audio / Video")
            .id(egui::Id::new("export_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(500.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Scope").strong());
                    if ui
                        .selectable_label(self.export.song_scope, "Song")
                        .clicked()
                    {
                        self.export.song_scope = true;
                    }
                    if ui
                        .selectable_label(!self.export.song_scope, "Pattern")
                        .clicked()
                    {
                        self.export.song_scope = false;
                    }
                });
                if !self.export.song_scope {
                    ui.horizontal(|ui| {
                        ui.label("Pattern");
                        egui::ComboBox::from_id_salt("export_pattern")
                            .selected_text(
                                self.project
                                    .patterns
                                    .iter()
                                    .find(|p| p.id == self.export.pattern_id)
                                    .map(|p| p.name.clone())
                                    .unwrap_or_else(|| "Pattern".into()),
                            )
                            .show_ui(ui, |ui| {
                                for pattern in &self.project.patterns {
                                    if ui
                                        .selectable_label(
                                            self.export.pattern_id == pattern.id,
                                            &pattern.name,
                                        )
                                        .clicked()
                                    {
                                        self.export.pattern_id = pattern.id;
                                    }
                                }
                            });
                    });
                }
                ui.horizontal(|ui| {
                    ui.label("Format");
                    egui::ComboBox::from_id_salt("export_format")
                        .selected_text(RedrawApp::format_name(self.export.format_index))
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            for index in 0..13 {
                                if ui
                                    .selectable_label(
                                        self.export.format_index == index,
                                        RedrawApp::format_name(index),
                                    )
                                    .clicked()
                                {
                                    self.export.format_index = index;
                                    format_changed = true;
                                }
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label("Sample rate");
                    egui::ComboBox::from_id_salt("export_sr")
                        .selected_text(format!("{} Hz", self.export.sample_rate))
                        .show_ui(ui, |ui| {
                            for rate in [44_100u32, 48_000, 88_200, 96_000, 192_000] {
                                if ui
                                    .selectable_label(
                                        self.export.sample_rate == rate,
                                        format!("{rate} Hz"),
                                    )
                                    .clicked()
                                {
                                    self.export.sample_rate = rate;
                                }
                            }
                        });
                    ui.label("Tail");
                    ui.add(
                        egui::DragValue::new(&mut self.export.tail_seconds)
                            .range(0.0..=30.0)
                            .speed(0.1)
                            .suffix(" s"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Output");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.export.path)
                            .desired_width(330.0),
                    );
                    if ui.button("Browse...").clicked() {
                        browse = true;
                    }
                });
                ui.label(
                    RichText::new(
                        "Native WAV/AIFF is written in-tree. MP3, OGG, FLAC and video use ffmpeg; REDRAW can fetch a runtime build under vendor/ffmpeg when none is installed.",
                    )
                    .small()
                    .color(palette.text_dim),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    let enabled = !self.export.busy;
                    if ui
                        .add_enabled(enabled, egui::Button::new("Export"))
                        .clicked()
                    {
                        export = true;
                    }
                    if self.export.busy {
                        ui.spinner();
                        ui.label(RichText::new("Rendering...").color(palette.warn));
                    } else if ui.button("Close").clicked() {
                        self.export.open = false;
                    }
                });
                if !self.export.status.is_empty() {
                    ui.label(
                        RichText::new(&self.export.status).small().color(if self.export.error {
                            palette.rec
                        } else {
                            palette.text_dim
                        }),
                    );
                }
                if let Some(summary) = &self.export.summary {
                    ui.label(
                        RichText::new(format!(
                            "{} frames | {:.2} s | peak {:.2} dBFS",
                            summary.frames,
                            summary.duration_seconds,
                            redraw_shared::gain_to_db(summary.peak)
                        ))
                        .small()
                        .color(palette.ok),
                    );
                }
            });
        if format_changed {
            self.update_export_path_extension();
        }
        if browse {
            let ext = self.current_export_extension();
            let extensions = [ext];
            let filters = [("Output file", &extensions[..])];
            if let Some(path) = self.pick_save_file(
                "Choose export file",
                &format!("{}.{ext}", crate::app::sanitize_file_stem(&self.project.name)),
                &filters,
            ) {
                self.export.path = path.to_string_lossy().into_owned();
            }
        }
        if export {
            self.start_export();
        }
        if !open {
            self.export.open = false;
        }
    }
}



