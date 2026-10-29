//! REDRAW pattern sequencing: model editing, event schedule building and the
//! pure PPQ transport clock.
//!
//! Three layers live here:
//!
//! * **Editing** — allocation-light helpers that keep [`Pattern`] invariants
//!   (channel step width, velocity/velocity/nudge clamps, note ids, pattern
//!   selection). They are used by the UI and by tests, never by the audio
//!   callback.
//! * **Scheduling** — [`build_schedule`] turns patterns into sorted, loopable
//!   note events with swing and gate. This is the single implementation used by
//!   `redraw-audio-engine`, which re-exports it.
//! * **Transport** — [`TransportClock`] is a deterministic musical clock in
//!   ticks (960 PPQ) with play/stop/seek, tempo, time signature and loop
//!   wrapping. It has no audio dependency and is fully unit-testable; the
//!   engine keeps its own sample-accurate loop for now and the arrangement
//!   phase shares this clock's semantics.
#![forbid(unsafe_code)]

pub use redraw_shared as shared;

use redraw_shared::{Note, Pattern, PatternChannel, Step, StudioProject, PPQ};
use std::fmt;

pub const CRATE_PHASE: u8 = 9;

/// One 16th-note step in ticks (240 at 960 PPQ).
pub const STEP_TICKS: u64 = PPQ as u64 / 4;
/// Default step gate as a fraction of a step.
pub const DEFAULT_GATE: f32 = 0.75;
/// Maximum absolute step nudge in ticks (half a step).
pub const MAX_STEP_NUDGE: i16 = 120;
pub const MIN_BPM: f32 = 10.0;
pub const MAX_BPM: f32 = 999.0;
pub const DEFAULT_BPM: f32 = 140.0;

/// Pattern colours used when creating patterns (original palette).
pub const PATTERN_COLORS: [[u8; 3]; 8] = [
    [86, 168, 122],
    [91, 141, 201],
    [190, 130, 90],
    [156, 110, 180],
    [180, 170, 90],
    [90, 170, 180],
    [190, 95, 110],
    [120, 140, 170],
];

// ------------------------------------------------------------------ schedule

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    NoteOn,
    NoteOff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchedEvent {
    pub tick: u64,
    pub kind: EventKind,
    pub channel: u32,
    pub key: u8,
    pub velocity: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduledPattern {
    pub id: u32,
    pub name: String,
    /// Pattern length in ticks.
    pub length: u64,
    /// Sorted loop events: offs before ons at equal ticks.
    pub events: Vec<SchedEvent>,
}

/// Swing and gate shaping for [`build_schedule_with`].
#[derive(Clone, Copy, Debug)]
pub struct ScheduleOptions {
    /// 50 % = straight, up to 75 % for maximum shuffle.
    pub swing: f32,
    /// Note length as a fraction of a step (0.05..=1.0).
    pub gate: f32,
}

impl Default for ScheduleOptions {
    fn default() -> Self {
        Self {
            swing: 0.5,
            gate: DEFAULT_GATE,
        }
    }
}

impl ScheduleOptions {
    pub fn new(swing: f32, gate: f32) -> Self {
        Self {
            swing: swing.clamp(0.0, 1.0),
            gate: gate.clamp(0.05, 1.0),
        }
    }
}

/// Build loopable event lists for every pattern with default gate.
/// Swing applies to odd 16th steps (0 at 50 %, up to +1/3 step at 75 %).
pub fn build_schedule(patterns: &[Pattern], swing: f32) -> Vec<ScheduledPattern> {
    build_schedule_with(
        patterns,
        ScheduleOptions {
            swing,
            gate: DEFAULT_GATE,
        },
    )
}

/// Build schedules with explicit swing and gate. Steps may be nudged by
/// [`MAX_STEP_NUDGE`] ticks in either direction; piano-roll notes are clipped to
/// the pattern length and always get at least one tick of gate.
pub fn build_schedule_with(patterns: &[Pattern], options: ScheduleOptions) -> Vec<ScheduledPattern> {
    let swing = options.swing.clamp(0.0, 1.0);
    let gate = options.gate.clamp(0.05, 1.0);
    let swing_shift =
        (((swing - 0.5) / 0.25).max(0.0) * (STEP_TICKS as f32 / 3.0)) as i64;
    let gate_ticks = ((STEP_TICKS as f32) * gate).round().max(1.0) as u64;

    patterns
        .iter()
        .map(|pat| {
            let length = (pat.length_steps as u64 * STEP_TICKS).max(STEP_TICKS);
            let mut events: Vec<SchedEvent> = Vec::new();
            for pc in &pat.channels {
                for (i, step) in pc.steps.iter().enumerate() {
                    if !step.active {
                        continue;
                    }
                    let base = i as u64 * STEP_TICKS;
                    let shift = if i % 2 == 1 { swing_shift } else { 0 };
                    let nudge = step.nudge.clamp(-MAX_STEP_NUDGE, MAX_STEP_NUDGE) as i64;
                    let on = (base as i64 + shift + nudge)
                        .clamp(0, length.saturating_sub(1) as i64) as u64;
                    let off = (on + gate_ticks).min(length);
                    events.push(SchedEvent {
                        tick: on,
                        kind: EventKind::NoteOn,
                        channel: pc.channel_id,
                        key: 60,
                        velocity: step.velocity.clamp(1, 127),
                    });
                    events.push(SchedEvent {
                        tick: off,
                        kind: EventKind::NoteOff,
                        channel: pc.channel_id,
                        key: 60,
                        velocity: 0,
                    });
                }
                for note in &pc.notes {
                    if note.start >= length {
                        continue;
                    }
                    let on = note.start;
                    let mut off = on
                        .saturating_add(note.length.max(1))
                        .min(length);
                    if off <= on {
                        off = (on + 1).min(length);
                    }
                    if off <= on {
                        continue;
                    }
                    events.push(SchedEvent {
                        tick: on,
                        kind: EventKind::NoteOn,
                        channel: pc.channel_id,
                        key: note.key.min(127),
                        velocity: note.velocity.clamp(1, 127),
                    });
                    events.push(SchedEvent {
                        tick: off,
                        kind: EventKind::NoteOff,
                        channel: pc.channel_id,
                        key: note.key.min(127),
                        velocity: 0,
                    });
                }
            }
            events.sort_by(|a, b| {
                a.tick
                    .cmp(&b.tick)
                    .then_with(|| {
                        (a.kind == EventKind::NoteOn).cmp(&(b.kind == EventKind::NoteOn))
                    })
                    .then_with(|| a.channel.cmp(&b.channel))
                    .then_with(|| a.key.cmp(&b.key))
            });
            ScheduledPattern {
                id: pat.id,
                name: pat.name.clone(),
                length,
                events,
            }
        })
        .collect()
}

// ------------------------------------------------------------- step editing

fn channel_ensure<'a>(pattern: &'a mut Pattern, channel_id: u32) -> &'a mut PatternChannel {
    let steps = pattern.length_steps.max(1) as usize;
    let pc = pattern.channel_mut(channel_id, steps as u32);
    if pc.steps.len() < steps {
        pc.steps.resize(steps, Step::default());
    }
    pc
}

