//! File, project and edit operations behind the REDRAW menus and panels.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

use redraw_shared::{
    ClipKind, ClipState, FxSlotState, PluginFactory, Step, StudioProject, TrackKind, MAX_CHANNELS,
    MAX_PATTERNS, PPQ,
};

use crate::app::{
    sanitize_file_stem, DialogKind, FxStrip, LowerTab, PluginTarget, StepPaint,
};
use crate::RedrawApp;

impl RedrawApp {
    // ------------------------------------------------------------ file menu

    pub fn new_project(&mut self) {
        if self.dirty && !self.confirm_discard("Create a new project?") {
            return;
        }
        self.project = redraw_shared::StudioProject::default();
        self.current_path = None;
        self.dirty = false;
        self.history.clear();
        self.plugin_windows.clear();
        self.export.open = false;
        self.export.summary = None;
        self.export.path = self
            .prefs
            .export_dir
            .join("untitled.wav")
            .to_string_lossy()
            .into_owned();
        self.selection.channel_id = self.project.channels.first().map(|c| c.id).unwrap_or(0);
        self.selection.strip = self
            .project
            .channel(self.selection.channel_id)
            .map(|c| c.mixer_index)
            .unwrap_or(0);
        self.selection.clips.clear();
        self.selection.automation = None;
        self.project.current_pattern = 0;
        self.piano = redraw_piano_roll::PianoRoll::new(self.active_pattern_id(), self.selection.channel_id);
        self.piano.set_grid(redraw_piano_roll::GRID_1_16);
        self.piano_view = Default::default();
        self.playlist_view = Default::default();
        self.automation_view = Default::default();
        self.note_drag = None;
        self.playlist_view.drag = None;
        self.audio.set_playlist_mode(false);
        self.sync_audio();
        self.sync_transport();
        self.set_status("New project");
    }

    pub fn open_dialog(&mut self) {
        if self.dirty && !self.confirm_discard("Open another project?") {
            return;
        }
        if let Some(path) = self.pick_open_file(
            "Open project",
            &[
                ("REDRAW project", &["rdp"]),
                ("Legacy FLP project (partial)", &["flp"]),
            ],
        ) {
            self.open_path(&path);
        }
    }

    pub fn open_path(&mut self, path: &Path) {
        if !self.path_within_root(path) {
            self.set_error(format!(
                "Project files must stay inside {}",
                self.root.display()
            ));
            return;
        }
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let result: Result<(StudioProject, Option<usize>), String> = match extension.as_str() {
            "rdp" => redraw_fileio::load_project(path)
                .map(|project| (project, None))
                .map_err(|error| error.to_string()),
            "flp" => redraw_fileio::load_flp(path)
                .map(|import| {
                    let warnings = import.warnings.len();
                    (import.project, Some(warnings))
                })
                .map_err(|error| error.to_string()),
            _ => {
                self.set_error("Unsupported project extension (expected .rdp or .flp)");
                return;
            }
        };
        match result {
            Ok((mut project, warnings)) => {
                redraw_fileio::repair_project(&mut project);
                self.project = project;
                self.current_path = Some(path.to_path_buf());
                self.ensure_selection();
                self.piano = redraw_piano_roll::PianoRoll::new(
                    self.active_pattern_id(),
                    self.selection.channel_id,
                );
                self.piano.set_grid(redraw_piano_roll::GRID_1_16);
                self.history.clear();
                self.dirty = false;
                self.plugin_windows.clear();
                self.selection.clips.clear();
                self.selection.automation = None;
                self.export.song_scope = !self.project.playlist.clips.is_empty();
                self.export.pattern_id = self.active_pattern_id();
                self.audio.set_playlist_mode(false);
                self.sync_audio();
                self.sync_transport();
                self.remember_recent(path);
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                if let Some(warnings) = warnings {
                    self.set_status(format!("Opened {name} (partial FLP import, {warnings} warnings)"));
                } else {
                    self.set_status(format!("Opened {name}"));
                }
            }
            Err(error) => self.set_error(format!("Open failed: {error}")),
        }
    }

    pub fn save(&mut self) -> bool {
        let Some(path) = self.current_path.clone() else {
            return self.save_as();
        };
        if path.extension().and_then(|e| e.to_str()) != Some("rdp") {
            return self.save_as();
        }
        self.save_to(&path)
    }

    pub fn save_as(&mut self) -> bool {
        let default = format!("{}.rdp", sanitize_file_stem(&self.project.name));
        let Some(mut path) = self.pick_save_file("Save REDRAW project", &default, &[("REDRAW project", &["rdp"])]) else {
            return false;
        };
        if path.extension().is_none() {
            path.set_extension("rdp");
        }
        self.save_to(&path)
    }

    pub fn save_copy(&mut self) -> bool {
        let default = format!("{} copy.rdp", sanitize_file_stem(&self.project.name));
        let Some(mut path) = self.pick_save_file("Save project copy", &default, &[("REDRAW project", &["rdp"])]) else {
            return false;
        };
        if path.extension().is_none() {
            path.set_extension("rdp");
        }
        self.write_project(&path)
    }

    fn save_to(&mut self, path: &Path) -> bool {
        if !self.write_project(path) {
            return false;
        }
        self.current_path = Some(path.to_path_buf());
        self.dirty = false;
        self.remember_recent(path);
        let _ = self.autosave.end_session_clean();
        self.set_status(format!("Saved {}", path.display()));
        true
    }

