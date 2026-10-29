//! REDRAW shared contracts: audio types, parameter model, plugin traits, project model.
//! This crate is dependency-light on purpose; all realtime crates depend on it.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub type Sample = f32;

pub const SAMPLE_RATE: u32 = 48_000;
pub const BLOCK_SIZE: usize = 512;
pub const PPQ: u32 = 960;
pub const MAX_CHANNELS: usize = 64;
pub const MAX_INSERTS: usize = 8;
pub const MAX_SENDS: usize = 4;
pub const MAX_VOICES: usize = 64;
pub const MAX_PATTERNS: usize = 256;
pub const APP_NAME: &str = "REDRAW";
pub const APP_TAGLINE: &str = "Rust-Engineered Digital Rhythmic Audio Workstation";

// ---------------------------------------------------------------- math helpers

#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    if db <= -90.0 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

#[inline]
pub fn gain_to_db(g: f32) -> f32 {
    20.0 * g.max(1e-9).log10()
}

#[inline]
pub fn midi_note_hz(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

#[inline]
pub fn clamp01(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// Constant-power pan; pan in -1..=1.
#[inline]
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * 0.25 * std::f32::consts::PI;
    (a.cos(), a.sin())
}

#[inline]
pub fn tick_to_samples(tick: f64, bpm: f32, sr: f32) -> f64 {
    tick * (60.0 * sr as f64) / (PPQ as f64 * bpm.max(1.0) as f64)
}

#[inline]
pub fn samples_to_tick(samples: f64, bpm: f32, sr: f32) -> f64 {
    samples * (PPQ as f64 * bpm.max(1.0) as f64) / (60.0 * sr as f64)
}

/// Bars/beats/16ths display from ticks (4/4 display base, 960 PPQ).
pub fn tick_to_bar_beat(tick: u64) -> (u32, u32, u32) {
    let bar = tick / (PPQ as u64 * 4);
    let rem = tick % (PPQ as u64 * 4);
    let beat = rem / PPQ as u64;
    let step = (rem % PPQ as u64) / (PPQ as u64 / 4);
    (bar as u32 + 1, beat as u32 + 1, step as u32 + 1)
}

// ---------------------------------------------------------------- parameters

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParamCurve {
    Linear,
    Pow2,
    Sqrt,
    /// Exponential when min > 0 (frequency-like), otherwise linear.
    Exp,
}

#[derive(Clone, Copy, Debug)]
pub struct ParamSpec {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub curve: ParamCurve,
    pub unit: &'static str,
}

impl ParamSpec {
    pub const fn new(
        name: &'static str,
        min: f32,
        max: f32,
        default: f32,
        unit: &'static str,
    ) -> Self {
        Self {
            name,
            min,
            max,
            default,
            curve: ParamCurve::Linear,
            unit,
        }
    }
    pub const fn curve(mut self, c: ParamCurve) -> Self {
        self.curve = c;
        self
    }

    /// value -> 0..1
    pub fn normalize(&self, v: f32) -> f32 {
        let v = v.clamp(self.min, self.max);
        let span = (self.max - self.min).max(1e-9);
        let n = (v - self.min) / span;
        match self.curve {
            ParamCurve::Linear => n,
            ParamCurve::Pow2 => n.sqrt(),
            ParamCurve::Sqrt => n * n,
            ParamCurve::Exp => {
                if self.min > 0.0 {
                    (v / self.min).ln() / (self.max / self.min).ln()
                } else {
                    n
                }
            }
        }
    }

    /// 0..1 -> value
    pub fn denormalize(&self, n: f32) -> f32 {
        let n = n.clamp(0.0, 1.0);
        let span = self.max - self.min;
        match self.curve {
            ParamCurve::Linear => self.min + span * n,
            ParamCurve::Pow2 => self.min + span * n * n,
            ParamCurve::Sqrt => self.min + span * n.sqrt(),
            ParamCurve::Exp => {
                if self.min > 0.0 {
                    self.min * (self.max / self.min).powf(n)
                } else {
                    self.min + span * n
                }
            }
        }
    }
}

// ---------------------------------------------------------------- plugin traits

/// Instrument contract. Render must not allocate; implementors preallocate everything.
pub trait Synth: Send {
    fn synth_type(&self) -> &'static str;
    fn reset(&mut self);
    fn note_on(&mut self, note: u8, velocity: f32);
    fn note_off(&mut self, note: u8);
    fn all_notes_off(&mut self);
    fn render(&mut self, left: &mut [f32], right: &mut [f32], sr: f32);
    fn params(&self) -> &'static [ParamSpec];
    fn set_param(&mut self, index: usize, value: f32);
    fn get_param(&self, index: usize) -> f32;
    fn active_voices(&self) -> usize;
    fn set_sample_rate(&mut self, _sr: f32) {}
    /// Instances created from one engine may carry a registry display name.
    fn set_display_name(&mut self, _name: &'static str) {}
    fn get_param_count(&self) -> usize {
        self.params().len()
    }
    fn get_param_name(&self, index: usize) -> &'static str {
        self.params().get(index).map(|p| p.name).unwrap_or("")
    }
    /// Alias kept for the shared contract wording.
    fn render_into_buffer(&mut self, left: &mut [f32], right: &mut [f32], sr: f32) {
        self.render(left, right, sr)
    }
}

/// Effect contract. `process` must not allocate; bypass is exact passthrough.
pub trait Effect: Send {
    fn effect_type(&self) -> &'static str;
    fn reset(&mut self);
    fn process(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, tempo: f32);
    fn params(&self) -> &'static [ParamSpec];
    fn set_param(&mut self, index: usize, value: f32);
    fn get_param(&self, index: usize) -> f32;
    fn set_bypass(&mut self, bypass: bool);
    fn is_bypassed(&self) -> bool;
    fn get_param_count(&self) -> usize {
        self.params().len()
    }
    fn get_param_name(&self, index: usize) -> &'static str {
        self.params().get(index).map(|p| p.name).unwrap_or("")
    }
    fn process_in_place(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, tempo: f32) {
        self.process(left, right, sr, tempo)
    }
    /// Controller-type effects (peak/XY/formula) expose their modulator output.
    fn controller_value(&self) -> f32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectSlot {
    Insert(u8),
    Send(u8),
    MasterInsert(u8),
}

/// Control-thread plugin factory. The audio engine never calls this: graphs are built
/// by the UI/export layer and installed at block boundaries, so construction cannot
/// allocate inside the audio callback.
pub trait PluginFactory: Send + Sync {
    fn create_synth(&self, type_name: &str) -> Option<Box<dyn Synth>>;
    fn create_effect(&self, type_name: &str) -> Option<Box<dyn Effect>>;
    fn synth_names(&self) -> &'static [&'static str];
    fn effect_names(&self) -> &'static [&'static str];

    fn make_synth_with_defaults(&self, type_name: &str) -> Option<(Box<dyn Synth>, Vec<f32>)> {
        self.create_synth(type_name).map(|synth| {
            let defaults = synth.params().iter().map(|p| p.default).collect();
            (synth, defaults)
        })
    }
    fn make_effect_with_defaults(&self, type_name: &str) -> Option<(Box<dyn Effect>, Vec<f32>)> {
        self.create_effect(type_name).map(|fx| {
            let defaults = fx.params().iter().map(|p| p.default).collect();
            (fx, defaults)
        })
    }
}

