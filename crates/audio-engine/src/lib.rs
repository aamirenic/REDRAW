//! REDRAW audio engine: sample-accurate transport, lock-free command bus, voice pool,
//! pattern scheduling and the cpal/WASAPI session.
//!
//! Realtime contract: the audio callback performs no allocation, no locking and no
//! syscall on the continuous path. Structural changes (instrument/effect install) are
//! built on the control thread and swapped in at block boundaries; the displaced
//! objects travel back through a garbage ring and are dropped by the control thread.
#![forbid(unsafe_code)]

use redraw_arrangement::{build_arrangement, ArrangementSchedule};
pub use redraw_automation::AutomationSchedule;
use redraw_automation::{pan_value, tempo_bpm, volume_db};
use redraw_mixer::{InsertChain, MixerRt, Meters, SendRt, MAX_STRIPS};
use redraw_shared::{
    AutoTarget, FxSlotState, PluginFactory, StudioProject, Synth, BLOCK_SIZE, MAX_CHANNELS,
    MAX_INSERTS, MAX_SENDS, MAX_VOICES, PPQ,
};
use rtrb::{Consumer, Producer};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

pub const COMMAND_CAPACITY: usize = 4096;
pub const GARBAGE_CAPACITY: usize = 512;
/// Automation is evaluated at ~1.3 ms intervals at 48 kHz. This keeps tempo and
/// fader curves smooth without turning parameter setters into per-sample work.
pub const AUTOMATION_CONTROL_FRAMES: usize = 64;

// ------------------------------------------------------------------ transport

#[derive(Clone, Copy, Debug)]
pub struct TransportConfig {
    pub bpm: f32,
    pub time_sig: (u8, u8),
    pub swing: f32,
    pub loop_enabled: bool,
    pub loop_start: u64,
    pub loop_end: u64,
    pub pattern_index: usize,
    pub playing: bool,
    pub playlist_mode: bool,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            bpm: 140.0,
            time_sig: (4, 4),
            swing: 0.5,
            loop_enabled: false,
            loop_start: 0,
            loop_end: PPQ as u64 * 16,
            pattern_index: 0,
            playing: false,
            playlist_mode: false,
        }
    }
}

// ------------------------------------------------------------------ schedule
//
// Event scheduling lives in `redraw-sequencer` so the engine, arrangement and
// UI share one implementation. Re-exported here for existing callers.
pub use redraw_sequencer::{
    build_schedule, build_schedule_with, EventKind, ScheduleOptions, SchedEvent,
    ScheduledPattern,
};

// ------------------------------------------------------------------ graph

/// One channel in the realtime graph.
pub struct ChannelNode {
    pub id: u32,
    pub strip_index: usize,
    pub synth: Box<dyn Synth>,
}

impl ChannelNode {
    #[inline]
    pub fn synth(&self) -> &dyn Synth {
        &*self.synth
    }
}

/// Complete realtime render graph: channels + mixer. Built on the control thread.
pub struct EngineGraph {
    pub channels: Vec<ChannelNode>,
    pub mixer: MixerRt,
}

fn load_chain(
    chain: &mut InsertChain,
    slots: &[Option<FxSlotState>],
    factory: &dyn PluginFactory,
) {
    for (i, s) in slots.iter().take(MAX_INSERTS).enumerate() {
        if let Some(state) = s {
            if let Some(mut fx) = factory.create_effect(&state.effect_type) {
                let count = fx.get_param_count();
                for (pi, v) in state.params.iter().enumerate().take(count) {
                    fx.set_param(pi, *v);
                }
                fx.set_bypass(state.bypass);
                let _old = chain.set(i, Some(fx));
            }
        }
    }
}

impl EngineGraph {
    pub fn from_project(project: &StudioProject, factory: &dyn PluginFactory, sr: f32) -> Self {
        let mut mixer = MixerRt::new(MAX_STRIPS, sr);
        mixer.master_gain_db = project.mixer.master.volume_db;
        for (strip, st) in project.mixer.strips.iter().take(MAX_STRIPS).enumerate() {
            mixer.strips[strip].gain_db = st.volume_db;
            mixer.strips[strip].pan = st.pan;
            mixer.strips[strip].mute = st.mute;
            mixer.strips[strip].solo = st.solo;
            for s in 0..MAX_SENDS {
                mixer.strips[strip].sends[s] = SendRt {
                    level_db: st.sends[s].level_db,
                    pre_fader: st.sends[s].pre_fader,
                    target: st.sends[s].target,
                };
            }
            load_chain(&mut mixer.strips[strip].inserts, &st.inserts, factory);
        }
        load_chain(&mut mixer.master_inserts, &project.mixer.master.inserts, factory);
        for b in 0..MAX_SENDS {
            mixer.buses[b].gain_db = project.mixer.bus_volumes_db[b];
            load_chain(&mut mixer.buses[b].inserts, &project.mixer.bus_inserts[b], factory);
        }
        mixer.recompute_solo();

        let mut channels = Vec::with_capacity(MAX_CHANNELS);
        for ch in project.channels.iter().take(MAX_CHANNELS) {
            let mut synth = factory
                .create_synth(&ch.synth_type)
                .unwrap_or_else(|| Box::new(SilentSynth));
            synth.set_sample_rate(sr);
            let param_count = synth.get_param_count();
            for (i, v) in ch.params.iter().enumerate().take(param_count) {
                synth.set_param(i, *v);
            }
            let strip_index = (ch.mixer_index as usize).min(MAX_STRIPS - 1);
            channels.push(ChannelNode {
                id: ch.id,
                strip_index,
                synth,
            });
        }
        Self { channels, mixer }
    }

    pub fn slot_for(&self, channel_id: u32) -> Option<usize> {
        self.channels.iter().position(|c| c.id == channel_id)
    }
}

/// Fallback instrument for unknown types so the graph never breaks.
pub struct SilentSynth;

impl Default for SilentSynth {
    fn default() -> Self {
        Self
    }
}

impl Synth for SilentSynth {
    fn synth_type(&self) -> &'static str {
        "Silent"
    }
    fn reset(&mut self) {}
    fn note_on(&mut self, _note: u8, _velocity: f32) {}
    fn note_off(&mut self, _note: u8) {}
    fn all_notes_off(&mut self) {}
    fn render(&mut self, left: &mut [f32], right: &mut [f32], _sr: f32) {
        left.fill(0.0);
        right.fill(0.0);
    }
    fn params(&self) -> &'static [redraw_shared::ParamSpec] {
        &[]
    }
    fn set_param(&mut self, _i: usize, _v: f32) {}
    fn get_param(&self, _i: usize) -> f32 {
        0.0
    }
    fn active_voices(&self) -> usize {
        0
    }
}

// ------------------------------------------------------------------ voice pool

#[derive(Clone, Copy, Debug)]
struct ActiveVoice {
    channel: usize,
    key: u8,
    age: u64,
}

struct VoicePool {
    voices: Vec<ActiveVoice>,
    limit: usize,
}

impl VoicePool {
    fn new(limit: usize) -> Self {
        Self {
            voices: Vec::with_capacity(limit),
            limit,
        }
    }
    #[inline]
    fn contains(&self, channel: usize, key: u8) -> bool {
        self.voices
            .iter()
            .any(|v| v.channel == channel && v.key == key)
    }
    #[inline]
    fn remove(&mut self, channel: usize, key: u8) -> bool {
        if let Some(pos) = self
            .voices
            .iter()
            .position(|v| v.channel == channel && v.key == key)
        {
            self.voices.swap_remove(pos);
            true
        } else {
            false
        }
    }
    #[inline]
    fn note_on(&mut self, channel: usize, key: u8, age: u64) -> Option<ActiveVoice> {
        let mut stolen = None;
        if self.voices.len() >= self.limit && !self.voices.is_empty() {
            if let Some(pos) = self
                .voices
                .iter()
                .enumerate()
                .min_by_key(|(_, v)| v.age)
                .map(|(i, _)| i)
            {
                stolen = Some(self.voices.swap_remove(pos));
            }
        }
        self.voices.push(ActiveVoice { channel, key, age });
        stolen
    }
    fn clear(&mut self) {
        self.voices.clear();
    }
    fn len(&self) -> usize {
        self.voices.len()
    }
}

