//! Live MIDI input and quantize-on-record.
//!
//! `midir` runs a platform callback on its own thread. The callback only parses
//! the raw bytes and pushes a small [`LiveMidiEvent`] through an `mpsc` channel;
//! the UI thread drains it in [`LiveMidi::poll`]. No MIDI work ever happens in
//! the audio callback. Device enumeration is re-done on every [`LiveMidi::ports`]
//! call, which is how hot-plug polling is implemented by the UI.
#![forbid(unsafe_code)]

use crate::{cc_pan_to_note, CC_PAN};
use midir::{MidiInput, MidiInputConnection};
use redraw_shared::{CcEvent, Note, Pattern};
use redraw_sequencer::add_note;
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveMidiEvent {
    NoteOn { channel: u8, key: u8, velocity: u8 },
    NoteOff { channel: u8, key: u8, velocity: u8 },
    ControlChange { channel: u8, controller: u8, value: u8 },
    /// -8192..=8191, 0 = centre.
    PitchBend { channel: u8, value: i16 },
    ProgramChange { channel: u8, program: u8 },
}

/// Parse one complete channel-voice message. Note-on velocity 0 is normalized to
/// [`LiveMidiEvent::NoteOff`]. System/common and realtime messages are ignored.
pub fn parse_message(bytes: &[u8]) -> Option<LiveMidiEvent> {
    let status = *bytes.first()?;
    if status < 0x80 || status >= 0xF0 {
        return None;
    }
    if bytes.len() < 2 {
        return None;
    }
    let channel = status & 0x0F;
    let data1 = bytes[1];
    let needs_second = matches!(status & 0xF0, 0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0);
    if needs_second && bytes.len() < 3 {
        return None;
    }
    let data2 = bytes.get(2).copied().unwrap_or(0);
    if data1 >= 0x80 || data2 >= 0x80 {
        return None;
    }
    match status & 0xF0 {
        0x80 => Some(LiveMidiEvent::NoteOff {
            channel,
            key: data1,
            velocity: data2,
        }),
        0x90 => {
            if data2 == 0 {
                Some(LiveMidiEvent::NoteOff {
                    channel,
                    key: data1,
                    velocity: 0,
                })
            } else {
                Some(LiveMidiEvent::NoteOn {
                    channel,
                    key: data1,
                    velocity: data2,
                })
            }
        }
        0xB0 => Some(LiveMidiEvent::ControlChange {
            channel,
            controller: data1,
            value: data2,
        }),
        0xC0 => Some(LiveMidiEvent::ProgramChange {
            channel,
            program: data1,
        }),
        0xE0 => {
            let raw = ((data2 as i16) << 7) | data1 as i16;
            Some(LiveMidiEvent::PitchBend {
                channel,
                value: raw - 8192,
            })
        }
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiPortInfo {
    /// Index into the current port list; pass to [`LiveMidi::connect`].
    pub index: usize,
    pub name: String,
}

/// Owns one open MIDI input connection and exposes a non-blocking event queue.
pub struct LiveMidi {
    connection: Option<MidiInputConnection<()>>,
    tx: Sender<LiveMidiEvent>,
    rx: Receiver<LiveMidiEvent>,
    connected_port: Option<String>,
    client_name: String,
}

impl Default for LiveMidi {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveMidi {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            connection: None,
            tx,
            rx,
            connected_port: None,
            client_name: "REDRAW MIDI".to_string(),
        }
    }

    /// Rescan input ports. Call periodically for hot-plug polling.
    pub fn ports(&self) -> Result<Vec<MidiPortInfo>, String> {
        let input = MidiInput::new(&self.client_name).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for (index, port) in input.ports().iter().enumerate() {
            let name = input
                .port_name(port)
                .unwrap_or_else(|_| format!("MIDI Input {}", index + 1));
            out.push(MidiPortInfo { index, name });
        }
        Ok(out)
    }

    /// Open the port at `index` (from [`ports`](Self::ports)). Any previous
    /// connection is closed first on success.
    pub fn connect(&mut self, index: usize) -> Result<String, String> {
        let input = MidiInput::new(&self.client_name).map_err(|e| e.to_string())?;
        let ports = input.ports();
        let port = ports
            .get(index)
            .ok_or_else(|| format!("MIDI input {index} is not available"))?;
        let name = input
            .port_name(port)
            .unwrap_or_else(|_| format!("MIDI Input {}", index + 1));
        let tx = self.tx.clone();
        let connection = input
            .connect(
                port,
                "redraw-input",
                move |_stamp, message, _| {
                    if let Some(event) = parse_message(message) {
                        let _ = tx.send(event);
                    }
                },
                (),
            )
            .map_err(|e| e.to_string())?;
        self.disconnect();
        self.connection = Some(connection);
        self.connected_port = Some(name.clone());
        Ok(name)
    }

    pub fn disconnect(&mut self) {
        if let Some(connection) = self.connection.take() {
            connection.close();
        }
        self.connected_port = None;
    }

    pub fn is_connected(&self) -> bool {
        self.connection.is_some()
    }

    pub fn connected_port(&self) -> Option<&str> {
        self.connected_port.as_deref()
    }

    /// Drain all events received since the last poll.
    pub fn poll(&self) -> Vec<LiveMidiEvent> {
        self.rx.try_iter().collect()
    }

    pub fn poll_into(&self, out: &mut Vec<LiveMidiEvent>) -> usize {
        let before = out.len();
        out.extend(self.rx.try_iter());
        out.len() - before
    }
}

// ------------------------------------------------------------------ recorder

#[derive(Clone, Copy, Debug)]
struct ActiveNote {
    start: u64,
    velocity: u8,
    pan: i8,
}

/// Records note and CC events into a pattern. Note starts can be quantized on
/// record (floor to the grid, so a note never starts after its note-off).
#[derive(Clone, Debug)]
pub struct MidiRecorder {
    pub channel_filter: Option<u8>,
    pub quantize: Option<u64>,
    active: [Option<ActiveNote>; 128],
    last_pan: [i8; 16],
    pub recorded_notes: usize,
    pub recorded_cc: usize,
}

impl Default for MidiRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiRecorder {
    pub fn new() -> Self {
        Self {
            channel_filter: None,
            quantize: None,
            active: [None; 128],
            last_pan: [0; 16],
            recorded_notes: 0,
            recorded_cc: 0,
        }
    }

    pub fn set_channel_filter(&mut self, channel: Option<u8>) {
        self.channel_filter = channel.map(|c| c & 0x0F);
    }

    pub fn set_quantize(&mut self, grid: Option<u64>) {
        self.quantize = grid.filter(|g| *g > 0);
    }

    pub fn active_count(&self) -> usize {
        self.active.iter().filter(|n| n.is_some()).count()
    }

    pub fn active_keys(&self) -> Vec<u8> {
        self.active
            .iter()
            .enumerate()
            .filter_map(|(i, n)| n.map(|_| i as u8))
            .collect()
    }

    fn accepts(&self, channel: u8) -> bool {
        self.channel_filter
            .map(|c| c == (channel & 0x0F))
            .unwrap_or(true)
    }

    fn quantized_start(&self, tick: u64) -> u64 {
        match self.quantize {
            Some(grid) if grid > 0 => tick / grid * grid,
            _ => tick,
        }
    }

    /// Feed one event at the current transport tick. Returns the note that was
    /// completed by a note-off (or by a retrigger), if any.
    pub fn on_event(
        &mut self,
        pattern: &mut Pattern,
        channel_id: u32,
        tick: u64,
        event: LiveMidiEvent,
    ) -> Option<Note> {
        match event {
            LiveMidiEvent::NoteOn {
                channel,
                key,
                velocity,
            } => {
                if !self.accepts(channel) {
                    return None;
                }
                if velocity == 0 {
                    return self.finish_key(pattern, channel_id, key, tick);
                }
                let key = key.min(127);
                let mut completed = None;
                if self.active[key as usize].is_some() {
                    completed = self.finish_key(pattern, channel_id, key, tick);
                }
                self.active[key as usize] = Some(ActiveNote {
                    start: self.quantized_start(tick),
                    velocity: velocity.clamp(1, 127),
                    pan: self.last_pan[(channel & 0x0F) as usize],
                });
                completed
            }
            LiveMidiEvent::NoteOff { channel, key, .. } => {
                if !self.accepts(channel) {
                    return None;
                }
                self.finish_key(pattern, channel_id, key.min(127), tick)
            }
            LiveMidiEvent::ControlChange {
                channel,
                controller,
                value,
            } => {
                if !self.accepts(channel) {
                    return None;
                }
                let channel = channel & 0x0F;
                if controller == CC_PAN {
                    self.last_pan[channel as usize] = cc_pan_to_note(value);
                }
                let pc = pattern.channel_mut(channel_id, pattern.length_steps.max(1));
                pc.cc.push(CcEvent {
                    tick,
                    controller: controller.min(127),
                    value: value.min(127),
                });
                pc.cc.sort_by_key(|c| (c.tick, c.controller));
                self.recorded_cc += 1;
                None
            }
            LiveMidiEvent::PitchBend { .. } | LiveMidiEvent::ProgramChange { .. } => None,
        }
    }

    fn finish_key(
        &mut self,
        pattern: &mut Pattern,
        channel_id: u32,
        key: u8,
        tick: u64,
    ) -> Option<Note> {
        let active = self.active[key as usize].take()?;
        let length = tick.saturating_sub(active.start).max(1);
        let note = Note {
            id: 0,
            key,
            start: active.start,
            length,
            velocity: active.velocity,
            pan: active.pan,
            release: 64,
        };
        let stored = add_note(pattern, channel_id, note);
        self.recorded_notes += 1;
        Some(stored)
    }

    /// Close every held note at `tick` (stop / all-notes-off).
    pub fn finish(&mut self, pattern: &mut Pattern, channel_id: u32, tick: u64) -> usize {
        let mut finished = 0;
        for key in 0..128u8 {
            if self.active[key as usize].is_some()
                && self
                    .finish_key(pattern, channel_id, key, tick)
                    .is_some()
            {
                finished += 1;
            }
        }
        finished
    }

    /// Recording panic: finalize held notes so no note is lost, then clear state.
    pub fn panic(&mut self, pattern: &mut Pattern, channel_id: u32, tick: u64) -> usize {
        self.finish(pattern, channel_id, tick)
    }

    /// Forget held notes without writing them.
    pub fn clear_active(&mut self) {
        self.active = [None; 128];
        self.last_pan = [0; 16];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redraw_shared::Note;

    fn pattern() -> Pattern {
        Pattern::new(1, "Rec", [9, 9, 9], 16)
    }

    #[test]
    fn parses_channel_messages() {
        assert_eq!(
            parse_message(&[0x90, 60, 100]),
            Some(LiveMidiEvent::NoteOn {
                channel: 0,
                key: 60,
                velocity: 100
            })
        );
        assert_eq!(
            parse_message(&[0x8F, 60, 40]),
            Some(LiveMidiEvent::NoteOff {
                channel: 15,
                key: 60,
                velocity: 40
            })
        );
        assert_eq!(
            parse_message(&[0x93, 64, 0]),
            Some(LiveMidiEvent::NoteOff {
                channel: 3,
                key: 64,
                velocity: 0
            })
        );
        assert_eq!(
            parse_message(&[0xB2, 10, 96]),
            Some(LiveMidiEvent::ControlChange {
                channel: 2,
                controller: 10,
                value: 96
            })
        );
        assert_eq!(
            parse_message(&[0xE0, 0x00, 0x40]),
            Some(LiveMidiEvent::PitchBend {
                channel: 0,
                value: 0
            })
        );
        assert_eq!(
            parse_message(&[0xC0, 5]),
            Some(LiveMidiEvent::ProgramChange {
                channel: 0,
                program: 5
            })
        );
        assert_eq!(parse_message(&[0xF8]), None);
        assert_eq!(parse_message(&[0x80, 0x80, 0]), None);
        assert_eq!(parse_message(&[0x90]), None);
        assert_eq!(parse_message(&[0x90, 60]), None);
        assert_eq!(parse_message(&[0xB0, 10]), None);
        assert_eq!(parse_message(&[]), None);
    }

    #[test]
    fn records_notes_with_quantize_and_pan() {
        let mut pattern = pattern();
        let mut rec = MidiRecorder::new();
        rec.set_quantize(Some(240));
        assert_eq!(
            rec.on_event(
                &mut pattern,
                1,
                100,
                LiveMidiEvent::ControlChange {
                    channel: 0,
                    controller: 10,
                    value: 96
                }
            ),
            None
        );
        assert_eq!(
            rec.on_event(
                &mut pattern,
                1,
                260,
                LiveMidiEvent::NoteOn {
                    channel: 0,
                    key: 60,
                    velocity: 100
                }
            ),
            None
        );
        assert_eq!(rec.active_count(), 1);
        let completed = rec.on_event(
            &mut pattern,
            1,
            500,
            LiveMidiEvent::NoteOff {
                channel: 0,
                key: 60,
                velocity: 0,
            },
        );
        let note = completed.expect("completed note");
        assert_eq!(note.start, 240);
        assert_eq!(note.length, 260);
        assert_eq!(note.velocity, 100);
        assert_eq!(note.pan, 32);
        assert_eq!(pattern.channel(1).unwrap().notes.len(), 1);
        assert_eq!(pattern.channel(1).unwrap().cc.len(), 1);
        assert_eq!(rec.recorded_notes, 1);
        assert_eq!(rec.recorded_cc, 1);
    }

    #[test]
    fn channel_filter_retrigger_and_finish() {
        let mut pattern = pattern();
        let mut rec = MidiRecorder::new();
        rec.set_channel_filter(Some(2));
        assert_eq!(
            rec.on_event(
                &mut pattern,
                1,
                0,
                LiveMidiEvent::NoteOn {
                    channel: 1,
                    key: 60,
                    velocity: 100
                }
            ),
            None
        );
        assert_eq!(rec.active_count(), 0);
        rec.on_event(
            &mut pattern,
            1,
            0,
            LiveMidiEvent::NoteOn {
                channel: 2,
                key: 60,
                velocity: 100
            },
        );
        let first = rec.on_event(
            &mut pattern,
            1,
            120,
            LiveMidiEvent::NoteOn {
                channel: 2,
                key: 60,
                velocity: 90
            },
        );
        assert!(first.is_some());
        assert_eq!(first.unwrap().length, 120);
        assert_eq!(rec.active_count(), 1);
        let finished = rec.finish(&mut pattern, 1, 240);
        assert_eq!(finished, 1);
        assert_eq!(rec.active_count(), 0);
        assert_eq!(pattern.channel(1).unwrap().notes.len(), 2);
        let notes = &pattern.channel(1).unwrap().notes;
        assert_eq!(notes[1].start, 120);
        assert_eq!(notes[1].length, 120);
    }

    #[test]
    fn panic_finishes_held_notes() {
        let mut pattern = pattern();
        let mut rec = MidiRecorder::new();
        rec.on_event(
            &mut pattern,
            1,
            0,
            LiveMidiEvent::NoteOn {
                channel: 0,
                key: 64,
                velocity: 100
            },
        );
        rec.on_event(
            &mut pattern,
            1,
            0,
            LiveMidiEvent::NoteOn {
                channel: 0,
                key: 67,
                velocity: 100
            },
        );
        assert_eq!(rec.panic(&mut pattern, 1, 480), 2);
        assert_eq!(rec.active_count(), 0);
        assert_eq!(pattern.channel(1).unwrap().notes.len(), 2);
        assert!(pattern
            .channel(1)
            .unwrap()
            .notes
            .iter()
            .all(|n: &Note| n.length == 480));
    }

    #[test]
    fn ignores_bend_program_and_wrong_channels() {
        let mut pattern = pattern();
        let mut rec = MidiRecorder::new();
        rec.set_channel_filter(Some(5));
        assert_eq!(
            rec.on_event(
                &mut pattern,
                1,
                0,
                LiveMidiEvent::PitchBend {
                    channel: 5,
                    value: 2000
                }
            ),
            None
        );
        assert_eq!(
            rec.on_event(
                &mut pattern,
                1,
                0,
                LiveMidiEvent::ProgramChange {
                    channel: 5,
                    program: 3
                }
            ),
            None
        );
        assert_eq!(
            rec.on_event(
                &mut pattern,
                1,
                0,
                LiveMidiEvent::ControlChange {
                    channel: 4,
                    controller: 1,
                    value: 20
                }
            ),
            None
        );
        assert!(pattern
            .channel(1)
            .map(|c| c.cc.is_empty())
            .unwrap_or(true));
    }
}
