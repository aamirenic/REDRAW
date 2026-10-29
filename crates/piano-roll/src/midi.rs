//! Standard MIDI File (SMF) import and export.
//!
//! Type 0 and type 1 files are supported through `midly`. Everything is
//! converted to the project's 960 PPQ grid. The importer keeps every note's
//! original MIDI channel and control-change lane; [`SmfImport::to_patterns`]
//! turns each non-empty track into a REDRAW pattern, with one pattern channel
//! per MIDI channel. Textual tempo maps are returned for callers that want them
//! (the project itself currently stores one BPM, see D-020).
#![forbid(unsafe_code)]

use crate::cc_pan_to_note;
use midly::num::{u15, u24, u28, u4, u7};
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind,
};
use redraw_shared::{CcEvent, Note, Pattern, PatternChannel, StudioProject, MAX_PATTERNS, PPQ};
use redraw_sequencer::{add_note, STEP_TICKS};
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

pub const PROJECT_PPQ: u16 = PPQ as u16;
pub const DEFAULT_US_PER_QUARTER: u32 = 500_000;
pub const MAX_DELTA: u64 = 0x0FFF_FFFF;
const DEFAULT_STEPS: u32 = 16;

#[derive(Clone, Debug)]
pub enum MidiError {
    Parse(String),
    Io(String),
    /// SMPTE timecode timing is not supported; SMF musical time must be PPQ.
    SmpteTiming,
    UnsupportedFormat(u8),
    /// A track delta would not fit the 28-bit MIDI variable-length integer.
    TickOverflow,
}

impl fmt::Display for MidiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MidiError::Parse(msg) => write!(f, "invalid MIDI file: {msg}"),
            MidiError::Io(msg) => write!(f, "MIDI I/O error: {msg}"),
            MidiError::SmpteTiming => write!(f, "SMPTE timecode MIDI timing is unsupported"),
            MidiError::UnsupportedFormat(fmt) => write!(f, "unsupported MIDI format {fmt}"),
            MidiError::TickOverflow => write!(f, "MIDI event delta exceeds 28 bits"),
        }
    }
}

impl std::error::Error for MidiError {}

