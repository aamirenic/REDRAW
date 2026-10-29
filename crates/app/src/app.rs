//! Application state, shell layout, menus and transport.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Frame, Layout, Margin, Pos2, Rect, RichText, Stroke,
};
use redraw_fileio::autosave::AutosaveManager;
use redraw_fileio::RecoveryOffer;
use redraw_piano_roll::live::{LiveMidi, MidiPortInfo, MidiRecorder};
use redraw_piano_roll::{PianoRoll, Tool};
use redraw_plugins::BuiltinFactory;
use redraw_shared::{PluginFactory, StudioProject, PPQ};

use crate::audio::AudioHost;
use crate::theme::{Palette, ThemeKind};
use crate::widgets::{icon_button, Icon};

// ------------------------------------------------------------------ state

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LowerTab {
    Rack,
    PianoRoll,
    Mixer,
    Automation,
}

impl LowerTab {
    pub const ALL: [Self; 4] = [
        Self::Rack,
        Self::PianoRoll,
        Self::Mixer,
        Self::Automation,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Rack => "Channel Rack",
            Self::PianoRoll => "Piano Roll",
            Self::Mixer => "Mixer",
            Self::Automation => "Automation",
        }
    }

    pub const fn icon(self) -> Icon {
        match self {
            Self::Rack => Icon::Rack,
            Self::PianoRoll => Icon::Piano,
            Self::Mixer => Icon::Mixer,
            Self::Automation => Icon::Auto,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FxStrip {
    Channel(usize),
    Master,
    Bus(usize),
}

impl FxStrip {
    pub fn label(self) -> String {
        match self {
            Self::Channel(i) => format!("Channel {}", i + 1),
            Self::Master => "Master".into(),
            Self::Bus(i) => format!("Bus {}", (b'A' + i as u8) as char),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginTarget {
    Synth(u32),
    Fx { strip: FxStrip, slot: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Preferences,
    MidiSettings,
    AudioSettings,
    ProjectInfo,
    Statistics,
    Shortcuts,
    About,
    Diagnostics,
    PatternLength,
    RenamePattern,
    AddMarker,
    Quantize,
    Humanize,
    Transpose,
    AddChannel,
    ReplaceSynth,
    AddEffect,
    ScanVst,
    CloseConfirm,
    Recovery,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub until: Instant,
    pub error: bool,
}

#[derive(Clone, Debug)]
pub struct Preferences {
    pub theme: ThemeKind,
    pub custom: Palette,
    pub scale: f32,
    pub tooltips: bool,
    pub autosave: bool,
    pub export_dir: PathBuf,
    pub recent_limit: usize,
}

impl Preferences {
    pub fn default_for(root: &Path) -> Self {
        Self {
            theme: ThemeKind::Dark,
            custom: Palette::dark(),
            scale: 1.0,
            tooltips: true,
            autosave: true,
            export_dir: root.join("exports"),
            recent_limit: 10,
        }
    }
}

pub struct LayoutState {
    pub show_browser: bool,
    pub show_inspector: bool,
    pub show_playlist: bool,
    pub show_plugin_windows: bool,
    pub lower_tab: LowerTab,
    pub lower_height: f32,
    pub browser_width: f32,
    pub inspector_width: f32,
    pub playlist_zoom: f32,
    pub fullscreen: bool,
}

impl Default for LayoutState {
    fn default() -> Self {
        Self {
            show_browser: true,
            show_inspector: true,
            show_playlist: true,
            show_plugin_windows: true,
            lower_tab: LowerTab::Rack,
            lower_height: 330.0,
            browser_width: 218.0,
            inspector_width: 286.0,
            playlist_zoom: 1.0,
            fullscreen: false,
        }
    }
}

#[derive(Default)]
pub struct SelectionState {
    pub channel_id: u32,
    pub strip: usize,
    pub clips: Vec<u64>,
    pub automation: Option<u64>,
}

pub struct History {
    undo: VecDeque<StudioProject>,
    redo: Vec<StudioProject>,
    pending: Option<StudioProject>,
    limit: usize,
}

impl Default for History {
    fn default() -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            pending: None,
            limit: 64,
        }
    }
}

impl History {
    pub fn begin(&mut self, project: &StudioProject) {
        if self.pending.is_none() {
            self.pending = Some(project.clone());
        }
    }

    pub fn commit(&mut self) {
        if let Some(before) = self.pending.take() {
            self.undo.push_back(before);
            while self.undo.len() > self.limit {
                self.undo.pop_front();
            }
            self.redo.clear();
        }
    }

    pub fn cancel(&mut self) {
        self.pending = None;
    }

    pub fn undo(&mut self, current: &StudioProject) -> Option<StudioProject> {
        self.pending = None;
        let previous = self.undo.pop_back()?;
        self.redo.push(current.clone());
        Some(previous)
    }

    pub fn redo(&mut self, current: &StudioProject) -> Option<StudioProject> {
        self.pending = None;
        let next = self.redo.pop()?;
        self.undo.push_back(current.clone());
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.pending.is_some()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending = None;
    }
}

pub struct NoteDrag {
    pub ids: Vec<u64>,
    pub mode: NoteDragMode,
    pub start_pointer_tick: u64,
    pub start_pointer_key: u8,
    pub originals: Vec<redraw_shared::Note>,
    pub changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteDragMode {
    Move,
    Resize,
    Velocity,
    Pan,
}

pub struct ClipDrag {
    pub id: u64,
    pub mode: ClipDragMode,
    pub start_pointer_tick: u64,
    pub original_start: u64,
    pub original_track: usize,
    pub original_length: u64,
    pub changed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipDragMode {
    Move,
    Resize,
}

pub struct AutoDrag {
    pub clip_id: u64,
    pub point: usize,
    pub changed: bool,
}

#[derive(Clone, Copy)]
pub struct StepPaint {
    pub channel_id: u32,
    pub step: u32,
    pub active: bool,
}

pub struct PianoView {
    pub px_per_tick: f32,
    pub row_height: f32,
    pub lane_height: f32,
    pub active_cc: u8,
    pub last_length: u64,
}

impl Default for PianoView {
    fn default() -> Self {
        Self {
            px_per_tick: 0.10,
            row_height: 13.0,
            lane_height: 46.0,
            active_cc: 1,
            last_length: redraw_shared::PPQ as u64 / 4,
        }
    }
}

pub struct PlaylistView {
    pub px_per_tick: f32,
    pub snap: u64,
    pub drag: Option<ClipDrag>,
    pub selected_track: usize,
}

impl Default for PlaylistView {
    fn default() -> Self {
        Self {
            px_per_tick: 0.035,
            snap: redraw_shared::PPQ as u64 / 4,
            drag: None,
            selected_track: 0,
        }
    }
}

pub struct AutomationView {
    pub px_per_tick: f32,
    pub selected_point: Option<usize>,
    pub drag: Option<AutoDrag>,
}

impl Default for AutomationView {
    fn default() -> Self {
        Self {
            px_per_tick: 0.12,
            selected_point: None,
            drag: None,
        }
    }
}

#[derive(Default)]
pub struct BrowserState {
    pub search: String,
    pub category: usize,
    pub expanded: [bool; 6],
}

pub struct ExportState {
    pub open: bool,
    pub song_scope: bool,
    pub pattern_id: u32,
    pub format_index: usize,
    pub sample_rate: u32,
    pub tail_seconds: f32,
    pub path: String,
    pub status: String,
    pub busy: bool,
    pub rx: Option<Receiver<Result<redraw_export::ExportSummary, String>>>,
    pub summary: Option<redraw_export::ExportSummary>,
    pub error: bool,
}

impl ExportState {
    pub fn new(default_dir: &Path) -> Self {
        Self {
            open: false,
            song_scope: true,
            pattern_id: 1,
            format_index: 0,
            sample_rate: 48_000,
            tail_seconds: 2.0,
            path: default_dir.join("untitled.wav").to_string_lossy().into_owned(),
            status: String::new(),
            busy: false,
            rx: None,
            summary: None,
            error: false,
        }
    }
}

// ------------------------------------------------------------------ app

pub struct RedrawApp {
    pub project: StudioProject,
    pub factory: BuiltinFactory,
    pub audio: AudioHost,
    pub palette: Palette,
    pub prefs: Preferences,
    pub history: History,
    pub dirty: bool,
    pub current_path: Option<PathBuf>,
    pub recent: Vec<PathBuf>,
    pub autosave: AutosaveManager,
    pub recovery: Option<RecoveryOffer>,
    pub status: String,
    pub toast: Option<Toast>,
    pub layout: LayoutState,
    pub selection: SelectionState,
    pub piano: PianoRoll,
    pub piano_view: PianoView,
    pub playlist_view: PlaylistView,
    pub automation_view: AutomationView,
    pub browser: BrowserState,
    pub dialog: Option<DialogKind>,
    pub export: ExportState,
    pub plugin_windows: Vec<PluginTarget>,
    pub midi: LiveMidi,
    pub midi_recorder: MidiRecorder,
    pub midi_ports: Vec<MidiPortInfo>,
    pub midi_port: Option<usize>,
    pub preview_keys: Vec<(u32, u8)>,
    pub quantize: redraw_piano_roll::QuantizeOptions,
    pub humanize: redraw_piano_roll::HumanizeOptions,
    pub transpose_semitones: i32,
    pub pattern_length_steps: u32,
    pub rename_text: String,
    pub marker_text: String,
    pub piano_preview: Option<u8>,
    pub automation_target_index: usize,
    pub pending_fx: Option<(FxStrip, usize)>,
    pub pending_channel: Option<u32>,
    pub pending_pattern: Option<usize>,
    pub clip_clipboard: Vec<redraw_shared::ClipState>,
    pub note_drag: Option<NoteDrag>,
    pub step_paint: Option<StepPaint>,
    pub schedule_dirty: bool,
    pub record_armed: bool,
    pub root: PathBuf,
    pub user_data: PathBuf,
    pub log: Vec<String>,
    pub vst_plugins: Vec<redraw_vst::PluginEntry>,
    pub vst_roots: Vec<PathBuf>,
    pub frame: u64,
    pub close_confirm: bool,
    pub allow_close: bool,
    pub last_title: String,
    pub synth_specs: std::collections::HashMap<String, &'static [redraw_shared::ParamSpec]>,
    pub effect_specs: std::collections::HashMap<String, &'static [redraw_shared::ParamSpec]>,
}

impl RedrawApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let root = workspace_root();
        let user_data = root.join("user-data");
        let _ = std::fs::create_dir_all(&user_data);
        let _ = std::fs::create_dir_all(root.join("exports"));
        let prefs = load_preferences(&root);
        let recent = load_recent(&user_data);
        let autosave = AutosaveManager::new(&user_data);
        let recovery = autosave.startup().ok().filter(RecoveryOffer::has_recovery);

        let project = StudioProject::default();
        let factory = BuiltinFactory::new();
        let audio = AudioHost::new(&project, &factory, false);
        let palette = if prefs.theme == ThemeKind::Custom {
            prefs.custom
        } else {
            Palette::for_kind(prefs.theme)
        };
        palette.apply(&cc.egui_ctx, prefs.theme, prefs.scale, prefs.tooltips);

        let selected_channel = project.channels.first().map(|c| c.id).unwrap_or(1);
        let selected_strip = project
            .channel(selected_channel)
            .map(|c| c.mixer_index)
            .unwrap_or(0);
        let pattern_id = project.pattern().map(|p| p.id).unwrap_or(1);
        let mut piano = PianoRoll::new(pattern_id, selected_channel);
        piano.set_grid(redraw_piano_roll::GRID_1_16);

        let export = ExportState::new(&prefs.export_dir);
        let mut app = Self {
            project,
            factory,
            audio,
            palette,
            prefs,
            history: History::default(),
            dirty: false,
            current_path: None,
            recent,
            autosave,
            recovery,
            status: "Ready".into(),
            toast: None,
            layout: LayoutState::default(),
            selection: SelectionState {
                channel_id: selected_channel,
                strip: selected_strip,
                clips: Vec::new(),
                automation: None,
            },
            piano,
            piano_view: PianoView::default(),
            playlist_view: PlaylistView::default(),
            automation_view: AutomationView::default(),
            browser: BrowserState {
                expanded: [true, false, false, false, false, false],
                ..Default::default()
            },
            dialog: None,
            export,
            plugin_windows: Vec::new(),
            midi: LiveMidi::new(),
            midi_recorder: MidiRecorder::new(),
            midi_ports: Vec::new(),
            midi_port: None,
            preview_keys: Vec::new(),
            quantize: redraw_piano_roll::QuantizeOptions::default(),
            humanize: redraw_piano_roll::HumanizeOptions::default(),
            transpose_semitones: 12,
            pattern_length_steps: 16,
            rename_text: String::new(),
            marker_text: "Marker".into(),
            piano_preview: None,
            automation_target_index: 0,
            pending_fx: None,
            pending_channel: None,
            pending_pattern: None,
            clip_clipboard: Vec::new(),
            note_drag: None,
            step_paint: None,
            schedule_dirty: false,
            record_armed: false,
            root,
            user_data,
            log: Vec::new(),
            vst_plugins: Vec::new(),
            vst_roots: default_vst_roots(),
            frame: 0,
            close_confirm: false,
            allow_close: false,
            last_title: String::new(),
            synth_specs: std::collections::HashMap::new(),
            effect_specs: std::collections::HashMap::new(),
        };
        if app.recovery.is_some() {
            app.dialog = Some(DialogKind::Recovery);
        }
        app.log_line("REDRAW started");
        if let Some(error) = app.audio.error.clone() {
            app.log_line(&format!("audio unavailable: {error}"));
        }
        app.status = if app.audio.is_running() {
            format!("Audio: {} @ {} Hz", app.audio.device_name(), app.audio.sample_rate())
        } else {
            "Audio unavailable - open Audio Settings to retry".into()
        };
        app
    }

    // -------------------------------------------------------------- helpers

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    pub fn pattern(&self) -> Option<&redraw_shared::Pattern> {
        self.project.patterns.get(self.project.current_pattern)
    }

    pub fn pattern_mut(&mut self) -> Option<&mut redraw_shared::Pattern> {
        self.project.patterns.get_mut(self.project.current_pattern)
    }

    pub fn selected_channel(&self) -> Option<&redraw_shared::ChannelInfo> {
        self.project.channel(self.selection.channel_id)
    }

    pub fn selected_channel_mut(&mut self) -> Option<&mut redraw_shared::ChannelInfo> {
        self.project.channel_mut(self.selection.channel_id)
    }

    pub fn selected_strip(&self) -> Option<&redraw_shared::MixerStripState> {
        self.project.mixer.strips.get(self.selection.strip)
    }

    pub fn selected_strip_mut(&mut self) -> Option<&mut redraw_shared::MixerStripState> {
        self.project.mixer.strips.get_mut(self.selection.strip)
    }

    pub fn selected_clip(&self) -> Option<&redraw_shared::ClipState> {
        let id = *self.selection.clips.first()?;
        redraw_arrangement::find_clip(&self.project, id)
    }

    pub fn selected_automation(&self) -> Option<&redraw_shared::AutomationClipState> {
        let id = self.selection.automation?;
        redraw_automation::find_clip(&self.project, id)
    }

    pub fn ensure_selection(&mut self) {
        if self.project.channels.is_empty() {
            self.selection.channel_id = 0;
            self.selection.strip = 0;
        } else if self.project.channel(self.selection.channel_id).is_none() {
            self.selection.channel_id = self.project.channels[0].id;
            self.selection.strip = self.project.channels[0].mixer_index;
        }
        if self.project.current_pattern >= self.project.patterns.len() {
            self.project.current_pattern = 0;
        }
        self.selection.clips.retain(|id| {
            self.project.playlist.clips.iter().any(|c| c.id == *id)
        });
        if let Some(id) = self.selection.automation {
            if redraw_automation::find_clip(&self.project, id).is_none() {
                self.selection.automation = None;
            }
        }
        if let Some(pattern) = self.project.pattern() {
            if self.piano.pattern_id != pattern.id {
                self.piano.pattern_id = pattern.id;
                self.piano.clear_selection();
            }
        }
        if self.piano.channel_id != self.selection.channel_id {
            self.piano.channel_id = self.selection.channel_id;
            self.piano.clear_selection();
        }
        self.selection.strip = self.selection.strip.min(redraw_shared::MAX_CHANNELS - 1);
    }

    pub fn begin_edit(&mut self) {
        self.history.begin(&self.project);
    }

    pub fn commit_edit(&mut self) {
        self.history.commit();
        self.dirty = true;
    }

    pub fn edit<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.begin_edit();
        let out = f(self);
        self.commit_edit();
        out
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.history.undo(&self.project) {
            self.project = previous;
            self.ensure_selection();
            self.sync_audio();
            self.dirty = true;
            self.set_status("Undo");
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.history.redo(&self.project) {
            self.project = next;
            self.ensure_selection();
            self.sync_audio();
            self.dirty = true;
            self.set_status("Redo");
        }
    }

    pub fn sync_audio(&mut self) {
        self.ensure_selection();
        self.audio.set_project(&self.project, &self.factory);
        self.audio.set_playlist_mode(self.audio.playlist_mode);
    }

    pub fn sync_transport(&mut self) {
        self.audio.set_bpm(self.project.bpm);
        self.audio.set_swing(self.project.swing);
        self.audio.set_loop(
            self.project.playlist.loop_enabled,
            self.project.playlist.loop_start,
            self.project.playlist.loop_end,
        );
        self.audio.set_pattern(self.project.current_pattern);
    }

    pub fn sync_schedules(&mut self) {
        self.audio.set_project(&self.project, &self.factory);
        self.schedule_dirty = false;
    }

    pub fn set_status(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.status = text.clone();
        self.log_line(&text);
        self.toast = Some(Toast {
            text,
            until: Instant::now() + Duration::from_secs(4),
            error: false,
        });
    }

    pub fn set_error(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.status = text.clone();
        self.log_line(&format!("error: {text}"));
        self.toast = Some(Toast {
            text,
            until: Instant::now() + Duration::from_secs(7),
            error: true,
        });
    }

    pub fn log_line(&mut self, text: &str) {
        let line = format!("[{}] {text}", timestamp());
        self.log.push(line.clone());
        if self.log.len() > 400 {
            self.log.remove(0);
        }
        let path = self.user_data.join("redraw.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            use std::io::Write;
            let _ = writeln!(file, "{line}");
        }
    }

    pub fn set_lower_tab(&mut self, tab: LowerTab) {
        self.layout.lower_tab = tab;
        if tab == LowerTab::PianoRoll {
            self.sync_piano_ids();
        }
    }

    pub fn sync_piano_ids(&mut self) {
        if let Some(pattern) = self.project.pattern() {
            self.piano.pattern_id = pattern.id;
        }
        self.piano.channel_id = self.selection.channel_id;
    }

    pub fn set_pattern_index(&mut self, index: usize) {
        if index >= self.project.patterns.len() {
            return;
        }
        self.project.current_pattern = index;
        self.audio.set_pattern(index);
        self.sync_piano_ids();
        self.piano.clear_selection();
        self.dirty = true;
        if let Some(pattern) = self.project.pattern() {
            self.set_status(format!("Pattern: {}", pattern.name));
        }
    }

    pub fn set_channel(&mut self, channel_id: u32) {
        if let Some(channel) = self.project.channel(channel_id) {
            self.selection.channel_id = channel_id;
            self.selection.strip = channel.mixer_index;
            self.piano.channel_id = channel_id;
            self.piano.clear_selection();
            self.set_status(format!("Channel: {}", channel.name));
        }
    }

    pub fn active_pattern_id(&self) -> u32 {
        self.project.pattern().map(|p| p.id).unwrap_or(1)
    }

    pub fn bar_ticks(&self) -> u64 {
        let den = self.project.time_sig.1.max(1) as u64;
        let beat = (PPQ as u64 * 4) / den;
        beat * self.project.time_sig.0.max(1) as u64
    }

    pub fn beat_ticks(&self) -> u64 {
        let den = self.project.time_sig.1.max(1) as u64;
        (PPQ as u64 * 4) / den
    }

    pub fn snap_ticks(&self) -> u64 {
        self.playlist_view.snap.max(1)
    }

    pub fn current_tick(&self) -> u64 {
        self.audio.position().max(0.0).round() as u64
    }

    pub fn tick_text(&self, tick: u64) -> String {
        let bar = self.bar_ticks().max(1);
        let beat = self.beat_ticks().max(1);
        let b = tick / bar;
        let rem = tick % bar;
        let be = rem / beat;
        let step = (rem % beat) / (beat / 4).max(1);
        format!("{}:{}:{}", b + 1, be + 1, step + 1)
    }

    pub fn apply_theme(&mut self, ctx: &egui::Context) {
        self.palette = if self.prefs.theme == ThemeKind::Custom {
            self.prefs.custom
        } else {
            Palette::for_kind(self.prefs.theme)
        };
        self.palette
            .apply(ctx, self.prefs.theme, self.prefs.scale, self.prefs.tooltips);
    }

    pub fn default_export_format(&self) -> redraw_export::ExportFormat {
        use redraw_export::{ExportFormat, WavBitDepth};
        match self.export.format_index {
            0 => ExportFormat::Wav(WavBitDepth::Pcm24),
            1 => ExportFormat::Wav(WavBitDepth::Pcm16),
            2 => ExportFormat::Wav(WavBitDepth::Pcm32),
            3 => ExportFormat::Wav(WavBitDepth::Float32),
            4 => ExportFormat::Aiff(redraw_export::AiffBitDepth::Pcm16),
            5 => ExportFormat::Aiff(redraw_export::AiffBitDepth::Pcm24),
            6 => ExportFormat::Aiff(redraw_export::AiffBitDepth::Pcm32),
            7 => ExportFormat::Mp3,
            8 => ExportFormat::Ogg,
            9 => ExportFormat::Flac,
            10 => ExportFormat::Mp4,
            11 => ExportFormat::Mov,
            12 => ExportFormat::Avi,
            _ => ExportFormat::Wav(WavBitDepth::Pcm24),
        }
    }

    pub fn format_name(index: usize) -> &'static str {
        const NAMES: [&str; 13] = [
            "WAV 24-bit PCM",
            "WAV 16-bit PCM",
            "WAV 32-bit PCM",
            "WAV 32-bit float",
            "AIFF 16-bit PCM",
            "AIFF 24-bit PCM",
            "AIFF 32-bit PCM",
            "MP3 (ffmpeg)",
            "OGG Vorbis (ffmpeg)",
            "FLAC (ffmpeg)",
            "MP4 video (ffmpeg)",
            "MOV video (ffmpeg)",
            "AVI video (ffmpeg)",
        ];
        NAMES.get(index).copied().unwrap_or("WAV 24-bit PCM")
    }

    pub fn specs_for_synth(&mut self, type_name: &str) -> &'static [redraw_shared::ParamSpec] {
        if let Some(specs) = self.synth_specs.get(type_name) {
            return specs;
        }
        let specs = self
            .factory
            .create_synth(type_name)
            .map(|synth| synth.params())
            .unwrap_or(&[]);
        self.synth_specs.insert(type_name.to_string(), specs);
        specs
    }

    pub fn specs_for_effect(&mut self, type_name: &str) -> &'static [redraw_shared::ParamSpec] {
        if let Some(specs) = self.effect_specs.get(type_name) {
            return specs;
        }
        let specs = self
            .factory
            .create_effect(type_name)
            .map(|effect| effect.params())
            .unwrap_or(&[]);
        self.effect_specs.insert(type_name.to_string(), specs);
        specs
    }

    pub fn preview_note_on(&mut self, key: u8) {
        let channel = self.selection.channel_id;
        if channel == 0 {
            return;
        }
        self.audio.note_on(channel, key, 0.8);
        if !self.preview_keys.iter().any(|(c, k)| *c == channel && *k == key) {
            self.preview_keys.push((channel, key));
        }
    }

    pub fn preview_note_off(&mut self, key: u8) {
        let channel = self.selection.channel_id;
        self.audio.note_off(channel, key);
        self.preview_keys.retain(|(c, k)| !(*c == channel && *k == key));
    }

    pub fn all_notes_off(&mut self) {
        for (channel, key) in self.preview_keys.drain(..) {
            self.audio.note_off(channel, key);
        }
        self.audio.all_notes_off();
    }

    pub fn refresh_midi_ports(&mut self) {
        match self.midi.ports() {
            Ok(ports) => {
                self.midi_ports = ports;
                if self.midi_port.is_none() && !self.midi_ports.is_empty() {
                    self.midi_port = Some(0);
                }
            }
            Err(error) => {
                self.midi_ports.clear();
                self.set_error(format!("MIDI: {error}"));
            }
        }
    }

    pub fn connect_midi(&mut self, index: usize) {
        match self.midi.connect(index) {
            Ok(name) => {
                self.midi_port = Some(index);
                self.set_status(format!("MIDI input: {name}"));
            }
            Err(error) => self.set_error(format!("MIDI connection failed: {error}")),
        }
    }

    // -------------------------------------------------------------- shell

    fn menu_bar(&mut self, ctx: &egui::Context) {
        let palette = self.palette;
        egui::TopBottomPanel::top("menu_bar")
            .exact_height(25.0)
            .frame(
                Frame::NONE
                    .fill(palette.panel)
                    .inner_margin(Margin::symmetric(4, 2)),
            )
            .show(ctx, |ui| {
                egui::menu::bar(ui, |ui| {
                    ui.menu_button("File", |ui| {
                        self.menu_item(ui, "New", "Ctrl+N", |app| app.new_project());
                        self.menu_item(ui, "Open...", "Ctrl+O", |app| app.open_dialog());
                        ui.menu_button("Open Recent", |ui| {
                            if app_recent_is_empty(self) {
                                ui.add_enabled(false, egui::Button::new("No recent projects"));
                            } else {
                                let recent = self.recent.clone();
                                for path in recent {
                                    let label = path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| path.display().to_string());
                                    if ui.button(label).clicked() {
                                        self.open_path(&path);
                                        ui.close_menu();
                                    }
                                }
                            }
                        });
                        ui.separator();
                        self.menu_item(ui, "Save", "Ctrl+S", |app| { app.save(); });
                        self.menu_item(ui, "Save As...", "Ctrl+Shift+S", |app| { app.save_as(); });
                        self.menu_item(ui, "Save Copy...", "", |app| { app.save_copy(); });
                        ui.separator();
                        self.menu_item(ui, "Export Audio/Video...", "Ctrl+E", |app| {
                            app.open_export_dialog()
                        });
                        self.menu_item(ui, "Export MIDI...", "", |app| app.export_midi_dialog());
                        self.menu_item(ui, "Import MIDI...", "", |app| app.import_midi_dialog());
                        ui.separator();
                        self.menu_item(ui, "Autosave Now", "", |app| app.autosave_now());
                        self.menu_item(ui, "Recover Autosave", "", |app| app.recover_autosave());
                        self.menu_item(ui, "Preferences...", "F10", |app| {
                            app.dialog = Some(DialogKind::Preferences)
                        });
                        ui.separator();
                        self.menu_item(ui, "Quit", "Alt+F4", |app| app.request_close(ctx));
                    });
                    ui.menu_button("Edit", |ui| {
                        ui.add_enabled_ui(self.history.can_undo(), |ui| {
                            self.menu_item(ui, "Undo", "Ctrl+Z", |app| app.undo());
                        });
                        ui.add_enabled_ui(self.history.can_redo(), |ui| {
                            self.menu_item(ui, "Redo", "Ctrl+Y", |app| app.redo());
                        });
                        ui.separator();
                        self.menu_item(ui, "Cut", "Ctrl+X", |app| app.cut());
                        self.menu_item(ui, "Copy", "Ctrl+C", |app| app.copy());
                        self.menu_item(ui, "Paste", "Ctrl+V", |app| app.paste());
                        self.menu_item(ui, "Delete", "Del", |app| app.delete_selection());
                        self.menu_item(ui, "Select All", "Ctrl+A", |app| app.select_all());
                        self.menu_item(ui, "Deselect", "Esc", |app| app.deselect());
                        self.menu_item(ui, "Duplicate", "Ctrl+D", |app| app.duplicate());
                        ui.separator();
                        self.menu_item(ui, "Quantize...", "Q", |app| {
                            app.dialog = Some(DialogKind::Quantize)
                        });
                        self.menu_item(ui, "Humanize...", "H", |app| {
                            app.dialog = Some(DialogKind::Humanize)
                        });
                        self.menu_item(ui, "Transpose...", "", |app| {
                            app.dialog = Some(DialogKind::Transpose)
                        });
                        self.menu_item(ui, "Nudge Left", "", |app| app.nudge(-1));
                        self.menu_item(ui, "Nudge Right", "", |app| app.nudge(1));
                    });
                    ui.menu_button("Project", |ui| {
                        self.menu_item(ui, "Add Channel...", "", |app| {
                            app.dialog = Some(DialogKind::AddChannel)
                        });
                        self.menu_item(ui, "Add Pattern", "", |app| app.add_pattern());
                        self.menu_item(ui, "Clone Pattern", "", |app| app.clone_pattern());
                        self.menu_item(ui, "Rename Pattern...", "", |app| app.open_rename_pattern());
                        self.menu_item(ui, "Pattern Length...", "", |app| {
                            app.pattern_length_steps = app.pattern().map(|p| p.length_steps).unwrap_or(16);
                            app.dialog = Some(DialogKind::PatternLength)
                        });
                        self.menu_item(ui, "Add Marker...", "", |app| {
                            app.marker_text = format!("Marker {}", app.project.playlist.markers.len() + 1);
                            app.dialog = Some(DialogKind::AddMarker)
                        });
                        self.menu_item(ui, "Tempo Automation", "", |app| app.add_tempo_automation());
                        self.menu_item(ui, "Project Info...", "", |app| {
                            app.dialog = Some(DialogKind::ProjectInfo)
                        });
                        self.menu_item(ui, "Statistics...", "", |app| {
                            app.dialog = Some(DialogKind::Statistics)
                        });
                    });
                    ui.menu_button("View", |ui| {
                        self.menu_check(ui, "Browser", "F8", self.layout.show_browser, |app| {
                            app.layout.show_browser = !app.layout.show_browser
                        });
                        self.menu_check(ui, "Playlist", "F5", self.layout.show_playlist, |app| {
                            app.layout.show_playlist = !app.layout.show_playlist
                        });
                        self.menu_check(
                            ui,
                            "Channel Rack",
                            "F6",
                            self.layout.lower_tab == LowerTab::Rack,
                            |app| app.set_lower_tab(LowerTab::Rack),
                        );
                        self.menu_check(
                            ui,
                            "Piano Roll",
                            "F7",
                            self.layout.lower_tab == LowerTab::PianoRoll,
                            |app| app.set_lower_tab(LowerTab::PianoRoll),
                        );
                        self.menu_check(
                            ui,
                            "Mixer",
                            "F9",
                            self.layout.lower_tab == LowerTab::Mixer,
                            |app| app.set_lower_tab(LowerTab::Mixer),
                        );
                        self.menu_check(ui, "Inspector", "", self.layout.show_inspector, |app| {
                            app.layout.show_inspector = !app.layout.show_inspector
                        });
                        self.menu_check(
                            ui,
                            "Plugin Windows",
                            "",
                            self.layout.show_plugin_windows,
                            |app| {
                                app.layout.show_plugin_windows = !app.layout.show_plugin_windows
                            },
                        );
                        ui.separator();
                        ui.menu_button("Theme", |ui| {
                            for kind in ThemeKind::ALL {
                                if ui
                                    .selectable_label(self.prefs.theme == kind, kind.label())
                                    .clicked()
                                {
                                    self.prefs.theme = kind;
                                    self.apply_theme(ctx);
                                    self.save_preferences();
                                    ui.close_menu();
                                }
                            }
                        });
                        self.menu_item(ui, "Full Screen", "F11", |app| {
                            app.layout.fullscreen = !app.layout.fullscreen;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(
                                app.layout.fullscreen,
                            ));
                        });
                        self.menu_item(ui, "Reset Layout", "", |app| app.reset_layout());
                    });
                    ui.menu_button("Tools", |ui| {
                        self.menu_item(ui, "MIDI Settings...", "", |app| {
                            app.refresh_midi_ports();
                            app.dialog = Some(DialogKind::MidiSettings)
                        });
                        self.menu_item(ui, "Audio Settings...", "", |app| {
                            app.dialog = Some(DialogKind::AudioSettings)
                        });
                        self.menu_item(ui, "Scan VST Plugins", "", |app| app.scan_vst_plugins());
                        self.menu_item(ui, "Render and Export...", "Ctrl+E", |app| {
                            app.open_export_dialog()
                        });
                        self.menu_item(ui, "Stress Test", "", |app| app.stress_test());
                    });
                    ui.menu_button("Help", |ui| {
                        self.menu_item(ui, "Keyboard Shortcuts", "", |app| {
                            app.dialog = Some(DialogKind::Shortcuts)
                        });
                        self.menu_item(ui, "About REDRAW", "", |app| {
                            app.dialog = Some(DialogKind::About)
                        });
                        self.menu_item(ui, "Diagnostics", "", |app| {
                            app.dialog = Some(DialogKind::Diagnostics)
                        });
                        self.menu_item(ui, "Open Log Folder", "", |app| app.open_log_folder());
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{}  |  {}",
                                self.project.name,
                                if self.dirty { "unsaved" } else { "saved" }
                            ))
                            .small()
                            .color(self.palette.text_dim),
                        );
                    });
                });
            });
    }

    fn menu_item(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        shortcut: &str,
        action: impl FnOnce(&mut Self),
    ) {
        let mut button = egui::Button::new(label).frame(false);
        if !shortcut.is_empty() {
            button = button.shortcut_text(shortcut);
        }
        if ui.add_sized([ui.available_width(), 18.0], button).clicked() {
            action(self);
            ui.close_menu();
        }
    }

    fn menu_check(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        shortcut: &str,
        checked: bool,
        action: impl FnOnce(&mut Self),
    ) {
        let mut text = if checked {
            format!("* {label}")
        } else {
            format!("  {label}")
        };
        if !shortcut.is_empty() {
            text.push_str(&format!("    {shortcut}"));
        }
        if ui
            .add_sized(
                [ui.available_width(), 18.0],
                egui::Button::new(text).frame(false),
            )
            .clicked()
        {
            action(self);
            ui.close_menu();
        }
    }

    fn transport_bar(&mut self, ctx: &egui::Context) {
        let palette = self.palette;
        egui::TopBottomPanel::top("transport")
            .exact_height(37.0)
            .frame(
                Frame::NONE
                    .fill(palette.panel_alt)
                    .inner_margin(Margin::symmetric(6, 4)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if icon_button(
                        ui,
                        &palette,
                        Icon::Rewind,
                        26.0,
                        false,
                        "Play from start (Ctrl+Space)",
                    )
                    .clicked()
                    {
                        self.audio.play_from_start();
                    }
                    let playing = self.audio.is_playing();
                    if icon_button(
                        ui,
                        &palette,
                        if playing { Icon::Stop } else { Icon::Play },
                        27.0,
                        playing,
                        "Play / Stop (Space)",
                    )
                    .clicked()
                    {
                        self.audio.toggle();
                    }
                    if icon_button(
                        ui,
                        &palette,
                        Icon::Record,
                        26.0,
                        self.record_armed,
                        "Record live MIDI (R)",
                    )
                    .clicked()
                    {
                        self.toggle_record();
                    }
                    if icon_button(
                        ui,
                        &palette,
                        Icon::Loop,
                        26.0,
                        self.project.playlist.loop_enabled,
                        "Loop playlist (L)",
                    )
                    .clicked()
                    {
                        self.toggle_loop();
                    }
                    ui.separator();

                    let mut bpm = self.project.bpm;
                    let bpm_resp = ui.add(
                        egui::DragValue::new(&mut bpm)
                            .range(10.0..=999.0)
                            .speed(0.25)
                            .suffix(" BPM")
                            .fixed_decimals(2),
                    );
                    if bpm_resp.changed() {
                        self.project.bpm = bpm.clamp(10.0, 999.0);
                        self.audio.set_bpm(self.project.bpm);
                        self.dirty = true;
                    }
                    let mut sig_num = self.project.time_sig.0 as i32;
                    let mut sig_den = self.project.time_sig.1 as i32;
                    ui.label("  ");
                    if ui
                        .add(egui::DragValue::new(&mut sig_num).range(1..=16))
                        .changed()
                    {
                        self.project.time_sig.0 = sig_num.clamp(1, 16) as u8;
                        self.dirty = true;
                    }
                    ui.label("/");
                    if ui
                        .add(egui::DragValue::new(&mut sig_den).range(1..=16))
                        .changed()
                    {
                        self.project.time_sig.1 = sig_den.clamp(1, 16) as u8;
                        self.dirty = true;
                    }
                    ui.label("  ");
                    let pattern_label = self
                        .pattern()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "Pattern".into());
                    let patterns: Vec<(usize, String)> = self
                        .project
                        .patterns
                        .iter()
                        .enumerate()
                        .map(|(index, pattern)| (index, pattern.name.clone()))
                        .collect();
                    let current_pattern = self.project.current_pattern;
                    egui::ComboBox::from_id_salt("transport_pattern")
                        .selected_text(pattern_label)
                        .width(132.0)
                        .show_ui(ui, |ui| {
                            for (index, name) in &patterns {
                                if ui
                                    .selectable_label(current_pattern == *index, name)
                                    .clicked()
                                {
                                    self.set_pattern_index(*index);
                                }
                            }
                        });
                    ui.separator();
                    let song_mode = self.audio.playlist_mode;
                    if ui
                        .selectable_label(!song_mode, "Pattern")
                        .on_hover_text("Pattern mode (loop the selected pattern)")
                        .clicked()
                    {
                        self.set_playlist_mode(false);
                    }
                    if ui
                        .selectable_label(song_mode, "Song")
                        .on_hover_text("Playlist mode")
                        .clicked()
                    {
                        self.set_playlist_mode(true);
                    }
                    ui.separator();
                    let tick = self.current_tick();
                    let pos = self.tick_text(tick);
                    ui.label(
                        RichText::new(pos)
                            .monospace()
                            .strong()
                            .color(palette.accent)
                            .size(14.0),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let cpu = self.audio.cpu_permille();
                        ui.label(
                            RichText::new(format!("CPU {:.1}%", cpu as f32 / 10.0))
                                .small()
                                .color(if cpu > 700 {
                                    palette.rec
                                } else if cpu > 400 {
                                    palette.warn
                                } else {
                                    palette.text_dim
                                }),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(format!(
                                "{} Hz / {}",
                                self.audio.sample_rate(),
                                if self.audio.block_size() == 0 {
                                    "auto".into()
                                } else {
                                    format!("{}", self.audio.block_size())
                                }
                            ))
                            .small()
                            .color(palette.text_dim),
                        );
                    });
                });
            });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        let palette = self.palette;
        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(23.0)
            .frame(
                Frame::NONE
                    .fill(palette.panel)
                    .inner_margin(Margin::symmetric(6, 3)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let device = if self.audio.is_running() {
                        format!(
                            "{} @ {} Hz / {}",
                            self.audio.device_name(),
                            self.audio.sample_rate(),
                            if self.audio.block_size() == 0 {
                                "auto".to_string()
                            } else {
                                self.audio.block_size().to_string()
                            }
                        )
                    } else {
                        "No audio device".to_string()
                    };
                    ui.label(RichText::new(device).small().color(palette.text_dim));
                    ui.separator();
                    ui.label(
                        RichText::new(format!("Voices {}", self.audio.voices()))
                            .small()
                            .color(palette.text_dim),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format!("CPU {:.1}%", self.audio.cpu_permille() as f32 / 10.0))
                            .small()
                            .color(palette.text_dim),
                    );
                    ui.separator();
                    let sel = if self.layout.lower_tab == LowerTab::PianoRoll {
                        format!("{} note(s) selected", self.piano.selection.len())
                    } else if !self.selection.clips.is_empty() {
                        format!("{} clip(s) selected", self.selection.clips.len())
                    } else {
                        self.selected_channel()
                            .map(|c| format!("Channel: {}", c.name))
                            .unwrap_or_else(|| "No selection".into())
                    };
                    ui.label(RichText::new(sel).small().color(palette.text_dim));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Some(toast) = &self.toast {
                            ui.label(
                                RichText::new(&toast.text)
                                    .small()
                                    .color(if toast.error { palette.rec } else { palette.ok }),
                            );
                        } else {
                            ui.label(
                                RichText::new(&self.status)
                                    .small()
                                    .color(palette.text_dim),
                            );
                        }
                    });
                });
            });
    }

    fn central_ui(&mut self, ctx: &egui::Context) {
        let palette = self.palette;
        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(palette.bg))
            .show(ctx, |ui| {
                let full = ui.available_rect_before_wrap();
                if !self.layout.show_playlist {
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(full), |ui| {
                        ui.set_clip_rect(full);
                        self.lower_editor_ui(ui);
                    });
                    return;
                }
                let splitter_h = 6.0;
                let min_lower = 150.0;
                let max_lower = (full.height() * 0.78).max(min_lower);
                let lower_h = self.layout.lower_height.clamp(min_lower, max_lower);
                let lower_top = full.bottom() - lower_h;
                let playlist_rect = Rect::from_min_max(
                    full.min,
                    Pos2::new(full.right(), lower_top - splitter_h),
                );
                let splitter_rect = Rect::from_min_max(
                    Pos2::new(full.left(), lower_top - splitter_h),
                    Pos2::new(full.right(), lower_top),
                );
                let lower_rect = Rect::from_min_max(
                    Pos2::new(full.left(), lower_top),
                    full.max,
                );
                ui.allocate_new_ui(
                    egui::UiBuilder::new().max_rect(playlist_rect),
                    |ui| {
                        ui.set_clip_rect(playlist_rect);
                        self.playlist_ui(ui);
                    },
                );
                let split = ui.interact(
                    splitter_rect,
                    egui::Id::new("lower_editor_splitter"),
                    egui::Sense::drag(),
                );
                if split.dragged() {
                    if let Some(pos) = split.interact_pointer_pos() {
                        self.layout.lower_height =
                            (full.bottom() - pos.y).clamp(min_lower, max_lower);
                    }
                }
                let split_color = if split.hovered() || split.dragged() {
                    palette.accent.gamma_multiply(0.55)
                } else {
                    palette.stroke
                };
                ui.painter().rect_filled(
                    splitter_rect,
                    egui::CornerRadius::ZERO,
                    split_color,
                );
                ui.allocate_new_ui(
                    egui::UiBuilder::new().max_rect(lower_rect),
                    |ui| {
                        ui.set_clip_rect(lower_rect);
                        ui.set_width(lower_rect.width());
                        ui.painter().rect_filled(
                            lower_rect,
                            egui::CornerRadius::ZERO,
                            palette.panel,
                        );
                        self.lower_editor_ui(ui);
                    },
                );
            });
    }

    fn lower_editor_ui(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette;
        ui.horizontal(|ui| {
            for tab in LowerTab::ALL {
                let selected = self.layout.lower_tab == tab;
                let icon = tab.icon();
                let response = ui
                    .add(
                        egui::Button::new(
                            RichText::new(tab.label())
                                .size(11.5)
                                .color(if selected { palette.text } else { palette.text_dim }),
                        )
                        .selected(selected)
                        .frame(true),
                    )
                    .on_hover_text(match tab {
                        LowerTab::Rack => "Channel rack and step sequencer (F6)",
                        LowerTab::PianoRoll => "Piano roll editor (F7)",
                        LowerTab::Mixer => "Mixer (F9)",
                        LowerTab::Automation => "Automation editor",
                    });
                let _ = icon;
                if response.clicked() {
                    self.set_lower_tab(tab);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.layout.lower_tab == LowerTab::PianoRoll {
                    ui.label(
                        RichText::new(format!(
                            "{} notes",
                            self.pattern()
                                .map(|p| p
                                    .channel(self.selection.channel_id)
                                    .map(|c| c.notes.len())
                                    .unwrap_or(0))
                                .unwrap_or(0)
                        ))
                        .small()
                        .color(palette.text_dim),
                    );
                }
            });
        });
        ui.separator();
        match self.layout.lower_tab {
            LowerTab::Rack => self.rack_ui(ui),
            LowerTab::PianoRoll => self.piano_roll_ui(ui),
            LowerTab::Mixer => self.mixer_ui(ui),
            LowerTab::Automation => self.automation_ui(ui),
        }
    }

    fn plugin_windows_ui(&mut self, ctx: &egui::Context) {
        if !self.layout.show_plugin_windows {
            return;
        }
        let targets = self.plugin_windows.clone();
        for target in targets {
            self.plugin_window_ui(ctx, &target);
        }
    }

    // -------------------------------------------------------------- shortcuts

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.wants_keyboard_input();
        let input = ctx.input(|i| i.clone());
        let ctrl = input.modifiers.ctrl || input.modifiers.command;
        let shift = input.modifiers.shift;
        let alt = input.modifiers.alt;
        let pressed = |key: egui::Key| input.key_pressed(key);

        if pressed(egui::Key::Escape) {
            if self.dialog.is_some() {
                self.dialog = None;
            } else if !self.plugin_windows.is_empty() {
                self.plugin_windows.clear();
            } else {
                self.deselect();
            }
            return;
        }
        if ctrl && shift && pressed(egui::Key::S) {
            self.save_as();
            return;
        }
        if ctrl && pressed(egui::Key::S) {
            self.save();
            return;
        }
        if ctrl && pressed(egui::Key::O) {
            self.open_dialog();
            return;
        }
        if ctrl && pressed(egui::Key::N) {
            self.new_project();
            return;
        }
        if ctrl && pressed(egui::Key::E) {
            self.open_export_dialog();
            return;
        }
        if ctrl && shift && pressed(egui::Key::Z) || ctrl && pressed(egui::Key::Y) {
            self.redo();
            return;
        }
        if ctrl && pressed(egui::Key::Z) {
            self.undo();
            return;
        }
        if ctrl && pressed(egui::Key::A) {
            self.select_all();
            return;
        }
        if ctrl && pressed(egui::Key::C) {
            self.copy();
            return;
        }
        if ctrl && pressed(egui::Key::X) {
            self.cut();
            return;
        }
        if ctrl && pressed(egui::Key::V) {
            self.paste();
            return;
        }
        if ctrl && pressed(egui::Key::D) {
            self.duplicate();
            return;
        }
        if ctrl && pressed(egui::Key::Space) {
            self.audio.play_from_start();
            return;
        }
        if pressed(egui::Key::F5) {
            self.layout.show_playlist = !self.layout.show_playlist;
            return;
        }
        if pressed(egui::Key::F6) {
            self.set_lower_tab(LowerTab::Rack);
            return;
        }
        if pressed(egui::Key::F7) {
            self.set_lower_tab(LowerTab::PianoRoll);
            return;
        }
        if pressed(egui::Key::F8) {
            self.layout.show_browser = !self.layout.show_browser;
            return;
        }
        if pressed(egui::Key::F9) {
            self.set_lower_tab(LowerTab::Mixer);
            return;
        }
        if pressed(egui::Key::F10) {
            self.dialog = Some(DialogKind::Preferences);
            return;
        }
        if pressed(egui::Key::F11) {
            self.layout.fullscreen = !self.layout.fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.layout.fullscreen));
            return;
        }
        if typing {
            return;
        }
        if pressed(egui::Key::Space) {
            self.audio.toggle();
        } else if pressed(egui::Key::R) {
            self.toggle_record();
        } else if pressed(egui::Key::L) {
            self.toggle_loop();
        } else if pressed(egui::Key::Delete) || pressed(egui::Key::Backspace) {
            self.delete_selection();
        } else if pressed(egui::Key::Q) {
            self.dialog = Some(DialogKind::Quantize);
        } else if pressed(egui::Key::H) {
            self.dialog = Some(DialogKind::Humanize);
        } else if self.layout.lower_tab == LowerTab::PianoRoll {
            if pressed(egui::Key::Num1) {
                self.piano.tool = Tool::Draw;
            } else if pressed(egui::Key::Num2) {
                self.piano.tool = Tool::Select;
            } else if pressed(egui::Key::Num3) {
                self.piano.tool = Tool::Erase;
            } else if pressed(egui::Key::Num4) {
                self.piano.tool = Tool::Split;
            } else if pressed(egui::Key::Num5) {
                self.piano.tool = Tool::Merge;
            }
        }
        let _ = alt;
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if self.allow_close {
            return;
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.dirty {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.close_confirm = true;
                self.dialog = Some(DialogKind::CloseConfirm);
            } else {
                self.allow_close = true;
            }
        }
    }

    fn request_close(&mut self, ctx: &egui::Context) {
        if self.dirty {
            self.close_confirm = true;
            self.dialog = Some(DialogKind::CloseConfirm);
        } else {
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn poll_audio_and_midi(&mut self) {
        self.audio.collect_garbage();
        let events = self.midi.poll();
        for event in events {
            use redraw_piano_roll::live::LiveMidiEvent;
            match event {
                LiveMidiEvent::NoteOn { key, velocity, .. } => {
                    if velocity > 0 {
                        self.preview_note_on(key);
                        if self.record_armed {
                            let tick = self.current_tick();
                            let pattern = self.project.pattern_mut();
                            if let Some(pattern) = pattern {
                                let _ = self.midi_recorder.on_event(
                                    pattern,
                                    self.selection.channel_id,
                                    tick,
                                    event,
                                );
                                self.dirty = true;
                                self.schedule_dirty = true;
                            }
                        }
                    } else {
                        self.preview_note_off(key);
                    }
                }
                LiveMidiEvent::NoteOff { key, .. } => {
                    self.preview_note_off(key);
                    if self.record_armed {
                        let tick = self.current_tick();
                        if let Some(pattern) = self.project.pattern_mut() {
                            let _ = self.midi_recorder.on_event(
                                pattern,
                                self.selection.channel_id,
                                tick,
                                event,
                            );
                            self.dirty = true;
                            self.schedule_dirty = true;
                        }
                    }
                }
                LiveMidiEvent::ControlChange { .. } | LiveMidiEvent::PitchBend { .. } => {
                    if self.record_armed {
                        let tick = self.current_tick();
                        if let Some(pattern) = self.project.pattern_mut() {
                            let _ = self.midi_recorder.on_event(
                                pattern,
                                self.selection.channel_id,
                                tick,
                                event,
                            );
                            self.dirty = true;
                        }
                    }
                }
                LiveMidiEvent::ProgramChange { .. } => {}
            }
        }
        if self.schedule_dirty && self.note_drag.is_none() && self.playlist_view.drag.is_none() {
            self.sync_schedules();
        }
    }

    fn poll_export(&mut self) {
        let Some(rx) = &self.export.rx else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(summary)) => {
                self.export.busy = false;
                self.export.rx = None;
                self.export.summary = Some(summary);
                self.export.error = false;
                self.export.status = format!(
                    "Exported {} frames ({:.2} s) at {} Hz",
                    summary.frames, summary.duration_seconds, summary.sample_rate
                );
                self.set_status(format!("Export complete: {}", self.export.path));
            }
            Ok(Err(error)) => {
                self.export.busy = false;
                self.export.rx = None;
                self.export.error = true;
                self.export.status = error.clone();
                self.set_error(format!("Export failed: {error}"));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.export.busy = false;
                self.export.rx = None;
                self.export.error = true;
                self.export.status = "Export thread stopped unexpectedly".into();
            }
        }
    }

    fn autosave_tick(&mut self) {
        if !self.prefs.autosave || !self.dirty {
            return;
        }
        match self
            .autosave
            .autosave(&self.project, self.current_path.as_deref(), false)
        {
            Ok(Some(path)) => {
                self.log_line(&format!("autosaved {}", path.display()));
            }
            Ok(None) => {}
            Err(error) => {
                self.log_line(&format!("autosave failed: {error}"));
            }
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let name = if self.project.name.trim().is_empty() {
            "Untitled"
        } else {
            self.project.name.trim()
        };
        let title = format!("{name}{} - REDRAW", if self.dirty { " *" } else { "" });
        if title != self.last_title {
            self.last_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    // -------------------------------------------------------------- actions used by UI

    pub fn toggle_record(&mut self) {
        self.record_armed = !self.record_armed;
        if self.record_armed {
            self.set_status("Record armed: live MIDI will be written to the current pattern");
        } else {
            self.set_status("Record disarmed");
        }
    }

    pub fn toggle_loop(&mut self) {
        self.project.playlist.loop_enabled = !self.project.playlist.loop_enabled;
        self.audio.set_loop(
            self.project.playlist.loop_enabled,
            self.project.playlist.loop_start,
            self.project.playlist.loop_end,
        );
        self.dirty = true;
        self.set_status(if self.project.playlist.loop_enabled {
            "Playlist loop enabled"
        } else {
            "Playlist loop disabled"
        });
    }

    pub fn set_playlist_mode(&mut self, song: bool) {
        self.audio.set_playlist_mode(song);
        self.set_status(if song {
            "Song mode (playlist)"
        } else {
            "Pattern mode"
        });
    }

    pub fn reset_layout(&mut self) {
        self.layout = LayoutState::default();
        self.set_status("Layout reset");
    }

    pub fn open_log_folder(&mut self) {
        let path = self.user_data.clone();
        let _ = std::fs::create_dir_all(&path);
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("explorer").arg(&path).spawn();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&path).spawn();
        #[cfg(all(unix, not(target_os = "macos")))]
        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
        self.set_status(format!("Log folder: {}", path.display()));
    }

    pub fn save_preferences(&mut self) {
        let mut text = String::new();
        text.push_str(&format!("theme={}\n", theme_key(self.prefs.theme)));
        text.push_str(&format!("scale={}\n", self.prefs.scale));
        text.push_str(&format!("tooltips={}\n", self.prefs.tooltips));
        text.push_str(&format!("autosave={}\n", self.prefs.autosave));
        text.push_str(&format!(
            "export_dir={}\n",
            self.prefs.export_dir.to_string_lossy()
        ));
        let path = self.user_data.join("settings.txt");
        let _ = std::fs::write(path, text);
    }
}

// ------------------------------------------------------------------ eframe

impl eframe::App for RedrawApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame += 1;
        self.handle_close(ctx);
        self.poll_export();
        self.poll_audio_and_midi();
        self.handle_shortcuts(ctx);
        self.menu_bar(ctx);
        self.transport_bar(ctx);
        self.status_bar(ctx);

        let palette = self.palette;
        if self.layout.show_browser {
            egui::SidePanel::left("browser_panel")
                .resizable(true)
                .default_width(self.layout.browser_width)
                .min_width(160.0)
                .max_width(420.0)
                .frame(
                    Frame::NONE
                        .fill(palette.panel)
                        .inner_margin(Margin::symmetric(6, 5)),
                )
                .show(ctx, |ui| {
                    self.layout.browser_width = ui.available_width() + 12.0;
                    self.browser_ui(ui);
                });
        }
        if self.layout.show_inspector {
            egui::SidePanel::right("inspector_panel")
                .resizable(true)
                .default_width(self.layout.inspector_width)
                .min_width(220.0)
                .max_width(440.0)
                .frame(
                    Frame::NONE
                        .fill(palette.panel)
                        .inner_margin(Margin::symmetric(7, 5)),
                )
                .show(ctx, |ui| {
                    self.layout.inspector_width = ui.available_width() + 14.0;
                    self.inspector_ui(ui);
                });
        }
        self.central_ui(ctx);
        self.plugin_windows_ui(ctx);
        self.dialogs_ui(ctx);
        self.export_dialog_ui(ctx);

        self.autosave_tick();
        self.update_title(ctx);

        if let Some(toast) = &self.toast {
            if Instant::now() >= toast.until {
                self.toast = None;
            }
        }
        if self.audio.is_playing() {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if self.export.busy {
            ctx.request_repaint_after(Duration::from_millis(60));
        } else if self.toast.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.all_notes_off();
        self.midi.disconnect();
        self.save_preferences();
        self.save_recent();
        if let Err(error) = self.autosave.end_session_clean() {
            self.log_line(&format!("clean shutdown marker failed: {error}"));
        }
        self.log_line("REDRAW stopped");
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.palette.bg.to_normalized_gamma_f32()
    }
}

