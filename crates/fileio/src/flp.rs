//! Tolerant legacy FLP project import and best-effort core export.
//!
//! The public container grammar used here is:
//!
//! ```text
//! FLhd <u32 chunk_size=6> <i16 format> <u16 channels> <u16 ppq>
//! FLdt <u32 event_bytes> <event>*
//! event := id:u8, payload
//!   id 0..=63    -> 1 payload byte
//!   id 64..=127  -> 2 payload bytes, LE
//!   id 128..=191 -> 4 payload bytes, LE
//!   id 192..=255 -> LEB128 length + payload
//! ```
//!
//! The `FLdt` size field was confirmed against the shipped Empty/Basic/demo
//! files during Phase 13; the Phase-1 probe had treated those four bytes as
//! events. Compressed event id 202 is inflated when its payload starts with a
//! valid zlib header.
//!
//! Import maps a documented common core (tempo, signature, swing, project
//! names, patterns, notes, channel names/colors/levels) into `StudioProject`.
//! Events REDRAW does not interpret are retained in `StudioProject::flp_source`
//! and re-emitted on export. A dedicated event (id 250) carries a compressed
//! REDRAW sidecar so a `.flp` exported by REDRAW round-trips losslessly inside
//! REDRAW while remaining a structurally valid FLP for external tools.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use redraw_shared::{
    ChannelInfo, FlpRawEvent, FlpSource, Note, Pattern, PatternChannel, PlaylistState, Step,
    StudioProject, MAX_CHANNELS, PPQ,
};

use crate::{atomic_write, repair_project, MAX_PATTERN_STEPS, MAX_PROJECT_BYTES};

/// Event id used for the REDRAW lossless sidecar.
pub const FLP_SIDECAR_EVENT_ID: u8 = 250;
const SIDECAR_MAGIC: &[u8; 8] = b"RDPXFLP1";
const SIDECAR_VERSION: u16 = 1;
const MAX_EVENT_PAYLOAD: usize = 64 * 1024 * 1024;
const MAX_EVENTS: usize = 2_000_000;
const MAX_NESTED_DEPTH: u8 = 3;

/// FLP-specific errors.
#[derive(Debug)]
pub enum FlpError {
    Io(std::io::Error),
    Invalid(String),
}

impl std::fmt::Display for FlpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Invalid(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for FlpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Invalid(_) => None,
        }
    }
}

impl From<std::io::Error> for FlpError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

pub type FlpResult<T> = std::result::Result<T, FlpError>;

/// FLP file header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlpHeader {
    pub format: i16,
    pub channels: u16,
    pub ppq: u16,
}

impl Default for FlpHeader {
    fn default() -> Self {
        Self {
            format: 0,
            channels: 0,
            ppq: PPQ as u16,
        }
    }
}

/// One decoded FLP event. For fixed-width ids the payload has the fixed size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlpEvent {
    pub id: u8,
    pub payload: Vec<u8>,
}

/// Header plus the (decompressed) event stream.
#[derive(Clone, Debug)]
pub struct FlpDocument {
    pub header: FlpHeader,
    pub events: Vec<FlpEvent>,
}

/// Result of importing an FLP.
#[derive(Clone, Debug)]
pub struct FlpImport {
    pub project: StudioProject,
    pub sidecar_used: bool,
    pub unknown_events: usize,
    pub warnings: Vec<String>,
}

/// Parse an FLP container without converting it to a project.
pub fn parse_flp(bytes: &[u8]) -> FlpResult<FlpDocument> {
    if bytes.len() < 22 {
        return Err(FlpError::Invalid("FLP is shorter than its header".into()));
    }
    let hd = find_magic(bytes, b"FLhd", 0, 1024)
        .ok_or_else(|| FlpError::Invalid("missing FLhd header".into()))?;
    let hd_size = read_u32(bytes, hd + 4)
        .ok_or_else(|| FlpError::Invalid("truncated FLhd size".into()))? as usize;
    if hd_size < 6 {
        return Err(FlpError::Invalid(format!(
            "FLhd chunk size {hd_size} is smaller than 6"
        )));
    }
    let header_end = hd
        .checked_add(8)
        .and_then(|v| v.checked_add(hd_size))
        .ok_or_else(|| FlpError::Invalid("FLhd chunk range overflow".into()))?;
    if header_end > bytes.len() {
        return Err(FlpError::Invalid("FLhd chunk is truncated".into()));
    }
    let format = read_i16(bytes, hd + 8)
        .ok_or_else(|| FlpError::Invalid("truncated FLhd fields".into()))?;
    let channels = read_u16(bytes, hd + 10)
        .ok_or_else(|| FlpError::Invalid("truncated FLhd fields".into()))?;
    let ppq = read_u16(bytes, hd + 12)
        .ok_or_else(|| FlpError::Invalid("truncated FLhd fields".into()))?;

    let dt = if bytes.get(header_end..header_end + 4) == Some(b"FLdt") {
        header_end
    } else {
        find_magic(bytes, b"FLdt", header_end, 4096)
            .ok_or_else(|| FlpError::Invalid("missing FLdt data chunk".into()))?
    };

    // Current files carry a u32 byte count after FLdt. Phase-1 evidence
    // originally missed it; accept a size-less legacy stream as a fallback.
    let declared = read_u32(bytes, dt + 4).unwrap_or(0) as usize;
    let after_size = dt + 8;
    let declared_end = after_size.checked_add(declared);
    let use_declared = declared_end
        .map(|end| {
            end <= bytes.len()
                && (end == bytes.len() || bytes[end..].iter().all(|b| *b == 0))
        })
        .unwrap_or(false);
    let (events_start, events_end) = if use_declared {
        (after_size, declared_end.unwrap())
    } else {
        (dt + 4, bytes.len())
    };
    let events = parse_event_stream(&bytes[events_start..events_end], 0)?;

    Ok(FlpDocument {
        header: FlpHeader {
            format,
            channels,
            ppq: if ppq == 0 { PPQ as u16 } else { ppq },
        },
        events,
    })
}