// ------------------------------------------------------------------ tempo

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TempoChange {
    /// Project ticks (960 PPQ).
    pub tick: u64,
    pub us_per_quarter: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TempoMap {
    pub changes: Vec<TempoChange>,
}

impl Default for TempoMap {
    fn default() -> Self {
        Self {
            changes: vec![TempoChange {
                tick: 0,
                us_per_quarter: DEFAULT_US_PER_QUARTER,
            }],
        }
    }
}

impl TempoMap {
    /// Sort changes and guarantee a tempo at tick 0 (120 BPM when the file
    /// starts with a later tempo event). Later events at the same tick win.
    pub fn new(changes: Vec<TempoChange>) -> Self {
        let mut by_tick = BTreeMap::new();
        for change in changes {
            if change.us_per_quarter > 0 {
                by_tick.insert(change.tick, change.us_per_quarter);
            }
        }
        if by_tick.is_empty() {
            return Self::default();
        }
        if by_tick.keys().next().copied().unwrap_or(0) > 0 {
            by_tick.insert(0, DEFAULT_US_PER_QUARTER);
        }
        Self {
            changes: by_tick
                .into_iter()
                .map(|(tick, us_per_quarter)| TempoChange {
                    tick,
                    us_per_quarter,
                })
                .collect(),
        }
    }

    pub fn bpm_at(&self, tick: u64) -> f32 {
        let us = self
            .changes
            .iter()
            .take_while(|c| c.tick <= tick)
            .last()
            .map(|c| c.us_per_quarter)
            .unwrap_or(DEFAULT_US_PER_QUARTER);
        60_000_000.0 / us.max(1) as f32
    }

    pub fn initial_bpm(&self) -> f32 {
        self.bpm_at(0)
    }
}

// ------------------------------------------------------------------ import

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportedNote {
    pub key: u8,
    pub start: u64,
    pub length: u64,
    pub velocity: u8,
    pub channel: u8,
    pub pan: i8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportedCc {
    pub tick: u64,
    pub controller: u8,
    pub value: u8,
    pub channel: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedTrack {
    pub name: String,
    pub notes: Vec<ImportedNote>,
    pub cc: Vec<ImportedCc>,
}

impl ImportedTrack {
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty() && self.cc.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmfImport {
    /// 0 = single track, 1 = parallel tracks.
    pub format: u8,
    /// Source ticks per quarter note.
    pub ppq: u16,
    pub tempo: TempoMap,
    pub time_signature: (u8, u8),
    pub tracks: Vec<ImportedTrack>,
}

impl SmfImport {
    pub fn note_count(&self) -> usize {
        self.tracks.iter().map(|t| t.notes.len()).sum()
    }

    pub fn initial_bpm(&self) -> f32 {
        self.tempo.initial_bpm()
    }

    /// Convert non-empty SMF tracks into patterns. One pattern channel is
    /// created per source MIDI channel (`midi channel + 1`).
    pub fn to_patterns(&self, first_id: u32, color: [u8; 3]) -> Vec<Pattern> {
        let mut patterns = Vec::new();
        let mut next_id = first_id;
        for (index, track) in self.tracks.iter().enumerate() {
            if track.is_empty() {
                continue;
            }
            let name = if track.name.trim().is_empty() {
                format!("MIDI Track {}", index + 1)
            } else {
                track.name.clone()
            };
            let mut pattern = Pattern::new(next_id, name, color, DEFAULT_STEPS);
            next_id = next_id.saturating_add(1);

            let note_end = track
                .notes
                .iter()
                .map(|n| n.start.saturating_add(n.length))
                .max()
                .unwrap_or(0);
            let cc_end = track.cc.iter().map(|c| c.tick.saturating_add(1)).max().unwrap_or(0);
            let needed = note_end.max(cc_end).max(1);
            let mut steps = needed.div_ceil(STEP_TICKS) as u32;
            steps = steps.max(DEFAULT_STEPS);
            steps = steps.div_ceil(DEFAULT_STEPS) * DEFAULT_STEPS;
            pattern.length_steps = steps;

            for note in &track.notes {
                let channel_id = note.channel as u32 + 1;
                let stored = add_note(
                    &mut pattern,
                    channel_id,
                    Note {
                        id: 0,
                        key: note.key.min(127),
                        start: note.start,
                        length: note.length.max(1),
                        velocity: note.velocity.clamp(1, 127),
                        pan: note.pan.clamp(crate::MIN_PAN, crate::MAX_PAN),
                        release: 64,
                    },
                );
                let _ = stored;
            }
            for cc in &track.cc {
                let channel_id = cc.channel as u32 + 1;
                let pc = pattern.channel_mut(channel_id, steps);
                pc.cc.push(CcEvent {
                    tick: cc.tick,
                    controller: cc.controller.min(127),
                    value: cc.value.min(127),
                });
            }
            for pc in &mut pattern.channels {
                pc.cc.sort_by_key(|c| (c.tick, c.controller));
                let mut deduped: Vec<CcEvent> = Vec::with_capacity(pc.cc.len());
                for event in pc.cc.drain(..) {
                    if deduped.last().map(|prev: &CcEvent| {
                        prev.tick == event.tick && prev.controller == event.controller
                    }) == Some(true)
                    {
                        deduped.pop();
                    }
                    deduped.push(event);
                }
                pc.cc = deduped;
            }
            patterns.push(pattern);
        }
        patterns
    }
}

#[derive(Clone, Copy)]
struct ActiveNote {
    start_src: u64,
    velocity: u8,
    pan: i8,
}

/// Parse an SMF from bytes. Tempo and time-signature events are collected from
/// every track; notes and CC events are quantized to the 960-PPQ project grid.
pub fn import_smf(data: &[u8]) -> Result<SmfImport, MidiError> {
    let smf = Smf::parse(data).map_err(|e| MidiError::Parse(e.to_string()))?;
    let (format, ppq) = match smf.header.format {
        Format::SingleTrack => (0u8, match smf.header.timing {
            Timing::Metrical(q) => q.as_int(),
            Timing::Timecode(..) => return Err(MidiError::SmpteTiming),
        }),
        Format::Parallel => (1u8, match smf.header.timing {
            Timing::Metrical(q) => q.as_int(),
            Timing::Timecode(..) => return Err(MidiError::SmpteTiming),
        }),
        Format::Sequential => {
            return Err(MidiError::UnsupportedFormat(2));
        }
    };
    if ppq == 0 {
        return Err(MidiError::Parse("zero PPQ".into()));
    }
    let convert = |tick: u64| -> u64 {
        ((tick as u128 * PROJECT_PPQ as u128 + ppq as u128 / 2) / ppq as u128) as u64
    };

    let mut tempo_changes: Vec<TempoChange> = Vec::new();
    let mut time_signatures: Vec<(u64, (u8, u8))> = Vec::new();
    let mut tracks = Vec::with_capacity(smf.tracks.len());

    for track in smf.tracks.iter() {
        let mut abs_src = 0u64;
        let mut name = String::new();
        let mut active: BTreeMap<(u8, u8), Vec<ActiveNote>> = BTreeMap::new();
        let mut last_pan = [0i8; 16];
        let mut notes = Vec::new();
        let mut cc = Vec::new();

        for event in track {
            abs_src = abs_src.saturating_add(event.delta.as_int() as u64);
            let tick = convert(abs_src);
            match event.kind {
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int() & 0x0F;
                    match message {
                        MidiMessage::NoteOn { key, vel } => {
                            let key = key.as_int() & 0x7F;
                            let velocity = vel.as_int() & 0x7F;
                            if velocity == 0 {
                                close_note(
                                    &mut active,
                                    ch,
                                    key,
                                    abs_src,
                                    &mut notes,
                                    convert,
                                );
                            } else {
                                active.entry((ch, key)).or_default().push(ActiveNote {
                                    start_src: abs_src,
                                    velocity,
                                    pan: last_pan[ch as usize],
                                });
                            }
                        }
                        MidiMessage::NoteOff { key, .. } => {
                            close_note(
                                &mut active,
                                ch,
                                key.as_int() & 0x7F,
                                abs_src,
                                &mut notes,
                                convert,
                            );
                        }
                        MidiMessage::Controller { controller, value } => {
                            let controller = controller.as_int() & 0x7F;
                            let value = value.as_int() & 0x7F;
                            if let Some(existing) = cc.iter_mut().find(|c: &&mut ImportedCc| {
                                c.tick == tick
                                    && c.channel == ch
                                    && c.controller == controller
                            }) {
                                existing.value = value;
                            } else {
                                cc.push(ImportedCc {
                                    tick,
                                    controller,
                                    value,
                                    channel: ch,
                                });
                            }
                            if controller == crate::CC_PAN {
                                last_pan[ch as usize] = cc_pan_to_note(value);
                            }
                        }
                        _ => {}
                    }
                }
                TrackEventKind::Meta(meta) => match meta {
                    MetaMessage::TrackName(bytes) => {
                        if name.is_empty() {
                            name = String::from_utf8_lossy(bytes)
                                .trim_end_matches('\0')
                                .to_string();
                        }
                    }
                    MetaMessage::Tempo(us) => {
                        let us = us.as_int();
                        if us > 0 {
                            tempo_changes.push(TempoChange {
                                tick,
                                us_per_quarter: us,
                            });
                        }
                    }
                    MetaMessage::TimeSignature(num, den_pow, _, _) => {
                        let den = 1u16
                            .checked_shl(den_pow as u32)
                            .unwrap_or(4)
                            .min(64) as u8;
                        time_signatures.push((tick, (num.max(1), den.max(1))));
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        let end_tick = convert(abs_src);
        let mut keys: Vec<(u8, u8)> = active.keys().copied().collect();
        keys.sort_unstable();
        for key in keys {
            if let Some(queue) = active.get_mut(&key) {
                while let Some(note) = queue.first().copied() {
                    queue.remove(0);
                    notes.push(ImportedNote {
                        key: key.1,
                        start: convert(note.start_src),
                        length: end_tick.saturating_sub(convert(note.start_src)).max(1),
                        velocity: note.velocity,
                        channel: key.0,
                        pan: note.pan,
                    });
                }
            }
        }
        notes.sort_by_key(|n| (n.start, n.channel, n.key, n.length));
        cc.sort_by_key(|c| (c.tick, c.channel, c.controller, c.value));
        tracks.push(ImportedTrack { name, notes, cc });
    }

    let tempo = TempoMap::new(tempo_changes);
    time_signatures.sort_by_key(|(tick, _)| *tick);
    let time_signature = time_signatures
        .first()
        .map(|(_, ts)| *ts)
        .unwrap_or((4, 4));
    Ok(SmfImport {
        format,
        ppq,
        tempo,
        time_signature,
        tracks,
    })
}

fn close_note(
    active: &mut BTreeMap<(u8, u8), Vec<ActiveNote>>,
    channel: u8,
    key: u8,
    abs_src: u64,
    notes: &mut Vec<ImportedNote>,
    convert: impl Fn(u64) -> u64,
) {
    let Some(queue) = active.get_mut(&(channel, key)) else {
        return;
    };
    if queue.is_empty() {
        return;
    }
    let note = queue.remove(0);
    let start = convert(note.start_src);
    let end = convert(abs_src);
    notes.push(ImportedNote {
        key,
        start,
        length: end.saturating_sub(start).max(1),
        velocity: note.velocity,
        channel,
        pan: note.pan,
    });
}

pub fn import_smf_file(path: impl AsRef<Path>) -> Result<SmfImport, MidiError> {
    let data = std::fs::read(path).map_err(|e| MidiError::Io(e.to_string()))?;
    import_smf(&data)
}

/// Import a file and append its non-empty tracks to `project` with fresh pattern
/// IDs. The first imported tempo becomes the project BPM, and the first time
/// signature becomes the project signature. Returns the new pattern IDs.
pub fn import_into_project(
    project: &mut StudioProject,
    data: &[u8],
    color: [u8; 3],
) -> Result<Vec<u32>, MidiError> {
    let import = import_smf(data)?;
    let first_id = project
        .patterns
        .iter()
        .map(|p| p.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
        .max(project.next_pattern_id.max(1));
    let imported = import.to_patterns(first_id, color);
    if imported.is_empty() {
        return Ok(Vec::new());
    }
    let room = MAX_PATTERNS.saturating_sub(project.patterns.len());
    let mut ids = Vec::new();
    for pattern in imported.into_iter().take(room) {
        ids.push(pattern.id);
        project.next_pattern_id = project.next_pattern_id.max(pattern.id.saturating_add(1));
        project.patterns.push(pattern);
    }
    project.bpm = import.initial_bpm().clamp(10.0, 999.0);
    project.time_sig = import.time_signature;
    if let Some(first) = ids.first() {
        if let Some(index) = project.patterns.iter().position(|p| p.id == *first) {
            project.current_pattern = index;
        }
    }
    Ok(ids)
}

pub fn import_file_into_project(
    project: &mut StudioProject,
    path: impl AsRef<Path>,
    color: [u8; 3],
) -> Result<Vec<u32>, MidiError> {
    let data = std::fs::read(path).map_err(|e| MidiError::Io(e.to_string()))?;
    import_into_project(project, &data, color)
}

// ------------------------------------------------------------------ export

/// Export one pattern as an SMF type 0 file.
pub fn export_pattern_smf(pattern: &Pattern, bpm: f32) -> Result<Vec<u8>, MidiError> {
    let mut smf = Smf::new(Header::new(
        Format::SingleTrack,
        Timing::Metrical(u15::new(PROJECT_PPQ)),
    ));
    let track = build_track(
        &pattern.channels,
        Some(pattern.name.as_str()),
        Some(bpm),
        Some((4, 4)),
    )?;
    smf.tracks.push(track);
    write_smf(&smf)
}

/// Export all patterns as an SMF type 1 file (one conductor track plus one
/// track per pattern, as required by D-020).
pub fn export_project_smf(project: &StudioProject) -> Result<Vec<u8>, MidiError> {
    let mut smf = Smf::new(Header::new(
        Format::Parallel,
        Timing::Metrical(u15::new(PROJECT_PPQ)),
    ));
    let conductor_name = "REDRAW".to_string();
    let conductor = build_track(
        &[],
        Some(conductor_name.as_str()),
        Some(project.bpm),
        Some(project.time_sig),
    )?;
    smf.tracks.push(conductor);
    for pattern in &project.patterns {
        if pattern.channels.iter().all(|c| c.notes.is_empty() && c.cc.is_empty()) {
            continue;
        }
        smf.tracks.push(build_track(
            &pattern.channels,
            Some(pattern.name.as_str()),
            None,
            None,
        )?);
    }
    write_smf(&smf)
}

pub fn write_smf_file(path: impl AsRef<Path>, bytes: &[u8]) -> Result<(), MidiError> {
    std::fs::write(path, bytes).map_err(|e| MidiError::Io(e.to_string()))
}

struct TimedEvent<'a> {
    tick: u64,
    order: u8,
    kind: TrackEventKind<'a>,
}

fn build_track<'a>(
    channels: &'a [PatternChannel],
    name: Option<&'a str>,
    bpm: Option<f32>,
    time_sig: Option<(u8, u8)>,
) -> Result<midly::Track<'a>, MidiError> {
    let mut events: Vec<TimedEvent<'a>> = Vec::new();
    if let Some(name) = name.filter(|n| !n.is_empty()) {
        events.push(TimedEvent {
            tick: 0,
            order: 0,
            kind: TrackEventKind::Meta(MetaMessage::TrackName(name.as_bytes())),
        });
    }
    if let Some(bpm) = bpm {
        let us = (60_000_000.0 / bpm.clamp(4.0, 999.0)).round() as u32;
        events.push(TimedEvent {
            tick: 0,
            order: 0,
            kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new(
                us.clamp(1, 0x00FF_FFFF),
            ))),
        });
    }
    if let Some((num, den)) = time_sig {
        let den_pow = den.max(1).next_power_of_two().trailing_zeros() as u8;
        events.push(TimedEvent {
            tick: 0,
            order: 0,
            kind: TrackEventKind::Meta(MetaMessage::TimeSignature(
                num.max(1),
                den_pow,
                24,
                8,
            )),
        });
    }

    let mut end_tick = 0u64;
    for pc in channels {
        let midi_channel = ((pc.channel_id.saturating_sub(1)) % 16) as u8;
        for note in &pc.notes {
            let start = note.start;
            let stop = note.start.saturating_add(note.length.max(1));
            end_tick = end_tick.max(stop);
            events.push(TimedEvent {
                tick: start,
                order: 3,
                kind: TrackEventKind::Midi {
                    channel: u4::new(midi_channel),
                    message: MidiMessage::NoteOn {
                        key: u7::new(note.key.min(127)),
                        vel: u7::new(note.velocity.clamp(1, 127)),
                    },
                },
            });
            events.push(TimedEvent {
                tick: stop,
                order: 1,
                kind: TrackEventKind::Midi {
                    channel: u4::new(midi_channel),
                    message: MidiMessage::NoteOff {
                        key: u7::new(note.key.min(127)),
                        vel: u7::new(0),
                    },
                },
            });
        }
        for cc in &pc.cc {
            end_tick = end_tick.max(cc.tick);
            events.push(TimedEvent {
                tick: cc.tick,
                order: 2,
                kind: TrackEventKind::Midi {
                    channel: u4::new(midi_channel),
                    message: MidiMessage::Controller {
                        controller: u7::new(cc.controller.min(127)),
                        value: u7::new(cc.value.min(127)),
                    },
                },
            });
        }
    }
    events.push(TimedEvent {
        tick: end_tick,
        order: 5,
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    events.sort_by_key(|e| (e.tick, e.order));

    let mut track: midly::Track<'a> = Vec::with_capacity(events.len());
    let mut last_tick = 0u64;
    for event in events {
        let delta = event.tick.saturating_sub(last_tick);
        if delta > MAX_DELTA {
            return Err(MidiError::TickOverflow);
        }
        track.push(TrackEvent {
            delta: u28::new(delta as u32),
            kind: event.kind,
        });
        last_tick = event.tick;
    }
    Ok(track)
}

fn write_smf(smf: &Smf<'_>) -> Result<Vec<u8>, MidiError> {
    let mut bytes = Vec::new();
    smf.write_std(&mut bytes)
        .map_err(|e| MidiError::Io(e.to_string()))?;
    Ok(bytes)
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use redraw_shared::Note;

    fn test_pattern() -> Pattern {
        let mut pattern = Pattern::new(5, "Lead", [90, 160, 90], 16);
        add_note(
            &mut pattern,
            1,
            Note {
                id: 0,
                key: 60,
                start: 240,
                length: 480,
                velocity: 90,
                pan: 0,
                release: 64,
            },
        );
        add_note(
            &mut pattern,
            2,
            Note {
                id: 0,
                key: 64,
                start: 0,
                length: 240,
                velocity: 77,
                pan: 0,
                release: 64,
            },
        );
        let pc = pattern.channel_mut(2, 16);
        pc.cc.push(CcEvent {
            tick: 120,
            controller: 74,
            value: 99,
        });
        pattern
    }

    #[test]
    fn single_track_round_trip() {
        let pattern = test_pattern();
        let bytes = export_pattern_smf(&pattern, 128.0).unwrap();
        let import = import_smf(&bytes).unwrap();
        assert_eq!(import.format, 0);
        assert_eq!(import.ppq, 960);
        assert!((import.initial_bpm() - 128.0).abs() < 0.01);
        assert_eq!(import.tracks.len(), 1);
        assert_eq!(import.tracks[0].name, "Lead");
        assert_eq!(import.note_count(), 2);
        assert_eq!(import.tracks[0].cc.len(), 1);
        assert_eq!(import.tracks[0].cc[0].controller, 74);
        assert_eq!(import.tracks[0].cc[0].value, 99);

        let patterns = import.to_patterns(20, [1, 2, 3]);
        assert_eq!(patterns.len(), 1);
        let p = &patterns[0];
        assert_eq!(p.id, 20);
        assert_eq!(p.name, "Lead");
        assert_eq!(p.channels.len(), 2);
        let n1 = p.channel(1).unwrap().notes[0];
        assert_eq!((n1.key, n1.start, n1.length, n1.velocity), (60, 240, 480, 90));
        let n2 = p.channel(2).unwrap().notes[0];
        assert_eq!((n2.key, n2.start, n2.length), (64, 0, 240));
        assert_eq!(p.channel(2).unwrap().cc[0].tick, 120);
        assert_eq!(p.channel(2).unwrap().cc[0].value, 99);
    }

    #[test]
    fn project_export_is_type_one_with_one_track_per_pattern() {
        let mut project = StudioProject::default();
        project.bpm = 100.0;
        project.patterns = vec![test_pattern(), Pattern::new(6, "Bass", [1, 1, 1], 16)];
        add_note(
            &mut project.patterns[1],
            3,
            Note {
                id: 0,
                key: 36,
                start: 960,
                length: 240,
                velocity: 110,
                pan: 0,
                release: 64,
            },
        );
        let bytes = export_project_smf(&project).unwrap();
        let import = import_smf(&bytes).unwrap();
        assert_eq!(import.format, 1);
        assert_eq!(import.tracks.len(), 3);
        assert_eq!(import.tracks[0].name, "REDRAW");
        assert_eq!(import.tracks[0].notes.len(), 0);
        assert_eq!(import.tracks[1].name, "Lead");
        assert_eq!(import.tracks[2].name, "Bass");
        let patterns = import.to_patterns(1, [0, 0, 0]);
        assert_eq!(patterns.len(), 2);
        assert_eq!(patterns[0].name, "Lead");
        assert_eq!(patterns[1].name, "Bass");
        assert_eq!(patterns[1].channel(3).unwrap().notes[0].key, 36);
    }

    #[test]
    fn import_appends_with_fresh_ids_and_applies_tempo() {
        let mut project = StudioProject::default();
        project.bpm = 140.0;
        project.patterns[0].id = 7;
        project.next_pattern_id = 8;
        let bytes = export_pattern_smf(&test_pattern(), 96.0).unwrap();

        let first = import_into_project(&mut project, &bytes, [4, 5, 6]).unwrap();
        assert_eq!(first, vec![8]);
        assert!((project.bpm - 96.0).abs() < 0.01);
        assert_eq!(project.time_sig, (4, 4));
        assert_eq!(project.next_pattern_id, 9);
        let position = project
            .patterns
            .iter()
            .position(|p| p.id == 8)
            .expect("imported pattern");
        assert_eq!(project.current_pattern, position);
        assert_eq!(project.patterns[position].name, "Lead");

        let second = import_into_project(&mut project, &bytes, [4, 5, 6]).unwrap();
        assert_eq!(second, vec![9]);
        assert_eq!(project.patterns.len(), 3);
    }

    fn raw_smf(ppq: u16) -> Vec<u8> {
        let mut smf = Smf::new(Header::new(
            Format::SingleTrack,
            Timing::Metrical(u15::new(ppq)),
        ));
        let mut track: midly::Track = Vec::new();
        let mut push = |delta: u32, kind: TrackEventKind<'static>| {
            track.push(TrackEvent {
                delta: u28::new(delta),
                kind,
            });
        };
        push(
            0,
            TrackEventKind::Meta(MetaMessage::TrackName(b"Piano")),
        );
        push(
            0,
            TrackEventKind::Meta(MetaMessage::Tempo(u24::new(600_000))),
        );
        push(
            0,
            TrackEventKind::Meta(MetaMessage::TimeSignature(3, 2, 24, 8)),
        );
        push(
            0,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::Controller {
                    controller: u7::new(10),
                    value: u7::new(127),
                },
            },
        );
        push(
            0,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOn {
                    key: u7::new(60),
                    vel: u7::new(100),
                },
            },
        );
        push(
            480,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOff {
                    key: u7::new(60),
                    vel: u7::new(0),
                },
            },
        );
        push(
            0,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOn {
                    key: u7::new(64),
                    vel: u7::new(80),
                },
            },
        );
        push(
            240,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOn {
                    key: u7::new(64),
                    vel: u7::new(0),
                },
            },
        );
        push(0, TrackEventKind::Meta(MetaMessage::EndOfTrack));
        smf.tracks.push(track);
        let mut bytes = Vec::new();
        smf.write_std(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn imports_ppq_conversion_tempo_signature_and_pan() {
        let import = import_smf(&raw_smf(480)).unwrap();
        assert_eq!(import.ppq, 480);
        assert_eq!(import.note_count(), 2);
        assert!((import.initial_bpm() - 100.0).abs() < 0.01);
        assert_eq!(import.time_signature, (3, 4));
        let notes = &import.tracks[0].notes;
        assert_eq!((notes[0].key, notes[0].start, notes[0].length), (60, 0, 960));
        assert_eq!((notes[1].key, notes[1].start, notes[1].length), (64, 960, 480));
        assert_eq!(notes[0].pan, 63);
        let patterns = import.to_patterns(1, [0, 0, 0]);
        assert_eq!(patterns[0].channel(1).unwrap().notes.len(), 2);
        assert_eq!(patterns[0].length_steps, 16);
    }

    #[test]
    fn overlapping_notes_are_paired_fifo_and_velocity_zero_closes() {
        let mut smf = Smf::new(Header::new(
            Format::SingleTrack,
            Timing::Metrical(u15::new(960)),
        ));
        let mut track: midly::Track = Vec::new();
        let mut push = |delta: u32, kind: TrackEventKind<'static>| {
            track.push(TrackEvent {
                delta: u28::new(delta),
                kind,
            });
        };
        let on = |key: u8, vel: u8| TrackEventKind::Midi {
            channel: u4::new(0),
            message: MidiMessage::NoteOn {
                key: u7::new(key),
                vel: u7::new(vel),
            },
        };
        push(0, on(60, 100));
        push(120, on(60, 90));
        push(120, on(60, 0)); // closes the first note at tick 240
        push(120, on(60, 0)); // closes the second at tick 360
        push(0, TrackEventKind::Meta(MetaMessage::EndOfTrack));
        smf.tracks.push(track);
        let mut bytes = Vec::new();
        smf.write_std(&mut bytes).unwrap();
        let import = import_smf(&bytes).unwrap();
        let notes = &import.tracks[0].notes;
        assert_eq!(notes.len(), 2);
        assert_eq!((notes[0].start, notes[0].length), (0, 240));
        assert_eq!((notes[1].start, notes[1].length), (120, 240));
    }

    #[test]
    fn rejects_smpte_bad_data_and_zero_ppq() {
        let mut smf = Smf::new(Header::new(
            Format::SingleTrack,
            Timing::Timecode(midly::Fps::Fps25, 40),
        ));
        smf.tracks.push(Vec::new());
        let mut bytes = Vec::new();
        smf.write_std(&mut bytes).unwrap();
        assert!(matches!(import_smf(&bytes), Err(MidiError::SmpteTiming)));
        assert!(matches!(import_smf(b"not midi"), Err(MidiError::Parse(_))));

        let mut smf = Smf::new(Header::new(
            Format::SingleTrack,
            Timing::Metrical(u15::new(0)),
        ));
        smf.tracks.push(Vec::new());
        let mut bytes = Vec::new();
        smf.write_std(&mut bytes).unwrap();
        assert!(matches!(import_smf(&bytes), Err(MidiError::Parse(_))));
    }

    #[test]
    fn unclosed_notes_end_with_track() {
        let mut smf = Smf::new(Header::new(
            Format::SingleTrack,
            Timing::Metrical(u15::new(960)),
        ));
        let mut track: midly::Track = Vec::new();
        track.push(TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOn {
                    key: u7::new(48),
                    vel: u7::new(100),
                },
            },
        });
        track.push(TrackEvent {
            delta: u28::new(480),
            kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
        });
        smf.tracks.push(track);
        let mut bytes = Vec::new();
        smf.write_std(&mut bytes).unwrap();
        let import = import_smf(&bytes).unwrap();
        assert_eq!(import.tracks[0].notes[0].length, 480);
    }
}