// ------------------------------------------------------------------ free helpers

pub fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or(manifest)
}

pub fn app_recent_is_empty(app: &RedrawApp) -> bool {
    app.recent.is_empty()
}

pub fn theme_key(kind: ThemeKind) -> &'static str {
    match kind {
        ThemeKind::Dark => "dark",
        ThemeKind::Light => "light",
        ThemeKind::Midnight => "midnight",
        ThemeKind::Custom => "custom",
    }
}

pub fn theme_from_key(key: &str) -> ThemeKind {
    match key.trim().to_ascii_lowercase().as_str() {
        "light" => ThemeKind::Light,
        "midnight" => ThemeKind::Midnight,
        "custom" => ThemeKind::Custom,
        _ => ThemeKind::Dark,
    }
}

pub fn load_preferences(root: &Path) -> Preferences {
    let mut prefs = Preferences::default_for(root);
    let path = root.join("user-data").join("settings.txt");
    let Ok(text) = std::fs::read_to_string(path) else {
        return prefs;
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "theme" => prefs.theme = theme_from_key(value),
            "scale" => {
                if let Ok(v) = value.trim().parse::<f32>() {
                    prefs.scale = v.clamp(0.75, 1.75);
                }
            }
            "tooltips" => prefs.tooltips = value.trim() != "false",
            "autosave" => prefs.autosave = value.trim() != "false",
            "export_dir" => {
                let value = value.trim();
                if !value.is_empty() {
                    prefs.export_dir = PathBuf::from(value);
                }
            }
            _ => {}
        }
    }
    prefs
}