// ------------------------------------------------------------------ commands

pub enum Command {
    InstallGraph(Box<EngineGraph>),
    SetSchedules(Box<Vec<ScheduledPattern>>),
    SetArrangement(Box<ArrangementSchedule>),
    SetAutomation(Box<AutomationSchedule>),
    Play,
    Stop,
    Toggle,
    Seek(u64),
    SetBpm(f32),
    SetSwing(f32),
    SetLoop {
        enabled: bool,
        start: u64,
        end: u64,
    },
    SetPattern(usize),
    SetPlaylistMode(bool),
    NoteOn {
        channel: u32,
        key: u8,
        velocity: f32,
    },
    NoteOff {
        channel: u32,
        key: u8,
    },
    AllNotesOff,
    SetSynthParam {
        channel: u32,
        param: u32,
        value: f32,
    },
    SetChannelGain {
        channel: u32,
        db: f32,
    },
    SetChannelPan {
        channel: u32,
        pan: f32,
    },
    SetChannelMute {
        channel: u32,
        mute: bool,
    },
    SetChannelSolo {
        channel: u32,
        solo: bool,
    },
    SetSend {
        channel: u32,
        send: u8,
        level_db: f32,
        pre: bool,
        target: u8,
    },
    SetFxParam {
        channel: u32,
        slot: u8,
        param: u32,
        value: f32,
    },
    SetFxBypass {
        channel: u32,
        slot: u8,
        bypass: bool,
    },
    SetMasterGain(f32),
}

/// Displaced realtime objects. Never inspected: dropped by the control thread.
#[allow(dead_code)] // fields exist only so the box survives the ring buffer
enum Garbage {
    Graph(Box<EngineGraph>),
    Schedules(Box<Vec<ScheduledPattern>>),
    Arrangement(Box<ArrangementSchedule>),
    Automation(Box<AutomationSchedule>),
}

// ------------------------------------------------------------------ engine

pub struct EngineRt {
    sr: f32,
    cfg: TransportConfig,
    playing: bool,
    abs_pos: f64,
    pat_pos: f64,
    cursor: usize,
    graph: Option<Box<EngineGraph>>,
    schedules: Box<Vec<ScheduledPattern>>,
    arrangement: Box<ArrangementSchedule>,
    automation: Box<AutomationSchedule>,
    arr_cursor: usize,
    cmd_rx: Consumer<Command>,
    garbage_tx: Producer<Garbage>,
    meters: Arc<Meters>,
    voices: VoicePool,
    scratch_l: [f32; BLOCK_SIZE],
    scratch_r: [f32; BLOCK_SIZE],
    out_l: [f32; BLOCK_SIZE],
    out_r: [f32; BLOCK_SIZE],
    age: u64,
    cpu_avg: f32,
}

impl EngineRt {
    fn ticks_per_sample(&self) -> f64 {
        (self.cfg.bpm.max(1.0) as f64 * PPQ as f64) / (60.0 * self.sr.max(1.0) as f64)
    }
    fn pattern_length_ticks(&self) -> u64 {
        self.schedules
            .get(self.cfg.pattern_index)
            .map(|s| s.length)
            .unwrap_or(0)
    }
    fn first_event_at(&self, tick: f64) -> usize {
        self.schedules
            .get(self.cfg.pattern_index)
            .map(|s| {
                s.events
                    .partition_point(|e| (e.tick as f64) < tick - 1e-9)
            })
            .unwrap_or(0)
    }
    fn next_event_tick(&self, length: u64) -> f64 {
        match self.schedules.get(self.cfg.pattern_index) {
            Some(s) if self.cursor < s.events.len() => {
                (s.events[self.cursor].tick as f64).max(self.pat_pos)
            }
            _ => length as f64,
        }
    }
    fn fire_due(&mut self) {
        loop {
            let ev = self
                .schedules
                .get(self.cfg.pattern_index)
                .and_then(|s| s.events.get(self.cursor))
                .copied();
            match ev {
                Some(ev) if (ev.tick as f64) <= self.pat_pos + 1e-6 => {
                    self.cursor += 1;
                    self.dispatch(ev);
                }
                _ => break,
            }
        }
    }
    fn arr_first_event_at(&self, tick: f64) -> usize {
        self.arrangement
            .events
            .partition_point(|e| (e.tick as f64) < tick - 1e-9)
    }
    fn arr_next_event_tick(&self, limit: u64) -> f64 {
        match self.arrangement.events.get(self.arr_cursor) {
            Some(ev) => (ev.tick as f64).max(self.abs_pos),
            None => limit as f64,
        }
    }
    fn fire_arr_due(&mut self) {
        loop {
            let ev = self.arrangement.events.get(self.arr_cursor).copied();
            match ev {
                Some(ev) if (ev.tick as f64) <= self.abs_pos + 1e-6 => {
                    self.arr_cursor += 1;
                    self.dispatch(ev);
                }
                _ => break,
            }
        }
    }
    fn rewind(&mut self) {
        self.all_notes_off();
        self.abs_pos = 0.0;
        self.pat_pos = 0.0;
        self.cursor = 0;
        self.arr_cursor = 0;
    }
    fn loop_active(&self) -> bool {
        self.cfg.loop_enabled && self.cfg.loop_end > self.cfg.loop_start
    }
    fn at_song_end(&self) -> bool {
        self.cfg.playlist_mode
            && !self.loop_active()
            && self.arrangement.length > 0
            && self.abs_pos >= self.arrangement.length as f64 - 1e-9
    }
    fn dispatch(&mut self, ev: SchedEvent) {
        let slot = match self.graph.as_ref().and_then(|g| g.slot_for(ev.channel)) {
            Some(s) => s,
            None => return,
        };
        match ev.kind {
            EventKind::NoteOn => self.trigger(slot, ev.key, ev.velocity as f32 / 127.0),
            EventKind::NoteOff => self.release(slot, ev.key),
        }
    }
    fn trigger(&mut self, slot: usize, key: u8, velocity: f32) {
        if self.voices.contains(slot, key) {
            if let Some(graph) = self.graph.as_mut() {
                if let Some(ch) = graph.channels.get_mut(slot) {
                    ch.synth.note_off(key);
                }
            }
            self.voices.remove(slot, key);
        }
        if let Some(stolen) = self.voices.note_on(slot, key, self.age) {
            if let Some(graph) = self.graph.as_mut() {
                if let Some(ch) = graph.channels.get_mut(stolen.channel) {
                    ch.synth.note_off(stolen.key);
                }
            }
        }
        self.age = self.age.wrapping_add(1);
        if let Some(graph) = self.graph.as_mut() {
            if let Some(ch) = graph.channels.get_mut(slot) {
                ch.synth.note_on(key, velocity);
            }
        }
    }
    fn release(&mut self, slot: usize, key: u8) {
        if self.voices.remove(slot, key) {
            if let Some(graph) = self.graph.as_mut() {
                if let Some(ch) = graph.channels.get_mut(slot) {
                    ch.synth.note_off(key);
                }
            }
        }
    }
    fn all_notes_off(&mut self) {
        if let Some(graph) = self.graph.as_mut() {
            for ch in &mut graph.channels {
                ch.synth.all_notes_off();
            }
        }
        self.voices.clear();
    }

    /// Pattern automation follows the looping pattern position; playlist
    /// automation follows the absolute song position.
    fn automation_tick(&self) -> f64 {
        if self.cfg.playlist_mode || self.pattern_length_ticks() == 0 {
            self.abs_pos
        } else {
            self.pat_pos
        }
    }