/// Resize the pattern (in 16th steps) and every channel's step row.
pub fn set_pattern_length(pattern: &mut Pattern, steps: u32) {
    pattern.length_steps = steps.max(1);
    let n = pattern.length_steps as usize;
    for pc in &mut pattern.channels {
        pc.steps.resize(n, Step::default());
    }
}

/// Toggle a step and return its new state. Out-of-range steps are rejected and
/// activation stamps `velocity` (clamped to 1..=127).
pub fn toggle_step(
    pattern: &mut Pattern,
    channel_id: u32,
    step: usize,
    velocity: u8,
) -> bool {
    if step >= pattern.length_steps.max(1) as usize {
        return false;
    }
    let pc = channel_ensure(pattern, channel_id);
    let s = &mut pc.steps[step];
    s.active = !s.active;
    if s.active {
        s.velocity = velocity.clamp(1, 127);
    }
    s.active
}

/// Explicitly set a step's active flag. Returns false when the index is invalid.
pub fn set_step_active(
    pattern: &mut Pattern,
    channel_id: u32,
    step: usize,
    active: bool,
) -> bool {
    if step >= pattern.length_steps.max(1) as usize {
        return false;
    }
    let pc = channel_ensure(pattern, channel_id);
    pc.steps[step].active = active;
    true
}

/// Set a step's velocity, clamped to 1..=127.
pub fn set_step_velocity(
    pattern: &mut Pattern,
    channel_id: u32,
    step: usize,
    velocity: u8,
) -> bool {
    if step >= pattern.length_steps.max(1) as usize {
        return false;
    }
    let pc = channel_ensure(pattern, channel_id);
    pc.steps[step].velocity = velocity.clamp(1, 127);
    true
}

/// Add to a step's tick nudge (clamped to ±[`MAX_STEP_NUDGE`]). Returns the
/// resulting nudge.
pub fn nudge_step(
    pattern: &mut Pattern,
    channel_id: u32,
    step: usize,
    delta: i16,
) -> i16 {
    if step >= pattern.length_steps.max(1) as usize {
        return 0;
    }
    let pc = channel_ensure(pattern, channel_id);
    let s = &mut pc.steps[step];
    s.nudge = (s.nudge as i32 + delta as i32)
        .clamp(-(MAX_STEP_NUDGE as i32), MAX_STEP_NUDGE as i32) as i16;
    s.nudge
}