pub fn load_recent(user_data: &Path) -> Vec<PathBuf> {
    let path = user_data.join("recent.txt");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .take(10)
        .collect()
}

impl RedrawApp {
    pub fn save_recent(&mut self) {
        let text = self
            .recent
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        let _ = std::fs::write(self.user_data.join("recent.txt"), text);
    }

    pub fn remember_recent(&mut self, path: &Path) {
        let path = path.to_path_buf();
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path);
        self.recent.truncate(self.prefs.recent_limit.max(1));
        self.save_recent();
    }

    pub fn current_export_extension(&self) -> &'static str {
        self.default_export_format().extension()
    }

    pub fn update_export_path_extension(&mut self) {
        let ext = self.current_export_extension();
        let path = PathBuf::from(&self.export.path);
        let new_path = path.with_extension(ext);
        self.export.path = new_path.to_string_lossy().into_owned();
    }

    pub fn open_export_dialog(&mut self) {
        self.export.pattern_id = self.active_pattern_id();
        self.export.song_scope = !self.project.playlist.clips.is_empty();
        self.export.sample_rate = self.project.sample_rate.clamp(8_000, 192_000);
        self.export.open = true;
        if self.export.path.is_empty() {
            self.export.path = self
                .prefs
                .export_dir
                .join(format!(
                    "{}.{}",
                    sanitize_file_stem(&self.project.name),
                    self.current_export_extension()
                ))
                .to_string_lossy()
                .into_owned();
        }
    }

    pub fn append_log(&mut self, text: impl Into<String>) {
        self.log_line(&text.into());
    }
}