    /// Apply every lane at `tick`. Lanes are visited in project order, so a later
    /// lane targeting the same control wins. No allocation or locking here.
    fn apply_automation_at(&mut self, tick: f64) {
        if self.automation.is_empty() {
            return;
        }
        let Some(graph) = self.graph.as_mut() else {
            return;
        };
        let mut next_bpm = None;
        for lane in &self.automation.lanes {
            let value = lane.value_at(tick);
            match &lane.target {
                AutoTarget::Tempo => next_bpm = Some(tempo_bpm(value)),
                AutoTarget::MasterVolume => {
                    graph.mixer.master_gain_db = volume_db(value);
                }
                AutoTarget::ChannelVolume { channel_id } => {
                    if let Some(slot) = graph.slot_for(*channel_id) {
                        let strip = graph.channels[slot].strip_index;
                        graph.mixer.set_gain_db(strip, volume_db(value));
                    }
                }
                AutoTarget::ChannelPan { channel_id } => {
                    if let Some(slot) = graph.slot_for(*channel_id) {
                        let strip = graph.channels[slot].strip_index;
                        graph.mixer.set_pan(strip, pan_value(value));
                    }
                }
                AutoTarget::ChannelParam { channel_id, param } => {
                    if let Some(slot) = graph.slot_for(*channel_id) {
                        let spec = graph.channels[slot]
                            .synth
                            .params()
                            .get(*param as usize)
                            .copied();
                        if let Some(spec) = spec {
                            graph.channels[slot]
                                .synth
                                .set_param(*param as usize, spec.denormalize(value));
                        }
                    }
                }
                AutoTarget::FxParam {
                    channel_id,
                    slot,
                    param,
                } => {
                    if let Some(channel) = graph.slot_for(*channel_id) {
                        let strip = graph.channels[channel].strip_index;
                        let spec = graph
                            .mixer
                            .strips
                            .get(strip)
                            .and_then(|s| s.inserts.get(*slot as usize))
                            .and_then(|fx| fx.params().get(*param as usize).copied());
                        if let Some(spec) = spec {
                            if let Some(strip) = graph.mixer.strips.get_mut(strip) {
                                strip.inserts.set_param(
                                    *slot as usize,
                                    *param as usize,
                                    spec.denormalize(value),
                                );
                            }
                        }
                    }
                }
                AutoTarget::SendLevel { channel_id, send } => {
                    if let Some(channel) = graph.slot_for(*channel_id) {
                        let strip = graph.channels[channel].strip_index;
                        let current = graph
                            .mixer
                            .strips
                            .get(strip)
                            .and_then(|s| s.sends.get(*send as usize))
                            .copied();
                        if let Some(current) = current {
                            graph.mixer.set_send(
                                strip,
                                *send as usize,
                                volume_db(value),
                                current.pre_fader,
                                current.target,
                            );
                        }
                    }
                }
            }
        }
        if let Some(bpm) = next_bpm {
            self.cfg.bpm = bpm.clamp(10.0, 999.0);
        }
    }

    fn drain_commands(&mut self) {
        let mut count = 0;
        while let Ok(cmd) = self.cmd_rx.pop() {
            self.apply(cmd);
            count += 1;
            if count >= 1024 {
                break;
            }
        }
    }

    fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::InstallGraph(mut new_graph) => {
                self.all_notes_off();
                // Keep the meters shared with the control handle. A rebuilt graph
                // owns a fresh MixerRt and would otherwise publish peaks to an Arc
                // the UI never reads.
                new_graph.mixer.meters = self.meters.clone();
                let old = self.graph.replace(new_graph);
                if let Some(old) = old {
                    if let Err(g) = self.garbage_tx.push(Garbage::Graph(old)) {
                        std::mem::forget(g);
                    }
                }
                if !self.schedules.is_empty() {
                    self.cursor = self.first_event_at(self.pat_pos);
                }
            }
            Command::SetSchedules(sched) => {
                let old = std::mem::replace(&mut self.schedules, sched);
                if let Err(g) = self.garbage_tx.push(Garbage::Schedules(old)) {
                    std::mem::forget(g);
                }
                self.cursor = self.first_event_at(self.pat_pos);
            }
            Command::SetArrangement(arr) => {
                let old = std::mem::replace(&mut self.arrangement, arr);
                if let Err(g) = self.garbage_tx.push(Garbage::Arrangement(old)) {
                    std::mem::forget(g);
                }
                self.arr_cursor = self.arr_first_event_at(self.abs_pos);
            }
            Command::SetAutomation(schedule) => {
                let old = std::mem::replace(&mut self.automation, schedule);
                if let Err(g) = self.garbage_tx.push(Garbage::Automation(old)) {
                    std::mem::forget(g);
                }
                self.apply_automation_at(self.automation_tick());
            }
            Command::Play => {
                if self.at_song_end() {
                    self.rewind();
                }
                self.playing = true;
            }
            Command::Stop => {
                self.playing = false;
                self.all_notes_off();
            }
            Command::Toggle => {
                if self.playing {
                    self.playing = false;
                    self.all_notes_off();
                } else {
                    if self.at_song_end() {
                        self.rewind();
                    }
                    self.playing = true;
                }
            }
            Command::Seek(tick) => {
                self.all_notes_off();
                self.abs_pos = tick as f64;
                if self.cfg.playlist_mode {
                    self.arr_cursor = self.arr_first_event_at(self.abs_pos);
                } else {
                    let len = self.pattern_length_ticks();
                    self.pat_pos = if len > 0 {
                        (tick as f64) % len as f64
                    } else {
                        0.0
                    };
                    self.cursor = self.first_event_at(self.pat_pos);
                }
                self.apply_automation_at(self.automation_tick());
            }
            Command::SetBpm(bpm) => self.cfg.bpm = bpm.clamp(10.0, 999.0),
            Command::SetSwing(s) => self.cfg.swing = s.clamp(0.0, 1.0),
            Command::SetLoop {
                enabled,
                start,
                end,
            } => {
                self.cfg.loop_enabled = enabled;
                self.cfg.loop_start = start;
                self.cfg.loop_end = end;
            }
            Command::SetPattern(idx) => {
                self.cfg.pattern_index = idx.min(self.schedules.len().saturating_sub(1));
                self.pat_pos = 0.0;
                self.cursor = 0;
                self.all_notes_off();
                if !self.cfg.playlist_mode {
                    self.abs_pos = 0.0;
                }
                self.apply_automation_at(self.automation_tick());
            }
            Command::SetPlaylistMode(on) => {
                if self.cfg.playlist_mode != on {
                    self.cfg.playlist_mode = on;
                    self.rewind();
                    self.apply_automation_at(self.automation_tick());
                }
            }
            Command::NoteOn {
                channel,
                key,
                velocity,
            } => {
                if let Some(slot) = self.graph.as_ref().and_then(|g| g.slot_for(channel)) {
                    self.trigger(slot, key, velocity);
                }
            }
            Command::NoteOff { channel, key } => {
                if let Some(slot) = self.graph.as_ref().and_then(|g| g.slot_for(channel)) {
                    self.release(slot, key);
                }
            }
            Command::AllNotesOff => self.all_notes_off(),
            Command::SetSynthParam { channel, param, value } => {
                if let Some(graph) = self.graph.as_mut() {
                    if let Some(slot) = graph.slot_for(channel) {
                        graph.channels[slot].synth.set_param(param as usize, value);
                    }
                }
            }
            Command::SetChannelGain { channel, db } => {
                if let Some(graph) = self.graph.as_mut() {
                    if let Some(slot) = graph.slot_for(channel) {
                        let strip = graph.channels[slot].strip_index;
                        graph.mixer.set_gain_db(strip, db);
                    }
                }
            }
            Command::SetChannelPan { channel, pan } => {
                if let Some(graph) = self.graph.as_mut() {
                    if let Some(slot) = graph.slot_for(channel) {
                        let strip = graph.channels[slot].strip_index;
                        graph.mixer.set_pan(strip, pan);
                    }
                }
            }
            Command::SetChannelMute { channel, mute } => {
                if let Some(graph) = self.graph.as_mut() {
                    if let Some(slot) = graph.slot_for(channel) {
                        let strip = graph.channels[slot].strip_index;
                        graph.mixer.set_mute(strip, mute);
                    }
                }
            }
            Command::SetChannelSolo { channel, solo } => {
                if let Some(graph) = self.graph.as_mut() {
                    if let Some(slot) = graph.slot_for(channel) {
                        let strip = graph.channels[slot].strip_index;
                        graph.mixer.set_solo(strip, solo);
                    }
                }
            }
            Command::SetSend {
                channel,
                send,
                level_db,
                pre,
                target,
            } => {
                if let Some(graph) = self.graph.as_mut() {
                    if let Some(slot) = graph.slot_for(channel) {
                        let strip = graph.channels[slot].strip_index;
                        graph.mixer.set_send(strip, send as usize, level_db, pre, target);
                    }
                }
            }
            Command::SetFxParam {
                channel,
                slot,
                param,
                value,
            } => {
                if let Some(graph) = self.graph.as_mut() {
                    if let Some(ch) = graph.slot_for(channel) {
                        let strip = graph.channels[ch].strip_index;
                        graph
                            .mixer
                            .strips
                            .get_mut(strip)
                            .map(|s| s.inserts.set_param(slot as usize, param as usize, value));
                    }
                }
            }
            Command::SetFxBypass {
                channel,
                slot,
                bypass,
            } => {
                if let Some(graph) = self.graph.as_mut() {
                    if let Some(ch) = graph.slot_for(channel) {
                        let strip = graph.channels[ch].strip_index;
                        graph
                            .mixer
                            .strips
                            .get_mut(strip)
                            .map(|s| s.inserts.set_bypass(slot as usize, bypass));
                    }
                }
            }
            Command::SetMasterGain(db) => {
                if let Some(graph) = self.graph.as_mut() {
                    graph.mixer.master_gain_db = db.clamp(-90.0, 12.0);
                }
            }
        }
    }

    /// Render `frames` frames of silent/decaying audio (when stopped).
    fn render_segment(&mut self, frames: usize) {
        if frames == 0 {
            return;
        }
        let Some(graph) = self.graph.as_mut() else {
            self.scratch_l[..frames].fill(0.0);
            self.scratch_r[..frames].fill(0.0);
            return;
        };
        let sr = self.sr;
        let bpm = self.cfg.bpm;
        {
            let channels = &mut graph.channels;
            let mixer = &mut graph.mixer;
            for node in channels.iter_mut() {
                node.synth.render(&mut self.scratch_l[..frames], &mut self.scratch_r[..frames], sr);
                mixer.process_strip(
                    node.strip_index,
                    &mut self.scratch_l[..frames],
                    &mut self.scratch_r[..frames],
                    sr,
                    bpm,
                );
            }
        }
    }

    fn render_playing(&mut self, n: usize) {
        if self.cfg.playlist_mode {
            self.render_playlist(n);
        } else {
            self.render_pattern(n);
        }
    }

    /// Pattern mode: the current pattern loops forever.
    fn render_pattern(&mut self, n: usize) {
        let len = self.pattern_length_ticks();
        if len == 0 {
            let tps = self.ticks_per_sample();
            self.render_segment(n);
            self.abs_pos += n as f64 * tps;
            return;
        }
        let tps = self.ticks_per_sample();
        let mut produced = 0usize;
        let mut guard = 0;
        while produced < n && guard < 65_536 {
            guard += 1;
            self.fire_due();
            if self.pat_pos >= len as f64 - 1e-9 {
                self.pat_pos = 0.0;
                self.cursor = 0;
                continue;
            }
            let next = self.next_event_tick(len);
            let delta = (next - self.pat_pos).max(0.0);
            let mut frames = if delta <= 1e-9 {
                1
            } else {
                (delta / tps).ceil() as usize
            };
            if frames == 0 {
                frames = 1;
            }
            let frames = frames.min(n - produced);
            self.render_segment(frames);
            produced += frames;
            let adv = frames as f64 * tps;
            self.pat_pos += adv;
            self.abs_pos += adv;
        }
        if produced < n {
            let rest = n - produced;
            self.render_segment(rest);
            let adv = rest as f64 * tps;
            self.pat_pos += adv;
            self.abs_pos += adv;
        }
    }

    /// Playlist mode: absolute-tick events. With loop enabled the transport wraps
    /// at `loop_end` to `loop_start`; otherwise playback stops at the song end.
    fn render_playlist(&mut self, n: usize) {
        let tps = self.ticks_per_sample();
        let loop_on = self.loop_active();
        let loop_start = self.cfg.loop_start;
        let loop_end = self.cfg.loop_end;
        let song_end = self.arrangement.length;

        // Empty song with no loop: keep the clock running silently.
        if !loop_on && song_end == 0 {
            self.render_segment(n);
            self.abs_pos += n as f64 * tps;
            return;
        }

        let mut produced = 0usize;
        let mut guard = 0;
        while produced < n && guard < 65_536 {
            guard += 1;
            if loop_on {
                if self.abs_pos >= loop_end as f64 - 1e-9 {
                    // Notes at the loop seam are cut; the next pass starts clean.
                    // Carry any sub-tick overshoot so odd tempos do not drift.
                    let span = (loop_end - loop_start) as f64;
                    let overflow = if span > 0.0 {
                        (self.abs_pos - loop_end as f64) % span
                    } else {
                        0.0
                    };
                    self.all_notes_off();
                    self.abs_pos = loop_start as f64 + overflow;
                    // Re-arm from the loop start, not the carried position, so a
                    // note exactly on the seam still retriggers.
                    self.arr_cursor = self.arr_first_event_at(loop_start as f64);
                    continue;
                }
            } else if self.abs_pos >= song_end as f64 - 1e-9 {
                self.playing = false;
                self.all_notes_off();
                break;
            }

            self.fire_arr_due();

            let limit = if loop_on { loop_end } else { song_end };
            let boundary = self.arr_next_event_tick(limit).min(limit as f64);
            let delta = (boundary - self.abs_pos).max(0.0);
            let mut frames = if delta <= 1e-9 {
                1
            } else {
                (delta / tps).ceil() as usize
            };
            if frames == 0 {
                frames = 1;
            }
            let frames = frames.min(n - produced);
            self.render_segment(frames);
            produced += frames;
            self.abs_pos += frames as f64 * tps;
            if !loop_on && self.abs_pos >= song_end as f64 - 1e-9 {
                self.playing = false;
                self.all_notes_off();
                break;
            }
        }
        if produced < n {
            let rest = n - produced;
            self.render_segment(rest);
            if self.playing {
                self.abs_pos += rest as f64 * tps;
            }
        }
    }

    fn process_block(&mut self, frames: usize) {
        let t0 = Instant::now();
        self.drain_commands();
        let n = frames.min(BLOCK_SIZE);
        if let Some(graph) = self.graph.as_mut() {
            graph.mixer.begin_block();
        }
        if self.playing {
            if self.automation.is_empty() {
                self.render_playing(n);
            } else {
                // Subdivide the block so tempo/level/parameter automation is
                // refreshed at control rate while note scheduling stays exact.
                let mut produced = 0usize;
                while produced < n {
                    if !self.playing {
                        self.render_segment(n - produced);
                        break;
                    }
                    self.apply_automation_at(self.automation_tick());
                    let chunk = (n - produced).min(AUTOMATION_CONTROL_FRAMES);
                    self.render_playing(chunk);
                    produced += chunk;
                }
            }
        } else {
            self.render_segment(n);
        }
        if let Some(graph) = self.graph.as_mut() {
            graph
                .mixer
                .finish(&mut self.out_l[..n], &mut self.out_r[..n], n, self.sr, self.cfg.bpm);
            graph.mixer.publish_meters();
        } else {
            self.out_l[..n].fill(0.0);
            self.out_r[..n].fill(0.0);
        }
        self.meters.store_position(self.abs_pos);
        self.meters.set_playing(self.playing);
        self.meters.set_voices(self.voices.len());
        let elapsed = t0.elapsed().as_secs_f32();
        let budget = n as f32 / self.sr.max(1.0);
        let load = if budget > 0.0 { elapsed / budget } else { 0.0 };
        self.cpu_avg = self.cpu_avg * 0.9 + load.min(4.0) * 0.1;
        self.meters
            .cpu_permille
            .store((self.cpu_avg * 1000.0) as u32, Ordering::Relaxed);
    }

    /// Process interleaved stereo f32 audio (any buffer size, chunked to the rt block).
    pub fn process_interleaved(&mut self, data: &mut [f32]) {
        let frames = data.len() / 2;
        let mut off = 0usize;
        while off < frames {
            let n = (frames - off).min(BLOCK_SIZE);
            self.process_block(n);
            for i in 0..n {
                data[(off + i) * 2] = self.out_l[i];
                data[(off + i) * 2 + 1] = self.out_r[i];
            }
            off += n;
        }
        if data.len() % 2 == 1 {
            data[data.len() - 1] = 0.0;
        }
    }

    /// Offline/exports: process exactly one planar block (≤ 512 frames), returns slices.
    #[allow(clippy::mut_range_end)]
    pub fn render_planar(&mut self, frames: usize) -> (&[f32], &[f32]) {
        let n = frames.min(BLOCK_SIZE);
        self.process_block(n);
        (&self.out_l[..n], &self.out_r[..n])
    }

    pub fn position_ticks(&self) -> f64 {
        self.abs_pos
    }
    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }
    pub fn is_playing(&self) -> bool {
        self.playing
    }
    pub fn meters(&self) -> Arc<Meters> {
        self.meters.clone()
    }
}