    fn write_project(&mut self, path: &Path) -> bool {
        if !self.path_within_root(path) {
            self.set_error(format!(
                "Project files must stay inside {}",
                self.root.display()
            ));
            return false;
        }
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                self.set_error(format!("Cannot create folder: {error}"));
                return false;
            }
        }
        match redraw_fileio::save_project(path, &self.project) {
            Ok(()) => true,
            Err(error) => {
                self.set_error(format!("Save failed: {error}"));
                false
            }
        }
    }

    pub fn export_midi_dialog(&mut self) {
        let default = format!("{}.mid", sanitize_file_stem(&self.project.name));
        let Some(path) = self.pick_save_file(
            "Export MIDI",
            &default,
            &[("Standard MIDI File", &["mid"])],
        ) else {
            return;
        };
        let bytes = match redraw_piano_roll::midi::export_project_smf(&self.project) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.set_error(format!("MIDI export failed: {error}"));
                return;
            }
        };
        match redraw_piano_roll::midi::write_smf_file(&path, &bytes) {
            Ok(()) => self.set_status(format!("MIDI exported to {}", path.display())),
            Err(error) => self.set_error(format!("MIDI write failed: {error}")),
        }
    }

    pub fn import_midi_dialog(&mut self) {
        let Some(path) = self.pick_open_file(
            "Import MIDI",
            &[
                ("Standard MIDI File", &["mid", "midi"]),
                ("All files", &["*"]),
            ],
        ) else {
            return;
        };
        let data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(error) => {
                self.set_error(format!("Cannot read MIDI: {error}"));
                return;
            }
        };
        let color = crate::theme::PATTERN_COLORS[self.project.patterns.len() % 16];
        let ids = match redraw_piano_roll::midi::import_into_project(&mut self.project, &data, color) {
            Ok(ids) => ids,
            Err(error) => {
                self.set_error(format!("MIDI import failed: {error}"));
                return;
            }
        };
        if ids.is_empty() {
            self.set_status("MIDI import produced no notes");
            return;
        }
        self.begin_edit();
        let mut mapping: HashMap<u32, u32> = HashMap::new();
        let mut used: HashSet<u32> = HashSet::new();
        for id in &ids {
            if let Some(pattern) = self.project.patterns.iter().find(|p| p.id == *id) {
                for pc in &pattern.channels {
                    used.insert(pc.channel_id);
                }
            }
        }
        let mut ordered: Vec<u32> = used.into_iter().collect();
        ordered.sort_unstable();
        for old in ordered {
            let id = self.project.next_channel_id.max(1);
            let name = format!("MIDI Ch {old}");
            let color = crate::theme::PATTERN_COLORS[(id as usize) % 16];
            let new_id = self.project.add_channel("Aurora", &name, color);
            if let Some(channel) = self.project.channel_mut(new_id) {
                if let Some(synth) = self.factory.create_synth("Aurora") {
                    channel.params = synth.params().iter().map(|p| p.default).collect();
                }
            }
            mapping.insert(old, new_id);
        }
        for pattern in self.project.patterns.iter_mut().filter(|p| ids.contains(&p.id)) {
            for pc in &mut pattern.channels {
                if let Some(new_id) = mapping.get(&pc.channel_id) {
                    pc.channel_id = *new_id;
                }
            }
        }
        if let Some(first_id) = ids.first() {
            if let Some(index) = self.project.patterns.iter().position(|p| p.id == *first_id) {
                self.project.current_pattern = index;
            }
        }
        if let Some(new_id) = mapping.values().next().copied() {
            self.selection.channel_id = new_id;
            self.selection.strip = self
                .project
                .channel(new_id)
                .map(|c| c.mixer_index)
                .unwrap_or(0);
        }
        self.piano = redraw_piano_roll::PianoRoll::new(
            self.active_pattern_id(),
            self.selection.channel_id,
        );
        self.commit_edit();
        self.sync_audio();
        self.set_status(format!("Imported {} MIDI pattern(s)", ids.len()));
    }

    pub fn autosave_now(&mut self) {
        match self
            .autosave
            .autosave(&self.project, self.current_path.as_deref(), true)
        {
            Ok(Some(path)) => self.set_status(format!("Autosaved {}", path.display())),
            Ok(None) => self.set_status("Autosave skipped"),
            Err(error) => self.set_error(format!("Autosave failed: {error}")),
        }
    }

    pub fn recover_autosave(&mut self) {
        match self.autosave.recover_latest() {
            Ok(Some((entry, mut project))) => {
                redraw_fileio::repair_project(&mut project);
                self.project = project;
                self.current_path = None;
                self.dirty = true;
                self.history.clear();
                self.ensure_selection();
                self.piano = redraw_piano_roll::PianoRoll::new(
                    self.active_pattern_id(),
                    self.selection.channel_id,
                );
                self.dialog = None;
                self.sync_audio();
                self.sync_transport();
                self.set_status(format!("Recovered {}", entry.path.display()));
            }
            Ok(None) => self.set_status("No autosave snapshot available"),
            Err(error) => self.set_error(format!("Recovery failed: {error}")),
        }
    }

    // ------------------------------------------------------------ edit menu

    pub fn copy(&mut self) {
        if self.layout.lower_tab == LowerTab::PianoRoll {
            let count = if let Some(pattern) = self.project.pattern() {
                self.piano.copy(pattern)
            } else {
                0
            };
            if count > 0 {
                self.set_status(format!("Copied {count} note(s)"));
                return;
            }
        }
        let clips: Vec<ClipState> = self
            .selection
            .clips
            .iter()
            .filter_map(|id| redraw_arrangement::find_clip(&self.project, *id).cloned())
            .collect();
        if !clips.is_empty() {
            let count = clips.len();
            self.clip_clipboard = clips;
            self.set_status(format!("Copied {count} clip(s)"));
        }
    }

    pub fn cut(&mut self) {
        if self.layout.lower_tab == LowerTab::PianoRoll {
            self.begin_edit();
            let count = if let Some(pattern) = self.project.pattern_mut() {
                self.piano.cut(pattern)
            } else {
                0
            };
            if count > 0 {
                self.commit_edit();
                self.schedule_dirty = true;
                self.set_status(format!("Cut {count} note(s)"));
                return;
            }
            self.history.cancel();
        }
        if self.selection.clips.is_empty() {
            return;
        }
        self.copy();
        self.begin_edit();
        for id in &self.selection.clips {
            redraw_arrangement::delete_clip(&mut self.project, *id);
        }
        self.selection.clips.clear();
        self.commit_edit();
        self.sync_schedules();
    }

    pub fn paste(&mut self) {
        if self.layout.lower_tab == LowerTab::PianoRoll && !self.piano.clipboard.is_empty() {
            let tick = self.current_tick();
            self.begin_edit();
            let ids = if let Some(pattern) = self.project.pattern_mut() {
                self.piano.paste(pattern, tick)
            } else {
                Vec::new()
            };
            if ids.is_empty() {
                self.history.cancel();
                return;
            }
            self.commit_edit();
            self.schedule_dirty = true;
            self.set_status(format!("Pasted {} note(s)", ids.len()));
            return;
        }
        if self.clip_clipboard.is_empty() {
            return;
        }
        let base = self
            .clip_clipboard
            .iter()
            .map(|c| c.start)
            .min()
            .unwrap_or(0);
        let at = self.current_tick();
        let track = self.playlist_view.selected_track;
        let clips = self.clip_clipboard.clone();
        self.begin_edit();
        let mut ids = Vec::new();
        for mut clip in clips {
            clip.id = 0;
            clip.start = at.saturating_add(clip.start.saturating_sub(base));
            clip.track = clip.track.min(self.project.playlist.tracks.len().saturating_sub(1));
            let _ = track;
            if let Some(id) = redraw_arrangement::add_clip(&mut self.project, clip) {
                ids.push(id);
            }
        }
        if ids.is_empty() {
            self.history.cancel();
            return;
        }
        self.selection.clips = ids;
        self.commit_edit();
        self.sync_schedules();
        self.set_status(format!("Pasted {} clip(s)", self.selection.clips.len()));
    }

    pub fn delete_selection(&mut self) {
        if self.layout.lower_tab == LowerTab::PianoRoll && !self.piano.selection.is_empty() {
            self.begin_edit();
            let removed = if let Some(pattern) = self.project.pattern_mut() {
                self.piano.delete_selected(pattern)
            } else {
                0
            };
            self.commit_edit();
            self.schedule_dirty = true;
            self.set_status(format!("Deleted {removed} note(s)"));
            return;
        }
        if !self.selection.clips.is_empty() {
            self.begin_edit();
            let ids = self.selection.clips.clone();
            for id in ids {
                redraw_arrangement::delete_clip(&mut self.project, id);
            }
            self.selection.clips.clear();
            self.commit_edit();
            self.sync_schedules();
            self.set_status("Deleted selected clip(s)");
            return;
        }
        if self.layout.lower_tab == LowerTab::Automation {
            if let Some(id) = self.selection.automation {
                self.begin_edit();
                redraw_automation::remove_clip(&mut self.project, id);
                self.selection.automation = None;
                self.commit_edit();
                self.sync_schedules();
                self.set_status("Deleted automation clip");
            }
        }
    }

    pub fn select_all(&mut self) {
        if self.layout.lower_tab == LowerTab::PianoRoll {
            if let Some(pattern) = self.project.pattern() {
                self.piano.select_all(pattern);
            }
        } else {
            self.selection.clips = self.project.playlist.clips.iter().map(|c| c.id).collect();
        }
    }

    pub fn deselect(&mut self) {
        self.selection.clips.clear();
        self.selection.automation = None;
        self.piano.clear_selection();
    }

    pub fn duplicate(&mut self) {
        if self.layout.lower_tab == LowerTab::PianoRoll && !self.piano.selection.is_empty() {
            let length = self.pattern().map(|p| p.length_ticks()).unwrap_or(PPQ as u64);
            self.begin_edit();
            let ids = if let Some(pattern) = self.project.pattern_mut() {
                self.piano.duplicate(pattern, length.max(1))
            } else {
                Vec::new()
            };
            if ids.is_empty() {
                self.history.cancel();
                return;
            }
            self.commit_edit();
            self.schedule_dirty = true;
            self.set_status(format!("Duplicated {} note(s)", ids.len()));
            return;
        }
        if self.selection.clips.is_empty() {
            return;
        }
        self.begin_edit();
        let mut ids = Vec::new();
        for id in self.selection.clips.clone() {
            if let Some(new_id) = redraw_arrangement::duplicate_clip(&mut self.project, id) {
                ids.push(new_id);
            }
        }
        self.selection.clips = ids;
        self.commit_edit();
        self.sync_schedules();
    }

    pub fn nudge(&mut self, direction: i32) {
        let delta = self.snap_ticks() as i64 * direction.signum() as i64;
        if self.layout.lower_tab == LowerTab::PianoRoll && !self.piano.selection.is_empty() {
            self.begin_edit();
            if let Some(pattern) = self.project.pattern_mut() {
                self.piano.nudge_selected(pattern, delta, 0);
            }
            self.commit_edit();
            self.schedule_dirty = true;
            return;
        }
        if self.selection.clips.is_empty() {
            return;
        }
        self.begin_edit();
        for id in self.selection.clips.clone() {
            if let Some(clip) = redraw_arrangement::clip_mut(&mut self.project, id) {
                clip.start = (clip.start as i64 + delta).max(0) as u64;
            }
        }
        self.commit_edit();
        self.sync_schedules();
    }

    // ------------------------------------------------------------ project menu

    pub fn add_pattern(&mut self) {
        if self.project.patterns.len() >= MAX_PATTERNS {
            self.set_error(format!("Pattern limit ({MAX_PATTERNS}) reached"));
            return;
        }
        self.begin_edit();
        let id = self.project.next_pattern_id.max(1);
        self.project.next_pattern_id = id.saturating_add(1);
        let index = self.project.patterns.len();
        let color = crate::theme::PATTERN_COLORS[index % 16];
        let mut pattern = redraw_shared::Pattern::new(id, format!("Pattern {}", index + 1), color, 16);
        for channel in &self.project.channels {
            pattern.channel_mut(channel.id, 16);
        }
        self.project.patterns.push(pattern);
        self.project.current_pattern = index;
        self.commit_edit();
        self.sync_piano_ids();
        self.audio.set_pattern(index);
        self.dirty = true;
        self.set_status(format!("Added Pattern {}", index + 1));
    }

    pub fn clone_pattern(&mut self) {
        if self.project.patterns.len() >= MAX_PATTERNS {
            self.set_error(format!("Pattern limit ({MAX_PATTERNS}) reached"));
            return;
        }
        let Some(current) = self.project.pattern().cloned() else {
            return;
        };
        self.begin_edit();
        let id = self.project.next_pattern_id.max(1);
        self.project.next_pattern_id = id.saturating_add(1);
        let mut copy = current;
        copy.id = id;
        copy.name = format!("{} copy", copy.name);
        self.project.patterns.push(copy);
        self.project.current_pattern = self.project.patterns.len() - 1;
        self.commit_edit();
        self.sync_piano_ids();
        self.audio.set_pattern(self.project.current_pattern);
        self.dirty = true;
        self.set_status("Pattern cloned");
    }

    pub fn open_rename_pattern(&mut self) {
        self.rename_text = self
            .pattern()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Pattern".into());
        self.dialog = Some(DialogKind::RenamePattern);
    }

    pub fn add_tempo_automation(&mut self) {
        let length = redraw_arrangement::playlist_length(&self.project)
            .max(self.bar_ticks().saturating_mul(16))
            .max(1);
        let value = redraw_automation::tempo_normalized(self.project.bpm);
        self.begin_edit();
        if let Some(id) = redraw_automation::add_clip(
            &mut self.project,
            redraw_shared::AutoTarget::Tempo,
            length,
            value,
            [214, 160, 92],
        ) {
            self.selection.automation = Some(id);
            self.layout.lower_tab = LowerTab::Automation;
            self.commit_edit();
            self.sync_schedules();
            self.set_status("Tempo automation added");
        } else {
            self.history.cancel();
            self.set_error("Automation clip limit reached");
        }
    }

    pub fn add_channel_by_name(&mut self, synth_type: &str) -> u32 {
        if self.project.channels.len() >= MAX_CHANNELS {
            self.set_error(format!("Channel limit ({MAX_CHANNELS}) reached"));
            return 0;
        }
        self.begin_edit();
        let base = synth_type.to_string();
        let mut name = base.clone();
        let mut n = 2;
        while self.project.channels.iter().any(|c| c.name == name) {
            name = format!("{base} {n}");
            n += 1;
        }
        let color = crate::theme::PATTERN_COLORS[self.project.channels.len() % 16];
        let id = self.project.add_channel(synth_type, &name, color);
        if let Some(synth) = self.factory.create_synth(synth_type) {
            let defaults: Vec<f32> = synth.params().iter().map(|p| p.default).collect();
            if let Some(channel) = self.project.channel_mut(id) {
                channel.params = defaults;
            }
        }
        self.selection.channel_id = id;
        self.selection.strip = self
            .project
            .channel(id)
            .map(|c| c.mixer_index)
            .unwrap_or(0);
        self.piano.channel_id = id;
        self.commit_edit();
        self.sync_audio();
        self.set_status(format!("Added {name}"));
        id
    }

    pub fn replace_synth_by_name(&mut self, channel_id: u32, synth_type: &str) {
        self.begin_edit();
        let defaults = self
            .factory
            .create_synth(synth_type)
            .map(|synth| synth.params().iter().map(|p| p.default).collect::<Vec<f32>>())
            .unwrap_or_default();
        if let Some(channel) = self.project.channel_mut(channel_id) {
            channel.synth_type = synth_type.to_string();
            channel.params = defaults;
        }
        self.commit_edit();
        self.sync_audio();
        self.set_status(format!("Instrument replaced with {synth_type}"));
    }

    pub fn add_effect_by_name(&mut self, strip: FxStrip, effect_type: &str) {
        let params = self
            .factory
            .create_effect(effect_type)
            .map(|fx| fx.params().iter().map(|p| p.default).collect::<Vec<f32>>())
            .unwrap_or_default();
        let slot_state = FxSlotState {
            effect_type: effect_type.to_string(),
            params,
            bypass: false,
        };
        let inserted = match strip {
            FxStrip::Channel(index) => self
                .project
                .mixer
                .strips
                .get_mut(index)
                .and_then(|s| s.inserts.iter_mut().position(|slot| slot.is_none()))
                .map(|slot| {
                    self.project.mixer.strips[index].inserts[slot] = Some(slot_state);
                    slot
                }),
            FxStrip::Master => self
                .project
                .mixer
                .master
                .inserts
                .iter_mut()
                .position(|slot| slot.is_none())
                .map(|slot| {
                    self.project.mixer.master.inserts[slot] = Some(slot_state);
                    slot
                }),
            FxStrip::Bus(index) => self
                .project
                .mixer
                .bus_inserts
                .get_mut(index)
                .and_then(|slots| slots.iter_mut().position(|slot| slot.is_none()))
                .map(|slot| {
                    self.project.mixer.bus_inserts[index][slot] = Some(slot_state);
                    slot
                }),
        };
        if let Some(slot) = inserted {
            self.dirty = true;
            self.sync_schedules();
            self.set_status(format!("Added {effect_type} to slot {}", slot + 1));
        } else {
            self.set_error("All insert slots are occupied");
        }
    }

    pub fn add_effect_to_slot(&mut self, strip: FxStrip, slot: usize, effect_type: &str) {
        let params = self
            .factory
            .create_effect(effect_type)
            .map(|fx| fx.params().iter().map(|p| p.default).collect::<Vec<f32>>())
            .unwrap_or_default();
        let state = FxSlotState {
            effect_type: effect_type.to_string(),
            params,
            bypass: false,
        };
        match strip {
            FxStrip::Channel(index) => {
                if let Some(strip) = self.project.mixer.strips.get_mut(index) {
                    if let Some(target) = strip.inserts.get_mut(slot) {
                        *target = Some(state);
                    }
                }
            }
            FxStrip::Master => {
                if let Some(target) = self.project.mixer.master.inserts.get_mut(slot) {
                    *target = Some(state);
                }
            }
            FxStrip::Bus(index) => {
                if let Some(target) = self
                    .project
                    .mixer
                    .bus_inserts
                    .get_mut(index)
                    .and_then(|slots| slots.get_mut(slot))
                {
                    *target = Some(state);
                }
            }
        }
        self.dirty = true;
        self.sync_schedules();
        self.set_status(format!("Loaded {effect_type}"));
    }

    // ------------------------------------------------------------ tools

    pub fn scan_vst_plugins(&mut self) {
        let started = Instant::now();
        let plugins = redraw_vst::discover(&self.vst_roots);
        let cache = self.user_data.join("vst-cache.json");
        if let Err(error) = redraw_vst::save_cache(&cache, &plugins) {
            self.log_line(&format!("VST cache write failed: {error}"));
        }
        let count = plugins.len();
        self.vst_plugins = plugins;
        self.dialog = Some(DialogKind::ScanVst);
        self.set_status(format!(
            "Scanned {count} VST module(s) in {:.0} ms",
            started.elapsed().as_secs_f32() * 1000.0
        ));
    }

    pub fn stress_test(&mut self) {
        let started = Instant::now();
        let mut events = 0usize;
        for _ in 0..120 {
            let schedules = redraw_audio_engine::build_schedule(&self.project.patterns, self.project.swing);
            events = schedules.iter().map(|s| s.events.len()).sum();
            std::hint::black_box(schedules);
        }
        let elapsed = started.elapsed();
        let text = format!(
            "Stress: 120 schedule builds, {events} events, {:.1} ms total",
            elapsed.as_secs_f64() * 1000.0
        );
        self.log_line(&text);
        self.set_status(text);
    }

    // ------------------------------------------------------------ export

    pub fn start_export(&mut self) {
        if self.export.busy {
            return;
        }
        let path = PathBuf::from(self.export.path.trim());
        if path.as_os_str().is_empty() {
            self.set_error("Choose an output file");
            return;
        }
        if !self.path_within_root(&path) {
            self.set_error(format!(
                "Exports must stay inside {}",
                self.root.display()
            ));
            return;
        }
        let scope = if self.export.song_scope {
            if self.project.playlist.clips.is_empty() {
                self.set_error("Song export needs at least one pattern clip in the playlist");
                return;
            }
            redraw_export::ExportScope::Song
        } else {
            redraw_export::ExportScope::Pattern(self.export.pattern_id)
        };
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                self.set_error(format!("Cannot create export folder: {error}"));
                return;
            }
        }
        let render = redraw_export::RenderOptions {
            scope,
            sample_rate: self.export.sample_rate.clamp(8_000, 192_000),
            tail_seconds: self.export.tail_seconds.clamp(0.0, 30.0),
        };
        let options = redraw_export::ExportOptions {
            render,
            ffmpeg: redraw_export::FfmpegOptions {
                app_root: Some(self.root.clone()),
                allow_download: true,
                ..Default::default()
            },
        };
        let format = self.default_export_format();
        let project = self.project.clone();
        let path_for_thread = path.clone();
        let (tx, rx) = mpsc::channel();
        self.export.rx = Some(rx);
        self.export.busy = true;
        self.export.error = false;
        self.export.summary = None;
        self.export.status = format!("Rendering {} ...", path.display());
        self.export.path = path.to_string_lossy().into_owned();
        std::thread::spawn(move || {
            let factory = redraw_plugins::BuiltinFactory::new();
            let result = redraw_export::export_file(
                &path_for_thread,
                &project,
                &factory,
                &options,
                format,
            )
            .map_err(|error| error.to_string());
            let _ = tx.send(result);
        });
    }

    // ------------------------------------------------------------ parameter sync

    pub fn update_channel_gain(&mut self, channel_id: u32, db: f32) {
        let db = db.clamp(-90.0, 12.0);
        let mut strip = None;
        if let Some(channel) = self.project.channel_mut(channel_id) {
            channel.volume_db = db;
            strip = Some(channel.mixer_index);
        }
        if let Some(strip) = strip {
            if let Some(state) = self.project.mixer.strips.get_mut(strip) {
                state.volume_db = db;
            }
        }
        self.audio.set_channel_gain(channel_id, db);
        self.dirty = true;
    }

    pub fn update_channel_pan(&mut self, channel_id: u32, pan: f32) {
        let pan = pan.clamp(-1.0, 1.0);
        let mut strip = None;
        if let Some(channel) = self.project.channel_mut(channel_id) {
            channel.pan = pan;
            strip = Some(channel.mixer_index);
        }
        if let Some(strip) = strip {
            if let Some(state) = self.project.mixer.strips.get_mut(strip) {
                state.pan = pan;
            }
        }
        self.audio.set_channel_pan(channel_id, pan);
        self.dirty = true;
    }

    pub fn update_channel_mute(&mut self, channel_id: u32, mute: bool) {
        let mut strip = None;
        if let Some(channel) = self.project.channel_mut(channel_id) {
            channel.mute = mute;
            strip = Some(channel.mixer_index);
        }
        if let Some(strip) = strip {
            if let Some(state) = self.project.mixer.strips.get_mut(strip) {
                state.mute = mute;
            }
        }
        self.audio.set_channel_mute(channel_id, mute);
        self.dirty = true;
    }

    pub fn update_channel_solo(&mut self, channel_id: u32, solo: bool) {
        let mut strip = None;
        if let Some(channel) = self.project.channel_mut(channel_id) {
            channel.solo = solo;
            strip = Some(channel.mixer_index);
        }
        if let Some(strip) = strip {
            if let Some(state) = self.project.mixer.strips.get_mut(strip) {
                state.solo = solo;
            }
        }
        self.audio.set_channel_solo(channel_id, solo);
        self.dirty = true;
    }

    pub fn update_channel_name(&mut self, channel_id: u32, name: &str) {
        let mut strip = None;
        if let Some(channel) = self.project.channel_mut(channel_id) {
            channel.name = name.to_string();
            strip = Some(channel.mixer_index);
        }
        if let Some(strip) = strip {
            if let Some(state) = self.project.mixer.strips.get_mut(strip) {
                state.name = name.to_string();
            }
        }
        self.dirty = true;
    }

    pub fn update_master_gain(&mut self, db: f32) {
        let db = db.clamp(-90.0, 12.0);
        self.project.mixer.master.volume_db = db;
        self.audio.set_master_gain(db);
        self.dirty = true;
    }

    pub fn set_strip_gain(&mut self, strip: usize, db: f32) {
        let db = db.clamp(-90.0, 12.0);
        if let Some(state) = self.project.mixer.strips.get_mut(strip) {
            state.volume_db = db;
        }
        if let Some(channel) = self
            .project
            .channels
            .iter_mut()
            .find(|c| c.mixer_index == strip)
        {
            channel.volume_db = db;
            self.audio.set_channel_gain(channel.id, db);
        }
        self.dirty = true;
    }

    pub fn set_strip_pan(&mut self, strip: usize, pan: f32) {
        let pan = pan.clamp(-1.0, 1.0);
        if let Some(state) = self.project.mixer.strips.get_mut(strip) {
            state.pan = pan;
        }
        if let Some(channel) = self
            .project
            .channels
            .iter_mut()
            .find(|c| c.mixer_index == strip)
        {
            channel.pan = pan;
            self.audio.set_channel_pan(channel.id, pan);
        }
        self.dirty = true;
    }

    pub fn set_strip_mute(&mut self, strip: usize, mute: bool) {
        if let Some(state) = self.project.mixer.strips.get_mut(strip) {
            state.mute = mute;
        }
        if let Some(channel) = self
            .project
            .channels
            .iter_mut()
            .find(|c| c.mixer_index == strip)
        {
            channel.mute = mute;
            self.audio.set_channel_mute(channel.id, mute);
        }
        self.dirty = true;
    }

    pub fn set_strip_solo(&mut self, strip: usize, solo: bool) {
        if let Some(state) = self.project.mixer.strips.get_mut(strip) {
            state.solo = solo;
        }
        if let Some(channel) = self
            .project
            .channels
            .iter_mut()
            .find(|c| c.mixer_index == strip)
        {
            channel.solo = solo;
            self.audio.set_channel_solo(channel.id, solo);
        }
        self.dirty = true;
    }

    // ------------------------------------------------------------ arrangement helpers

    pub fn add_pattern_clip_at(&mut self, track: usize, tick: u64) -> Option<u64> {
        let pattern_index = self.project.current_pattern;
        let length = self
            .pattern()
            .map(|p| p.length_ticks())
            .unwrap_or(PPQ as u64 * 4)
            .max(1);
        self.begin_edit();
        let id = redraw_arrangement::add_pattern_clip(
            &mut self.project,
            track,
            pattern_index,
            tick,
            length,
        );
        if let Some(id) = id {
            self.selection.clips = vec![id];
            self.playlist_view.selected_track = track;
            self.commit_edit();
            self.sync_schedules();
        } else {
            self.history.cancel();
            self.set_error("Select a pattern track first");
        }
        id
    }

    pub fn add_audio_clip(&mut self, path: &Path, track: usize, tick: u64) -> Option<u64> {
        if !self.path_within_root(path) {
            self.set_error(format!(
                "Audio files must stay inside {}",
                self.root.display()
            ));
            return None;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Audio clip".into());
        let length = self.bar_ticks().saturating_mul(4).max(PPQ as u64);
        let clip = ClipState {
            id: 0,
            track: track.min(self.project.playlist.tracks.len().saturating_sub(1)),
            start: tick,
            length,
            kind: ClipKind::Audio,
            source_id: 0,
            name,
            color: [90, 170, 180],
            looping: false,
            gain_db: 0.0,
            transpose: 0,
            stretch: 1.0,
            audio_path: Some(path.to_string_lossy().into_owned()),
        };
        self.begin_edit();
        let id = redraw_arrangement::add_clip(&mut self.project, clip);
        if let Some(id) = id {
            self.selection.clips = vec![id];
            self.commit_edit();
            self.sync_schedules();
            self.set_status("Audio clip placed (playback is not routed yet)");
        } else {
            self.history.cancel();
        }
        id
    }

    pub fn add_playlist_track(&mut self) {
        self.begin_edit();
        let track_name = format!("Track {}", self.project.playlist.tracks.len() + 1);
        let index = redraw_arrangement::add_track(
            &mut self.project,
            TrackKind::Pattern,
            track_name,
        );
        self.playlist_view.selected_track = index;
        self.commit_edit();
        self.sync_schedules();
    }

    pub fn remove_playlist_track(&mut self) {
        let index = self.playlist_view.selected_track;
        self.begin_edit();
        if redraw_arrangement::remove_track(&mut self.project, index) {
            self.playlist_view.selected_track = index.min(self.project.playlist.tracks.len() - 1);
            self.selection.clips.retain(|id| {
                redraw_arrangement::find_clip(&self.project, *id).is_some()
            });
            self.commit_edit();
            self.sync_schedules();
        } else {
            self.history.cancel();
            self.set_error("The last playlist track cannot be removed");
        }
    }

    pub fn toggle_step(&mut self, channel_id: u32, step: usize) {
        let steps = self.pattern().map(|p| p.length_steps as usize).unwrap_or(16);
        if step >= steps {
            return;
        }
        self.begin_edit();
        if let Some(pattern) = self.project.pattern_mut() {
            let pc = pattern.channel_mut(channel_id, pattern.length_steps.max(1));
            if let Some(state) = pc.steps.get_mut(step) {
                state.active = !state.active;
            }
        }
        self.commit_edit();
        self.sync_schedules();
    }

    pub fn paint_step(&mut self, channel_id: u32, step: usize, active: bool) {
        let steps = self.pattern().map(|p| p.length_steps as usize).unwrap_or(16);
        if step >= steps {
            return;
        }
        if let Some(pattern) = self.project.pattern_mut() {
            let pc = pattern.channel_mut(channel_id, pattern.length_steps.max(1));
            if let Some(state) = pc.steps.get_mut(step) {
                if state.active != active {
                    state.active = active;
                    self.dirty = true;
                    self.schedule_dirty = true;
                }
            }
        }
    }

    pub fn set_step_velocity(&mut self, channel_id: u32, step: usize, velocity: u8) {
        let steps = self.pattern().map(|p| p.length_steps as usize).unwrap_or(16);
        if step >= steps {
            return;
        }
        if let Some(pattern) = self.project.pattern_mut() {
            let pc = pattern.channel_mut(channel_id, pattern.length_steps.max(1));
            if let Some(state) = pc.steps.get_mut(step) {
                state.velocity = velocity.clamp(1, 127);
                self.dirty = true;
                self.schedule_dirty = true;
            }
        }
    }

    pub fn clear_pattern(&mut self) {
        self.begin_edit();
        let channel_id = self.selection.channel_id;
        if let Some(pattern) = self.project.pattern_mut() {
            if let Some(pc) = pattern.channels.iter_mut().find(|c| c.channel_id == channel_id) {
                for step in &mut pc.steps {
                    step.active = false;
                }
                pc.notes.clear();
                pc.cc.clear();
            }
        }
        self.piano.clear_selection();
        self.commit_edit();
        self.sync_schedules();
        self.set_status("Pattern channel cleared");
    }

    pub fn set_pattern_length(&mut self, steps: u32) {
        let steps = steps.clamp(1, 4096);
        self.begin_edit();
        if let Some(pattern) = self.project.pattern_mut() {
            pattern.length_steps = steps;
            for pc in &mut pattern.channels {
                pc.steps.resize(steps as usize, Step::default());
            }
        }
        self.commit_edit();
        self.sync_schedules();
        self.set_status(format!("Pattern length: {steps} steps"));
    }

    pub fn set_pattern_name(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if let Some(pattern) = self.project.pattern_mut() {
            pattern.name = name.to_string();
        }
        self.dirty = true;
    }

    pub fn set_automation_length(&mut self, id: u64, length: u64) {
        if let Some(clip) = redraw_automation::find_clip_mut(&mut self.project, id) {
            redraw_automation::set_clip_length(clip, length.max(1));
        }
        self.dirty = true;
        self.sync_schedules();
    }

    // ------------------------------------------------------------ dialogs / paths

    pub fn confirm_discard(&self, title: &str) -> bool {
        let result = rfd::MessageDialog::new()
            .set_title(title)
            .set_description("The current project has unsaved changes.")
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();
        matches!(result, rfd::MessageDialogResult::Yes)
    }

    pub fn pick_open_file(
        &mut self,
        title: &str,
        filters: &[(&str, &[&str])],
    ) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title(title)
            .set_directory(&self.root);
        for (name, extensions) in filters {
            dialog = dialog.add_filter(*name, extensions);
        }
        let path = dialog.pick_file();
        self.accept_dialog_path(path)
    }

    pub fn pick_save_file(
        &mut self,
        title: &str,
        default_name: &str,
        filters: &[(&str, &[&str])],
    ) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title(title)
            .set_directory(&self.root)
            .set_file_name(default_name);
        for (name, extensions) in filters {
            dialog = dialog.add_filter(*name, extensions);
        }
        let path = dialog.save_file();
        self.accept_dialog_path(path)
    }

    pub fn pick_folder(&mut self, title: &str) -> Option<PathBuf> {
        let path = rfd::FileDialog::new()
            .set_title(title)
            .set_directory(&self.root)
            .pick_folder();
        self.accept_dialog_path(path)
    }

    fn accept_dialog_path(&mut self, path: Option<PathBuf>) -> Option<PathBuf> {
        let path = path?;
        if !self.path_within_root(&path) {
            self.set_error(format!(
                "Files must stay inside {}",
                self.root.display()
            ));
            return None;
        }
        Some(path)
    }

    pub fn path_within_root(&self, path: &Path) -> bool {
        if path.starts_with(&self.root) {
            return true;
        }
        // Accept the canonical form when a dialog returns a normalized path.
        if let Ok(root) = self.root.canonicalize() {
            if let Ok(path) = path.canonicalize() {
                return path.starts_with(root);
            }
            if let Some(parent) = path.parent() {
                if let Ok(parent) = parent.canonicalize() {
                    if parent.starts_with(&root) {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn close_window(&mut self, ctx: &eframe::egui::Context) {
        self.allow_close = true;
        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
    }

    pub fn open_plugin_for_channel(&mut self, channel_id: u32) {
        let target = PluginTarget::Synth(channel_id);
        if !self.plugin_windows.contains(&target) {
            self.plugin_windows.push(target);
        }
        self.layout.show_plugin_windows = true;
    }

    pub fn open_plugin_for_fx(&mut self, strip: FxStrip, slot: usize) {
        let target = PluginTarget::Fx { strip, slot };
        if !self.plugin_windows.contains(&target) {
            self.plugin_windows.push(target);
        }
        self.layout.show_plugin_windows = true;
    }

    pub fn open_add_effect(&mut self, strip: FxStrip) {
        self.pending_fx = Some((strip, 0));
        self.dialog = Some(DialogKind::AddEffect);
    }

    pub fn channel_for_strip(&self, strip: usize) -> Option<u32> {
        self.project
            .channels
            .iter()
            .find(|c| c.mixer_index == strip)
            .map(|c| c.id)
    }

    pub fn plugin_target_title(&self, target: &PluginTarget) -> String {
        match target {
            PluginTarget::Synth(channel_id) => self
                .project
                .channel(*channel_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "Instrument".into()),
            PluginTarget::Fx { strip, slot } => {
                let effect = match strip {
                    FxStrip::Channel(index) => self
                        .project
                        .mixer
                        .strips
                        .get(*index)
                        .and_then(|s| s.inserts.get(*slot))
                        .and_then(|s| s.as_ref())
                        .map(|s| s.effect_type.clone()),
                    FxStrip::Master => self
                        .project
                        .mixer
                        .master
                        .inserts
                        .get(*slot)
                        .and_then(|s| s.as_ref())
                        .map(|s| s.effect_type.clone()),
                    FxStrip::Bus(index) => self
                        .project
                        .mixer
                        .bus_inserts
                        .get(*index)
                        .and_then(|slots| slots.get(*slot))
                        .and_then(|s| s.as_ref())
                        .map(|s| s.effect_type.clone()),
                };
                effect.unwrap_or_else(|| format!("{} slot {}", strip.label(), slot + 1))
            }
        }
    }

    pub fn active_step_paint(&self) -> Option<&StepPaint> {
        self.step_paint.as_ref()
    }
}

