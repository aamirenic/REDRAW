//! REDRAW piano roll: note selection, editing tools, musical helpers and MIDI
//! import/export.
//!
//! The audio callback never sees this crate. All functions allocate freely and
//! are meant for the control thread / UI. Notes live in the shared pattern model
//! ([`redraw_shared::Note`]); editing calls reuse the id and clamping rules from
//! `redraw-sequencer` so step and piano-roll editing stay consistent.
//!
//! Layout of the module:
//!
//! * selection / clipboard / draw / split / merge tools ([`PianoRoll`]),
//! * quantize (grid, swing, strength, lengths, velocities), humanize, legato,
//!   chord stamp, scale helpers and chord arpeggios,
//! * velocity/pan and MIDI CC lane helpers,
//! * [`midi`] — Standard MIDI File type 0/1 import/export through `midly`,
//! * [`live`] — `midir` input hub, message parser and quantize-on-record
//!   [`live::MidiRecorder`].
#![forbid(unsafe_code)]

pub use redraw_shared as shared;
pub use redraw_sequencer as sequencer;

pub mod live;
pub mod midi;

use redraw_shared::{CcEvent, Note, Pattern, PatternChannel, StudioProject, PPQ};
use redraw_sequencer::{add_note, quantize_tick};
use std::collections::BTreeSet;

pub const CRATE_PHASE: u8 = 11;

/// Grid choices from a quarter note down to a 128th note.
pub const GRID_QUARTER: u64 = PPQ as u64;
pub const GRID_1_8: u64 = PPQ as u64 / 2;
pub const GRID_1_16: u64 = PPQ as u64 / 4;
pub const GRID_1_32: u64 = PPQ as u64 / 8;
pub const GRID_1_64: u64 = PPQ as u64 / 16;
pub const GRID_1_128: u64 = PPQ as u64 / 32;
pub const GRIDS: [(u64, &str); 6] = [
    (GRID_QUARTER, "1/4"),
    (GRID_1_8, "1/8"),
    (GRID_1_16, "1/16"),
    (GRID_1_32, "1/32"),
    (GRID_1_64, "1/64"),
    (GRID_1_128, "1/128"),
];

pub const MIN_KEY: u8 = 0;
pub const MAX_KEY: u8 = 127;
pub const MIN_VELOCITY: u8 = 1;
pub const MAX_VELOCITY: u8 = 127;
pub const MIN_PAN: i8 = -64;
pub const MAX_PAN: i8 = 63;

/// Note names for one octave, sharps preferred.
pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Black keys within an octave, indexed from C.
pub const BLACK_KEYS: [bool; 12] = [
    false, true, false, true, false, false, true, false, true, false, true, false,
];

pub const DEFAULT_VELOCITY: u8 = 100;
pub const CC_PAN: u8 = 10;
pub const CC_VOLUME: u8 = 7;
pub const CC_MODULATION: u8 = 1;
pub const CC_EXPRESSION: u8 = 11;
pub const CC_SUSTAIN: u8 = 64;
pub const CC_BRIGHTNESS: u8 = 74;
/// Lanes the piano roll shows by default (modulation, expression, sustain, brightness).
pub const DEFAULT_CC_LANES: [u8; 4] = [CC_MODULATION, CC_EXPRESSION, CC_SUSTAIN, CC_BRIGHTNESS];

pub fn is_black_key(key: u8) -> bool {
    BLACK_KEYS[(key as usize) % 12]
}

/// Scientific pitch name, MIDI 60 = C4.
pub fn note_name(key: u8) -> String {
    let octave = (key as i32 / 12) - 1;
    format!("{}{}", NOTE_NAMES[(key as usize) % 12], octave)
}

pub fn is_accidental_name(name: &str) -> bool {
    name.contains('#')
}

// ------------------------------------------------------------------ scales

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Chromatic,
    Major,
    NaturalMinor,
    HarmonicMinor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
    MajorPentatonic,
    MinorPentatonic,
    Blues,
}

impl Scale {
    pub const ALL: [Scale; 12] = [
        Scale::Chromatic,
        Scale::Major,
        Scale::NaturalMinor,
        Scale::HarmonicMinor,
        Scale::Dorian,
        Scale::Phrygian,
        Scale::Lydian,
        Scale::Mixolydian,
        Scale::Locrian,
        Scale::MajorPentatonic,
        Scale::MinorPentatonic,
        Scale::Blues,
    ];