/// Create a realtime engine plus its control-side handle.
pub fn create_engine(
    sr: f32,
    mut graph: EngineGraph,
    schedules: Vec<ScheduledPattern>,
    cfg: TransportConfig,
    playing: bool,
) -> (EngineRt, EngineHandle) {
    graph.mixer.set_sample_rate(sr);
    let (cmd_tx, cmd_rx) = rtrb::RingBuffer::<Command>::new(COMMAND_CAPACITY);
    let (garbage_tx, garbage_rx) = rtrb::RingBuffer::<Garbage>::new(GARBAGE_CAPACITY);
    let meters = graph.mixer.meters.clone();
    let engine = EngineRt {
        sr,
        cfg,
        playing,
        abs_pos: 0.0,
        pat_pos: 0.0,
        cursor: 0,
        graph: Some(Box::new(graph)),
        schedules: Box::new(schedules),
        arrangement: Box::new(ArrangementSchedule::default()),
        automation: Box::new(AutomationSchedule::default()),
        arr_cursor: 0,
        cmd_rx,
        garbage_tx,
        meters: meters.clone(),
        voices: VoicePool::new(MAX_VOICES),
        scratch_l: [0.0; BLOCK_SIZE],
        scratch_r: [0.0; BLOCK_SIZE],
        out_l: [0.0; BLOCK_SIZE],
        out_r: [0.0; BLOCK_SIZE],
        age: 0,
        cpu_avg: 0.0,
    };
    let handle = EngineHandle {
        cmd_tx,
        garbage_rx,
        meters,
        sample_rate: sr as u32,
    };
    (engine, handle)
}

// ------------------------------------------------------------------ handle

pub struct EngineHandle {
    cmd_tx: Producer<Command>,
    garbage_rx: Consumer<Garbage>,
    meters: Arc<Meters>,
    pub sample_rate: u32,
}

impl EngineHandle {
    fn send(&mut self, cmd: Command) {
        let _ = self.cmd_tx.push(cmd);
    }
    pub fn meters(&self) -> Arc<Meters> {
        self.meters.clone()
    }
    pub fn position(&self) -> f64 {
        self.meters.position()
    }
    pub fn is_playing(&self) -> bool {
        self.meters.is_playing()
    }
    pub fn play(&mut self) {
        self.send(Command::Play)
    }
    pub fn stop(&mut self) {
        self.send(Command::Stop)
    }
    pub fn toggle(&mut self) {
        self.send(Command::Toggle)
    }
    pub fn seek(&mut self, tick: u64) {
        self.send(Command::Seek(tick))
    }
    pub fn set_bpm(&mut self, bpm: f32) {
        self.send(Command::SetBpm(bpm))
    }
    pub fn set_swing(&mut self, swing: f32) {
        self.send(Command::SetSwing(swing))
    }
    pub fn set_loop(&mut self, enabled: bool, start: u64, end: u64) {
        self.send(Command::SetLoop {
            enabled,
            start,
            end,
        })
    }
    pub fn set_pattern(&mut self, index: usize) {
        self.send(Command::SetPattern(index))
    }
    pub fn set_playlist_mode(&mut self, on: bool) {
        self.send(Command::SetPlaylistMode(on))
    }
    pub fn note_on(&mut self, channel: u32, key: u8, velocity: f32) {
        self.send(Command::NoteOn {
            channel,
            key,
            velocity,
        })
    }
    pub fn note_off(&mut self, channel: u32, key: u8) {
        self.send(Command::NoteOff { channel, key })
    }
    pub fn all_notes_off(&mut self) {
        self.send(Command::AllNotesOff)
    }
    pub fn set_synth_param(&mut self, channel: u32, param: u32, value: f32) {
        self.send(Command::SetSynthParam { channel, param, value })
    }
    pub fn set_channel_gain(&mut self, channel: u32, db: f32) {
        self.send(Command::SetChannelGain { channel, db })
    }
    pub fn set_channel_pan(&mut self, channel: u32, pan: f32) {
        self.send(Command::SetChannelPan { channel, pan })
    }
    pub fn set_channel_mute(&mut self, channel: u32, mute: bool) {
        self.send(Command::SetChannelMute { channel, mute })
    }
    pub fn set_channel_solo(&mut self, channel: u32, solo: bool) {
        self.send(Command::SetChannelSolo { channel, solo })
    }
    pub fn set_send(&mut self, channel: u32, send: u8, level_db: f32, pre: bool, target: u8) {
        self.send(Command::SetSend {
            channel,
            send,
            level_db,
            pre,
            target,
        })
    }
    pub fn set_fx_param(&mut self, channel: u32, slot: u8, param: u32, value: f32) {
        self.send(Command::SetFxParam {
            channel,
            slot,
            param,
            value,
        })
    }
    pub fn set_fx_bypass(&mut self, channel: u32, slot: u8, bypass: bool) {
        self.send(Command::SetFxBypass {
            channel,
            slot,
            bypass,
        })
    }
    pub fn set_master_gain(&mut self, db: f32) {
        self.send(Command::SetMasterGain(db))
    }
    pub fn install_graph(&mut self, graph: EngineGraph) {
        self.send(Command::InstallGraph(Box::new(graph)))
    }
    pub fn set_schedules(&mut self, schedules: Vec<ScheduledPattern>) {
        self.send(Command::SetSchedules(Box::new(schedules)))
    }
    pub fn set_arrangement(&mut self, arrangement: ArrangementSchedule) {
        self.send(Command::SetArrangement(Box::new(arrangement)))
    }
    pub fn set_automation(&mut self, automation: AutomationSchedule) {
        self.send(Command::SetAutomation(Box::new(automation)))
    }
    /// Rebuild the graph, pattern schedules and playlist from a project
    /// (control thread).
    pub fn set_project(&mut self, project: &StudioProject, factory: &dyn PluginFactory) {
        let graph = EngineGraph::from_project(project, factory, self.sample_rate as f32);
        let schedules = build_schedule(&project.patterns, project.swing);
        let arrangement = build_arrangement(project);
        let automation = AutomationSchedule::from_project(project);
        self.install_graph(graph);
        self.set_schedules(schedules);
        self.set_arrangement(arrangement);
        self.send(Command::SetBpm(project.bpm));
        self.send(Command::SetSwing(project.swing));
        // Install the new schedule before selecting the pattern: SetPattern
        // applies automation at the new play position, and the previous project's
        // schedule must not touch the freshly built graph.
        self.set_automation(automation);
        self.send(Command::SetPattern(project.current_pattern));
        self.send(Command::SetLoop {
            enabled: project.playlist.loop_enabled,
            start: project.playlist.loop_start,
            end: project.playlist.loop_end,
        });
    }
    /// Drain displaced graphs/schedules here so the audio thread never drops.
    pub fn collect_garbage(&mut self) {
        while let Ok(g) = self.garbage_rx.pop() {
            drop(g);
        }
    }
}

