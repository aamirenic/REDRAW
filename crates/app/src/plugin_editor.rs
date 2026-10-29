//! Floating instrument and effect editors.

use eframe::egui::{self, RichText, ScrollArea};

use crate::app::{FxStrip, PluginTarget};
use crate::widgets::param_knob;
use crate::RedrawApp;

impl RedrawApp {
    pub fn plugin_window_ui(&mut self, ctx: &egui::Context, target: &PluginTarget) {
        match target {
            PluginTarget::Synth(channel_id) => self.synth_window(ctx, *channel_id, target),
            PluginTarget::Fx { strip, slot } => self.fx_window(ctx, *strip, *slot, target),
        }
    }

    fn synth_window(&mut self, ctx: &egui::Context, channel_id: u32, target: &PluginTarget) {
        let Some(channel) = self.project.channel(channel_id).cloned() else {
            self.plugin_windows.retain(|t| t != target);
            return;
        };
        let mut window_open = true;
        let mut close_requested = false;
        let title = format!("{} - {}", channel.name, channel.synth_type);
        let specs = self.specs_for_synth(&channel.synth_type);
        let mut params = channel.params.clone();
        let mut changed = params.len() < specs.len();
        if params.len() < specs.len() {
            params.extend(specs[params.len()..].iter().map(|spec| spec.default));
        }
        let mut reset = false;
        let palette = self.palette;
        egui::Window::new(title)
            .id(egui::Id::new(("synth_window", channel_id)))
            .open(&mut window_open)
            .default_width(470.0)
            .default_height(360.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(5.0, 20.0), egui::Sense::hover());
                    ui.painter().rect_filled(
                        rect,
                        egui::CornerRadius::same(1),
                        crate::theme::color(channel.color),
                    );
                    ui.label(RichText::new(&channel.name).strong());
                    ui.label(
                        RichText::new(&channel.synth_type)
                            .small()
                            .color(palette.text_dim),
                    );
                });
                ui.separator();
                if specs.is_empty() {
                    ui.label(
                        RichText::new("This instrument exposes no editable parameters.")
                            .color(palette.text_dim),
                    );
                } else {
                    ScrollArea::vertical().show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for (index, spec) in specs.iter().enumerate() {
                                let Some(value) = params.get_mut(index) else {
                                    continue;
                                };
                                let response = param_knob(
                                    ui,
                                    ("synth_window_param", channel_id, index),
                                    spec,
                                    value,
                                    crate::theme::color(channel.color),
                                    &palette,
                                );
                                if response.changed {
                                    changed = true;
                                    self.audio.set_synth_param(channel_id, index as u32, *value);
                                }
                            }
                        });
                    });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Reset parameters").clicked() {
                        reset = true;
                    }
                    if ui.button("All notes off").clicked() {
                        self.audio.all_notes_off();
                    }
                    if ui.button("Close").clicked() {
                        close_requested = true;
                    }
                });
            });
        if close_requested {
            window_open = false;
        }
        if reset {
            let defaults: Vec<f32> = specs.iter().map(|spec| spec.default).collect();
            if let Some(channel) = self.project.channel_mut(channel_id) {
                channel.params = defaults.clone();
            }
            for (index, value) in defaults.iter().enumerate() {
                self.audio.set_synth_param(channel_id, index as u32, *value);
            }
            self.dirty = true;
        } else if changed {
            if let Some(channel) = self.project.channel_mut(channel_id) {
                channel.params = params;
            }
            self.dirty = true;
        }
        if !window_open {
            self.plugin_windows.retain(|t| t != target);
        }
    }

    fn fx_window(
        &mut self,
        ctx: &egui::Context,
        strip: FxStrip,
        slot: usize,
        target: &PluginTarget,
    ) {
        let (effect_type, mut params, mut bypass) = match self.fx_slot(strip, slot) {
            Some(slot) => (slot.effect_type.clone(), slot.params.clone(), slot.bypass),
            None => {
                self.plugin_windows.retain(|t| t != target);
                return;
            }
        };
        let specs = self.specs_for_effect(&effect_type);
        let mut window_open = true;
        let mut close_requested = false;
        let mut changed = params.len() < specs.len();
        if params.len() < specs.len() {
            params.extend(specs[params.len()..].iter().map(|spec| spec.default));
        }
        let mut removed = false;
        let palette = self.palette;
        let title = format!("{} - {}", strip.label(), effect_type);
        egui::Window::new(title)
            .id(egui::Id::new(("fx_window", strip, slot)))
            .open(&mut window_open)
            .default_width(440.0)
            .default_height(330.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&effect_type).strong());
                    if ui.checkbox(&mut bypass, "Bypass").changed() {
                        self.set_fx_bypass(strip, slot, bypass);
                        changed = true;
                    }
                });
                ui.separator();
                if specs.is_empty() {
                    ui.label(
                        RichText::new("This effect exposes no editable parameters.")
                            .color(palette.text_dim),
                    );
                } else {
                    ScrollArea::vertical().show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for (index, spec) in specs.iter().enumerate() {
                                let Some(value) = params.get_mut(index) else {
                                    continue;
                                };
                                let response = param_knob(
                                    ui,
                                    ("fx_window_param", strip, slot, index),
                                    spec,
                                    value,
                                    palette.accent,
                                    &palette,
                                );
                                if response.changed {
                                    changed = true;
                                    self.set_fx_param(strip, slot, index as u32, *value);
                                }
                            }
                        });
                    });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Remove effect").clicked() {
                        removed = true;
                    }
                    if ui.button("Close").clicked() {
                        close_requested = true;
                    }
                });
            });
        if close_requested {
            window_open = false;
        }
        if removed {
            self.remove_fx(strip, slot);
            window_open = false;
        } else if changed {
            self.write_fx(strip, slot, &params, bypass);
        }
        if !window_open {
            self.plugin_windows.retain(|t| t != target);
        }
    }

    fn fx_slot(&self, strip: FxStrip, slot: usize) -> Option<&redraw_shared::FxSlotState> {
        match strip {
            FxStrip::Channel(index) => self
                .project
                .mixer
                .strips
                .get(index)
                .and_then(|strip| strip.inserts.get(slot))
                .and_then(|slot| slot.as_ref()),
            FxStrip::Master => self
                .project
                .mixer
                .master
                .inserts
                .get(slot)
                .and_then(|slot| slot.as_ref()),
            FxStrip::Bus(index) => self
                .project
                .mixer
                .bus_inserts
                .get(index)
                .and_then(|slots| slots.get(slot))
                .and_then(|slot| slot.as_ref()),
        }
    }

    fn write_fx(&mut self, strip: FxStrip, slot: usize, params: &[f32], bypass: bool) {
        match strip {
            FxStrip::Channel(index) => {
                if let Some(slot_state) = self
                    .project
                    .mixer
                    .strips
                    .get_mut(index)
                    .and_then(|strip| strip.inserts.get_mut(slot))
                    .and_then(|slot| slot.as_mut())
                {
                    slot_state.params = params.to_vec();
                    slot_state.bypass = bypass;
                }
                if let Some(channel_id) = self.channel_for_strip(index) {
                    for (param, value) in params.iter().enumerate() {
                        self.audio
                            .set_fx_param(channel_id, slot as u8, param as u32, *value);
                    }
                    self.audio.set_fx_bypass(channel_id, slot as u8, bypass);
                }
            }
            FxStrip::Master => {
                if let Some(slot_state) = self
                    .project
                    .mixer
                    .master
                    .inserts
                    .get_mut(slot)
                    .and_then(|slot| slot.as_mut())
                {
                    slot_state.params = params.to_vec();
                    slot_state.bypass = bypass;
                }
                self.sync_schedules();
            }
            FxStrip::Bus(index) => {
                if let Some(slot_state) = self
                    .project
                    .mixer
                    .bus_inserts
                    .get_mut(index)
                    .and_then(|slots| slots.get_mut(slot))
                    .and_then(|slot| slot.as_mut())
                {
                    slot_state.params = params.to_vec();
                    slot_state.bypass = bypass;
                }
                self.sync_schedules();
            }
        }
        self.dirty = true;
    }

    fn set_fx_param(&mut self, strip: FxStrip, slot: usize, param: u32, value: f32) {
        if let FxStrip::Channel(index) = strip {
            if let Some(slot_state) = self
                .project
                .mixer
                .strips
                .get_mut(index)
                .and_then(|strip| strip.inserts.get_mut(slot))
                .and_then(|slot| slot.as_mut())
            {
                if let Some(target) = slot_state.params.get_mut(param as usize) {
                    *target = value;
                }
            }
            if let Some(channel_id) = self.channel_for_strip(index) {
                self.audio.set_fx_param(channel_id, slot as u8, param, value);
            }
            self.dirty = true;
        }
    }

    fn set_fx_bypass(&mut self, strip: FxStrip, slot: usize, bypass: bool) {
        if let FxStrip::Channel(index) = strip {
            if let Some(slot_state) = self
                .project
                .mixer
                .strips
                .get_mut(index)
                .and_then(|strip| strip.inserts.get_mut(slot))
                .and_then(|slot| slot.as_mut())
            {
                slot_state.bypass = bypass;
            }
            if let Some(channel_id) = self.channel_for_strip(index) {
                self.audio.set_fx_bypass(channel_id, slot as u8, bypass);
            }
            self.dirty = true;
        }
    }

    fn remove_fx(&mut self, strip: FxStrip, slot: usize) {
        match strip {
            FxStrip::Channel(index) => {
                if let Some(slot_state) = self
                    .project
                    .mixer
                    .strips
                    .get_mut(index)
                    .and_then(|strip| strip.inserts.get_mut(slot))
                {
                    *slot_state = None;
                }
                self.sync_schedules();
            }
            FxStrip::Master => {
                if let Some(slot_state) = self.project.mixer.master.inserts.get_mut(slot) {
                    *slot_state = None;
                }
                self.sync_schedules();
            }
            FxStrip::Bus(index) => {
                if let Some(slot_state) = self
                    .project
                    .mixer
                    .bus_inserts
                    .get_mut(index)
                    .and_then(|slots| slots.get_mut(slot))
                {
                    *slot_state = None;
                }
                self.sync_schedules();
            }
        }
        self.dirty = true;
    }
}