    pub fn intervals(self) -> &'static [u8] {
        match self {
            Scale::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            Scale::Major => &[0, 2, 4, 5, 7, 9, 11],
            Scale::NaturalMinor => &[0, 2, 3, 5, 7, 8, 10],
            Scale::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Scale::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Scale::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            Scale::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            Scale::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Scale::Locrian => &[0, 1, 3, 5, 6, 8, 10],
            Scale::MajorPentatonic => &[0, 2, 4, 7, 9],
            Scale::MinorPentatonic => &[0, 3, 5, 7, 10],
            Scale::Blues => &[0, 3, 5, 6, 7, 10],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Scale::Chromatic => "Chromatic",
            Scale::Major => "Major",
            Scale::NaturalMinor => "Natural Minor",
            Scale::HarmonicMinor => "Harmonic Minor",
            Scale::Dorian => "Dorian",
            Scale::Phrygian => "Phrygian",
            Scale::Lydian => "Lydian",
            Scale::Mixolydian => "Mixolydian",
            Scale::Locrian => "Locrian",
            Scale::MajorPentatonic => "Major Pentatonic",
            Scale::MinorPentatonic => "Minor Pentatonic",
            Scale::Blues => "Blues",
        }
    }

    /// Is `key` a member of the scale rooted at `root`?
    pub fn contains(self, root: u8, key: u8) -> bool {
        let interval = (key as i32 - root as i32).rem_euclid(12) as u8;
        self.intervals().contains(&interval)
    }

    /// All MIDI keys 0..=127 belonging to the scale.
    pub fn keys(self, root: u8) -> Vec<u8> {
        (MIN_KEY..=MAX_KEY)
            .filter(|k| self.contains(root, *k))
            .collect()
    }

    /// Nearest scale key. Ties resolve downward.
    pub fn snap(self, root: u8, key: u8) -> u8 {
        if self.contains(root, key) {
            return key;
        }
        let mut best = key;
        let mut best_dist = 128u8;
        for candidate in MIN_KEY..=MAX_KEY {
            if !self.contains(root, candidate) {
                continue;
            }
            let dist = candidate.abs_diff(key);
            if dist < best_dist {
                best = candidate;
                best_dist = dist;
            }
        }
        best
    }
}

// ------------------------------------------------------------------ chords

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chord {
    Major,
    Minor,
    Diminished,
    Augmented,
    Sus2,
    Sus4,
    Major7,
    Minor7,
    Dominant7,
    Add9,
}

impl Chord {
    pub const ALL: [Chord; 10] = [
        Chord::Major,
        Chord::Minor,
        Chord::Diminished,
        Chord::Augmented,
        Chord::Sus2,
        Chord::Sus4,
        Chord::Major7,
        Chord::Minor7,
        Chord::Dominant7,
        Chord::Add9,
    ];

    pub fn intervals(self) -> &'static [i8] {
        match self {
            Chord::Major => &[0, 4, 7],
            Chord::Minor => &[0, 3, 7],
            Chord::Diminished => &[0, 3, 6],
            Chord::Augmented => &[0, 4, 8],
            Chord::Sus2 => &[0, 2, 7],
            Chord::Sus4 => &[0, 5, 7],
            Chord::Major7 => &[0, 4, 7, 11],
            Chord::Minor7 => &[0, 3, 7, 10],
            Chord::Dominant7 => &[0, 4, 7, 10],
            Chord::Add9 => &[0, 4, 7, 14],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Chord::Major => "Major",
            Chord::Minor => "Minor",
            Chord::Diminished => "Diminished",
            Chord::Augmented => "Augmented",
            Chord::Sus2 => "Sus2",
            Chord::Sus4 => "Sus4",
            Chord::Major7 => "Major 7",
            Chord::Minor7 => "Minor 7",
            Chord::Dominant7 => "Dominant 7",
            Chord::Add9 => "Add 9",
        }
    }
}

pub fn chord_keys(root: u8, chord: Chord) -> Vec<u8> {
    chord
        .intervals()
        .iter()
        .map(|semi| (root as i32 + *semi as i32).clamp(0, 127) as u8)
        .collect()
}

// ------------------------------------------------------------------ state

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Draw,
    Select,
    Erase,
    Split,
    Merge,
}

#[derive(Clone, Debug)]
pub struct PianoRoll {
    pub pattern_id: u32,
    pub channel_id: u32,
    pub grid: u64,
    pub tool: Tool,
    pub selection: Vec<u64>,
    pub clipboard: Vec<Note>,
    pub scale_root: u8,
    pub scale: Scale,
    pub default_velocity: u8,
    pub default_pan: i8,
    /// Last drawn note length, reused by the draw tool until changed.
    pub last_length: u64,
    pub ghosts: Vec<u32>,
}

impl PianoRoll {
    pub fn new(pattern_id: u32, channel_id: u32) -> Self {
        Self {
            pattern_id,
            channel_id,
            grid: GRID_1_16,
            tool: Tool::Draw,
            selection: Vec::new(),
            clipboard: Vec::new(),
            scale_root: 0,
            scale: Scale::Chromatic,
            default_velocity: DEFAULT_VELOCITY,
            default_pan: 0,
            last_length: GRID_1_16,
            ghosts: Vec::new(),
        }
    }

    pub fn set_grid(&mut self, grid: u64) {
        self.grid = grid.max(1);
        self.last_length = self.grid;
    }

