//! Project I/O for REDRAW.
//!
//! * [`rdp`] - lossless native projects: a minimal ZIP container holding a
//!   JSON header and a zlib-compressed bincode body.
//! * [`flp`] - tolerant FLP import plus a best-effort core export with a
//!   REDRAW sidecar for lossless round-trips inside REDRAW.
//! * [`autosave`] - rotating crash-recovery snapshots under `user-data/`.
//!
//! All decoding is defensive: sizes are capped, ZIP names never become paths,
//! and a decoded project is structurally repaired before it reaches the audio
//! engine.
#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use redraw_shared::{
    PlaylistState, StudioProject, MAX_CHANNELS, MAX_INSERTS, MAX_PATTERNS, PPQ, SAMPLE_RATE,
};

pub mod autosave;
pub mod flp;
pub mod legacy;
pub mod rdp;

pub use autosave::{AutosaveEntry, AutosaveManager, RecoveryOffer};
pub use legacy::{
    migrate_legacy_type_names, resolve_legacy_type_name, LEGACY_EFFECT_ALIASES,
    LEGACY_SYNTH_ALIASES,
};
pub use flp::{
    export_flp, import_flp, load_flp, save_flp, FlpDocument, FlpError, FlpEvent, FlpHeader,
    FlpImport,
};
pub use rdp::{
    decode_project, encode_project, load_project, read_header, save_project, RdpHeader,
    RDP_FORMAT, RDP_VERSION,
};

/// Maximum accepted uncompressed project body (256 MiB).
pub const MAX_PROJECT_BYTES: usize = 256 * 1024 * 1024;

/// Defensive upper bound for the number of steps in one pattern after a load.
pub const MAX_PATTERN_STEPS: u32 = 4096;

/// Upper bound for automation lanes after a load (mirrors the engine cap).
pub const MAX_AUTOMATION_LANES: usize = 256;

/// Upper bound for points per automation lane after a load.
pub const MAX_AUTOMATION_POINTS: usize = 16_384;

/// Upper bound for all automation points after a load.
pub const MAX_TOTAL_AUTOMATION_POINTS: usize = 131_072;

/// Errors shared by RDP, FLP and autosave code.
#[derive(Debug)]
pub enum FileIoError {
    Io(io::Error),
    Zip(String),
    Invalid(String),
    Json(serde_json::Error),
    Bincode(Box<bincode::ErrorKind>),
    Flp(flp::FlpError),
}

impl fmt::Display for FileIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Zip(m) => write!(f, "project container error: {m}"),
            Self::Invalid(m) => write!(f, "invalid project file: {m}"),
            Self::Json(e) => write!(f, "project header JSON error: {e}"),
            Self::Bincode(e) => write!(f, "project body error: {e}"),
            Self::Flp(e) => write!(f, "FLP error: {e}"),
        }
    }
}

impl std::error::Error for FileIoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::Bincode(e) => Some(e),
            Self::Flp(e) => Some(e),
            Self::Zip(_) | Self::Invalid(_) => None,
        }
    }
}

impl From<io::Error> for FileIoError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for FileIoError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

impl From<Box<bincode::ErrorKind>> for FileIoError {
    fn from(e: Box<bincode::ErrorKind>) -> Self {
        Self::Bincode(e)
    }
}

impl From<flp::FlpError> for FileIoError {
    fn from(e: flp::FlpError) -> Self {
        Self::Flp(e)
    }
}

pub type Result<T> = std::result::Result<T, FileIoError>;

/// Atomically replace `path` with `bytes` using a temporary file in the same
/// directory. On Windows `rename` normally replaces the destination; if it
/// refuses, the destination is removed and the rename is retried once.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let file_name = path
        .file_name()
        .filter(|n| !n.is_empty())
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("redraw-project"));

    let mut last_collision = None;
    for _ in 0..16 {
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut tmp_name = OsString::from(".");
        tmp_name.push(&file_name);
        tmp_name.push(format!(".{}.{}.tmp", std::process::id(), serial));
        let tmp = parent.join(tmp_name);
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                last_collision = Some(e);
                continue;
            }
            Err(e) => return Err(e),
        };

        let written = file
            .write_all(bytes)
            .and_then(|_| file.sync_all());
        drop(file);
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }

        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(first_error) => {
                // Some Windows filesystem/antivirus combinations refuse to
                // replace an existing destination through std::fs::rename.
                // A same-directory remove+rename still avoids a truncated
                // destination on write failure.
                if path.exists() && fs::remove_file(path).is_ok() {
                    if fs::rename(&tmp, path).is_ok() {
                        return Ok(());
                    }
                }
                let _ = fs::remove_file(&tmp);
                return Err(first_error);
            }
        }
    }

    Err(last_collision.unwrap_or_else(|| {
        io::Error::new(io::ErrorKind::Other, "could not allocate a temporary file")
    }))
}