// ---------------------------------------------------------------- sequencer model

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Step {
    pub active: bool,
    pub velocity: u8,
    pub nudge: i16,
}

impl Default for Step {
    fn default() -> Self {
        Self {
            active: false,
            velocity: 100,
            nudge: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Note {
    pub id: u64,
    pub key: u8,
    pub start: u64, // ticks
    pub length: u64,
    pub velocity: u8,
    pub pan: i8, // -64..63
    pub release: u8,
}

impl Default for Note {
    fn default() -> Self {
        Self {
            id: 0,
            key: 60,
            start: 0,
            length: PPQ as u64 / 4,
            velocity: 100,
            pan: 0,
            release: 64,
        }
    }
}

/// One MIDI control-change event stored in a pattern channel (control lane).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CcEvent {
    /// Absolute pattern tick (960 PPQ).
    pub tick: u64,
    pub controller: u8,
    pub value: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PatternChannel {
    pub channel_id: u32,
    pub steps: Vec<Step>,
    pub notes: Vec<Note>,
    /// Control-change lanes recorded or imported for this channel.
    #[serde(default)]
    pub cc: Vec<CcEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pattern {
    pub id: u32,
    pub name: String,
    pub color: [u8; 3],
    pub length_steps: u32,
    pub channels: Vec<PatternChannel>,
}

impl Pattern {
    pub fn new(id: u32, name: impl Into<String>, color: [u8; 3], length_steps: u32) -> Self {
        Self {
            id,
            name: name.into(),
            color,
            length_steps,
            channels: Vec::new(),
        }
    }
    pub fn channel_mut(&mut self, channel_id: u32, steps: u32) -> &mut PatternChannel {
        if let Some(pos) = self.channels.iter().position(|c| c.channel_id == channel_id) {
            &mut self.channels[pos]
        } else {
            self.channels.push(PatternChannel {
                channel_id,
                steps: vec![Step::default(); steps as usize],
                notes: Vec::new(),
                cc: Vec::new(),
            });
            self.channels.last_mut().unwrap()
        }
    }
    pub fn channel(&self, channel_id: u32) -> Option<&PatternChannel> {
        self.channels.iter().find(|c| c.channel_id == channel_id)
    }
    /// Length in ticks.
    pub fn length_ticks(&self) -> u64 {
        self.length_steps as u64 * (PPQ as u64 / 4)
    }
}

// ---------------------------------------------------------------- arrangement model

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Pattern,
    Audio,
    Automation,
    Midi,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackState {
    pub name: String,
    pub color: [u8; 3],
    pub kind: TrackKind,
    pub mute: bool,
    pub solo: bool,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipKind {
    Pattern,
    Audio,
    Automation,
    Midi,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClipState {
    pub id: u64,
    pub track: usize,
    pub start: u64,
    pub length: u64,
    pub kind: ClipKind,
    pub source_id: u32,
    pub name: String,
    pub color: [u8; 3],
    pub looping: bool,
    pub gain_db: f32,
    pub transpose: i32,
    pub stretch: f32,
    /// Optional external audio (user file) — never a bundled asset.
    pub audio_path: Option<String>,
}

impl ClipState {
    pub fn end(&self) -> u64 {
        self.start + self.length
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Marker {
    pub name: String,
    pub color: [u8; 3],
    pub pos: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaylistState {
    pub tracks: Vec<TrackState>,
    pub clips: Vec<ClipState>,
    pub markers: Vec<Marker>,
    pub loop_enabled: bool,
    pub loop_start: u64,
    pub loop_end: u64,
}

impl Default for PlaylistState {
    fn default() -> Self {
        let colors = [
            [86, 168, 122],
            [91, 141, 201],
            [190, 130, 90],
            [156, 110, 180],
            [180, 170, 90],
            [90, 170, 180],
            [190, 95, 110],
            [120, 140, 170],
        ];
        Self {
            tracks: (0..8)
                .map(|i| TrackState {
                    name: format!("Track {}", i + 1),
                    color: colors[i],
                    kind: TrackKind::Pattern,
                    mute: false,
                    solo: false,
                    height: 74.0,
                })
                .collect(),
            clips: Vec::new(),
            markers: Vec::new(),
            loop_enabled: false,
            loop_start: 0,
            loop_end: PPQ as u64 * 16,
        }
    }
}

// ---------------------------------------------------------------- automation model

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interp {
    Linear,
    Smooth,
    Step,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct AutomationPoint {
    pub time: f64, // ticks
    pub value: f32,
    pub tension: f32, // -1..1 shapes segment
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AutoTarget {
    Tempo,
    MasterVolume,
    ChannelVolume { channel_id: u32 },
    ChannelPan { channel_id: u32 },
    ChannelParam { channel_id: u32, param: u32 },
    FxParam { channel_id: u32, slot: u8, param: u32 },
    SendLevel { channel_id: u32, send: u8 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutomationClipState {
    pub id: u64,
    pub name: String,
    pub color: [u8; 3],
    pub target: AutoTarget,
    pub points: Vec<AutomationPoint>,
    pub interp: Interp,
    pub enabled: bool,
    pub length: u64,
}

impl AutomationClipState {
    pub fn value_at(&self, tick: f64) -> f32 {
        automation_value(&self.points, self.interp, tick)
    }
}

/// Evaluate a sorted envelope at `tick`. Values outside the point span hold the
/// first/last point value. Unknown/empty envelopes return 0.0.
pub fn automation_value(points: &[AutomationPoint], interp: Interp, tick: f64) -> f32 {
    if points.is_empty() {
        return 0.0;
    }
    let first = points[0];
    if tick <= first.time {
        return first.value;
    }
    let last = *points.last().unwrap();
    if tick >= last.time {
        return last.value;
    }
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if tick >= a.time && tick <= b.time {
            let span = (b.time - a.time).max(1e-9);
            let mut t = ((tick - a.time) / span) as f32;
            match interp {
                Interp::Step => return a.value,
                Interp::Linear => {}
                Interp::Smooth => {
                    t = t * t * (3.0 - 2.0 * t);
                }
            }
            let tension = a.tension.clamp(-1.0, 1.0);
            if tension.abs() > 0.001 {
                let k = 2.0f32.powf(tension * 3.0);
                t = t.powf(k);
            }
            return a.value + (b.value - a.value) * t;
        }
    }
    last.value
}

// ---------------------------------------------------------------- mixer + channel model

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FxSlotState {
    pub effect_type: String,
    pub params: Vec<f32>,
    pub bypass: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SendState {
    pub level_db: f32,
    pub pre_fader: bool,
    pub target: u8, // 0..3 -> buses A..D
}

impl Default for SendState {
    fn default() -> Self {
        Self {
            level_db: -90.0,
            pre_fader: false,
            target: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MixerStripState {
    pub name: String,
    pub color: [u8; 3],
    pub volume_db: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub inserts: Vec<Option<FxSlotState>>,
    pub sends: [SendState; 4],
}

impl Default for MixerStripState {
    fn default() -> Self {
        Self {
            name: String::new(),
            color: [120, 140, 170],
            volume_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            inserts: vec![None; MAX_INSERTS],
            sends: [SendState::default(); 4],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MixerState {
    pub strips: Vec<MixerStripState>, // 64 channel strips
    pub master: MixerStripState,
    pub bus_names: [String; 4],
    /// Insert chains for the four send/return buses (A..D).
    pub bus_inserts: [Vec<Option<FxSlotState>>; 4],
    pub bus_volumes_db: [f32; 4],
}

impl Default for MixerState {
    fn default() -> Self {
        Self {
            strips: (0..MAX_CHANNELS)
                .map(|i| MixerStripState {
                    name: format!("Ch {}", i + 1),
                    ..Default::default()
                })
                .collect(),
            master: MixerStripState {
                name: "Master".into(),
                inserts: vec![None; MAX_INSERTS],
                ..Default::default()
            },
            bus_names: ["Reverb A".into(), "Delay B".into(), "Comp C".into(), "User D".into()],
            bus_inserts: std::array::from_fn(|_| vec![None; MAX_INSERTS]),
            bus_volumes_db: [0.0; 4],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelInfo {
    pub id: u32,
    pub name: String,
    pub color: [u8; 3],
    pub mixer_index: usize,
    pub synth_type: String,
    pub params: Vec<f32>,
    pub volume_db: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub note_offset: i32,
}

impl ChannelInfo {
    pub fn new(id: u32, name: impl Into<String>, synth_type: impl Into<String>, color: [u8; 3]) -> Self {
        Self {
            id,
            name: name.into(),
            color,
            mixer_index: (id as usize).saturating_sub(1) % MAX_CHANNELS,
            synth_type: synth_type.into(),
            params: Vec::new(),
            volume_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            note_offset: 0,
        }
    }
}

// ---------------------------------------------------------------- FLP import provenance

/// One FLP event retained verbatim from an imported project.
///
/// `payload` is the decoded event body (without the length prefix) so the
/// event can be re-encoded with the FLP width rules. Unknown events are kept
/// here; the lossless REDRAW state lives in `.rdp` and in the `.flp` sidecar.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FlpRawEvent {
    pub id: u8,
    pub payload: Vec<u8>,
}

/// Source metadata and unhandled events from an imported FLP.
///
/// Handled core events are represented by the normal project model. The raw
/// list exists so a re-save does not silently discard data REDRAW does not
/// understand yet.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FlpSource {
    pub format: i16,
    pub channels: u16,
    pub ppq: u16,
    pub version: String,
    #[serde(default)]
    pub events: Vec<FlpRawEvent>,
}

// ---------------------------------------------------------------- project

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StudioProject {
    pub version: u16,
    pub name: String,
    pub author: String,
    pub bpm: f32,
    pub time_sig: (u8, u8),
    pub swing: f32,
    pub patterns: Vec<Pattern>,
    pub current_pattern: usize,
    pub channels: Vec<ChannelInfo>,
    pub mixer: MixerState,
    pub playlist: PlaylistState,
    pub automation: Vec<AutomationClipState>,
    pub next_channel_id: u32,
    pub next_pattern_id: u32,
    pub next_clip_id: u64,
    pub next_note_id: u64,
    pub next_automation_id: u64,
    pub sample_rate: u32,
    /// Provenance from a `.flp` import. Not present for native `.rdp` files
    /// created from scratch.
    #[serde(default)]
    pub flp_source: Option<FlpSource>,
}

impl Default for StudioProject {
    fn default() -> Self {
        let mut p = Self {
            version: 1,
            name: "Untitled".into(),
            author: String::new(),
            bpm: 140.0,
            time_sig: (4, 4),
            swing: 0.5,
            patterns: vec![Pattern::new(1, "Pattern 1", [95, 179, 108], 16)],
            current_pattern: 0,
            channels: Vec::new(),
            mixer: MixerState::default(),
            playlist: PlaylistState::default(),
            automation: Vec::new(),
            next_channel_id: 1,
            next_pattern_id: 2,
            next_clip_id: 1,
            next_note_id: 1,
            next_automation_id: 1,
            sample_rate: SAMPLE_RATE,
            flp_source: None,
        };
        // A useful empty project: one synth channel + one audio-ish sampler channel.
        let mut ch = ChannelInfo::new(p.next_channel_id, "Aurora", "Aurora", [95, 179, 108]);
        p.next_channel_id += 1;
        p.channels.push(ch.clone());
        p.patterns[0].channel_mut(ch.id, 16);
        for i in [0usize, 4, 8, 12] {
            if let Some(pc) = p.patterns[0].channels.first_mut() {
                pc.steps[i].active = true;
            }
        }
        ch = ChannelInfo::new(p.next_channel_id, "KickForge", "KickForge", [214, 160, 92]);
        p.next_channel_id += 1;
        p.mixer.strips[ch.mixer_index].name = ch.name.clone();
        p.channels.push(ch.clone());
        p.patterns[0].channel_mut(ch.id, 16);
        p.mixer.strips[0].name = p.channels[0].name.clone();
        p.mixer.strips[1].name = p.channels[1].name.clone();
        p
    }
}

impl StudioProject {
    pub fn pattern(&self) -> Option<&Pattern> {
        self.patterns.get(self.current_pattern)
    }
    pub fn pattern_mut(&mut self) -> Option<&mut Pattern> {
        self.patterns.get_mut(self.current_pattern)
    }
    pub fn channel(&self, id: u32) -> Option<&ChannelInfo> {
        self.channels.iter().find(|c| c.id == id)
    }
    pub fn channel_mut(&mut self, id: u32) -> Option<&mut ChannelInfo> {
        self.channels.iter_mut().find(|c| c.id == id)
    }
    pub fn add_channel(&mut self, synth_type: &str, name: &str, color: [u8; 3]) -> u32 {
        let id = self.next_channel_id;
        self.next_channel_id += 1;
        let mut ch = ChannelInfo::new(id, name, synth_type, color);
        ch.mixer_index = (self.channels.len()) % MAX_CHANNELS;
        self.mixer.strips[ch.mixer_index].name = name.to_string();
        self.mixer.strips[ch.mixer_index].color = color;
        // ensure all patterns have this channel
        for pat in &mut self.patterns {
            pat.channel_mut(id, pat.length_steps);
        }
        // fill defaults from a registry-provided count later (UI sets them)
        self.channels.push(ch);
        id
    }
    pub fn pattern_names(&self) -> Vec<String> {
        self.patterns.iter().map(|p| p.name.clone()).collect()
    }
    /// All automation values for a given tick as (target, value).
    pub fn automation_at(&self, tick: f64) -> Vec<(AutoTarget, f32)> {
        self.automation
            .iter()
            .filter(|a| a.enabled)
            .map(|a| (a.target.clone(), a.value_at(tick)))
            .collect()
    }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_roundtrip() {
        for db in [-60.0, -12.0, -3.0, 0.0, 6.0] {
            let g = db_to_gain(db);
            assert!((gain_to_db(g) - db).abs() < 1e-3);
        }
        assert_eq!(db_to_gain(-100.0), 0.0);
    }

    #[test]
    fn pan_is_constant_power() {
        for p in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            let (l, r) = pan_gains(p);
            assert!((l * l + r * r - 1.0).abs() < 1e-4);
        }
        let (l, r) = pan_gains(0.0);
        assert!((l - r).abs() < 1e-6);
    }

    #[test]
    fn note_frequency() {
        assert!((midi_note_hz(69.0) - 440.0).abs() < 1e-3);
        assert!((midi_note_hz(60.0) - 261.6256).abs() < 0.01);
    }

    #[test]
    fn param_curves_roundtrip() {
        for curve in [ParamCurve::Linear, ParamCurve::Pow2, ParamCurve::Sqrt, ParamCurve::Exp] {
            let p = ParamSpec::new("cutoff", 20.0, 20000.0, 1000.0, "Hz").curve(curve);
            for v in [20.0, 100.0, 1000.0, 12000.0, 20000.0] {
                let n = p.normalize(v);
                assert!((0.0..=1.0).contains(&n));
                let back = p.denormalize(n);
                assert!((back - v).abs() < 0.5, "{curve:?} {v} -> {n} -> {back}");
            }
        }
    }

    #[test]
    fn automation_curve() {
        let a = AutomationClipState {
            id: 1,
            name: "auto".into(),
            color: [0, 0, 0],
            target: AutoTarget::Tempo,
            points: vec![
                AutomationPoint { time: 0.0, value: 0.0, tension: 0.0 },
                AutomationPoint { time: 100.0, value: 1.0, tension: 0.0 },
            ],
            interp: Interp::Linear,
            enabled: true,
            length: 100,
        };
        assert!(a.value_at(0.0).abs() < 1e-6);
        assert!((a.value_at(50.0) - 0.5).abs() < 1e-4);
        assert!((a.value_at(100.0) - 1.0).abs() < 1e-6);
        assert!((a.value_at(500.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn project_basics() {
        let mut p = StudioProject::default();
        assert_eq!(p.channels.len(), 2);
        let id = p.add_channel("TripleOsc", "Lead", [200, 100, 100]);
        assert!(p.channel(id).is_some());
        assert_eq!(p.patterns[0].channels.len(), 3);
        let (bar, beat, _step) = tick_to_bar_beat(PPQ as u64 * 4);
        assert_eq!((bar, beat), (2, 1));
    }
}