pub fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

pub fn sanitize_file_stem(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ' ') {
            out.push(ch);
        } else if !ch.is_whitespace() {
            out.push('_');
        }
    }
    let out = out.trim().to_string();
    if out.is_empty() {
        "untitled".into()
    } else {
        out
    }
}

pub fn default_vst_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(program_files) = std::env::var("ProgramFiles") {
        roots.push(
            PathBuf::from(&program_files)
                .join("Common Files")
                .join("VST2"),
        );
        roots.push(PathBuf::from(&program_files).join("VSTPlugins"));
    }
    if let Ok(program_files_x86) = std::env::var("ProgramFiles(x86)") {
        roots.push(
            PathBuf::from(program_files_x86)
                .join("Common Files")
                .join("VST2"),
        );
    }
    roots
}

pub fn draw_panel_header(ui: &mut egui::Ui, palette: &Palette, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong().size(12.0).color(palette.text));
        if !subtitle.is_empty() {
            ui.label(
                RichText::new(subtitle)
                    .small()
                    .color(palette.text_dim),
            );
        }
    });
    let rect = ui.available_rect_before_wrap();
    ui.painter().line_segment(
        [
            Pos2::new(rect.left(), ui.cursor().top() + 2.0),
            Pos2::new(rect.right(), ui.cursor().top() + 2.0),
        ],
        Stroke::new(1.0_f32, palette.grid),
    );
    ui.add_space(2.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_text_uses_time_signature() {
        let app = test_app();
        assert_eq!(app.tick_text(0), "1:1:1");
        assert_eq!(app.tick_text(app.beat_ticks()), "1:2:1");
        assert_eq!(app.tick_text(app.bar_ticks()), "2:1:1");
    }

    #[test]
    fn sanitize_file_stem_is_safe() {
        assert_eq!(sanitize_file_stem("My Song"), "My Song");
        assert_eq!(sanitize_file_stem("a/b:c"), "a_b_c");
        assert_eq!(sanitize_file_stem("   "), "untitled");
    }

    #[test]
    fn history_undo_redo_roundtrip() {
        let mut h = History::default();
        let mut p = StudioProject::default();
        h.begin(&p);
        p.name = "changed".into();
        h.commit();
        let previous = h.undo(&p).unwrap();
        assert_eq!(previous.name, "Untitled");
        let redo = h.redo(&previous).unwrap();
        assert_eq!(redo.name, "changed");
    }

    #[test]
    fn preferences_roundtrip_keys() {
        assert_eq!(theme_key(ThemeKind::Midnight), "midnight");
        assert_eq!(theme_from_key("light"), ThemeKind::Light);
        assert_eq!(theme_from_key("garbage"), ThemeKind::Dark);
    }

    fn test_app() -> RedrawApp {
        // Keep unit tests independent of the audio device and egui context by
        // constructing only the fields the formatting helpers need.
        let root = workspace_root();
        let project = StudioProject::default();
        let factory = BuiltinFactory::new();
        let audio = AudioHost::new(&project, &factory, false);
        let selected_channel = project.channels[0].id;
        let selected_strip = project.channels[0].mixer_index;
        RedrawApp {
            project,
            factory,
            audio,
            palette: Palette::dark(),
            prefs: Preferences::default_for(&root),
            history: History::default(),
            dirty: false,
            current_path: None,
            recent: Vec::new(),
            autosave: AutosaveManager::new(root.join("user-data")),
            recovery: None,
            status: String::new(),
            toast: None,
            layout: LayoutState::default(),
            selection: SelectionState {
                channel_id: selected_channel,
                strip: selected_strip,
                clips: Vec::new(),
                automation: None,
            },
            piano: PianoRoll::new(1, selected_channel),
            piano_view: PianoView::default(),
            playlist_view: PlaylistView::default(),
            automation_view: AutomationView::default(),
            browser: BrowserState::default(),
            dialog: None,
            export: ExportState::new(&root.join("exports")),
            plugin_windows: Vec::new(),
            midi: LiveMidi::new(),
            midi_recorder: MidiRecorder::new(),
            midi_ports: Vec::new(),
            midi_port: None,
            preview_keys: Vec::new(),
            quantize: redraw_piano_roll::QuantizeOptions::default(),
            humanize: redraw_piano_roll::HumanizeOptions::default(),
            transpose_semitones: 12,
            pattern_length_steps: 16,
            rename_text: String::new(),
            marker_text: String::new(),
            piano_preview: None,
            automation_target_index: 0,
            pending_fx: None,
            pending_channel: None,
            pending_pattern: None,
            clip_clipboard: Vec::new(),
            note_drag: None,
            step_paint: None,
            schedule_dirty: false,
            record_armed: false,
            root,
            user_data: workspace_root().join("user-data"),
            log: Vec::new(),
            vst_plugins: Vec::new(),
            vst_roots: Vec::new(),
            frame: 0,
            close_confirm: false,
            allow_close: false,
            last_title: String::new(),
            synth_specs: std::collections::HashMap::new(),
            effect_specs: std::collections::HashMap::new(),
        }
    }
}