/// Repair structural invariants that could otherwise panic the engine/UI.
///
/// This runs after every project load, not when saving, so a valid project is
/// preserved byte-for-byte through an RDP round-trip. Repairs are intentionally
/// conservative: out-of-range scalars are clamped, missing fixed-size mixer
/// arrays are padded, and orphan patterns/tracks are given safe defaults.
pub fn repair_project(project: &mut StudioProject) {
    project.version = project.version.max(1);
    if project.name.trim().is_empty() {
        project.name = "Untitled".into();
    }
    if !project.bpm.is_finite() {
        project.bpm = 140.0;
    }
    project.bpm = project.bpm.clamp(10.0, 999.0);

    let (mut numerator, mut denominator) = project.time_sig;
    if numerator == 0 {
        numerator = 4;
    }
    if denominator == 0 || !denominator.is_power_of_two() {
        denominator = 4;
    }
    project.time_sig = (numerator.min(32), denominator.min(32));

    if !project.swing.is_finite() {
        project.swing = 0.5;
    }
    project.swing = project.swing.clamp(0.0, 1.0);

    if project.patterns.is_empty() {
        project.patterns.push(redraw_shared::Pattern::new(
            1,
            "Pattern 1",
            [95, 179, 108],
            16,
        ));
    }
    project.patterns.truncate(MAX_PATTERNS);
    for (pattern_index, pattern) in project.patterns.iter_mut().enumerate() {
        if pattern.id == 0 {
            pattern.id = pattern_index as u32 + 1;
        }
        if pattern.name.trim().is_empty() {
            pattern.name = format!("Pattern {}", pattern_index + 1);
        }
        pattern.length_steps = pattern.length_steps.clamp(1, MAX_PATTERN_STEPS);
        pattern.channels.truncate(MAX_CHANNELS);
        for channel in &mut pattern.channels {
            if channel.channel_id == 0 {
                channel.channel_id = 1;
            }
            channel
                .steps
                .resize(pattern.length_steps as usize, redraw_shared::Step::default());
            for note in &mut channel.notes {
                note.key = note.key.min(127);
                note.velocity = note.velocity.min(127);
                note.pan = note.pan.clamp(-64, 63);
                note.release = note.release.min(127);
            }
        }
    }
    project.current_pattern = project.current_pattern.min(project.patterns.len() - 1);

    project.channels.truncate(MAX_CHANNELS);
    for (index, channel) in project.channels.iter_mut().enumerate() {
        if channel.id == 0 {
            channel.id = index as u32 + 1;
        }
        if channel.name.trim().is_empty() {
            channel.name = format!("Channel {}", index + 1);
        }
        if channel.synth_type.trim().is_empty() {
            channel.synth_type = "Aurora".into();
        }
        for value in &mut channel.params {
            if !value.is_finite() {
                *value = 0.0;
            }
        }
        if !channel.volume_db.is_finite() {
            channel.volume_db = 0.0;
        }
        channel.volume_db = channel.volume_db.clamp(-90.0, 12.0);
        if !channel.pan.is_finite() {
            channel.pan = 0.0;
        }
        channel.pan = channel.pan.clamp(-1.0, 1.0);
        channel.mixer_index %= MAX_CHANNELS;
    }

    while project.mixer.strips.len() < MAX_CHANNELS {
        project.mixer.strips.push(Default::default());
    }
    project.mixer.strips.truncate(MAX_CHANNELS);
    for (index, strip) in project.mixer.strips.iter_mut().enumerate() {
        if strip.name.trim().is_empty() {
            strip.name = format!("Ch {}", index + 1);
        }
        if !strip.volume_db.is_finite() {
            strip.volume_db = 0.0;
        }
        strip.volume_db = strip.volume_db.clamp(-90.0, 12.0);
        if !strip.pan.is_finite() {
            strip.pan = 0.0;
        }
        strip.pan = strip.pan.clamp(-1.0, 1.0);
        strip.inserts.resize(MAX_INSERTS, None);
        for send in &mut strip.sends {
            if !send.level_db.is_finite() {
                send.level_db = -90.0;
            }
            send.level_db = send.level_db.clamp(-90.0, 12.0);
            send.target = send.target.min(3);
        }
    }
    if project.mixer.master.name.trim().is_empty() {
        project.mixer.master.name = "Master".into();
    }
    if !project.mixer.master.volume_db.is_finite() {
        project.mixer.master.volume_db = 0.0;
    }
    project.mixer.master.volume_db = project.mixer.master.volume_db.clamp(-90.0, 12.0);
    if !project.mixer.master.pan.is_finite() {
        project.mixer.master.pan = 0.0;
    }
    project.mixer.master.pan = project.mixer.master.pan.clamp(-1.0, 1.0);
    project.mixer.master.inserts.resize(MAX_INSERTS, None);
    for send in &mut project.mixer.master.sends {
        send.target = send.target.min(3);
    }
    for inserts in &mut project.mixer.bus_inserts {
        inserts.resize(MAX_INSERTS, None);
    }
    for volume in &mut project.mixer.bus_volumes_db {
        if !volume.is_finite() {
            *volume = 0.0;
        }
        *volume = volume.clamp(-90.0, 12.0);
    }

    if project.playlist.tracks.is_empty() {
        project.playlist.tracks = PlaylistState::default().tracks;
    }
    project.playlist.tracks.truncate(128);
    for (index, track) in project.playlist.tracks.iter_mut().enumerate() {
        if track.name.trim().is_empty() {
            track.name = format!("Track {}", index + 1);
        }
        if !track.height.is_finite() {
            track.height = 74.0;
        }
        track.height = track.height.clamp(32.0, 300.0);
    }
    project.playlist.clips.truncate(262_144);
    for clip in &mut project.playlist.clips {
        if clip.track >= project.playlist.tracks.len() {
            clip.track = 0;
        }
        if clip.length == 0 {
            clip.length = 1;
        }
        if !clip.gain_db.is_finite() {
            clip.gain_db = 0.0;
        }
        clip.gain_db = clip.gain_db.clamp(-90.0, 24.0);
        if !clip.stretch.is_finite() {
            clip.stretch = 1.0;
        }
        clip.stretch = clip.stretch.clamp(0.01, 100.0);
        clip.transpose = clip.transpose.clamp(-48, 48);
    }
    project.playlist.markers.truncate(10_000);
    if project.playlist.loop_enabled && project.playlist.loop_end <= project.playlist.loop_start {
        project.playlist.loop_end = project.playlist
            .loop_start
            .saturating_add(PPQ as u64 * 4);
    }

    project.automation.truncate(MAX_AUTOMATION_LANES);
    let mut total_points = 0usize;
    for (index, lane) in project.automation.iter_mut().enumerate() {
        if lane.name.trim().is_empty() {
            lane.name = format!("Automation {}", index + 1);
        }
        lane.length = lane.length.max(1);
        lane.points.retain(|p| {
            p.time.is_finite() && p.value.is_finite() && p.tension.is_finite()
        });
        for point in &mut lane.points {
            point.time = point.time.clamp(0.0, lane.length as f64);
            point.value = point.value.clamp(0.0, 1.0);
            point.tension = point.tension.clamp(-1.0, 1.0);
        }
        lane.points.sort_by(|a, b| a.time.total_cmp(&b.time));
        let mut deduped: Vec<redraw_shared::AutomationPoint> =
            Vec::with_capacity(lane.points.len());
        for point in lane.points.drain(..) {
            if let Some(last) = deduped.last_mut() {
                if (last.time - point.time).abs() < 1e-9 {
                    *last = point;
                    continue;
                }
            }
            deduped.push(point);
        }
        deduped.truncate(MAX_AUTOMATION_POINTS);
        let remaining = MAX_TOTAL_AUTOMATION_POINTS.saturating_sub(total_points);
        deduped.truncate(remaining);
        total_points += deduped.len();
        lane.points = deduped;
    }

    let next_channel = project
        .channels
        .iter()
        .map(|c| c.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    project.next_channel_id = project.next_channel_id.max(next_channel).max(1);
    let next_pattern = project
        .patterns
        .iter()
        .map(|p| p.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    project.next_pattern_id = project.next_pattern_id.max(next_pattern).max(1);
    let next_clip = project
        .playlist
        .clips
        .iter()
        .map(|c| c.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    project.next_clip_id = project.next_clip_id.max(next_clip).max(1);
    let next_note = project
        .patterns
        .iter()
        .flat_map(|p| p.channels.iter())
        .flat_map(|c| c.notes.iter())
        .map(|n| n.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    project.next_note_id = project.next_note_id.max(next_note).max(1);
    let next_automation = project
        .automation
        .iter()
        .map(|a| a.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    project.next_automation_id = project
        .next_automation_id
        .max(next_automation)
        .max(1);

    project.sample_rate = SAMPLE_RATE;
    legacy::migrate_legacy_type_names(project);
}

/// Temporary directory helper used by this crate's tests.
#[cfg(test)]
pub(crate) fn test_dir(tag: &str) -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/test-tmp")
        .join("fileio");
    let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = root.join(format!(
        "{tag}-{}-{}",
        std::process::id(),
        serial
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test directory");
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    use redraw_shared::{ChannelInfo, Note, Pattern};

    #[test]
    fn repair_fixes_core_structural_fields() {
        let mut project = StudioProject::default();
        project.name.clear();
        project.bpm = f32::NAN;
        project.time_sig = (0, 7);
        project.swing = 9.0;
        project.current_pattern = 99;
        project.patterns[0].length_steps = 0;
        project.patterns[0].channels[0].steps.clear();
        project.patterns[0].channels[0].notes.push(Note {
            id: 0,
            key: 200,
            start: 0,
            length: 0,
            velocity: 255,
            pan: -100,
            release: 255,
        });
        project.channels.push(ChannelInfo::new(0, "", "", [0, 0, 0]));
        project.mixer.strips.clear();
        project.playlist.tracks.clear();
        project.playlist.loop_enabled = true;
        project.playlist.loop_start = 100;
        project.playlist.loop_end = 10;
        project.automation.push(redraw_shared::AutomationClipState {
            id: 0,
            name: String::new(),
            color: [0, 0, 0],
            target: redraw_shared::AutoTarget::MasterVolume,
            points: vec![
                redraw_shared::AutomationPoint {
                    time: f64::NAN,
                    value: 4.0,
                    tension: 9.0,
                },
                redraw_shared::AutomationPoint {
                    time: 1.0,
                    value: 0.5,
                    tension: 0.0,
                },
            ],
            interp: redraw_shared::Interp::Linear,
            enabled: true,
            length: 0,
        });

        repair_project(&mut project);

        assert_eq!(project.name, "Untitled");
        assert_eq!(project.bpm, 140.0);
        assert_eq!(project.time_sig, (4, 4));
        assert_eq!(project.swing, 1.0);
        assert_eq!(project.current_pattern, 0);
        assert_eq!(project.patterns[0].length_steps, 1);
        assert_eq!(project.patterns[0].channels[0].steps.len(), 1);
        assert_eq!(project.patterns[0].channels[0].notes[0].key, 127);
        assert_eq!(project.patterns[0].channels[0].notes[0].pan, -64);
        assert_eq!(project.mixer.strips.len(), MAX_CHANNELS);
        assert_eq!(project.mixer.strips[0].inserts.len(), MAX_INSERTS);
        assert!(!project.playlist.tracks.is_empty());
        assert_eq!(project.playlist.loop_end, 100 + PPQ as u64 * 4);
        assert_eq!(project.automation[0].length, 1);
        assert_eq!(project.automation[0].points.len(), 1);
        assert_eq!(project.automation[0].points[0].value, 0.5);
        assert!(project.next_pattern_id > 1);
        assert!(project.next_note_id > 0);
    }

    #[test]
    fn repair_keeps_valid_project_serialization_stable() {
        let mut project = StudioProject::default();
        let before = bincode::serialize(&project).unwrap();
        repair_project(&mut project);
        let after = bincode::serialize(&project).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn atomic_write_replaces_existing_file() {
        let dir = test_dir("atomic");
        let path = dir.join("project.rdp");
        atomic_write(&path, b"first").unwrap();
        atomic_write(&path, b"second").unwrap();
        assert_eq!(fs::read(path).unwrap(), b"second");
        let leftovers = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn pattern_helper_is_used() {
        let p = Pattern::new(7, "Seven", [1, 2, 3], 16);
        assert_eq!(p.id, 7);
        assert_eq!(p.length_ticks(), 3840);
    }
}