    fn channel<'a>(&self, pattern: &'a mut Pattern) -> Option<&'a mut PatternChannel> {
        pattern
            .channels
            .iter_mut()
            .find(|c| c.channel_id == self.channel_id)
    }

    fn channel_ref<'a>(&self, pattern: &'a Pattern) -> Option<&'a PatternChannel> {
        pattern.channel(self.channel_id)
    }

    // ---------------------------------------------------------- selection

    pub fn select(&mut self, ids: impl IntoIterator<Item = u64>) {
        self.selection.clear();
        self.selection.extend(ids);
        self.selection.sort_unstable();
        self.selection.dedup();
    }

    pub fn select_note(&mut self, id: u64) {
        if !self.selection.contains(&id) {
            self.selection.push(id);
        }
    }

    pub fn deselect_note(&mut self, id: u64) {
        self.selection.retain(|n| *n != id);
    }

    pub fn toggle_note(&mut self, id: u64) {
        if self.selection.contains(&id) {
            self.deselect_note(id);
        } else {
            self.select_note(id);
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    pub fn is_selected(&self, id: u64) -> bool {
        self.selection.contains(&id)
    }

    pub fn select_all(&mut self, pattern: &Pattern) {
        let Some(pc) = self.channel_ref(pattern) else {
            return;
        };
        self.select(pc.notes.iter().map(|n| n.id));
    }

    /// Select notes intersecting a tick and key rectangle.
    pub fn select_range(
        &mut self,
        pattern: &Pattern,
        from_tick: u64,
        to_tick: u64,
        low_key: u8,
        high_key: u8,
    ) -> usize {
        let (lo, hi) = if from_tick <= to_tick {
            (from_tick, to_tick)
        } else {
            (to_tick, from_tick)
        };
        let (low, high) = if low_key <= high_key {
            (low_key, high_key)
        } else {
            (high_key, low_key)
        };
        let Some(pc) = self.channel_ref(pattern) else {
            return 0;
        };
        let ids: Vec<u64> = pc
            .notes
            .iter()
            .filter(|n| n.start < hi && n.start.saturating_add(n.length) > lo)
            .filter(|n| n.key >= low && n.key <= high)
            .map(|n| n.id)
            .collect();
        self.select(ids);
        self.selection.len()
    }

    pub fn selected_notes(&self, pattern: &Pattern) -> Vec<Note> {
        let Some(pc) = self.channel_ref(pattern) else {
            return Vec::new();
        };
        let mut notes: Vec<Note> = pc
            .notes
            .iter()
            .filter(|n| self.selection.contains(&n.id))
            .copied()
            .collect();
        notes.sort_by_key(|n| (n.start, n.key, n.id));
        notes
    }

    pub fn note_at(&self, pattern: &Pattern, tick: u64, key: u8) -> Option<u64> {
        self.channel_ref(pattern)?
            .notes
            .iter()
            .filter(|n| n.key == key && n.start <= tick && n.start.saturating_add(n.length) > tick)
            .max_by_key(|n| n.start)
            .map(|n| n.id)
    }

    // ---------------------------------------------------------- clipboard

    pub fn copy(&mut self, pattern: &Pattern) -> usize {
        self.clipboard = self.selected_notes(pattern);
        self.clipboard.len()
    }

    pub fn cut(&mut self, pattern: &mut Pattern) -> usize {
        let copied = self.copy(pattern);
        if copied > 0 {
            self.delete_selected(pattern);
            self.clipboard.len()
        } else {
            0
        }
    }

    pub fn delete_selected(&mut self, pattern: &mut Pattern) -> usize {
        let selected = self.selection.clone();
        if selected.is_empty() {
            return 0;
        }
        let Some(pc) = self.channel(pattern) else {
            return 0;
        };
        let before = pc.notes.len();
        pc.notes.retain(|n| !selected.contains(&n.id));
        let removed = before - pc.notes.len();
        self.selection.clear();
        removed
    }

    pub fn paste(&mut self, pattern: &mut Pattern, at_tick: u64) -> Vec<u64> {
        if self.clipboard.is_empty() {
            return Vec::new();
        }
        let Some(base) = self.clipboard.iter().map(|n| n.start).min() else {
            return Vec::new();
        };
        let clipboard = self.clipboard.clone();
        let mut ids = Vec::with_capacity(clipboard.len());
        for mut note in clipboard {
            note.start = at_tick.saturating_add(note.start.saturating_sub(base));
            note.id = 0;
            let stored = add_note(pattern, self.channel_id, note);
            ids.push(stored.id);
        }
        self.select(ids.iter().copied());
        ids
    }

    pub fn duplicate(&mut self, pattern: &mut Pattern, offset_ticks: u64) -> Vec<u64> {
        let notes = self.selected_notes(pattern);
        if notes.is_empty() {
            return Vec::new();
        }
        let mut ids = Vec::with_capacity(notes.len());
        for mut note in notes {
            note.start = note.start.saturating_add(offset_ticks);
            note.id = 0;
            let stored = add_note(pattern, self.channel_id, note);
            ids.push(stored.id);
        }
        self.select(ids.iter().copied());
        ids
    }

    // -------------------------------------------------------------- tools

    /// Draw a grid-snapped note and select it.
    pub fn draw(&mut self, pattern: &mut Pattern, tick: u64, key: u8, length: Option<u64>) -> Note {
        let start = quantize_tick(tick, self.grid);
        let length = length.unwrap_or(self.last_length).max(1);
        self.last_length = length;
        let note = Note {
            id: 0,
            key: key.min(MAX_KEY),
            start,
            length,
            velocity: self.default_velocity.clamp(MIN_VELOCITY, MAX_VELOCITY),
            pan: self.default_pan.clamp(MIN_PAN, MAX_PAN),
            release: 64,
        };
        let stored = add_note(pattern, self.channel_id, note);
        self.select([stored.id]);
        stored
    }

    /// Split `note_id` at `at_tick`. Returns the new right-hand id.
    pub fn split(&mut self, pattern: &mut Pattern, note_id: u64, at_tick: u64) -> Option<u64> {
        let Some(pc) = self.channel(pattern) else {
            return None;
        };
        let pos = pc.notes.iter().position(|n| n.id == note_id)?;
        let note = pc.notes[pos];
        if at_tick <= note.start || at_tick >= note.start.saturating_add(note.length) {
            return None;
        }
        pc.notes[pos].length = at_tick - note.start;
        let right = Note {
            id: 0,
            start: at_tick,
            length: note.start.saturating_add(note.length) - at_tick,
            ..note
        };
        let stored = add_note(pattern, self.channel_id, right);
        self.selection = vec![note_id, stored.id];
        Some(stored.id)
    }

    /// Merge selected notes into one span (keeping the first note's key).
    pub fn merge_selected(&mut self, pattern: &mut Pattern) -> Option<u64> {
        if self.selection.len() < 2 {
            return None;
        }
        let selected = self.selection.clone();
        let Some(pc) = self.channel(pattern) else {
            return None;
        };
        let mut chosen = Vec::new();
        for note in &pc.notes {
            if selected.contains(&note.id) {
                chosen.push(*note);
            }
        }
        if chosen.len() < 2 {
            return None;
        }
        chosen.sort_by_key(|n| (n.start, n.id));
        let first = chosen[0];
        let start = chosen.iter().map(|n| n.start).min().unwrap_or(first.start);
        let end = chosen
            .iter()
            .map(|n| n.start.saturating_add(n.length))
            .max()
            .unwrap_or(first.start + first.length);
        let keep_id = first.id;
        pc.notes.retain(|n| !selected.contains(&n.id));
        let merged = Note {
            id: keep_id,
            start,
            length: end.saturating_sub(start).max(1),
            ..first
        };
        pc.notes.push(merged);
        self.selection = vec![keep_id];
        Some(keep_id)
    }

    // ---------------------------------------------------------- transforms

    pub fn set_selected_velocity(&mut self, pattern: &mut Pattern, velocity: u8) -> usize {
        self.for_each_note(pattern, |n| {
            n.velocity = velocity.clamp(MIN_VELOCITY, MAX_VELOCITY);
        })
    }

    pub fn set_selected_pan(&mut self, pattern: &mut Pattern, pan: i8) -> usize {
        self.for_each_note(pattern, |n| {
            n.pan = pan.clamp(MIN_PAN, MAX_PAN);
        })
    }

    pub fn nudge_selected(
        &mut self,
        pattern: &mut Pattern,
        delta_ticks: i64,
        delta_key: i32,
    ) -> usize {
        self.for_each_note(pattern, |n| {
            n.start = (n.start as i64 + delta_ticks).max(0) as u64;
            n.key = (n.key as i32 + delta_key).clamp(0, 127) as u8;
        })
    }

    fn for_each_note(&mut self, pattern: &mut Pattern, mut f: impl FnMut(&mut Note)) -> usize {
        let target = self.selected_or_all_ids(pattern);
        let Some(pc) = self.channel(pattern) else {
            return 0;
        };
        let mut count = 0;
        for note in &mut pc.notes {
            if target.is_empty() || target.contains(&note.id) {
                f(note);
                count += 1;
            }
        }
        count
    }

    fn selected_or_all_ids(&self, pattern: &Pattern) -> Vec<u64> {
        if !self.selection.is_empty() {
            return self.selection.clone();
        }
        self.channel_ref(pattern)
            .map(|pc| pc.notes.iter().map(|n| n.id).collect())
            .unwrap_or_default()
    }

    // ------------------------------------------------------------ quantize

    pub fn quantize(&mut self, pattern: &mut Pattern, options: QuantizeOptions) -> usize {
        let target = self.selected_or_all_ids(pattern);
        let Some(pc) = self.channel(pattern) else {
            return 0;
        };
        let grid = options.grid.max(1);
        let strength = options.strength.clamp(0.0, 1.0) as f64;
        let swing = options.swing.clamp(0.5, 0.75) as f64;
        let swing_shift = ((swing - 0.5) / 0.25) * (grid as f64 / 3.0);
        let mut changed = 0;
        for note in &mut pc.notes {
            if !target.is_empty() && !target.contains(&note.id) {
                continue;
            }
            let before_start = note.start;
            let before_length = note.length;
            let before_velocity = note.velocity;
            if strength > 0.0 {
                let index = (note.start + grid / 2) / grid;
                let base = index * grid;
                let swung = if index % 2 == 1 {
                    base as f64 + swing_shift
                } else {
                    base as f64
                };
                let snapped = (note.start as f64 + (swung - note.start as f64) * strength)
                    .round()
                    .max(0.0) as u64;
                note.start = snapped;
            }
            if options.lengths && strength > 0.0 {
                let base = (note.length as f64 / grid as f64).round().max(1.0) * grid as f64;
                let snapped = (note.length as f64 + (base - note.length as f64) * strength)
                    .round()
                    .max(1.0) as u64;
                note.length = snapped;
            }
            if options.velocities && strength > 0.0 {
                let level = (note.velocity as f64 / 127.0 * 7.0).round() / 7.0 * 127.0;
                let snapped = (note.velocity as f64 + (level - note.velocity as f64) * strength)
                    .round()
                    .clamp(MIN_VELOCITY as f64, MAX_VELOCITY as f64)
                    as u8;
                note.velocity = snapped;
            }
            if note.start != before_start
                || note.length != before_length
                || note.velocity != before_velocity
            {
                changed += 1;
            }
        }
        changed
    }

    // ------------------------------------------------------------ humanize

    pub fn humanize(&mut self, pattern: &mut Pattern, options: HumanizeOptions) -> usize {
        let target = self.selected_or_all_ids(pattern);
        let Some(pc) = self.channel(pattern) else {
            return 0;
        };
        let mut rng = options.seed;
        let mut count = 0;
        for note in &mut pc.notes {
            if !target.is_empty() && !target.contains(&note.id) {
                continue;
            }
            let dt = jitter(&mut rng, options.time as i32);
            note.start = (note.start as i64 + dt as i64).max(0) as u64;
            let dv = jitter(&mut rng, options.velocity as i32);
            note.velocity = (note.velocity as i32 + dv)
                .clamp(MIN_VELOCITY as i32, MAX_VELOCITY as i32) as u8;
            let dp = jitter(&mut rng, options.pan as i32);
            note.pan = (note.pan as i32 + dp).clamp(MIN_PAN as i32, MAX_PAN as i32) as i8;
            count += 1;
        }
        count
    }

    // -------------------------------------------------------------- legato

    /// Extend each selected note to the next selected note's start plus `overlap`
    /// ticks. The last note is left untouched.
    pub fn legato(&mut self, pattern: &mut Pattern, overlap: u64) -> usize {
        let target = self.selected_or_all_ids(pattern);
        let Some(pc) = self.channel(pattern) else {
            return 0;
        };
        let mut spans: Vec<(usize, u64)> = pc
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| target.is_empty() || target.contains(&n.id))
            .map(|(i, n)| (i, n.start))
            .collect();
        if spans.len() < 2 {
            return 0;
        }
        spans.sort_by_key(|(_, start)| *start);
        let mut changed = 0;
        for w in 0..spans.len() - 1 {
            let (i, start) = spans[w];
            let next_start = spans[w + 1].1;
            if next_start <= start {
                continue;
            }
            let new_len = next_start.saturating_sub(start).saturating_add(overlap).max(1);
            if pc.notes[i].length != new_len {
                pc.notes[i].length = new_len;
                changed += 1;
            }
        }
        changed
    }

    // ------------------------------------------------------------ arpeggio

    /// Replace the selected chord with an arpeggio derived from its keys.
    pub fn arpeggiate(&mut self, pattern: &mut Pattern, options: ArpOptions) -> Vec<u64> {
        let selected = self.selected_notes(pattern);
        if selected.is_empty() {
            return Vec::new();
        }
        let start = selected.iter().map(|n| n.start).min().unwrap_or(0);
        let velocity = selected.iter().map(|n| n.velocity).max().unwrap_or(100);
        let pan = selected[0].pan;
        let mut keys: Vec<u8> = selected
            .iter()
            .map(|n| n.key)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let octaves = options.octaves.max(1) as i32;
        if octaves > 1 {
            let base = keys.clone();
            for octave in 1..octaves {
                for key in &base {
                    let k = *key as i32 + octave * 12;
                    if k <= 127 {
                        keys.push(k as u8);
                    }
                }
            }
        }
        if keys.is_empty() {
            return Vec::new();
        }
        let mut order = keys.clone();
        match options.direction {
            ArpDirection::Up => {}
            ArpDirection::Down => order.reverse(),
            ArpDirection::UpDown => {
                order.extend(keys.iter().rev().skip(1).take(keys.len().saturating_sub(1)));
            }
        }
        let step = options.step.max(1);
        let length = options.note_length.max(1).min(step);
        let repeats = options.repeats.max(1) as usize;
        let ids_to_remove: Vec<u64> = selected.iter().map(|n| n.id).collect();
        if let Some(pc) = self.channel(pattern) {
            pc.notes.retain(|n| !ids_to_remove.contains(&n.id));
        }
        let mut ids = Vec::new();
        for repeat in 0..repeats {
            for (i, key) in order.iter().enumerate() {
                let note = Note {
                    id: 0,
                    key: *key,
                    start: start + (repeat * order.len() + i) as u64 * step,
                    length,
                    velocity,
                    pan,
                    release: selected[0].release,
                };
                let stored = add_note(pattern, self.channel_id, note);
                ids.push(stored.id);
            }
        }
        self.select(ids.iter().copied());
        ids
    }

    // -------------------------------------------------------------- chords

    pub fn stamp_chord(
        &mut self,
        pattern: &mut Pattern,
        root: u8,
        start_tick: u64,
        length: u64,
        chord: Chord,
    ) -> Vec<u64> {
        let start = quantize_tick(start_tick, self.grid);
        let length = length.max(1);
        let mut ids = Vec::new();
        for key in chord_keys(root, chord) {
            let note = Note {
                id: 0,
                key,
                start,
                length,
                velocity: self.default_velocity.clamp(MIN_VELOCITY, MAX_VELOCITY),
                pan: self.default_pan.clamp(MIN_PAN, MAX_PAN),
                release: 64,
            };
            let stored = add_note(pattern, self.channel_id, note);
            ids.push(stored.id);
        }
        self.select(ids.iter().copied());
        ids
    }

    // ------------------------------------------------------------ CC lanes

    pub fn add_cc(
        &self,
        pattern: &mut Pattern,
        tick: u64,
        controller: u8,
        value: u8,
    ) -> CcEvent {
        let tick = quantize_tick(tick, self.grid);
        let ev = CcEvent {
            tick,
            controller,
            value: value.min(127),
        };
        let pc = pattern.channel_mut(self.channel_id, pattern.length_steps.max(1));
        pc.cc.retain(|c| !(c.tick == tick && c.controller == controller));
        pc.cc.push(ev);
        pc.cc.sort_by_key(|c| (c.tick, c.controller));
        ev
    }

    pub fn clear_cc(&self, pattern: &mut Pattern, controller: Option<u8>) -> usize {
        let Some(pc) = self.channel(pattern) else {
            return 0;
        };
        let before = pc.cc.len();
        match controller {
            Some(c) => pc.cc.retain(|ev| ev.controller != c),
            None => pc.cc.clear(),
        }
        before - pc.cc.len()
    }

    /// Last CC value at or before `tick`, if any.
    pub fn cc_at(&self, pattern: &Pattern, controller: u8, tick: u64) -> Option<u8> {
        self.channel_ref(pattern)?
            .cc
            .iter()
            .filter(|c| c.controller == controller && c.tick <= tick)
            .max_by_key(|c| c.tick)
            .map(|c| c.value)
    }

    /// Notes from other patterns for ghost display, filtered by `self.ghosts`
    /// when it is non-empty.
    pub fn ghost_notes(&self, project: &StudioProject) -> Vec<GhostNote> {
        let mut out = Vec::new();
        for pattern in &project.patterns {
            if pattern.id == self.pattern_id {
                continue;
            }
            if !self.ghosts.is_empty() && !self.ghosts.contains(&pattern.id) {
                continue;
            }
            for pc in &pattern.channels {
                for note in &pc.notes {
                    out.push(GhostNote {
                        pattern_id: pattern.id,
                        channel_id: pc.channel_id,
                        note: *note,
                    });
                }
            }
        }
        out
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GhostNote {
    pub pattern_id: u32,
    pub channel_id: u32,
    pub note: Note,
}

#[derive(Clone, Copy, Debug)]
pub struct QuantizeOptions {
    pub grid: u64,
    /// 0..=1 blend from the original position to the grid.
    pub strength: f32,
    /// 0.5 = straight, up to 0.75 shuffle; applies to odd grid steps.
    pub swing: f32,
    pub lengths: bool,
    pub velocities: bool,
}

impl Default for QuantizeOptions {
    fn default() -> Self {
        Self {
            grid: GRID_1_16,
            strength: 1.0,
            swing: 0.5,
            lengths: false,
            velocities: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HumanizeOptions {
    /// ± ticks.
    pub time: i16,
    /// ± velocity units.
    pub velocity: i8,
    /// ± pan units.
    pub pan: i8,
    pub seed: u64,
}

impl Default for HumanizeOptions {
    fn default() -> Self {
        Self {
            time: 12,
            velocity: 10,
            pan: 4,
            seed: 0x9E37_79B9_7F4A_7C15,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArpDirection {
    Up,
    Down,
    UpDown,
}

#[derive(Clone, Copy, Debug)]
pub struct ArpOptions {
    pub step: u64,
    pub note_length: u64,
    pub octaves: u8,
    pub repeats: u8,
    pub direction: ArpDirection,
}

impl Default for ArpOptions {
    fn default() -> Self {
        Self {
            step: GRID_1_16 / 2,
            note_length: GRID_1_16 / 2,
            octaves: 1,
            repeats: 1,
            direction: ArpDirection::Up,
        }
    }
}

// -------------------------------------------------------------- free helpers

/// Grid choices as `(ticks, label)`.
pub fn grids() -> &'static [(u64, &'static str)] {
    &GRIDS
}

/// All notes in a channel sorted for display.
pub fn sorted_notes(pattern: &Pattern, channel_id: u32) -> Vec<Note> {
    let mut notes = pattern
        .channel(channel_id)
        .map(|c| c.notes.clone())
        .unwrap_or_default();
    notes.sort_by_key(|n| (n.start, n.key, n.id));
    notes
}

/// Furthest note end in ticks, or 0.
pub fn note_extent(pattern: &Pattern) -> u64 {
    pattern
        .channels
        .iter()
        .flat_map(|c| &c.notes)
        .map(|n| n.start.saturating_add(n.length))
        .max()
        .unwrap_or(0)
}

/// Pan CC (0..127, 64 centre) to the shared note pan (-64..63).
pub fn cc_pan_to_note(value: u8) -> i8 {
    (value as i32 - 64).clamp(MIN_PAN as i32, MAX_PAN as i32) as i8
}

/// Shared note pan (-64..63) to a 0..127 pan CC value.
pub fn note_pan_to_cc(pan: i8) -> u8 {
    (pan as i32 + 64).clamp(0, 127) as u8
}

fn jitter(rng: &mut u64, range: i32) -> i32 {
    if range <= 0 {
        return 0;
    }
    *rng = rng
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    let span = (range as i64) * 2 + 1;
    (((*rng >> 33) as i64) % span) as i32 - range
}

#[cfg(test)]
mod tests {
    use super::*;
    use redraw_sequencer::STEP_TICKS;

    fn pattern() -> Pattern {
        Pattern::new(1, "Roll", [90, 160, 90], 16)
    }

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

    #[test]
    fn names_black_keys_and_scales() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(61), "C#4");
        assert!(is_black_key(61));
        assert!(!is_black_key(60));
        assert!(Scale::Major.contains(0, 4));
        assert!(!Scale::Major.contains(0, 1));
        assert_eq!(Scale::Major.snap(0, 1), 0);
        assert_eq!(Scale::Major.snap(0, 6), 5);
        assert_eq!(Scale::MinorPentatonic.keys(0).len(), 54);
        let _ = Scale::ALL;
    }

    #[test]
    fn selection_copy_paste_and_duplicate() {
        let mut pat = pattern();
        let a = add_note(&mut pat, 1, note(0, 60, 0, 240));
        let b = add_note(&mut pat, 1, note(0, 64, 480, 240));
        let c = add_note(&mut pat, 1, note(0, 67, 960, 240));
        let mut roll = PianoRoll::new(1, 1);
        assert_eq!(roll.select_range(&pat, 0, 500, 60, 64), 2);
        assert_eq!(roll.copy(&pat), 2);
        let pasted = roll.paste(&mut pat, 1920);
        assert_eq!(pasted.len(), 2);
        assert_eq!(roll.selection.len(), 2);
        assert_eq!(pat.channel(1).unwrap().notes.len(), 5);
        assert_eq!(roll.selected_notes(&pat)[0].start, 1920);
        roll.select([c.id]);
        let dup = roll.duplicate(&mut pat, 240);
        assert_eq!(dup.len(), 1);
        assert_eq!(pat.channel(1).unwrap().notes.len(), 6);
        roll.clear_selection();
        assert_eq!(roll.delete_selected(&mut pat), 0);
        assert_eq!(pat.channel(1).unwrap().notes.len(), 6);
        let _ = (a, b);
    }

    #[test]
    fn draw_split_and_merge() {
        let mut pat = pattern();
        let mut roll = PianoRoll::new(1, 1);
        let drawn = roll.draw(&mut pat, 250, 64, None);
        assert_eq!(drawn.start, 240);
        assert_eq!(drawn.length, GRID_1_16);
        let right = roll.split(&mut pat, drawn.id, 360).unwrap();
        let notes = pat.channel(1).unwrap().notes.clone();
        let left = notes.iter().find(|n| n.id == drawn.id).unwrap();
        let new = notes.iter().find(|n| n.id == right).unwrap();
        assert_eq!(left.length, 120);
        assert_eq!(new.start, 360);
        assert_eq!(new.length, 120);
        assert_eq!(roll.merge_selected(&mut pat), Some(drawn.id));
        let merged = pat.channel(1).unwrap().notes.iter().find(|n| n.id == drawn.id).unwrap();
        assert_eq!(merged.start, 240);
        assert_eq!(merged.length, 240);
        assert_eq!(pat.channel(1).unwrap().notes.len(), 1);
    }

    #[test]
    fn chord_stamp_and_scale_snap() {
        let mut pat = pattern();
        let mut roll = PianoRoll::new(1, 1);
        let ids = roll.stamp_chord(&mut pat, 60, 241, 480, Chord::Minor7);
        assert_eq!(ids.len(), 4);
        let keys: Vec<u8> = roll.selected_notes(&pat).iter().map(|n| n.key).collect();
        assert_eq!(keys, vec![60, 63, 67, 70]);
        assert_eq!(chord_keys(60, Chord::Add9), vec![60, 64, 67, 74]);
    }

    #[test]
    fn quantize_strength_swing_lengths_and_velocity() {
        let mut pat = pattern();
        let a = add_note(&mut pat, 1, note(0, 60, 130, 200));
        let b = add_note(&mut pat, 1, note(0, 64, 370, 100));
        {
            let pc = pat.channel_mut(1, 16);
            pc.notes[0].velocity = 100;
            pc.notes[1].velocity = 30;
        }
        let mut roll = PianoRoll::new(1, 1);
        let changed = roll.quantize(
            &mut pat,
            QuantizeOptions {
                grid: STEP_TICKS,
                strength: 1.0,
                swing: 0.5,
                lengths: true,
                velocities: true,
            },
        );
        assert_eq!(changed, 2);
        let notes = sorted_notes(&pat, 1);
        assert_eq!(notes[0].start, 240);
        assert_eq!(notes[1].start, 480);
        assert_eq!(notes[0].length, 240);
        assert_eq!(notes[1].length, 240);
        assert_eq!(notes[0].velocity, 109);
        assert_eq!(notes[1].velocity, 36);
        let _ = (a, b);

        let mut pat = pattern();
        add_note(&mut pat, 1, note(0, 60, 200, 240));
        let mut roll = PianoRoll::new(1, 1);
        roll.quantize(
            &mut pat,
            QuantizeOptions {
                grid: STEP_TICKS,
                strength: 0.5,
                swing: 0.0,
                lengths: false,
                velocities: false,
            },
        );
        assert_eq!(pat.channel(1).unwrap().notes[0].start, 220);
    }

    #[test]
    fn humanize_is_bounded_and_deterministic() {
        let mut a = pattern();
        let mut b = pattern();
        for pat in [&mut a, &mut b] {
            add_note(pat, 1, note(0, 60, 480, 240));
            let pc = pat.channel_mut(1, 16);
            pc.notes[0].velocity = 64;
        }
        let opts = HumanizeOptions {
            time: 20,
            velocity: 10,
            pan: 5,
            seed: 7,
        };
        let mut ra = PianoRoll::new(1, 1);
        let mut rb = PianoRoll::new(1, 1);
        ra.humanize(&mut a, opts);
        rb.humanize(&mut b, opts);
        assert_eq!(a.channel(1).unwrap().notes[0], b.channel(1).unwrap().notes[0]);
        let n = a.channel(1).unwrap().notes[0];
        assert!(n.start >= 460 && n.start <= 500);
        assert!(n.velocity >= 54 && n.velocity <= 74);
        assert!(n.pan >= -5 && n.pan <= 5);
    }

    #[test]
    fn legato_and_arp() {
        let mut pat = pattern();
        add_note(&mut pat, 1, note(0, 60, 0, 120));
        add_note(&mut pat, 1, note(0, 60, 240, 120));
        add_note(&mut pat, 1, note(0, 60, 480, 120));
        let mut roll = PianoRoll::new(1, 1);
        assert_eq!(roll.legato(&mut pat, 0), 2);
        let notes = sorted_notes(&pat, 1);
        assert_eq!(notes[0].length, 240);
        assert_eq!(notes[1].length, 240);
        assert_eq!(notes[2].length, 120);

        let mut pat = pattern();
        let mut roll = PianoRoll::new(1, 1);
        roll.stamp_chord(&mut pat, 60, 0, 480, Chord::Major);
        let ids = roll.arpeggiate(
            &mut pat,
            ArpOptions {
                step: 120,
                note_length: 100,
                octaves: 2,
                repeats: 1,
                direction: ArpDirection::UpDown,
            },
        );
        assert_eq!(ids.len(), 11);
        let notes = sorted_notes(&pat, 1);
        assert_eq!(notes.len(), 11);
        assert_eq!(notes[0].key, 60);
        assert_eq!(notes[1].key, 64);
        assert_eq!(notes[2].key, 67);
        assert_eq!(notes[3].key, 72);
        assert_eq!(notes[4].key, 76);
        assert_eq!(notes[5].key, 79);
        assert_eq!(notes[6].key, 76);
        assert_eq!(notes[0].start, 0);
        assert_eq!(notes[1].start, 120);
        assert_eq!(notes[9].start, 1080);
    }

    #[test]
    fn cc_lanes_and_ghosts() {
        let mut pat = pattern();
        let mut roll = PianoRoll::new(1, 1);
        roll.set_grid(1);
        roll.add_cc(&mut pat, 10, CC_PAN, 64);
        roll.add_cc(&mut pat, 10, CC_PAN, 96);
        roll.add_cc(&mut pat, 20, CC_PAN, 32);
        assert_eq!(roll.cc_at(&pat, CC_PAN, 0), None);
        assert_eq!(roll.cc_at(&pat, CC_PAN, 10), Some(96));
        assert_eq!(roll.cc_at(&pat, CC_PAN, 15), Some(96));
        assert_eq!(pat.channel(1).unwrap().cc.len(), 2);
        assert_eq!(roll.clear_cc(&mut pat, Some(CC_PAN)), 2);
        assert!(pat.channel(1).unwrap().cc.is_empty());

        assert_eq!(note_extent(&pat), 0);
        let mut project = StudioProject::default();
        project.patterns[0] = pat;
        let mut other = Pattern::new(2, "Ghost", [10, 20, 30], 16);
        add_note(&mut other, 1, note(0, 72, 0, 240));
        project.patterns.push(other);
        let ghosts = roll.ghost_notes(&project);
        assert_eq!(ghosts.len(), 1);
        assert_eq!(ghosts[0].pattern_id, 2);
        roll.ghosts = vec![1];
        assert!(roll.ghost_notes(&project).is_empty());
        assert_eq!(cc_pan_to_note(0), -64);
        assert_eq!(cc_pan_to_note(127), 63);
        assert_eq!(note_pan_to_cc(-64), 0);
    }
}
