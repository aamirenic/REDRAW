//! Selection inspector for channels, clips and automation.

use eframe::egui::{self, RichText, ScrollArea};

use crate::app::{draw_panel_header, FxStrip, LowerTab};
use crate::widgets::{format_db, format_pan, param_knob, section_title};
use redraw_shared::PluginFactory;

use crate::RedrawApp;

impl RedrawApp {
    pub fn inspector_ui(&mut self, ui: &mut egui::Ui) {
        let subtitle = if !self.selection.clips.is_empty() {
            "clip"
        } else if self.selection.automation.is_some() && self.layout.lower_tab == LowerTab::Automation {
            "automation"
        } else {
            "channel"
        };
        draw_panel_header(ui, &self.palette, "Inspector", subtitle);
        if !self.selection.clips.is_empty() {
            self.clip_inspector(ui);
        } else if self.selection.automation.is_some() && self.layout.lower_tab == LowerTab::Automation {
            self.automation_inspector(ui);
        } else {
            self.channel_inspector(ui);
        }
    }

    fn channel_inspector(&mut self, ui: &mut egui::Ui) {
        let id = self.selection.channel_id;
        let Some(channel) = self.project.channel(id) else {
            ui.label(RichText::new("No channel selected").color(self.palette.text_dim));
            if ui.button("Add instrument...").clicked() {
                self.dialog = Some(crate::app::DialogKind::AddChannel);
            }
            return;
        };
        let (mut name, color, synth_type, mut params, mut volume, mut pan, mut mute, mut solo, mut offset, strip) = (
            channel.name.clone(),
            channel.color,
            channel.synth_type.clone(),
            channel.params.clone(),
            channel.volume_db,
            channel.pan,
            channel.mute,
            channel.solo,
            channel.note_offset,
            channel.mixer_index,
        );
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(6.0, 24.0), egui::Sense::hover());
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(1),
                crate::theme::color(color),
            );
            if ui
                .add(
                    egui::TextEdit::singleline(&mut name)
                        .font(egui::TextStyle::Heading)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                self.update_channel_name(id, &name);
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Instrument").small().color(self.palette.text_dim));
            let mut new_synth = None;
            egui::ComboBox::from_id_salt(("synth_combo", id))
                .selected_text(&synth_type)
                .width(150.0)
                .show_ui(ui, |ui| {
                    for candidate in self.factory.synth_names() {
                        if ui
                            .selectable_label(*candidate == synth_type, *candidate)
                            .clicked()
                        {
                            new_synth = Some(*candidate);
                        }
                    }
                });
            if let Some(candidate) = new_synth {
                self.replace_synth_by_name(id, candidate);
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Open instrument").clicked() {
                self.open_plugin_for_channel(id);
            }
            if ui.button("Replace...").clicked() {
                self.pending_channel = Some(id);
                self.dialog = Some(crate::app::DialogKind::ReplaceSynth);
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Slider::new(&mut volume, -90.0..=12.0)
                        .text("Vol")
                        .fixed_decimals(1),
                )
                .changed()
            {
                self.update_channel_gain(id, volume);
            }
            if ui
                .add(
                    egui::Slider::new(&mut pan, -1.0..=1.0)
                        .text("Pan")
                        .fixed_decimals(2),
                )
                .changed()
            {
                self.update_channel_pan(id, pan);
            }
        });
        ui.horizontal(|ui| {
            if ui.checkbox(&mut mute, "Mute").changed() {
                self.update_channel_mute(id, mute);
            }
            if ui.checkbox(&mut solo, "Solo").changed() {
                self.update_channel_solo(id, solo);
            }
            ui.label(
                RichText::new(format!("{}  |  strip {}", format_pan(pan), strip + 1))
                    .small()
                    .color(self.palette.text_dim),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Note offset");
            if ui
                .add(egui::DragValue::new(&mut offset).range(-48..=48).suffix(" st"))
                .changed()
            {
                if let Some(channel) = self.project.channel_mut(id) {
                    channel.note_offset = offset;
                }
                self.dirty = true;
            }
        });
        section_title(ui, &self.palette, "Instrument parameters");
        let specs = self.specs_for_synth(&synth_type);
        if params.len() < specs.len() {
            // Older/default projects may not have materialised every parameter
            // yet. Filling the canonical defaults is a repair, not a user edit,
            // so it must not put an asterisk on an otherwise clean project.
            params.extend(specs[params.len()..].iter().map(|spec| spec.default));
            if let Some(channel) = self.project.channel_mut(id) {
                channel.params = params.clone();
            }
        }
        if specs.is_empty() {
            ui.label(
                RichText::new("This instrument exposes no parameters.")
                    .small()
                    .color(self.palette.text_dim),
            );
        } else {
            let accent = crate::theme::color(color);
            let mut changed = false;
            ScrollArea::vertical()
                .max_height(220.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (index, spec) in specs.iter().enumerate() {
                            let Some(value) = params.get_mut(index) else {
                                continue;
                            };
                            let response = param_knob(
                                ui,
                                ("inspector_param", id, index),
                                spec,
                                value,
                                accent,
                                &self.palette,
                            );
                            if response.changed {
                                changed = true;
                                self.audio.set_synth_param(id, index as u32, *value);
                            }
                        }
                    });
                });
            if changed {
                if let Some(channel) = self.project.channel_mut(id) {
                    channel.params = params;
                }
                self.dirty = true;
            }
        }
        section_title(ui, &self.palette, "Insert chain");
        let inserts = self
            .project
            .mixer
            .strips
            .get(strip)
            .map(|s| s.inserts.clone())
            .unwrap_or_default();
        for (slot, effect) in inserts.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{:02}", slot + 1))
                        .monospace()
                        .small()
                        .color(self.palette.text_dim),
                );
                match effect {
                    Some(effect) => {
                        let mut bypass = effect.bypass;
                        if ui
                            .selectable_label(!bypass, &effect.effect_type)
                            .on_hover_text("Click to open the effect window")
                            .clicked()
                        {
                            self.open_plugin_for_fx(FxStrip::Channel(strip), slot);
                        }
                        if ui
                            .selectable_label(bypass, "B")
                            .on_hover_text("Bypass")
                            .clicked()
                        {
                            bypass = !bypass;
                            if let Some(slot_state) = self
                                .project
                                .mixer
                                .strips
                                .get_mut(strip)
                                .and_then(|s| s.inserts.get_mut(slot))
                                .and_then(|s| s.as_mut())
                            {
                                slot_state.bypass = bypass;
                            }
                            self.audio.set_fx_bypass(id, slot as u8, bypass);
                            self.dirty = true;
                        }
                    }
                    None => {
                        if ui
                            .selectable_label(false, RichText::new("empty").color(self.palette.text_dim))
                            .clicked()
                        {
                            self.open_add_effect(FxStrip::Channel(strip));
                        }
                    }
                }
            });
        }
        if ui.button("+ Add effect").clicked() {
            self.open_add_effect(FxStrip::Channel(strip));
        }
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "{} / {} dB / {}",
                synth_type,
                format_db(volume),
                format_pan(pan)
            ))
            .small()
            .color(self.palette.text_dim),
        );
    }

    fn clip_inspector(&mut self, ui: &mut egui::Ui) {
        let Some(clip_id) = self.selection.clips.first().copied() else {
            return;
        };
        let Some(clip) = redraw_arrangement::find_clip(&self.project, clip_id).cloned() else {
            ui.label(RichText::new("Clip no longer exists").color(self.palette.text_dim));
            return;
        };
        let mut name = clip.name.clone();
        let mut track = clip.track;
        let mut start = clip.start;
        let mut length = clip.length;
        let mut looping = clip.looping;
        let mut gain = clip.gain_db;
        let mut transpose = clip.transpose;
        let mut stretch = clip.stretch;
        let mut changed = false;
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(6.0, 24.0), egui::Sense::hover());
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(1),
                crate::theme::color(clip.color),
            );
            if ui
                .add(
                    egui::TextEdit::singleline(&mut name)
                        .font(egui::TextStyle::Heading)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                changed = true;
            }
        });
        ui.label(
            RichText::new(format!("{:?} clip", clip.kind))
                .small()
                .color(self.palette.text_dim),
        );
        ui.horizontal(|ui| {
            ui.label("Track");
            let track_count = self.project.playlist.tracks.len();
            egui::ComboBox::from_id_salt(("clip_track", clip_id))
                .selected_text(
                    self.project
                        .playlist
                        .tracks
                        .get(track)
                        .map(|t| t.name.clone())
                        .unwrap_or_else(|| format!("Track {}", track + 1)),
                )
                .show_ui(ui, |ui| {
                    for index in 0..track_count {
                        let label = self
                            .project
                            .playlist
                            .tracks
                            .get(index)
                            .map(|t| t.name.clone())
                            .unwrap_or_else(|| format!("Track {}", index + 1));
                        if ui.selectable_label(track == index, label).clicked() {
                            track = index;
                            changed = true;
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Start");
            if ui
                .add(egui::DragValue::new(&mut start).speed(10.0).suffix(" ticks"))
                .changed()
            {
                changed = true;
            }
            ui.label(self.tick_text(start));
        });
        ui.horizontal(|ui| {
            ui.label("Length");
            if ui
                .add(
                    egui::DragValue::new(&mut length)
                        .range(1..=u64::MAX)
                        .speed(10.0)
                        .suffix(" ticks"),
                )
                .changed()
            {
                changed = true;
            }
        });
        ui.checkbox(&mut looping, "Loop pattern content").changed().then(|| changed = true);
        ui.horizontal(|ui| {
            ui.label("Gain");
            if ui
                .add(egui::Slider::new(&mut gain, -60.0..=12.0).suffix(" dB").fixed_decimals(1))
                .changed()
            {
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Transpose");
            if ui
                .add(egui::DragValue::new(&mut transpose).range(-48..=48).suffix(" st"))
                .changed()
            {
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Stretch");
            if ui
                .add(egui::DragValue::new(&mut stretch).range(0.25..=4.0).speed(0.01))
                .changed()
            {
                changed = true;
            }
        });
        if let Some(path) = &clip.audio_path {
            ui.label(
                RichText::new(path)
                    .monospace()
                    .small()
                    .color(self.palette.text_dim),
            );
            ui.label(
                RichText::new("Audio clip playback is not routed in this phase.")
                    .small()
                    .color(self.palette.warn),
            );
        }
        if changed {
            let track_count = self.project.playlist.tracks.len();
            if let Some(target) = redraw_arrangement::clip_mut(&mut self.project, clip_id) {
                target.name = name;
                target.track = track.min(track_count.saturating_sub(1));
                target.start = start;
                target.length = length.max(1);
                target.looping = looping;
                target.gain_db = gain;
                target.transpose = transpose;
                target.stretch = stretch;
            }
            self.dirty = true;
            self.sync_schedules();
        }
        ui.separator();
        ui.horizontal(|ui| {
            if clip.kind == redraw_shared::ClipKind::Pattern {
                if ui.button("Open pattern").clicked() {
                    if let Some(index) = self
                        .project
                        .patterns
                        .iter()
                        .position(|p| p.id == clip.source_id)
                    {
                        self.set_pattern_index(index);
                        self.layout.lower_tab = LowerTab::PianoRoll;
                    }
                }
            }
            if ui.button("Duplicate").clicked() {
                self.begin_edit();
                if let Some(new_id) = redraw_arrangement::duplicate_clip(&mut self.project, clip_id) {
                    self.selection.clips = vec![new_id];
                }
                self.commit_edit();
                self.sync_schedules();
            }
            if ui.button("Delete").clicked() {
                self.delete_selection();
            }
        });
    }

    fn automation_inspector(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selection.automation else {
            return;
        };
        let Some(clip) = redraw_automation::find_clip(&self.project, id).cloned() else {
            ui.label(RichText::new("Automation clip no longer exists").color(self.palette.text_dim));
            return;
        };
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(6.0, 24.0), egui::Sense::hover());
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(1),
                crate::theme::color(clip.color),
            );
            ui.label(RichText::new(&clip.name).heading());
        });
        ui.label(
            RichText::new(redraw_automation::target_label(&clip.target))
                .small()
                .color(self.palette.text_dim),
        );
        ui.label(
            RichText::new(format!("{} points  |  {:?}", clip.points.len(), clip.interp))
                .small()
                .color(self.palette.text_dim),
        );
        ui.label(
            RichText::new(format!(
                "{} to {}",
                self.tick_text(0),
                self.tick_text(clip.length)
            ))
            .small()
            .color(self.palette.text_dim),
        );
        ui.horizontal(|ui| {
            if ui.button("Open editor").clicked() {
                self.layout.lower_tab = LowerTab::Automation;
            }
            if ui.button("Delete").clicked() {
                self.delete_selection();
            }
        });
        ui.separator();
        ui.label(
            RichText::new(
                "Automation is evaluated at the engine's 64-frame control rate. Project-order lanes use last-wins for the same target.",
            )
            .small()
            .color(self.palette.text_dim),
        );
    }
}