/// Set an absolute tick nudge (clamped). Returns the resulting nudge.
pub fn set_step_nudge(
    pattern: &mut Pattern,
    channel_id: u32,
    step: usize,
    nudge: i16,
) -> i16 {
    if step >= pattern.length_steps.max(1) as usize {
        return 0;
    }
    let pc = channel_ensure(pattern, channel_id);
    let s = &mut pc.steps[step];
    s.nudge = nudge.clamp(-MAX_STEP_NUDGE, MAX_STEP_NUDGE);
    s.nudge
}

/// Clear all steps of one channel, returning how many were active.
pub fn clear_steps(pattern: &mut Pattern, channel_id: u32) -> usize {
    let Some(pc) = pattern.channels.iter_mut().find(|c| c.channel_id == channel_id) else {
        return 0;
    };
    let cleared = pc.steps.iter().filter(|s| s.active).count();
    for s in &mut pc.steps {
        *s = Step::default();
    }
    cleared
}

/// Clear every step and note in the pattern, keeping channel rows.
pub fn clear_pattern(pattern: &mut Pattern) -> usize {
    let mut cleared = 0;
    for pc in &mut pattern.channels {
        cleared += pc.steps.iter().filter(|s| s.active).count();
        for s in &mut pc.steps {
            *s = Step::default();
        }
        pc.notes.clear();
    }
    cleared
}

// ------------------------------------------------------------- note editing