/// Serialize a document to a structurally valid FLP container.
pub fn write_flp(document: &FlpDocument) -> FlpResult<Vec<u8>> {
    let mut event_bytes = Vec::new();
    for event in &document.events {
        write_event(&mut event_bytes, event.id, &event.payload)?;
    }
    let event_size = u32::try_from(event_bytes.len())
        .map_err(|_| FlpError::Invalid("FLP event stream exceeds 4 GiB".into()))?;
    let mut out = Vec::with_capacity(22 + event_bytes.len());
    out.extend_from_slice(b"FLhd");
    out.extend_from_slice(&6u32.to_le_bytes());
    out.extend_from_slice(&document.header.format.to_le_bytes());
    out.extend_from_slice(&document.header.channels.to_le_bytes());
    out.extend_from_slice(&document.header.ppq.max(1).to_le_bytes());
    out.extend_from_slice(b"FLdt");
    out.extend_from_slice(&event_size.to_le_bytes());
    out.extend_from_slice(&event_bytes);
    Ok(out)
}

/// Import an FLP from memory, preferring a valid REDRAW sidecar.
pub fn import_flp(bytes: &[u8]) -> FlpResult<FlpImport> {
    let document = parse_flp(bytes)?;
    let mut warnings = Vec::new();

    if let Some(event) = document
        .events
        .iter()
        .find(|e| e.id == FLP_SIDECAR_EVENT_ID && e.payload.starts_with(SIDECAR_MAGIC))
    {
        match decode_sidecar(&event.payload) {
            Ok(mut project) => {
                repair_project(&mut project);
                let unknown_events = project
                    .flp_source
                    .as_ref()
                    .map_or(0, |source| source.events.len());
                return Ok(FlpImport {
                    project,
                    sidecar_used: true,
                    unknown_events,
                    warnings,
                });
            }
            Err(error) => warnings.push(format!(
                "REDRAW sidecar is invalid ({error}); importing core FLP events"
            )),
        }
    }

    let project = convert_document(&document, &mut warnings);
    let unknown_events = project
        .flp_source
        .as_ref()
        .map_or(0, |source| source.events.len());
    Ok(FlpImport {
        project,
        sidecar_used: false,
        unknown_events,
        warnings,
    })
}

/// Read and import an FLP file.
pub fn load_flp(path: impl AsRef<Path>) -> FlpResult<FlpImport> {
    let path = path.as_ref();
    let bytes = fs::read(path)?;
    let mut imported = import_flp(&bytes)?;
    if !imported.sidecar_used && imported.project.name == "Imported FLP" {
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            if !stem.trim().is_empty() {
                imported.project.name = stem.to_string();
            }
        }
    }
    Ok(imported)
}

/// Export a project as an FLP. Core events are FL-compatible best effort;
/// the REDRAW sidecar preserves the full project state for REDRAW's own reader.
pub fn export_flp(project: &StudioProject) -> FlpResult<Vec<u8>> {
    let source = project.flp_source.as_ref();
    let ppq = source
        .map(|s| s.ppq)
        .filter(|p| (24..=960).contains(p))
        .unwrap_or(PPQ as u16);
    let format = source.map_or(0, |s| s.format);
    let mut events = Vec::new();

    // Project metadata.
    push_event(&mut events, 199, ascii_payload(env!("CARGO_PKG_VERSION")));
    push_event(&mut events, 194, utf16_payload(&project.name));
    if !project.author.is_empty() {
        push_event(&mut events, 207, utf16_payload(&project.author));
    }
    push_event(
        &mut events,
        156,
        ((project.bpm.max(0.0) * 1000.0) as u32).to_le_bytes().to_vec(),
    );
    push_event(&mut events, 17, vec![project.time_sig.0.max(1)]);
    push_event(&mut events, 18, vec![project.time_sig.1.max(1)]);
    let swing = ((project.swing.clamp(0.5, 0.75) - 0.5) / 0.25 * 128.0) as u8;
    push_event(&mut events, 11, vec![swing]);

    // Channel rack. Use compact 0-based FL IIDs so note references are stable.
    let mut channel_iids = HashMap::new();
    for (index, channel) in project.channels.iter().take(MAX_CHANNELS).enumerate() {
        let iid = index as u16;
        channel_iids.insert(channel.id, iid);
        push_event(&mut events, 64, iid.to_le_bytes().to_vec());
        let kind = if channel.synth_type == "Sampler" { 0u8 } else { 2u8 };
        push_event(&mut events, 21, vec![kind]);
        push_event(&mut events, 203, utf16_payload(&channel.name));
        push_event(
            &mut events,
            128,
            vec![channel.color[0], channel.color[1], channel.color[2], 255],
        );
        push_event(
            &mut events,
            72,
            fl_volume_from_db(channel.volume_db).to_le_bytes().to_vec(),
        );
        push_event(
            &mut events,
            73,
            fl_pan_from_norm(channel.pan).to_le_bytes().to_vec(),
        );
    }

    // Pattern note blocks.
    for pattern in &project.patterns {
        let pattern_iid = pattern_iid(pattern);
        push_event(&mut events, 65, pattern_iid.to_le_bytes().to_vec());
        let mut note_bytes = Vec::new();
        for channel in &pattern.channels {
            let Some(&rack) = channel_iids.get(&channel.channel_id) else {
                continue;
            };
            for (step_index, step) in channel.steps.iter().enumerate() {
                if step.active {
                    let position = to_source_ticks(
                        step_index as u64 * (PPQ as u64 / 4),
                        ppq,
                    );
                    write_note(
                        &mut note_bytes,
                        position,
                        rack,
                        0, // step notes use length 0 in FLP
                        60,
                        step.velocity,
                        64,
                        64,
                    );
                }
            }
            for note in &channel.notes {
                let position = to_source_ticks(note.start, ppq);
                let length = if note.length == 0 {
                    0
                } else {
                    to_source_ticks(note.length, ppq).max(1)
                };
                write_note(
                    &mut note_bytes,
                    position,
                    rack,
                    length,
                    note.key,
                    note.velocity,
                    (note.pan as i16 + 64).clamp(0, 128) as u8,
                    note.release,
                );
            }
        }
        if !note_bytes.is_empty() {
            push_event(&mut events, 224, note_bytes);
        }
    }

    // Pattern metadata (FLP convention repeats PatternID.New before metadata).
    for pattern in &project.patterns {
        let pattern_iid = pattern_iid(pattern);
        push_event(&mut events, 65, pattern_iid.to_le_bytes().to_vec());
        push_event(&mut events, 193, utf16_payload(&pattern.name));
        push_event(
            &mut events,
            150,
            vec![pattern.color[0], pattern.color[1], pattern.color[2], 255],
        );
        let length_ticks =
            to_source_ticks(pattern.length_steps as u64 * (PPQ as u64 / 4), ppq);
        push_event(
            &mut events,
            164,
            (length_ticks.min(u32::MAX as u64) as u32)
                .to_le_bytes()
                .to_vec(),
        );
    }

    // Preserve unhandled source events as a raw block. Their original order is
    // retained, although the block is placed after the core events.
    if let Some(source) = source {
        for raw in &source.events {
            if raw.id == FLP_SIDECAR_EVENT_ID && raw.payload.starts_with(SIDECAR_MAGIC) {
                continue;
            }
            events.push(FlpEvent {
                id: raw.id,
                payload: raw.payload.clone(),
            });
        }
    }

    events.push(FlpEvent {
        id: FLP_SIDECAR_EVENT_ID,
        payload: encode_sidecar(project)?,
    });

    let document = FlpDocument {
        header: FlpHeader {
            format,
            channels: project.channels.len().min(u16::MAX as usize) as u16,
            ppq,
        },
        events,
    };
    write_flp(&document)
}