// ------------------------------------------------------------------ session

pub struct AudioSession {
    pub handle: EngineHandle,
    pub device_name: String,
    pub sample_rate: u32,
    pub block_size: u32,
    _stream: cpal::Stream,
}

/// Start the realtime engine on the default output device.
/// Prefers 48 kHz stereo f32 with a fixed 512-frame buffer, falling back to the
/// device default sample rate/buffer where the platform declines.
pub fn start_audio(
    graph: EngineGraph,
    schedules: Vec<ScheduledPattern>,
    cfg: TransportConfig,
) -> Result<AudioSession, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("No audio output device found")?;
    let device_name = device.name().unwrap_or_else(|_| "Output".into());

    let mut chosen: Option<cpal::SupportedStreamConfig> = None;
    let mut fallback: Option<cpal::SupportedStreamConfig> = None;
    if let Ok(configs) = device.supported_output_configs() {
        for c in configs {
            if c.channels() != 2 || c.sample_format() != cpal::SampleFormat::F32 {
                continue;
            }
            if fallback.is_none() {
                fallback = Some(c.with_max_sample_rate());
            }
            if c.min_sample_rate().0 <= 48_000 && c.max_sample_rate().0 >= 48_000 {
                chosen = Some(c.with_sample_rate(cpal::SampleRate(48_000)));
                break;
            }
        }
    }
    let supported = chosen
        .or(fallback)
        .or_else(|| device.default_output_config().ok())
        .ok_or("Device exposes no stereo f32 output configuration")?;

    // Probe candidate buffer sizes without installing the engine graph.
    let mut workable: Option<cpal::StreamConfig> = None;
    let mut last_err = String::new();
    for bs in [cpal::BufferSize::Fixed(512), cpal::BufferSize::Default] {
        let mut probe_cfg = supported.config();
        probe_cfg.buffer_size = bs;
        match device.build_output_stream::<f32, _, _>(
            &probe_cfg,
            |data: &mut [f32], _| {
                for x in data {
                    *x = 0.0;
                }
            },
            |_err| {},
            None,
        ) {
            Ok(probe) => {
                drop(probe);
                workable = Some(probe_cfg);
                break;
            }
            Err(e) => last_err = e.to_string(),
        }
    }
    let stream_config =
        workable.ok_or_else(|| format!("Could not open the audio output: {last_err}"))?;

    let sr = stream_config.sample_rate.0 as f32;
    let playing = cfg.playing;
    let (mut engine, handle) = create_engine(sr, graph, schedules, cfg, playing);
    let block_size = match stream_config.buffer_size {
        cpal::BufferSize::Fixed(v) => v,
        cpal::BufferSize::Default => 0,
    };
    let stream = device
        .build_output_stream::<f32, _, _>(
            &stream_config,
            move |data: &mut [f32], _| engine.process_interleaved(data),
            move |err| eprintln!("[redraw] audio stream error: {err}"),
            None,
        )
        .map_err(|e| format!("Stream build failed: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("Stream start failed: {e}"))?;
    Ok(AudioSession {
        handle,
        device_name,
        sample_rate: sr as u32,
        block_size,
        _stream: stream,
    })
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use redraw_automation::{add_clip, find_clip_mut, set_points, tempo_normalized};
    use redraw_dsp::{wrap01, Analyzer, TWO_PI};
    use redraw_shared::{midi_note_hz, AutomationPoint, Effect, Note, ParamSpec, Pattern};

    const TEST_SINE_PARAMS: [ParamSpec; 1] = [ParamSpec::new("level", 0.0, 100.0, 0.0, "")];
    const TEST_FX_PARAMS: [ParamSpec; 1] = [ParamSpec::new("mix", 0.0, 100.0, 0.0, "%")];

    struct TestEffect {
        param: f32,
        bypass: bool,
    }

    impl Default for TestEffect {
        fn default() -> Self {
            Self {
                param: 0.0,
                bypass: false,
            }
        }
    }

    impl Effect for TestEffect {
        fn effect_type(&self) -> &'static str {
            "TestFX"
        }
        fn reset(&mut self) {}
        fn process(&mut self, _left: &mut [f32], _right: &mut [f32], _sr: f32, _tempo: f32) {}
        fn params(&self) -> &'static [ParamSpec] {
            &TEST_FX_PARAMS
        }
        fn set_param(&mut self, index: usize, value: f32) {
            if index == 0 {
                self.param = value;
            }
        }
        fn get_param(&self, index: usize) -> f32 {
            if index == 0 {
                self.param
            } else {
                0.0
            }
        }
        fn set_bypass(&mut self, bypass: bool) {
            self.bypass = bypass;
        }
        fn is_bypassed(&self) -> bool {
            self.bypass
        }
        fn controller_value(&self) -> f32 {
            0.0
        }
    }

    /// Minimal test instrument: sustained sine at the played pitch.
    struct TestSine {
        phase: f32,
        freq: f32,
        active: bool,
        sr: f32,
        param: f32,
    }

    impl Default for TestSine {
        fn default() -> Self {
            Self {
                phase: 0.0,
                freq: 440.0,
                active: false,
                sr: 48_000.0,
                param: 0.0,
            }
        }
    }

    impl Synth for TestSine {
        fn synth_type(&self) -> &'static str {
            "Sine"
        }
        fn reset(&mut self) {
            self.active = false;
        }
        fn note_on(&mut self, note: u8, _velocity: f32) {
            self.freq = midi_note_hz(note as f32);
            self.active = true;
        }
        fn note_off(&mut self, _note: u8) {
            self.active = false;
        }
        fn all_notes_off(&mut self) {
            self.active = false;
        }
        fn render(&mut self, left: &mut [f32], right: &mut [f32], _sr: f32) {
            let n = left.len().min(right.len());
            for i in 0..n {
                let v = if self.active {
                    (TWO_PI * self.phase).sin() * 0.25
                } else {
                    0.0
                };
                left[i] = v;
                right[i] = v;
                self.phase = wrap01(self.phase + self.freq / self.sr);
            }
        }
        fn params(&self) -> &'static [redraw_shared::ParamSpec] {
            &TEST_SINE_PARAMS
        }
        fn set_param(&mut self, index: usize, value: f32) {
            if index == 0 {
                self.param = value;
            }
        }
        fn get_param(&self, index: usize) -> f32 {
            if index == 0 {
                self.param
            } else {
                0.0
            }
        }
        fn active_voices(&self) -> usize {
            self.active as usize
        }
    }

    struct TestFactory;

    impl PluginFactory for TestFactory {
        fn create_synth(&self, _name: &str) -> Option<Box<dyn Synth>> {
            Some(Box::new(TestSine::default()))
        }
        fn create_effect(&self, _name: &str) -> Option<Box<dyn Effect>> {
            None
        }
        fn synth_names(&self) -> &'static [&'static str] {
            &["Sine"]
        }
        fn effect_names(&self) -> &'static [&'static str] {
            &[]
        }
    }

    fn test_graph(channels: usize) -> EngineGraph {
        let mixer = MixerRt::new(64, 48_000.0);
        let nodes = (0..channels)
            .map(|i| {
                let mut synth = TestSine::default();
                synth.sr = 48_000.0;
                ChannelNode {
                    id: i as u32 + 1,
                    strip_index: i,
                    synth: Box::new(synth),
                }
            })
            .collect();
        EngineGraph { channels: nodes, mixer }
    }

    fn test_graph_with_fx() -> EngineGraph {
        let mut graph = test_graph(1);
        let _ = graph.mixer.strips[0]
            .inserts
            .set(0, Some(Box::new(TestEffect::default())));
        graph
    }

    fn test_pattern() -> Vec<Pattern> {
        let mut pat = Pattern::new(1, "Test", [90, 160, 90], 16);
        let pc = pat.channel_mut(1, 16);
        pc.notes.push(redraw_shared::Note {
            id: 1,
            key: 69,
            start: 0,
            length: 3840,
            velocity: 100,
            pan: 0,
            release: 64,
        });
        vec![pat]
    }

    fn playing_cfg() -> TransportConfig {
        let mut cfg = TransportConfig::default();
        cfg.bpm = 120.0;
        cfg.pattern_index = 0;
        cfg.playing = true;
        cfg
    }

    fn automation_project(
        target: AutoTarget,
        length: u64,
        points: &[(f64, f32)],
    ) -> StudioProject {
        let mut project = StudioProject::default();
        project.bpm = 120.0;
        let id = add_clip(&mut project, target, length, 0.0, [0; 3]).expect("automation clip");
        let clip = find_clip_mut(&mut project, id).expect("clip");
        let points: Vec<AutomationPoint> = points
            .iter()
            .map(|(time, value)| AutomationPoint {
                time: *time,
                value: *value,
                tension: 0.0,
            })
            .collect();
        set_points(clip, &points);
        project
    }

    #[test]
    fn sine_440_roundtrip() {
        let graph = test_graph(1);
        let schedules = build_schedule(&test_pattern(), 0.5);
        let (mut engine, _handle) = create_engine(48_000.0, graph, schedules, playing_cfg(), true);
        let mut out = vec![0.0f32; 48_000 * 2];
        engine.process_interleaved(&mut out);
        let window: Vec<f32> = out
            .chunks(2)
            .skip(12_000)
            .take(4096)
            .map(|f| f[0])
            .collect();
        let mut an = Analyzer::new(4096);
        let mags = an.analyze(&window);
        let (peak_bin, peak_val) = mags
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, v)| (i, *v))
            .unwrap();
        let hz = peak_bin as f32 * 48_000.0 / 4096.0;
        assert!(peak_val > 0.01, "peak {peak_val}");
        assert!((hz - 440.0).abs() < 15.0, "hz={hz}");
    }

    #[test]
    fn transport_position_matches_960ppq() {
        let graph = test_graph(1);
        let schedules = build_schedule(&test_pattern(), 0.5);
        let (mut engine, handle) = create_engine(48_000.0, graph, schedules, playing_cfg(), true);
        let mut out = vec![0.0f32; 48_000 * 2];
        engine.process_interleaved(&mut out);
        let pos = handle
            .meters()
            .position();
        // 1 s at 120 bpm = 2 beats = 1920 ticks @960 PPQ
        assert!((pos - 1920.0).abs() < 10.0, "pos={pos}");
    }

    #[test]
    fn voice_pool_caps_at_64() {
        let graph = test_graph(1);
        let schedules = build_schedule(&test_pattern(), 0.5);
        let mut cfg = TransportConfig::default();
        cfg.pattern_index = 0;
        let (mut engine, mut handle) = create_engine(48_000.0, graph, schedules, cfg, false);
        for i in 0..100 {
            handle.note_on(1, ((i % 120) + 8) as u8, 1.0);
        }
        let _ = engine.render_planar(512);
        assert!(engine.active_voices() <= 64, "voices={}", engine.active_voices());
        handle.all_notes_off();
        let _ = engine.render_planar(512);
        assert_eq!(engine.active_voices(), 0);
    }

    #[test]
    fn sixty_four_channels_render() {
        let graph = test_graph(64);
        let schedules = build_schedule(&test_pattern(), 0.5);
        let (mut engine, _handle) = create_engine(48_000.0, graph, schedules, playing_cfg(), true);
        let mut out = vec![0.0f32; 512 * 2 * 16];
        engine.process_interleaved(&mut out);
        assert!(out.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn ten_minutes_playback_no_artifacts() {
        let graph = test_graph(1);
        let schedules = build_schedule(&test_pattern(), 0.75); // swing on
        let (mut engine, _handle) = create_engine(48_000.0, graph, schedules, playing_cfg(), true);
        let total_frames = 600 * 48_000;
        let blocks = total_frames / BLOCK_SIZE;
        for b in 0..blocks {
            let mut buf = vec![0.0f32; BLOCK_SIZE * 2];
            engine.process_interleaved(&mut buf);
            for (i, v) in buf.iter().enumerate() {
                assert!(
                    v.is_finite(),
                    "non-finite sample at frame {} sample {i}",
                    b as u64 * BLOCK_SIZE as u64
                );
            }
            if b % 5000 == 0 {
                assert!(engine.is_playing());
            }
        }
        assert!(engine.position_ticks() > 0.0);
        assert_eq!(blocks, 56_250);
    }

    #[test]
    fn from_project_uses_factory() {
        let project = StudioProject::default();
        let graph = EngineGraph::from_project(&project, &TestFactory, 48_000.0);
        assert_eq!(graph.channels.len(), 2);
        for ch in &graph.channels {
            assert_eq!(ch.synth.synth_type(), "Sine");
        }
        assert_eq!(graph.mixer.strip_count(), 64);
        let schedules = build_schedule(&project.patterns, project.swing);
        assert_eq!(schedules.len(), project.patterns.len());
    }

    #[test]
    fn command_param_and_meter_flow() {
        let graph = test_graph(2);
        let schedules = build_schedule(&test_pattern(), 0.5);
        let (mut engine, mut handle) = create_engine(48_000.0, graph, schedules, playing_cfg(), true);
        handle.set_bpm(60.0);
        handle.set_synth_param(1, 0, 0.5);
        handle.set_channel_gain(1, -3.0);
        handle.note_on(1, 69, 1.0);
        let _ = engine.render_planar(512);
        assert!(engine.is_playing());
        handle.stop();
        let _ = engine.render_planar(512);
        assert!(!engine.is_playing());
        assert_eq!(engine.active_voices(), 0);
    }

    #[test]
    fn reinstalled_graph_publishes_to_handle_meters() {
        let graph = test_graph(1);
        let (mut engine, mut handle) =
            create_engine(48_000.0, graph, Vec::new(), playing_cfg(), true);
        handle.install_graph(test_graph(1));
        handle.note_on(1, 69, 1.0);
        render_frames(&mut engine, 512);
        let peak = handle.meters().peak(0);
        assert!(peak > 0.0, "handle meter stayed at zero: {peak}");
    }

    fn playlist_project(note_length: u64, clip_start: u64, clip_length: u64) -> StudioProject {
        let mut project = StudioProject::default();
        project.bpm = 120.0;
        project.patterns.truncate(1);
        project.patterns[0] = Pattern::new(1, "Playlist", [90, 160, 90], 16);
        let pc = project.patterns[0].channel_mut(1, 16);
        pc.notes.push(Note {
            id: 1,
            key: 69,
            start: 0,
            length: note_length,
            velocity: 100,
            pan: 0,
            release: 64,
        });
        project.playlist.tracks.truncate(1);
        project.playlist.clips.clear();
        project.next_clip_id = 1;
        redraw_arrangement::add_pattern_clip(&mut project, 0, 0, clip_start, clip_length)
            .expect("clip");
        project
    }

    fn playlist_cfg() -> TransportConfig {
        let mut cfg = playing_cfg();
        cfg.playlist_mode = true;
        cfg
    }

    fn render_frames(engine: &mut EngineRt, frames: usize) {
        let mut out = vec![0.0f32; frames * 2];
        engine.process_interleaved(&mut out);
    }

    #[test]
    fn playlist_plays_clips_at_absolute_ticks_then_stops_at_song_end() {
        let project = playlist_project(960, 960, 960);
        let schedules = build_schedule(&project.patterns, project.swing);
        let arrangement = build_arrangement(&project);
        assert_eq!(arrangement.length, 1920);
        let (mut engine, mut handle) =
            create_engine(48_000.0, test_graph(1), schedules, playlist_cfg(), true);
        handle.set_arrangement(arrangement);

        render_frames(&mut engine, 19_200); // 0.4 s
        assert_eq!(engine.active_voices(), 0, "clip must not start early");
        render_frames(&mut engine, 9_600); // 0.6 s
        assert_eq!(engine.active_voices(), 1, "clip note should be sounding");
        assert!(engine.is_playing());

        render_frames(&mut engine, 19_200); // 1.0 s: song end
        assert!(!engine.is_playing(), "playlist stops at the song end");
        assert_eq!(engine.active_voices(), 0);
        let pos = engine.position_ticks();
        assert!((pos - 1920.0).abs() < 0.5, "pos={pos}");

        handle.play();
        render_frames(&mut engine, 4_800);
        assert!(engine.is_playing(), "play at song end restarts");
        assert!(engine.position_ticks() < 200.0);
    }

    #[test]
    fn playlist_loop_wraps_and_replays_notes() {
        let project = playlist_project(240, 480, 480);
        let schedules = build_schedule(&project.patterns, project.swing);
        let arrangement = build_arrangement(&project);
        let (mut engine, mut handle) =
            create_engine(48_000.0, test_graph(1), schedules, playlist_cfg(), true);
        handle.set_arrangement(arrangement);
        handle.set_loop(true, 0, 1920);

        render_frames(&mut engine, 62_400); // 1.3 s: second pass
        assert!(engine.is_playing());
        assert_eq!(engine.active_voices(), 1, "note replays after loop wrap");
        let pos = engine.position_ticks();
        assert!((pos - 576.0).abs() < 22.0, "wrapped pos={pos}");

        render_frames(&mut engine, 4_800); // 1.4 s
        assert_eq!(engine.active_voices(), 0, "note off after replay");
        assert!(engine.is_playing());
    }

    #[test]
    fn playlist_seek_starts_at_requested_tick() {
        let project = playlist_project(480, 960, 480);
        let schedules = build_schedule(&project.patterns, project.swing);
        let (mut engine, mut handle) =
            create_engine(48_000.0, test_graph(1), schedules, playlist_cfg(), true);
        handle.set_arrangement(build_arrangement(&project));
        handle.seek(960);
        let _ = engine.render_planar(512);
        assert_eq!(engine.active_voices(), 1, "event at seek position fires");
        assert!(engine.position_ticks() > 960.0);
    }

    #[test]
    fn playlist_loop_does_not_drift_and_seam_notes_retrigger() {
        let arrangement = redraw_arrangement::ArrangementSchedule {
            length: 1100,
            events: vec![
                SchedEvent {
                    tick: 100,
                    kind: EventKind::NoteOn,
                    channel: 1,
                    key: 69,
                    velocity: 100,
                },
                SchedEvent {
                    tick: 500,
                    kind: EventKind::NoteOff,
                    channel: 1,
                    key: 69,
                    velocity: 0,
                },
            ],
        };
        let (mut engine, mut handle) =
            create_engine(48_000.0, test_graph(1), Vec::new(), playlist_cfg(), true);
        handle.set_arrangement(arrangement);
        handle.set_bpm(133.0);
        handle.set_loop(true, 100, 1100);
        render_frames(&mut engine, 480_000); // 10 s at 133 BPM
        let tps = 133.0 * 960.0 / (60.0 * 48_000.0);
        let expected = 100.0 + (480_000.0 * tps - 1100.0) % 1000.0;
        let pos = engine.position_ticks();
        assert!((pos - expected).abs() < 1.0, "pos={pos} expected={expected}");
        assert_eq!(engine.active_voices(), 1, "note at loop start retriggers");
        assert!(engine.is_playing());
    }

    #[test]
    fn automation_tempo_changes_transport_rate() {
        let project = automation_project(
            AutoTarget::Tempo,
            960,
            &[(0.0, tempo_normalized(240.0)), (960.0, tempo_normalized(240.0))],
        );
        let (mut engine, mut handle) =
            create_engine(48_000.0, test_graph(1), Vec::new(), playing_cfg(), true);
        handle.set_automation(AutomationSchedule::from_project(&project));
        render_frames(&mut engine, 48_000);
        let pos = engine.position_ticks();
        assert!((pos - 3840.0).abs() < 1.0, "240 BPM position={pos}");
    }

    #[test]
    fn automation_routes_last_lane_and_plugin_params() {
        let mut project = automation_project(
            AutoTarget::MasterVolume,
            960,
            &[(0.0, 0.0), (960.0, 0.0)],
        );
        add_clip(
            &mut project,
            AutoTarget::MasterVolume,
            960,
            1.0,
            [0; 3],
        )
        .expect("second master lane");
        add_clip(
            &mut project,
            AutoTarget::ChannelVolume { channel_id: 1 },
            960,
            1.0,
            [0; 3],
        )
        .expect("channel volume lane");
        add_clip(
            &mut project,
            AutoTarget::ChannelPan { channel_id: 1 },
            960,
            1.0,
            [0; 3],
        )
        .expect("channel pan lane");
        add_clip(
            &mut project,
            AutoTarget::SendLevel {
                channel_id: 1,
                send: 0,
            },
            960,
            1.0,
            [0; 3],
        )
        .expect("send lane");
        add_clip(
            &mut project,
            AutoTarget::ChannelParam {
                channel_id: 1,
                param: 0,
            },
            960,
            0.5,
            [0; 3],
        )
        .expect("parameter lane");
        add_clip(
            &mut project,
            AutoTarget::FxParam {
                channel_id: 1,
                slot: 0,
                param: 0,
            },
            960,
            0.25,
            [0; 3],
        )
        .expect("FX parameter lane");

        let (mut engine, mut handle) =
            create_engine(48_000.0, test_graph_with_fx(), Vec::new(), playing_cfg(), true);
        handle.set_automation(AutomationSchedule::from_project(&project));
        let _ = engine.render_planar(512);

        let graph = engine.graph.as_ref().unwrap();
        assert!(
            (graph.mixer.master_gain_db - 12.0).abs() < 0.01,
            "last master lane must win: {}",
            graph.mixer.master_gain_db
        );
        assert!((graph.mixer.strips[0].gain_db - 12.0).abs() < 0.01);
        assert!((graph.mixer.strips[0].pan - 1.0).abs() < 0.01);
        assert!((graph.mixer.strips[0].sends[0].level_db - 12.0).abs() < 0.01);
        let param = graph.channels[0].synth.get_param(0);
        assert!((param - 50.0).abs() < 0.01, "param={param}");
        let fx_param = graph.mixer.strips[0]
            .inserts
            .get(0)
            .expect("test effect")
            .get_param(0);
        assert!((fx_param - 25.0).abs() < 0.01, "fx_param={fx_param}");
    }

    #[test]
    fn automation_empty_target_values_are_safe() {
        let project = automation_project(
            AutoTarget::ChannelVolume { channel_id: 99 },
            960,
            &[(0.0, 1.0)],
        );
        let (mut engine, mut handle) =
            create_engine(48_000.0, test_graph(1), Vec::new(), playing_cfg(), true);
        handle.set_automation(AutomationSchedule::from_project(&project));
        let _ = engine.render_planar(512);
        assert!(engine.is_playing());
        assert_eq!(engine.active_voices(), 0);
    }

    #[test]
    fn playlist_mode_switch_rewinds_transport() {
        let project = playlist_project(960, 0, 1920);
        let schedules = build_schedule(&project.patterns, project.swing);
        let (mut engine, mut handle) =
            create_engine(48_000.0, test_graph(1), schedules, playing_cfg(), true);
        let _ = engine.render_planar(512);
        assert!(engine.position_ticks() > 0.0);

        handle.set_arrangement(build_arrangement(&project));
        handle.set_playlist_mode(true);
        let _ = engine.render_planar(512);
        assert!((engine.position_ticks() - 20.48).abs() < 0.01);
        assert_eq!(engine.active_voices(), 1);

        handle.set_playlist_mode(false);
        let _ = engine.render_planar(512);
        assert!((engine.position_ticks() - 20.48).abs() < 0.01);
        assert_eq!(engine.active_voices(), 1);
    }
}