/// Insert a note, normalizing length/velocity and assigning an id when `id == 0`.
/// Returns the stored note (including its final id).
pub fn add_note(pattern: &mut Pattern, channel_id: u32, mut note: Note) -> Note {
    note.length = note.length.max(1);
    note.velocity = note.velocity.clamp(1, 127);
    note.key = note.key.min(127);
    let pc = channel_ensure(pattern, channel_id);
    if note.id == 0 {
        note.id = pc
            .notes
            .iter()
            .map(|n| n.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
    }
    pc.notes.push(note);
    note
}

pub fn note_mut(
    pattern: &mut Pattern,
    channel_id: u32,
    note_id: u64,
) -> Option<&mut Note> {
    pattern
        .channels
        .iter_mut()
        .find(|c| c.channel_id == channel_id)?
        .notes
        .iter_mut()
        .find(|n| n.id == note_id)
}

pub fn find_note(pattern: &Pattern, channel_id: u32, note_id: u64) -> Option<&Note> {
    pattern
        .channel(channel_id)?
        .notes
        .iter()
        .find(|n| n.id == note_id)
}

/// Move a note's start and key, clamping to the timeline and MIDI range.
pub fn move_note(
    pattern: &mut Pattern,
    channel_id: u32,
    note_id: u64,
    start: i64,
    key: i32,
) -> bool {
    match note_mut(pattern, channel_id, note_id) {
        Some(n) => {
            n.start = start.max(0) as u64;
            n.key = key.clamp(0, 127) as u8;
            true
        }
        None => false,
    }
}

/// Relative move in ticks and semitones.
pub fn nudge_note(
    pattern: &mut Pattern,
    channel_id: u32,
    note_id: u64,
    delta_ticks: i64,
    delta_key: i32,
) -> bool {
    match note_mut(pattern, channel_id, note_id) {
        Some(n) => {
            n.start = (n.start as i64 + delta_ticks).max(0) as u64;
            n.key = (n.key as i32 + delta_key).clamp(0, 127) as u8;
            true
        }
        None => false,
    }
}

/// Set a note's length in ticks (minimum one tick).
pub fn resize_note(
    pattern: &mut Pattern,
    channel_id: u32,
    note_id: u64,
    length: u64,
) -> bool {
    match note_mut(pattern, channel_id, note_id) {
        Some(n) => {
            n.length = length.max(1);
            true
        }
        None => false,
    }
}

pub fn delete_note(pattern: &mut Pattern, channel_id: u32, note_id: u64) -> bool {
    let Some(pc) = pattern.channels.iter_mut().find(|c| c.channel_id == channel_id) else {
        return false;
    };
    let before = pc.notes.len();
    pc.notes.retain(|n| n.id != note_id);
    pc.notes.len() != before
}

/// Snap note starts to the nearest `grid` tick multiple. Lengths are preserved.
/// Returns the number of notes moved.
pub fn quantize_notes(pattern: &mut Pattern, channel_id: u32, grid: u64) -> usize {
    let grid = grid.max(1);
    let Some(pc) = pattern.channels.iter_mut().find(|c| c.channel_id == channel_id) else {
        return 0;
    };
    let mut moved = 0;
    for note in &mut pc.notes {
        let snapped = note
            .start
            .saturating_add(grid / 2)
            .saturating_div(grid)
            .saturating_mul(grid);
        if snapped != note.start {
            note.start = snapped;
            moved += 1;
        }
    }
    moved
}

/// Transpose notes, clamping to MIDI 0..=127. Returns the number changed.
pub fn transpose_notes(pattern: &mut Pattern, channel_id: u32, semitones: i32) -> usize {
    let Some(pc) = pattern.channels.iter_mut().find(|c| c.channel_id == channel_id) else {
        return 0;
    };
    let mut changed = 0;
    for note in &mut pc.notes {
        let key = (note.key as i32 + semitones).clamp(0, 127) as u8;
        if key != note.key {
            note.key = key;
            changed += 1;
        }
    }
    changed
}

pub fn clear_notes(pattern: &mut Pattern, channel_id: u32) -> usize {
    let Some(pc) = pattern.channels.iter_mut().find(|c| c.channel_id == channel_id) else {
        return 0;
    };
    let n = pc.notes.len();
    pc.notes.clear();
    n
}

// ---------------------------------------------------------- pattern management

/// Append a new empty 16-step pattern and return its index.
pub fn add_pattern(project: &mut StudioProject) -> usize {
    let id = project.next_pattern_id.max(1);
    project.next_pattern_id = id.saturating_add(1);
    let color = PATTERN_COLORS[project.patterns.len() % PATTERN_COLORS.len()];
    let pattern = Pattern::new(id, format!("Pattern {id}"), color, 16);
    project.patterns.push(pattern);
    project.patterns.len() - 1
}

/// Duplicate a pattern (including notes, with fresh note ids) right after the
/// source. Returns the new index.
pub fn duplicate_pattern(project: &mut StudioProject, index: usize) -> Option<usize> {
    let mut copy = project.patterns.get(index)?.clone();
    let id = project.next_pattern_id.max(1);
    project.next_pattern_id = id.saturating_add(1);
    copy.id = id;
    copy.name = format!("{} copy", copy.name);
    let mut next_note = project.next_note_id.max(1);
    for pattern in &project.patterns {
        for pc in &pattern.channels {
            for note in &pc.notes {
                next_note = next_note.max(note.id.saturating_add(1));
            }
        }
    }
    for pc in &mut copy.channels {
        for note in &mut pc.notes {
            note.id = next_note;
            next_note = next_note.saturating_add(1);
        }
    }
    project.next_note_id = next_note;
    project.patterns.insert(index + 1, copy);
    Some(index + 1)
}

/// Delete a pattern, keeping at least one. Adjusts `current_pattern`.
pub fn delete_pattern(project: &mut StudioProject, index: usize) -> bool {
    if project.patterns.len() <= 1 || index >= project.patterns.len() {
        return false;
    }
    project.patterns.remove(index);
    if project.current_pattern > index {
        project.current_pattern -= 1;
    }
    if project.current_pattern >= project.patterns.len() {
        project.current_pattern = project.patterns.len() - 1;
    }
    true
}

/// Select a pattern by index (bounds-checked).
pub fn select_pattern(project: &mut StudioProject, index: usize) -> bool {
    if index < project.patterns.len() {
        project.current_pattern = index;
        true
    } else {
        false
    }
}

// ------------------------------------------------------------------ transport

/// Musical position with 1-based bar/beat and 0-based ticks inside the beat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MusicalPosition {
    pub bar: u64,
    pub beat: u64,
    pub tick: u64,
}

impl fmt::Display for MusicalPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{:03}", self.bar, self.beat, self.tick)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoopRange {
    pub enabled: bool,
    pub start: u64,
    pub end: u64,
}

/// Normalize a time signature to musically sane bounds.
pub fn normalize_time_signature(ts: (u8, u8)) -> (u8, u8) {
    (ts.0.clamp(1, 16), ts.1.clamp(1, 16))
}

/// Ticks per beat for a signature (denominator 4 = a quarter = 960 ticks).
pub fn ticks_per_beat(ts: (u8, u8)) -> f64 {
    let (_, denominator) = normalize_time_signature(ts);
    PPQ as f64 * 4.0 / denominator as f64
}

/// Ticks per bar for a signature.
pub fn ticks_per_bar(ts: (u8, u8)) -> f64 {
    let (numerator, _) = normalize_time_signature(ts);
    ticks_per_beat(ts) * numerator as f64
}