/// Write an FLP atomically.
pub fn save_flp(path: impl AsRef<Path>, project: &StudioProject) -> FlpResult<()> {
    let bytes = export_flp(project)?;
    atomic_write(path.as_ref(), &bytes).map_err(FlpError::Io)?;
    Ok(())
}

// ------------------------------------------------------------------- conversion

#[derive(Clone, Debug)]
struct RawNote {
    position: u32,
    length: u32,
    key: u16,
    rack_channel: u16,
    velocity: u8,
    pan: u8,
    release: u8,
}

#[derive(Clone, Debug)]
struct RawPattern {
    name: String,
    color: [u8; 3],
    length_ticks: u64,
    notes: Vec<RawNote>,
}

impl Default for RawPattern {
    fn default() -> Self {
        Self {
            name: String::new(),
            color: [95, 179, 108],
            length_ticks: 0,
            notes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct RawChannel {
    iid: u16,
    kind: u8,
    name: String,
    color: [u8; 3],
    volume_db: f32,
    pan: f32,
}

impl Default for RawChannel {
    fn default() -> Self {
        Self {
            iid: 0,
            kind: 2,
            name: String::new(),
            color: [120, 140, 170],
            volume_db: 0.0,
            pan: 0.0,
        }
    }
}

fn convert_document(document: &FlpDocument, warnings: &mut Vec<String>) -> StudioProject {
    let source_ppq = if document.header.ppq == 0 {
        PPQ as u16
    } else {
        document.header.ppq
    };
    let mut patterns: BTreeMap<u16, RawPattern> = BTreeMap::new();
    let mut channels: BTreeMap<u16, RawChannel> = BTreeMap::new();
    let mut current_pattern = None;
    let mut current_channel = None;
    let mut title = String::new();
    let mut author = String::new();
    let mut version = String::new();
    let mut bpm = None;
    let mut time_sig = (4u8, 4u8);
    let mut saw_time_sig = false;
    let mut swing_raw = None;
    let mut unknown = Vec::new();
    let mut note_count = 0usize;

    for event in &document.events {
        match event.id {
            199 => version = decode_ascii(&event.payload),
            194 => title = decode_utf16(&event.payload),
            207 => author = decode_utf16(&event.payload),
            156 => {
                if let Some(value) = read_u32(&event.payload, 0) {
                    bpm = Some(value as f32 / 1000.0);
                }
            }
            17 => {
                if let Some(value) = event.payload.first() {
                    time_sig.0 = *value;
                    saw_time_sig = true;
                }
            }
            18 => {
                if let Some(value) = event.payload.first() {
                    time_sig.1 = *value;
                    saw_time_sig = true;
                }
            }
            11 => {
                if let Some(value) = event.payload.first() {
                    swing_raw = Some(*value);
                }
            }
            65 => {
                if let Some(iid) = read_u16(&event.payload, 0) {
                    current_pattern = Some(iid);
                    patterns.entry(iid).or_default();
                }
            }
            193 => {
                if let Some(iid) = current_pattern {
                    let name = decode_utf16(&event.payload);
                    patterns.entry(iid).or_default().name = name;
                } else {
                    unknown.push(raw(event));
                }
            }
            150 => {
                if let Some(iid) = current_pattern {
                    let color = color3(&event.payload);
                    patterns.entry(iid).or_default().color = color;
                } else {
                    unknown.push(raw(event));
                }
            }
            164 => {
                if let Some(iid) = current_pattern {
                    if let Some(length) = read_u32(&event.payload, 0) {
                        patterns.entry(iid).or_default().length_ticks = length as u64;
                    }
                } else {
                    unknown.push(raw(event));
                }
            }
            224 => {
                if let Some(iid) = current_pattern {
                    let pattern = patterns.entry(iid).or_default();
                    if event.payload.len() % 24 != 0 {
                        warnings.push(format!(
                            "pattern {iid} has a truncated note record ({} bytes)",
                            event.payload.len()
                        ));
                    }
                    for record in event.payload.chunks_exact(24) {
                        if note_count >= 1_000_000 {
                            warnings.push("FLP note count limit reached; remaining notes ignored".into());
                            break;
                        }
                        pattern.notes.push(RawNote {
                            position: read_u32(record, 0).unwrap_or(0),
                            length: read_u32(record, 8).unwrap_or(0),
                            key: read_u16(record, 12).unwrap_or(60),
                            rack_channel: read_u16(record, 6).unwrap_or(0),
                            velocity: record[21],
                            pan: record[20],
                            release: record[18],
                        });
                        note_count += 1;
                    }
                } else {
                    warnings.push("notes occurred before any pattern header; dropped".into());
                }
            }
            64 => {
                if let Some(iid) = read_u16(&event.payload, 0) {
                    current_channel = Some(iid);
                    channels.entry(iid).or_insert_with(|| RawChannel {
                        iid,
                        ..Default::default()
                    });
                }
            }
            21 => {
                if let Some(iid) = current_channel {
                    if let Some(kind) = event.payload.first() {
                        channels.entry(iid).or_default().kind = *kind;
                    }
                } else {
                    unknown.push(raw(event));
                }
            }
            203 => {
                if let Some(iid) = current_channel {
                    let name = decode_utf16(&event.payload);
                    channels.entry(iid).or_default().name = name;
                } else {
                    unknown.push(raw(event));
                }
            }
            128 => {
                if let Some(iid) = current_channel {
                    let color = color3(&event.payload);
                    channels.entry(iid).or_default().color = color;
                } else {
                    unknown.push(raw(event));
                }
            }
            72 => {
                if let Some(iid) = current_channel {
                    if let Some(word) = read_u16(&event.payload, 0) {
                        channels.entry(iid).or_default().volume_db = fl_volume_to_db(word);
                    }
                } else {
                    unknown.push(raw(event));
                }
            }
            73 => {
                if let Some(iid) = current_channel {
                    if let Some(word) = read_u16(&event.payload, 0) {
                        channels.entry(iid).or_default().pan = fl_pan_to_norm(word as i32);
                    }
                } else {
                    unknown.push(raw(event));
                }
            }
            219 => {
                if let Some(iid) = current_channel {
                    if event.payload.len() >= 24 {
                        let pan = read_i32(&event.payload, 0).unwrap_or(6400);
                        let volume = read_u32(&event.payload, 4).unwrap_or(10000);
                        let channel = channels.entry(iid).or_default();
                        channel.pan = fl_pan_to_norm(pan);
                        channel.volume_db =
                            fl_volume_to_db(volume.min(u16::MAX as u32) as u16);
                    } else {
                        unknown.push(raw(event));
                    }
                } else {
                    unknown.push(raw(event));
                }
            }
            FLP_SIDECAR_EVENT_ID if event.payload.starts_with(SIDECAR_MAGIC) => {
                // A sidecar was already considered by import_flp; if decoding
                // failed, do not copy a stale/corrupt sidecar into a new export.
            }
            _ => unknown.push(raw(event)),
        }
    }

    let mut project = StudioProject::default();
    project.patterns.clear();
    project.channels.clear();
    project.playlist = PlaylistState::default();
    project.automation.clear();
    project.mixer = Default::default();
    project.next_channel_id = 1;
    project.next_pattern_id = 1;
    project.next_clip_id = 1;
    project.next_note_id = 1;
    project.next_automation_id = 1;
    project.name = if title.trim().is_empty() {
        "Imported FLP".into()
    } else {
        title
    };
    project.author = author;
    project.bpm = bpm.filter(|v| v.is_finite()).unwrap_or(140.0);
    if saw_time_sig {
        project.time_sig = time_sig;
    }
    if let Some(raw_swing) = swing_raw {
        project.swing = 0.5 + (raw_swing as f32 / 128.0) * 0.25;
    }

    // Keep audio/instrument channels; automation channels (type 5) are not
    // instruments and have no meaningful REDRAW synth equivalent yet.
    let mut channel_id_map = HashMap::new();
    let mut next_mixer = 0usize;
    for raw in channels.values() {
        if raw.kind == 5 {
            continue;
        }
        if project.channels.len() >= MAX_CHANNELS {
            warnings.push(format!(
                "FLP channel limit reached; extra channels after {} ignored",
                MAX_CHANNELS
            ));
            break;
        }
        let id = project.next_channel_id;
        project.next_channel_id += 1;
        channel_id_map.insert(raw.iid, id);
        let name = if raw.name.trim().is_empty() {
            format!("Channel {}", id)
        } else {
            raw.name.clone()
        };
        let synth_type = if raw.kind == 0 { "Sampler" } else { "Aurora" };
        let mut channel = ChannelInfo::new(id, name, synth_type, raw.color);
        channel.mixer_index = next_mixer % MAX_CHANNELS;
        channel.volume_db = raw.volume_db;
        channel.pan = raw.pan;
        if let Some(strip) = project.mixer.strips.get_mut(channel.mixer_index) {
            strip.name = channel.name.clone();
            strip.color = channel.color;
            strip.volume_db = channel.volume_db;
            strip.pan = channel.pan;
        }
        next_mixer += 1;
        project.channels.push(channel);
    }

    if patterns.is_empty() {
        warnings.push("FLP has no pattern records; created one empty pattern".into());
        project.patterns.push(Pattern::new(1, "Pattern 1", [95, 179, 108], 16));
        project.next_pattern_id = 2;
    } else {
        for (index, raw) in patterns.values().enumerate() {
            let mut max_note_end = 0u64;
            for note in &raw.notes {
                let start = convert_ticks(note.position as u64, source_ppq);
                let length = if note.length == 0 {
                    PPQ as u64 / 4
                } else {
                    convert_ticks(note.length as u64, source_ppq).max(1)
                };
                max_note_end = max_note_end.max(start.saturating_add(length));
            }
            let mut length_steps = if raw.length_ticks == 0 {
                16
            } else {
                ((convert_ticks(raw.length_ticks, source_ppq) + 239) / 240) as u32
            };
            if max_note_end > 0 {
                length_steps = length_steps.max(((max_note_end + 239) / 240) as u32);
            }
            length_steps = length_steps.clamp(1, MAX_PATTERN_STEPS);

            let mut pattern = Pattern::new(
                (index + 1) as u32,
                if raw.name.trim().is_empty() {
                    format!("Pattern {}", index + 1)
                } else {
                    raw.name.clone()
                },
                raw.color,
                length_steps,
            );
            // Give every imported pattern a lane for each playable channel so
            // the step sequencer UI is immediately usable.
            for channel in &project.channels {
                pattern.channels.push(PatternChannel {
                    channel_id: channel.id,
                    steps: vec![Step::default(); length_steps as usize],
                    notes: Vec::new(),
                    cc: Vec::new(),
                });
            }
            for note in &raw.notes {
                let Some(&channel_id) = channel_id_map.get(&note.rack_channel) else {
                    continue;
                };
                let Some(lane) = pattern
                    .channels
                    .iter_mut()
                    .find(|lane| lane.channel_id == channel_id)
                else {
                    continue;
                };
                let start = convert_ticks(note.position as u64, source_ppq);
                let length = if note.length == 0 {
                    PPQ as u64 / 4
                } else {
                    convert_ticks(note.length as u64, source_ppq).max(1)
                };
                let id = project.next_note_id;
                project.next_note_id += 1;
                lane.notes.push(Note {
                    id,
                    key: note.key.min(127) as u8,
                    start,
                    length,
                    velocity: note.velocity.min(127).max(1),
                    pan: (note.pan as i16 - 64).clamp(-64, 63) as i8,
                    release: note.release.min(127),
                });
            }
            project.patterns.push(pattern);
        }
    }

    project.next_pattern_id = (project.patterns.len() + 1) as u32;
    project.current_pattern = 0;
    if project.channels.is_empty() && !patterns.is_empty() {
        warnings.push("FLP has no playable channel records".into());
    }
    project.flp_source = Some(FlpSource {
        format: document.header.format,
        channels: document.header.channels,
        ppq: document.header.ppq,
        version,
        events: unknown,
    });
    repair_project(&mut project);
    project
}

fn raw(event: &FlpEvent) -> FlpRawEvent {
    FlpRawEvent {
        id: event.id,
        payload: event.payload.clone(),
    }
}

// ------------------------------------------------------------------- sidecar

fn encode_sidecar(project: &StudioProject) -> FlpResult<Vec<u8>> {
    let body = bincode::serialize(project)
        .map_err(|e| FlpError::Invalid(format!("sidecar serialization failed: {e}")))?;
    if body.len() > MAX_PROJECT_BYTES {
        return Err(FlpError::Invalid("sidecar project is too large".into()));
    }
    let compressed = zlib_compress(&body)?;
    let body_len = u32::try_from(body.len())
        .map_err(|_| FlpError::Invalid("sidecar project exceeds 4 GiB".into()))?;
    let mut payload = Vec::with_capacity(14 + compressed.len());
    payload.extend_from_slice(SIDECAR_MAGIC);
    payload.extend_from_slice(&SIDECAR_VERSION.to_le_bytes());
    payload.extend_from_slice(&body_len.to_le_bytes());
    payload.extend_from_slice(&compressed);
    Ok(payload)
}

fn decode_sidecar(payload: &[u8]) -> FlpResult<StudioProject> {
    if payload.len() < 14 || !payload.starts_with(SIDECAR_MAGIC) {
        return Err(FlpError::Invalid("missing REDRAW sidecar magic".into()));
    }
    let version = read_u16(payload, 8).unwrap_or(0);
    if version == 0 || version > SIDECAR_VERSION {
        return Err(FlpError::Invalid(format!(
            "unsupported sidecar version {version}"
        )));
    }
    let uncompressed = read_u32(payload, 10).unwrap_or(0) as usize;
    if uncompressed > MAX_PROJECT_BYTES {
        return Err(FlpError::Invalid(format!(
            "sidecar body is {uncompressed} bytes, limit is {MAX_PROJECT_BYTES}"
        )));
    }
    let body = zlib_decompress(&payload[14..], MAX_PROJECT_BYTES)?;
    if body.len() != uncompressed {
        return Err(FlpError::Invalid(format!(
            "sidecar body size mismatch: header {uncompressed}, actual {}",
            body.len()
        )));
    }
    let mut cursor = Cursor::new(body.as_slice());
    let project: StudioProject = bincode::deserialize_from(&mut cursor)
        .map_err(|e| FlpError::Invalid(format!("sidecar decode failed: {e}")))?;
    if cursor.position() as usize != body.len() {
        return Err(FlpError::Invalid(
            "trailing bytes after sidecar project".into(),
        ));
    }
    Ok(project)
}

// ------------------------------------------------------------------- event codec

fn parse_event_stream(data: &[u8], depth: u8) -> FlpResult<Vec<FlpEvent>> {
    if depth > MAX_NESTED_DEPTH {
        return Err(FlpError::Invalid("nested FLP compression is too deep".into()));
    }
    let mut events = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        if events.len() >= MAX_EVENTS {
            return Err(FlpError::Invalid("FLP event count limit reached".into()));
        }
        let id = data[pos];
        pos += 1;
        let (payload, next) = match id {
            0..=63 => {
                let end = pos + 1;
                if end > data.len() {
                    return Err(FlpError::Invalid(format!(
                        "truncated 1-byte event id {id} at {pos}"
                    )));
                }
                (data[pos..end].to_vec(), end)
            }
            64..=127 => {
                let end = pos + 2;
                if end > data.len() {
                    return Err(FlpError::Invalid(format!(
                        "truncated 2-byte event id {id} at {pos}"
                    )));
                }
                (data[pos..end].to_vec(), end)
            }
            128..=191 => {
                let end = pos + 4;
                if end > data.len() {
                    return Err(FlpError::Invalid(format!(
                        "truncated 4-byte event id {id} at {pos}"
                    )));
                }
                (data[pos..end].to_vec(), end)
            }
            _ => {
                let (length, after_len) = read_leb128(data, pos)?;
                pos = after_len;
                if length > MAX_EVENT_PAYLOAD {
                    return Err(FlpError::Invalid(format!(
                        "event id {id} payload is {length} bytes, limit is {MAX_EVENT_PAYLOAD}"
                    )));
                }
                let end = pos
                    .checked_add(length)
                    .ok_or_else(|| FlpError::Invalid("event payload range overflow".into()))?;
                if end > data.len() {
                    return Err(FlpError::Invalid(format!(
                        "event id {id} payload is truncated (need {length}, have {})",
                        data.len() - pos
                    )));
                }
                (data[pos..end].to_vec(), end)
            }
        };
        pos = next;

        if id == 202 {
            if payload.len() >= 2 {
                let zlib_header = payload[0] == 0x78
                    && (u16::from_be_bytes([payload[0], payload[1]]) % 31) == 0;
                if zlib_header {
                    let inner = zlib_decompress(&payload, MAX_PROJECT_BYTES)?;
                    let nested = parse_event_stream(&inner, depth + 1)?;
                    events.extend(nested);
                    continue;
                }
            }
        }
        events.push(FlpEvent { id, payload });
    }
    Ok(events)
}

fn write_event(out: &mut Vec<u8>, id: u8, payload: &[u8]) -> FlpResult<()> {
    let expected = match id {
        0..=63 => Some(1),
        64..=127 => Some(2),
        128..=191 => Some(4),
        _ => None,
    };
    if let Some(width) = expected {
        if payload.len() != width {
            return Err(FlpError::Invalid(format!(
                "event id {id} needs {width} payload bytes, got {}",
                payload.len()
            )));
        }
    } else if payload.len() > MAX_EVENT_PAYLOAD {
        return Err(FlpError::Invalid(format!(
            "event id {id} payload exceeds limit"
        )));
    }
    out.push(id);
    if expected.is_none() {
        write_leb128(out, payload.len());
    }
    out.extend_from_slice(payload);
    Ok(())
}

fn read_leb128(data: &[u8], mut pos: usize) -> FlpResult<(usize, usize)> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        if pos >= data.len() {
            return Err(FlpError::Invalid("truncated LEB128 length".into()));
        }
        let byte = data[pos];
        pos += 1;
        if shift >= 64 {
            return Err(FlpError::Invalid("LEB128 length overflow".into()));
        }
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    let value = usize::try_from(value)
        .map_err(|_| FlpError::Invalid("LEB128 length does not fit usize".into()))?;
    Ok((value, pos))
}

fn write_leb128(out: &mut Vec<u8>, mut value: usize) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn push_event(out: &mut Vec<FlpEvent>, id: u8, payload: Vec<u8>) {
    out.push(FlpEvent { id, payload });
}

// ------------------------------------------------------------------- small helpers

fn find_magic(data: &[u8], magic: &[u8; 4], start: usize, limit: usize) -> Option<usize> {
    if magic.len() != 4 || start > data.len() {
        return None;
    }
    let end = data.len().min(start.saturating_add(limit));
    if end < 4 {
        return None;
    }
    (start..=end - 4).find(|&pos| data.get(pos..pos + 4) == Some(magic))
}

fn read_u16(data: &[u8], pos: usize) -> Option<u16> {
    let bytes: [u8; 2] = data.get(pos..pos + 2)?.try_into().ok()?;
    Some(u16::from_le_bytes(bytes))
}

fn read_i16(data: &[u8], pos: usize) -> Option<i16> {
    let bytes: [u8; 2] = data.get(pos..pos + 2)?.try_into().ok()?;
    Some(i16::from_le_bytes(bytes))
}

fn read_u32(data: &[u8], pos: usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(pos..pos + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn read_i32(data: &[u8], pos: usize) -> Option<i32> {
    let bytes: [u8; 4] = data.get(pos..pos + 4)?.try_into().ok()?;
    Some(i32::from_le_bytes(bytes))
}

fn decode_ascii(payload: &[u8]) -> String {
    let text = String::from_utf8_lossy(payload);
    text.trim_end_matches('\0').to_string()
}

fn decode_utf16(payload: &[u8]) -> String {
    let units: Vec<u16> = payload
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    let text = String::from_utf16_lossy(&units);
    text.trim_end_matches('\0').to_string()
}

fn ascii_payload(text: &str) -> Vec<u8> {
    let mut out = text.as_bytes().to_vec();
    out.push(0);
    out
}

fn utf16_payload(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 2 + 2);
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn color3(payload: &[u8]) -> [u8; 3] {
    if payload.len() >= 3 {
        [payload[0], payload[1], payload[2]]
    } else {
        [120, 140, 170]
    }
}

fn pattern_iid(pattern: &Pattern) -> u16 {
    pattern.id.min(u16::MAX as u32) as u16
}

fn convert_ticks(tick: u64, source_ppq: u16) -> u64 {
    let ppq = if source_ppq == 0 {
        PPQ as u64
    } else {
        source_ppq as u64
    };
    tick.saturating_mul(PPQ as u64)
        .saturating_add(ppq / 2)
        / ppq
}

fn to_source_ticks(tick: u64, source_ppq: u16) -> u64 {
    let ppq = if source_ppq == 0 {
        PPQ as u64
    } else {
        source_ppq as u64
    };
    tick.saturating_mul(ppq)
        .saturating_add(PPQ as u64 / 2)
        / PPQ as u64
}

fn fl_volume_to_db(word: u16) -> f32 {
    if word == 0 || !(word as f32).is_finite() {
        -90.0
    } else {
        (20.0 * (word as f32 / 10_000.0).log10()).clamp(-90.0, 12.0)
    }
}

fn fl_volume_from_db(db: f32) -> u16 {
    if !db.is_finite() || db <= -90.0 {
        return 0;
    }
    let linear = 10f32.powf(db / 20.0) * 10_000.0;
    linear.clamp(0.0, 12_800.0) as u16
}

fn fl_pan_to_norm(pan: i32) -> f32 {
    (pan as f32 / 6400.0 - 1.0).clamp(-1.0, 1.0)
}

fn fl_pan_from_norm(pan: f32) -> u16 {
    (((pan.clamp(-1.0, 1.0) + 1.0) * 6400.0) as u16).min(12_800)
}

fn write_note(
    out: &mut Vec<u8>,
    position: u64,
    rack_channel: u16,
    length: u64,
    key: u8,
    velocity: u8,
    pan: u8,
    release: u8,
) {
    out.extend_from_slice(&(position.min(u32::MAX as u64) as u32).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // flags
    out.extend_from_slice(&rack_channel.to_le_bytes());
    out.extend_from_slice(&(length.min(u32::MAX as u64) as u32).to_le_bytes());
    out.extend_from_slice(&(key as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // group
    out.push(120); // fine pitch: no detune
    out.push(0);
    out.push(release.min(128));
    out.push(0); // MIDI channel / note color
    out.push(pan);
    out.push(velocity.min(128));
    out.push(128);
    out.push(128);
}

fn zlib_compress(data: &[u8]) -> FlpResult<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

fn zlib_decompress(data: &[u8], max: usize) -> FlpResult<Vec<u8>> {
    let decoder = ZlibDecoder::new(data);
    let mut limited = decoder.take(max as u64 + 1);
    let mut out = Vec::new();
    limited.read_to_end(&mut out)?;
    if out.len() > max {
        return Err(FlpError::Invalid(format!(
            "decompressed FLP data exceeds {max} bytes"
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir;
    use redraw_shared::{AutoTarget, AutomationClipState};

    fn event(id: u8, payload: Vec<u8>) -> FlpEvent {
        FlpEvent { id, payload }
    }

    fn base_events() -> Vec<FlpEvent> {
        vec![
            event(199, ascii_payload("24.2.99.4720")),
            event(194, utf16_payload("Test Song")),
            event(207, utf16_payload("Test Author")),
            event(156, 140_000u32.to_le_bytes().to_vec()),
            event(17, vec![4]),
            event(18, vec![4]),
            event(11, vec![32]),
        ]
    }

    fn note_bytes(position: u32, rack: u16, length: u32, key: u16) -> Vec<u8> {
        let mut out = Vec::new();
        write_note(
            &mut out,
            position as u64,
            rack,
            length as u64,
            key as u8,
            100,
            64,
            64,
        );
        out
    }

    fn external_document() -> FlpDocument {
        let mut events = base_events();
        events.push(event(64, 0u16.to_le_bytes().to_vec()));
        events.push(event(21, vec![0]));
        events.push(event(203, utf16_payload("Imported Sampler")));
        events.push(event(128, vec![12, 34, 56, 255]));
        events.push(event(
            219,
            {
                let mut v = Vec::new();
                v.extend_from_slice(&6400i32.to_le_bytes());
                v.extend_from_slice(&10000u32.to_le_bytes());
                v.extend_from_slice(&0u32.to_le_bytes());
                v.extend_from_slice(&0u32.to_le_bytes());
                v.extend_from_slice(&0u32.to_le_bytes());
                v.extend_from_slice(&0u32.to_le_bytes());
                v
            },
        ));
        events.push(event(65, 1u16.to_le_bytes().to_vec()));
        events.push(event(224, note_bytes(0, 0, 48, 60)));
        events.push(event(224, note_bytes(96, 0, 24, 64)));
        events.push(event(65, 1u16.to_le_bytes().to_vec()));
        events.push(event(193, utf16_payload("Verse")));
        events.push(event(150, vec![90, 100, 110, 255]));
        events.push(event(164, 384u32.to_le_bytes().to_vec()));
        events.push(event(247, vec![1, 2, 3, 4, 5]));
        FlpDocument {
            header: FlpHeader {
                format: 0,
                channels: 1,
                ppq: 96,
            },
            events,
        }
    }

    #[test]
    fn flp_roundtrip_with_sidecar_is_lossless() {
        let mut project = StudioProject::default();
        project.name = "Sidecar Song".into();
        project.bpm = 127.0;
        project.patterns[0].name = "Drop".into();
        project.patterns[0].channels[0].notes.push(Note {
            id: 7,
            key: 67,
            start: 240,
            length: 480,
            velocity: 111,
            pan: 10,
            release: 80,
        });
        project.next_note_id = 8;
        project.automation.push(AutomationClipState {
            id: 1,
            name: "Vol".into(),
            color: [1, 2, 3],
            target: AutoTarget::MasterVolume,
            points: vec![redraw_shared::AutomationPoint {
                time: 0.0,
                value: 0.5,
                tension: 0.0,
            }],
            interp: redraw_shared::Interp::Linear,
            enabled: true,
            length: 960,
        });
        project.next_automation_id = 2;

        let bytes = export_flp(&project).unwrap();
        assert_eq!(&bytes[..4], b"FLhd");
        assert_eq!(&bytes[14..18], b"FLdt");
        let document = parse_flp(&bytes).unwrap();
        assert!(document
            .events
            .iter()
            .any(|e| e.id == FLP_SIDECAR_EVENT_ID));
        let imported = import_flp(&bytes).unwrap();
        assert!(imported.sidecar_used);
        assert_eq!(
            bincode::serialize(&project).unwrap(),
            bincode::serialize(&imported.project).unwrap()
        );
    }

    #[test]
    fn flp_core_import_reads_patterns_notes_channels_and_levels() {
        let document = external_document();
        let bytes = write_flp(&document).unwrap();
        let imported = import_flp(&bytes).unwrap();
        assert!(!imported.sidecar_used);
        assert_eq!(imported.unknown_events, 1);
        let project = imported.project;
        assert_eq!(project.name, "Test Song");
        assert_eq!(project.author, "Test Author");
        assert_eq!(project.bpm, 140.0);
        assert_eq!(project.time_sig, (4, 4));
        assert!((project.swing - 0.5625).abs() < 1e-4);
        assert_eq!(project.channels.len(), 1);
        assert_eq!(project.channels[0].name, "Imported Sampler");
        assert_eq!(project.channels[0].synth_type, "Sampler");
        assert_eq!(project.channels[0].color, [12, 34, 56]);
        assert_eq!(project.channels[0].volume_db, 0.0);
        assert_eq!(project.patterns.len(), 1);
        assert_eq!(project.patterns[0].name, "Verse");
        assert_eq!(project.patterns[0].length_steps, 16);
        let notes = &project.patterns[0].channels[0].notes;
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].start, 0);
        assert_eq!(notes[0].length, 480);
        assert_eq!(notes[1].start, 960);
        assert_eq!(notes[1].key, 64);
        assert_eq!(notes[1].pan, 0);
        let source = project.flp_source.unwrap();
        assert_eq!(source.ppq, 96);
        assert_eq!(source.version, "24.2.99.4720");
        assert_eq!(source.events.len(), 1);
        assert_eq!(source.events[0].id, 247);
    }

    #[test]
    fn flp_export_preserves_unknown_events_and_core_events() {
        let document = external_document();
        let bytes = write_flp(&document).unwrap();
        let imported = import_flp(&bytes).unwrap();
        let exported = export_flp(&imported.project).unwrap();
        let reparsed = parse_flp(&exported).unwrap();
        assert!(reparsed.events.iter().any(|e| e.id == 156));
        assert!(reparsed.events.iter().any(|e| e.id == 224));
        assert!(reparsed.events.iter().any(|e| e.id == 247 && e.payload == [1, 2, 3, 4, 5]));
        assert!(reparsed
            .events
            .iter()
            .any(|e| e.id == FLP_SIDECAR_EVENT_ID));
    }

    #[test]
    fn flp_compressed_event_202_is_inflated() {
        let mut inner = Vec::new();
        write_event(&mut inner, 156, &128_000u32.to_le_bytes()).unwrap();
        write_event(&mut inner, 194, &utf16_payload("Compressed")).unwrap();
        let compressed = zlib_compress(&inner).unwrap();
        let document = FlpDocument {
            header: FlpHeader {
                format: 0,
                channels: 0,
                ppq: 960,
            },
            events: vec![event(202, compressed)],
        };
        let bytes = write_flp(&document).unwrap();
        let parsed = parse_flp(&bytes).unwrap();
        assert!(parsed.events.iter().any(|e| e.id == 156));
        assert!(!parsed.events.iter().any(|e| e.id == 202));
        let imported = import_flp(&bytes).unwrap();
        assert_eq!(imported.project.bpm, 128.0);
        assert_eq!(imported.project.name, "Compressed");
    }

    #[test]
    fn flp_truncated_event_is_rejected() {
        let mut bytes = write_flp(&FlpDocument {
            header: FlpHeader::default(),
            events: vec![event(156, 140_000u32.to_le_bytes().to_vec())],
        })
        .unwrap();
        bytes.truncate(bytes.len() - 2);
        assert!(parse_flp(&bytes).is_err());
    }

    #[test]
    fn flp_save_load_with_path_name_and_sidecar_fallback() {
        let dir = test_dir("flp");
        let path = dir.join("My Import.flp");
        let imported = import_flp(&write_flp(&external_document()).unwrap()).unwrap();
        save_flp(&path, &imported.project).unwrap();
        let loaded = load_flp(&path).unwrap();
        assert!(loaded.sidecar_used);
        assert_eq!(loaded.project.name, "Test Song");

        // An invalid sidecar must not prevent core import.
        let mut document = external_document();
        document.events.push(event(
            FLP_SIDECAR_EVENT_ID,
            {
                let mut p = SIDECAR_MAGIC.to_vec();
                p.extend_from_slice(&1u16.to_le_bytes());
                p.extend_from_slice(&10u32.to_le_bytes());
                p.extend_from_slice(&[0, 1, 2, 3]);
                p
            },
        ));
        let bytes = write_flp(&document).unwrap();
        let fallback = import_flp(&bytes).unwrap();
        assert!(!fallback.sidecar_used);
        assert!(fallback
            .warnings
            .iter()
            .any(|warning| warning.contains("sidecar")));
        assert_eq!(fallback.project.bpm, 140.0);
    }

    #[test]
    fn flp_ppq_conversion_roundtrips_note_positions() {
        let document = external_document();
        let bytes = write_flp(&document).unwrap();
        let imported = import_flp(&bytes).unwrap();
        let exported = export_flp(&imported.project).unwrap();
        let parsed = parse_flp(&exported).unwrap();
        assert_eq!(parsed.header.ppq, 96);
        let notes = parsed
            .events
            .iter()
            .find(|e| e.id == 224)
            .expect("notes");
        assert_eq!(u32::from_le_bytes(notes.payload[0..4].try_into().unwrap()), 0);
        assert_eq!(
            u32::from_le_bytes(notes.payload[24..28].try_into().unwrap()),
            96
        );
    }

    #[test]
    fn flp_does_not_export_automation_as_fake_core_data() {
        let _ = AutoTarget::Tempo; // keep the import used without dead-code noise
        let project = StudioProject::default();
        let bytes = export_flp(&project).unwrap();
        let document = parse_flp(&bytes).unwrap();
        assert!(document.events.iter().all(|e| e.id != 223));
        assert!(document.events.iter().all(|e| e.id != 224 || e.payload.len() % 24 == 0));
    }

    #[test]
    #[ignore = "reads a caller-supplied external FLP via REDRAW_FLP_FIXTURE"]
    fn imports_external_flp_fixture_if_configured() {
        let Ok(path) = std::env::var("REDRAW_FLP_FIXTURE") else {
            return;
        };
        let imported = load_flp(&path).expect("import external fixture");
        let notes: usize = imported
            .project
            .patterns
            .iter()
            .flat_map(|pattern| pattern.channels.iter())
            .map(|channel| channel.notes.len())
            .sum();
        eprintln!(
            "fixture {}: {} channels, {} patterns, {} notes, {} unknown events, source ppq {:?}",
            path,
            imported.project.channels.len(),
            imported.project.patterns.len(),
            notes,
            imported.unknown_events,
            imported.project.flp_source.as_ref().map(|source| source.ppq),
        );
        assert!(!imported.project.patterns.is_empty());
        assert!(!imported.project.channels.is_empty());
        assert!(!imported.sidecar_used);
        assert!(imported.unknown_events > 0);

        // Re-exporting must retain every unhandled source event in the raw
        // block (the sidecar test separately covers exact project fidelity).
        let mut expected: BTreeMap<(u8, Vec<u8>), usize> = BTreeMap::new();
        for event in &imported
            .project
            .flp_source
            .as_ref()
            .expect("source metadata")
            .events
        {
            *expected
                .entry((event.id, event.payload.clone()))
                .or_default() += 1;
        }
        let exported = export_flp(&imported.project).expect("re-export fixture");
        let mut actual: BTreeMap<(u8, Vec<u8>), usize> = BTreeMap::new();
        for event in parse_flp(&exported).expect("parse re-export").events {
            *actual.entry((event.id, event.payload)).or_default() += 1;
        }
        for (event, count) in expected {
            assert!(
                actual.get(&event).copied().unwrap_or(0) >= count,
                "raw event {} was not preserved",
                event.0
            );
        }
    }
}