pub fn quantize_tick(tick: u64, grid: u64) -> u64 {
    let grid = grid.max(1);
    tick.saturating_add(grid / 2)
        .saturating_div(grid)
        .saturating_mul(grid)
}

pub fn beats_to_ticks(beats: f64) -> f64 {
    beats * PPQ as f64
}

pub fn ticks_to_beats(ticks: f64) -> f64 {
    ticks / PPQ as f64
}

/// A deterministic musical clock. It never touches audio; callers advance it by
/// frame count (realtime) or tick count (offline/tests).
#[derive(Clone, Debug)]
pub struct TransportClock {
    sample_rate: f32,
    bpm: f32,
    time_sig: (u8, u8),
    ticks: f64,
    playing: bool,
    loop_enabled: bool,
    loop_start: u64,
    loop_end: u64,
    wraps: u64,
}

impl Default for TransportClock {
    fn default() -> Self {
        Self::new(48_000.0)
    }
}

impl TransportClock {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate: sample_rate.max(1.0),
            bpm: DEFAULT_BPM,
            time_sig: (4, 4),
            ticks: 0.0,
            playing: false,
            loop_enabled: false,
            loop_start: 0,
            loop_end: PPQ as u64 * 16,
            wraps: 0,
        }
    }

    /// Clock initialized from a project's tempo, signature and playlist loop.
    pub fn from_project(project: &StudioProject) -> Self {
        let mut clock = Self::new(project.sample_rate.max(1) as f32);
        clock.bpm = project.bpm.clamp(MIN_BPM, MAX_BPM);
        clock.time_sig = normalize_time_signature(project.time_sig);
        clock.loop_enabled = project.playlist.loop_enabled;
        if project.playlist.loop_end > project.playlist.loop_start {
            clock.loop_start = project.playlist.loop_start;
            clock.loop_end = project.playlist.loop_end;
        }
        clock
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(1.0);
    }

    pub fn bpm(&self) -> f32 {
        self.bpm
    }

    /// Set tempo, clamped to 10..=999 BPM.
    pub fn set_bpm(&mut self, bpm: f32) {
        self.bpm = bpm.clamp(MIN_BPM, MAX_BPM);
    }

    pub fn time_signature(&self) -> (u8, u8) {
        self.time_sig
    }

    pub fn set_time_signature(&mut self, ts: (u8, u8)) {
        self.time_sig = normalize_time_signature(ts);
    }

    /// Current position in ticks.
    pub fn ticks(&self) -> f64 {
        self.ticks
    }

    /// Jump to an absolute tick position (negative values clamp to zero).
    pub fn seek(&mut self, tick: f64) {
        self.ticks = tick.max(0.0);
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn play(&mut self) {
        self.playing = true;
    }

    pub fn stop(&mut self) {
        self.playing = false;
    }

    pub fn toggle(&mut self) {
        self.playing = !self.playing;
    }

    pub fn loop_range(&self) -> LoopRange {
        LoopRange {
            enabled: self.loop_enabled,
            start: self.loop_start,
            end: self.loop_end,
        }
    }

    pub fn loop_enabled(&self) -> bool {
        self.loop_enabled
    }

    /// Enable/disable looping. Enabling requires `end > start`; disabling is
    /// always accepted and preserves the last valid range.
    pub fn set_loop(&mut self, enabled: bool, start: u64, end: u64) -> bool {
        if enabled && end <= start {
            return false;
        }
        if end > start {
            self.loop_start = start;
            self.loop_end = end;
        }
        self.loop_enabled = enabled;
        true
    }

    pub fn wrap_count(&self) -> u64 {
        self.wraps
    }

    pub fn reset_wrap_count(&mut self) {
        self.wraps = 0;
    }

    pub fn ticks_per_sample(&self) -> f64 {
        (self.bpm.max(1.0) as f64 * PPQ as f64)
            / (60.0 * self.sample_rate.max(1.0) as f64)
    }

    pub fn ticks_per_beat(&self) -> f64 {
        ticks_per_beat(self.time_sig)
    }

    pub fn ticks_per_bar(&self) -> f64 {
        ticks_per_bar(self.time_sig)
    }

    /// Advance by audio frames at the current tempo. Returns true when the loop
    /// wrapped during this advance.
    pub fn advance(&mut self, frames: usize) -> bool {
        if !self.playing || frames == 0 {
            return false;
        }
        let ticks = frames as f64 * self.ticks_per_sample();
        self.advance_ticks(ticks)
    }

    /// Advance by an explicit tick count (offline rendering and tests).
    pub fn advance_ticks(&mut self, ticks: f64) -> bool {
        if !self.playing || ticks <= 0.0 {
            return false;
        }
        self.ticks += ticks;
        if self.loop_enabled && self.loop_end > self.loop_start {
            let end = self.loop_end as f64;
            if self.ticks >= end {
                let start = self.loop_start as f64;
                let span = end - start;
                self.ticks = start + (self.ticks - end) % span;
                self.wraps = self.wraps.wrapping_add(1);
                return true;
            }
        }
        false
    }

    /// Bar/beat/tick view of the current position.
    pub fn musical_position(&self) -> MusicalPosition {
        let beat_ticks = self.ticks_per_beat().max(1.0);
        let bar_ticks = self.ticks_per_bar().max(1.0);
        let ticks = self.ticks.max(0.0) as u64;
        let bar = ticks as f64 / bar_ticks;
        let bar_index = bar.floor() as u64;
        let in_bar = self.ticks.max(0.0) - bar_index as f64 * bar_ticks;
        let beat_index = (in_bar / beat_ticks).floor() as u64;
        let tick = (in_bar - beat_index as f64 * beat_ticks).floor() as u64;
        MusicalPosition {
            bar: bar_index + 1,
            beat: beat_index + 1,
            tick,
        }
    }

    /// Index of the 16th step currently under the playhead, wrapped to
    /// `length_steps`. Useful for step-sequencer highlighting.
    pub fn current_step(&self, length_steps: u32) -> Option<usize> {
        if length_steps == 0 {
            return None;
        }
        Some((self.ticks.max(0.0) as u64 / STEP_TICKS) as usize % length_steps as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: u64, key: u8, start: u64, length: u64) -> Note {
        Note {
            id,
            key,
            start,
            length,
            velocity: 100,
            pan: 0,
            release: 64,
        }
    }

    fn note_ons(sched: &ScheduledPattern) -> Vec<u64> {
        sched
            .events
            .iter()
            .filter(|e| e.kind == EventKind::NoteOn)
            .map(|e| e.tick)
            .collect()
    }

    #[test]
    fn schedule_orders_offs_before_ons() {
        let mut pat = Pattern::new(1, "P", [0, 0, 0], 16);
        let pc = pat.channel_mut(1, 16);
        pc.notes.push(note(1, 60, 0, 240));
        pc.notes.push(note(2, 62, 240, 240));
        let sched = build_schedule(&[pat], 0.5);
        assert_eq!(sched.len(), 1);
        assert_eq!(sched[0].length, 3840);
        let timeline: Vec<(u64, EventKind)> = sched[0]
            .events
            .iter()
            .map(|e| (e.tick, e.kind))
            .collect();
        assert_eq!(
            timeline,
            vec![
                (0, EventKind::NoteOn),
                (240, EventKind::NoteOff),
                (240, EventKind::NoteOn),
                (480, EventKind::NoteOff)
            ]
        );
        assert!(sched[0]
            .events
            .windows(2)
            .all(|w| w[0].tick <= w[1].tick));
    }

    #[test]
    fn swing_and_gate_shape_steps() {
        let mut pat = Pattern::new(1, "P", [0, 0, 0], 4);
        set_step_active(&mut pat, 1, 0, true);
        set_step_active(&mut pat, 1, 1, true);
        let straight = build_schedule(&[pat.clone()], 0.5);
        assert_eq!(note_ons(&straight[0]), vec![0, 240]);

        let swung = build_schedule_with(&[pat.clone()], ScheduleOptions::new(0.75, 0.75));
        assert_eq!(note_ons(&swung[0]), vec![0, 320]);

        let short = build_schedule_with(&[pat], ScheduleOptions::new(0.5, 0.5));
        assert!(short[0]
            .events
            .iter()
            .any(|e| e.tick == 120 && e.kind == EventKind::NoteOff));
    }

    #[test]
    fn step_nudge_is_signed_and_clamped() {
        let mut pat = Pattern::new(1, "P", [0, 0, 0], 4);
        set_step_active(&mut pat, 1, 1, true);
        assert_eq!(set_step_nudge(&mut pat, 1, 1, -120), -120);
        let early = build_schedule(&[pat.clone()], 0.5);
        assert_eq!(note_ons(&early[0]), vec![120]);

        assert_eq!(set_step_nudge(&mut pat, 1, 1, 999), MAX_STEP_NUDGE);
        let late = build_schedule(&[pat], 0.5);
        assert_eq!(note_ons(&late[0]), vec![360]);
    }

    #[test]
    fn notes_clip_to_pattern_and_keep_gate() {
        let mut pat = Pattern::new(1, "P", [0, 0, 0], 4);
        set_pattern_length(&mut pat, 4); // 960 ticks
        let pc = channel_ensure(&mut pat, 1);
        pc.notes.push(note(1, 60, 900, 1000));
        pc.notes.push(note(2, 64, 100, 0));
        let sched = build_schedule(&[pat], 0.5);
        let long: Vec<_> = sched[0]
            .events
            .iter()
            .filter(|e| e.key == 60)
            .map(|e| (e.tick, e.kind))
            .collect();
        assert_eq!(
            long,
            vec![(900, EventKind::NoteOn), (960, EventKind::NoteOff)]
        );
        let zero: Vec<_> = sched[0]
            .events
            .iter()
            .filter(|e| e.key == 64)
            .map(|e| (e.tick, e.kind))
            .collect();
        assert_eq!(
            zero,
            vec![(100, EventKind::NoteOn), (101, EventKind::NoteOff)]
        );
        let ons = sched[0]
            .events
            .iter()
            .filter(|e| e.kind == EventKind::NoteOn && e.key == 60)
            .count();
        assert_eq!(ons, 1);
    }

    #[test]
    fn step_editing_normalizes_values() {
        let mut pat = Pattern::new(1, "P", [0, 0, 0], 8);
        assert!(toggle_step(&mut pat, 5, 0, 90));
        assert_eq!(pat.channel(5).unwrap().steps[0].velocity, 90);
        assert!(!toggle_step(&mut pat, 5, 0, 20));
        assert!(!toggle_step(&mut pat, 5, 8, 100));

        assert!(set_step_velocity(&mut pat, 5, 0, 200));
        assert_eq!(pat.channel(5).unwrap().steps[0].velocity, 127);
        assert_eq!(nudge_step(&mut pat, 5, 0, 50), 50);
        assert_eq!(nudge_step(&mut pat, 5, 0, -500), -120);
        assert_eq!(clear_steps(&mut pat, 5), 0);
        assert!(set_step_active(&mut pat, 5, 2, true));
        assert_eq!(clear_steps(&mut pat, 5), 1);

        set_pattern_length(&mut pat, 4);
        assert_eq!(pat.length_steps, 4);
        assert_eq!(pat.channel(5).unwrap().steps.len(), 4);
    }

    #[test]
    fn note_editing_move_resize_delete() {
        let mut pat = Pattern::new(1, "P", [0, 0, 0], 16);
        let added = add_note(
            &mut pat,
            2,
            Note {
                id: 0,
                key: 60,
                start: 10,
                length: 0,
                velocity: 200,
                ..Default::default()
            },
        );
        assert_eq!(added.id, 1);
        assert_eq!(added.length, 1);
        assert_eq!(added.velocity, 127);
        let second = add_note(&mut pat, 2, note(5, 70, 480, 240));
        assert_eq!(second.id, 5);

        assert!(move_note(&mut pat, 2, 1, -5, 200));
        let moved = find_note(&pat, 2, 1).unwrap();
        assert_eq!(moved.start, 0);
        assert_eq!(moved.key, 127);
        assert!(resize_note(&mut pat, 2, 1, 0));
        assert_eq!(find_note(&pat, 2, 1).unwrap().length, 1);
        assert!(nudge_note(&mut pat, 2, 1, 25, -20));
        let nudged = find_note(&pat, 2, 1).unwrap();
        assert_eq!(nudged.start, 25);
        assert_eq!(nudged.key, 107);
        assert!(delete_note(&mut pat, 2, 5));
        assert!(find_note(&pat, 2, 5).is_none());
        assert!(!delete_note(&mut pat, 2, 5));
    }

    #[test]
    fn quantize_and_transpose_notes() {
        let mut pat = Pattern::new(1, "P", [0, 0, 0], 16);
        add_note(&mut pat, 1, note(1, 120, 100, 240));
        add_note(&mut pat, 1, note(2, 2, 130, 240));
        assert_eq!(quantize_notes(&mut pat, 1, 240), 2);
        assert_eq!(find_note(&pat, 1, 1).unwrap().start, 0);
        assert_eq!(find_note(&pat, 1, 2).unwrap().start, 240);
        assert_eq!(quantize_notes(&mut pat, 1, 240), 0);

        assert_eq!(transpose_notes(&mut pat, 1, 20), 2);
        assert_eq!(find_note(&pat, 1, 1).unwrap().key, 127);
        assert_eq!(find_note(&pat, 1, 2).unwrap().key, 22);
        assert_eq!(transpose_notes(&mut pat, 1, -10), 2);
        assert_eq!(find_note(&pat, 1, 1).unwrap().key, 117);
        assert_eq!(find_note(&pat, 1, 2).unwrap().key, 12);
        assert_eq!(clear_notes(&mut pat, 1), 2);
    }

    #[test]
    fn pattern_management_keeps_one_pattern() {
        let mut project = StudioProject::default();
        assert_eq!(project.patterns.len(), 1);
        add_note(
            &mut project.patterns[0],
            1,
            note(1, 60, 0, 240),
        );
        let index = add_pattern(&mut project);
        assert_eq!(index, 1);
        assert_eq!(project.patterns[1].id, 2);
        assert_eq!(project.next_pattern_id, 3);

        let dup = duplicate_pattern(&mut project, 0).expect("duplicate");
        assert_eq!(dup, 1);
        assert_eq!(project.patterns.len(), 3);
        assert_eq!(project.patterns[1].name, "Pattern 1 copy");
        assert_ne!(project.patterns[1].id, project.patterns[0].id);
        let original_note = &project.patterns[0].channels[0].notes[0];
        let copied_note = &project.patterns[1].channels[0].notes[0];
        assert_ne!(original_note.id, copied_note.id);

        assert!(select_pattern(&mut project, 2));
        assert!(!select_pattern(&mut project, 9));
        assert!(delete_pattern(&mut project, 1));
        assert_eq!(project.patterns.len(), 2);
        while project.patterns.len() > 1 {
            assert!(delete_pattern(&mut project, 0));
        }
        assert!(!delete_pattern(&mut project, 0));
        assert_eq!(project.current_pattern, 0);
    }

    #[test]
    fn clock_advances_at_tempo_and_freezes_when_stopped() {
        let mut clock = TransportClock::new(48_000.0);
        clock.set_bpm(120.0);
        assert!((clock.ticks_per_sample() - 0.04).abs() < 1e-12);
        assert!(!clock.advance(250)); // stopped
        assert_eq!(clock.ticks(), 0.0);

        clock.play();
        assert!(!clock.advance(250));
        assert!((clock.ticks() - 10.0).abs() < 1e-9);
        clock.stop();
        assert!(!clock.advance(250));
        assert!((clock.ticks() - 10.0).abs() < 1e-9);
        clock.seek(-5.0);
        assert_eq!(clock.ticks(), 0.0);
    }

    #[test]
    fn clock_loop_wraps_with_overflow() {
        let mut clock = TransportClock::new(48_000.0);
        clock.set_bpm(120.0);
        assert!(!clock.set_loop(true, 960, 960));
        assert!(clock.set_loop(true, 480, 960));
        clock.seek(900.0);
        clock.play();
        assert!(clock.advance_ticks(120.0));
        assert!((clock.ticks() - 540.0).abs() < 1e-9);
        assert_eq!(clock.wrap_count(), 1);
        assert_eq!(
            clock.loop_range(),
            LoopRange {
                enabled: true,
                start: 480,
                end: 960
            }
        );

        clock.seek(0.0);
        assert!(clock.advance_ticks(10_000.0));
        assert!((clock.ticks() - 880.0).abs() < 1e-9);
        assert!(clock.set_loop(false, 960, 480));
        assert!(!clock.loop_enabled());
    }

    #[test]
    fn clock_bar_beat_tick_and_steps() {
        let mut clock = TransportClock::new(48_000.0);
        clock.set_time_signature((4, 4));
        clock.seek(3840.0 + 960.0 + 7.0);
        assert_eq!(
            clock.musical_position(),
            MusicalPosition {
                bar: 2,
                beat: 2,
                tick: 7
            }
        );
        assert_eq!(clock.musical_position().to_string(), "2:2:007");

        assert!((ticks_per_beat((6, 8)) - 480.0).abs() < 1e-9);
        assert!((ticks_per_bar((6, 8)) - 2880.0).abs() < 1e-9);
        assert_eq!(normalize_time_signature((0, 0)), (1, 1));

        clock.seek(480.0);
        assert_eq!(clock.current_step(16), Some(2));
        clock.seek(3840.0);
        assert_eq!(clock.current_step(16), Some(0));
        assert_eq!(clock.current_step(0), None);
        assert_eq!(quantize_tick(130, 240), 240);
        assert_eq!(quantize_tick(100, 240), 0);
    }

    #[test]
    fn clock_from_project_uses_playlist_loop() {
        let mut project = StudioProject::default();
        project.bpm = 90.0;
        project.time_sig = (3, 4);
        project.playlist.loop_enabled = true;
        project.playlist.loop_start = 480;
        project.playlist.loop_end = 2400;
        let clock = TransportClock::from_project(&project);
        assert_eq!(clock.bpm(), 90.0);
        assert_eq!(clock.time_signature(), (3, 4));
        assert_eq!(clock.loop_range().start, 480);
        assert_eq!(clock.loop_range().end, 2400);
        assert!((clock.ticks_per_bar() - 2880.0).abs() < 1e-9);
        assert!(!clock.is_playing());
    }
}
