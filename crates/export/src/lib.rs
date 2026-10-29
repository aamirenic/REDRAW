//! Offline song/pattern rendering and file export.
//!
//! Rendering and encoding are deliberately separate: the engine produces
//! 512-frame planar blocks, and the WAV/AIFF/ffmpeg backends consume blocks in
//! a streaming sink. This keeps memory bounded even for long songs. Native WAV
//! and AIFF are written in-tree; compressed audio and video use the separately
//! installed ffmpeg binary described by D-005/D-023.

#![forbid(unsafe_code)]

mod aiff;
mod ffmpeg;
mod wav;

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use redraw_arrangement::build_arrangement;
use redraw_audio_engine::{
    build_schedule, create_engine, EngineGraph, EngineRt, TransportConfig,
};
use redraw_automation::AutomationSchedule;
use redraw_shared::{PluginFactory, StudioProject, BLOCK_SIZE, PPQ, SAMPLE_RATE};

pub use aiff::AiffBitDepth;
pub use ffmpeg::{ensure_ffmpeg, find_ffmpeg, FfmpegOptions};
pub use wav::WavBitDepth;

pub const CRATE_PHASE: u8 = 14;
/// Hard safety bound for one offline render (four hours at the export rate).
pub const MAX_EXPORT_SECONDS: f64 = 4.0 * 60.0 * 60.0;
/// Default decoded-audio tail after the last musical event.
pub const DEFAULT_TAIL_SECONDS: f32 = 2.0;
/// Video canvas dimensions used by the built-in still/visualizer export.
pub const VIDEO_WIDTH: u32 = 1280;
pub const VIDEO_HEIGHT: u32 = 720;
pub const VIDEO_FPS: u32 = 30;

/// What part of the project is rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportScope {
    /// The playlist from tick 0 to its last clip/song end.
    Song,
    /// One pattern, once, by stable pattern id.
    Pattern(u32),
}

impl Default for ExportScope {
    fn default() -> Self {
        Self::Song
    }
}

/// Rendering parameters that do not depend on the output container.
#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub scope: ExportScope,
    pub sample_rate: u32,
    /// Extra time rendered after the song/pattern ends so releases and delay
    /// tails are not clipped.
    pub tail_seconds: f32,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            scope: ExportScope::Song,
            sample_rate: SAMPLE_RATE,
            tail_seconds: DEFAULT_TAIL_SECONDS,
        }
    }
}

/// Native and ffmpeg-backed output formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Wav(WavBitDepth),
    Aiff(AiffBitDepth),
    Mp3,
    Ogg,
    Flac,
    Mp4,
    Mov,
    Avi,
}

impl Default for ExportFormat {
    fn default() -> Self {
        Self::Wav(WavBitDepth::Pcm24)
    }
}

impl ExportFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Wav(_) => "wav",
            Self::Aiff(_) => "aiff",
            Self::Mp3 => "mp3",
            Self::Ogg => "ogg",
            Self::Flac => "flac",
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Avi => "avi",
        }
    }

    pub const fn is_video(self) -> bool {
        matches!(self, Self::Mp4 | Self::Mov | Self::Avi)
    }
}

/// Complete export configuration, including native/ffmpeg selection.
#[derive(Clone, Debug, Default)]
pub struct ExportOptions {
    pub render: RenderOptions,
    pub ffmpeg: FfmpegOptions,
}

/// Facts about the rendered audio, valid for both native and ffmpeg output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportSummary {
    pub sample_rate: u32,
    pub frames: u64,
    pub duration_seconds: f64,
    /// Largest absolute input sample seen before integer/float encoding.
    pub peak: f32,
}

#[derive(Debug)]
pub enum ExportError {
    Io(io::Error),
    Hound(hound::Error),
    Invalid(String),
    NoAudio(String),
    Limit(String),
    FfmpegMissing { searched: Vec<PathBuf> },
    FfmpegFailed { status: Option<i32>, detail: String },
    Download(String),
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Hound(error) => write!(f, "WAV encoder error: {error}"),
            Self::Invalid(message) => write!(f, "invalid export: {message}"),
            Self::NoAudio(message) => write!(f, "nothing to export: {message}"),
            Self::Limit(message) => write!(f, "export limit reached: {message}"),
            Self::FfmpegMissing { searched } => {
                write!(f, "ffmpeg was not found (searched")?;
                if searched.is_empty() {
                    write!(f, " no configured locations")?;
                } else {
                    for path in searched {
                        write!(f, " {}", path.display())?;
                    }
                }
                write!(f, "); install ffmpeg or enable its runtime download")
            }
            Self::FfmpegFailed { status, detail } => {
                write!(f, "ffmpeg failed")?;
                if let Some(code) = status {
                    write!(f, " with status {code}")?;
                }
                if detail.trim().is_empty() {
                    Ok(())
                } else {
                    write!(f, ": {}", detail.trim())
                }
            }
            Self::Download(message) => write!(f, "ffmpeg download failed: {message}"),
        }
    }
}

impl std::error::Error for ExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Hound(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ExportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<hound::Error> for ExportError {
    fn from(error: hound::Error) -> Self {
        Self::Hound(error)
    }
}

pub type Result<T> = std::result::Result<T, ExportError>;

/// Render a project into 512-frame stereo blocks.
///
/// Playback is deterministic and starts at tick 0. Song mode uses the full
/// arrangement and ignores the project loop setting; pattern mode renders one
/// pass of the selected pattern. Both modes append the requested release tail.
pub fn render_project<F>(
    project: &StudioProject,
    factory: &dyn PluginFactory,
    options: &RenderOptions,
    mut sink: F,
) -> Result<ExportSummary>
where
    F: FnMut(&[f32], &[f32]) -> Result<()>,
{
    validate_render_options(options)?;
    let sr = options.sample_rate as f32;
    let tail_frames = (options.tail_seconds.max(0.0) as f64 * options.sample_rate as f64)
        .round() as u64;
    let max_frames = (MAX_EXPORT_SECONDS * options.sample_rate as f64) as u64;

    let (schedules, arrangement, playlist_mode, target_ticks) = match options.scope {
        ExportScope::Song => {
            let arrangement = build_arrangement(project);
            if arrangement.length == 0 || arrangement.events.is_empty() {
                return Err(ExportError::NoAudio(
                    "the playlist has no pattern events; add a pattern clip or export a pattern"
                        .into(),
                ));
            }
            if arrangement.length > u32::MAX as u64 {
                return Err(ExportError::Invalid(
                    "arrangement length exceeds the export limit".into(),
                ));
            }
            (Vec::new(), Some(arrangement), true, 0)
        }
        ExportScope::Pattern(pattern_id) => {
            let pattern = project
                .patterns
                .iter()
                .find(|pattern| pattern.id == pattern_id)
                .ok_or_else(|| {
                    ExportError::Invalid(format!("pattern id {pattern_id} does not exist"))
                })?;
            let schedules = build_schedule(std::slice::from_ref(pattern), project.swing);
            let length = schedules.first().map(|schedule| schedule.length).unwrap_or(0);
            if length == 0 || schedules.iter().all(|schedule| schedule.events.is_empty()) {
                return Err(ExportError::NoAudio(format!(
                    "pattern {pattern_id} has no playable events"
                )));
            }
            (schedules, None, false, length)
        }
    };

    let mut config = TransportConfig::default();
    config.bpm = project.bpm.clamp(10.0, 999.0);
    config.time_sig = project.time_sig;
    config.swing = project.swing.clamp(0.0, 0.75);
    config.pattern_index = 0;
    config.playlist_mode = playlist_mode;
    config.loop_enabled = false;
    config.playing = true;

    let graph = EngineGraph::from_project(project, factory, sr);
    let (mut engine, mut handle) = create_engine(sr, graph, schedules, config, true);
    if let Some(arrangement) = arrangement {
        handle.set_arrangement(arrangement);
    }
    handle.set_automation(AutomationSchedule::from_project(project));

    let mut block_l = vec![0.0f32; BLOCK_SIZE];
    let mut block_r = vec![0.0f32; BLOCK_SIZE];
    let mut frames = 0u64;
    let mut peak = 0.0f32;

    if playlist_mode {
        while engine.is_playing() {
            if frames >= max_frames {
                return Err(ExportError::Limit(format!(
                    "song did not finish within {MAX_EXPORT_SECONDS:.0} seconds"
                )));
            }
            render_block(&mut engine, &mut block_l, &mut block_r, &mut peak, &mut sink)?;
            handle.collect_garbage();
            frames = frames.saturating_add(BLOCK_SIZE as u64);
        }
    } else {
        while engine.is_playing()
            && engine.position_ticks() + 1e-9 < target_ticks as f64
        {
            if frames >= max_frames {
                return Err(ExportError::Limit(format!(
                    "pattern did not finish within {MAX_EXPORT_SECONDS:.0} seconds"
                )));
            }
            render_block(&mut engine, &mut block_l, &mut block_r, &mut peak, &mut sink)?;
            handle.collect_garbage();
            frames = frames.saturating_add(BLOCK_SIZE as u64);
        }
        if engine.is_playing() {
            handle.stop();
        }
    }

    // Render the release/delay tail after the transport has stopped. The first
    // tail block also drains a pending Stop command, so all notes enter release
    // on the same control-thread boundary.
    let mut remaining_tail = tail_frames;
    while remaining_tail > 0 {
        let frames_now = (remaining_tail as usize).min(BLOCK_SIZE);
        let (left, right) = engine.render_planar(frames_now);
        for i in 0..frames_now {
            block_l[i] = finite(left[i]);
            block_r[i] = finite(right[i]);
            peak = peak.max(block_l[i].abs()).max(block_r[i].abs());
        }
        sink(&block_l[..frames_now], &block_r[..frames_now])?;
        handle.collect_garbage();
        frames = frames.saturating_add(frames_now as u64);
        remaining_tail -= frames_now as u64;
    }

    Ok(ExportSummary {
        sample_rate: options.sample_rate,
        frames,
        duration_seconds: frames as f64 / options.sample_rate as f64,
        peak,
    })
}

/// Render and write one output file atomically.
pub fn export_file(
    path: impl AsRef<Path>,
    project: &StudioProject,
    factory: &dyn PluginFactory,
    options: &ExportOptions,
    format: ExportFormat,
) -> Result<ExportSummary> {
    let path = path.as_ref();
    let pending = PendingFile::new(path)?;
    let temp = pending.temp_path().to_path_buf();
    let summary = match format {
        ExportFormat::Wav(depth) => wav::write(&temp, project, factory, &options.render, depth),
        ExportFormat::Aiff(depth) => aiff::write(&temp, project, factory, &options.render, depth),
        ExportFormat::Mp3 | ExportFormat::Ogg | ExportFormat::Flac => {
            let executable = options.ffmpeg.resolve()?;
            let output = ffmpeg::FfmpegOutput::Audio(format);
            ffmpeg::encode(&executable, &temp, output, options, project, factory)
        }
        ExportFormat::Mp4 | ExportFormat::Mov | ExportFormat::Avi => {
            let executable = options.ffmpeg.resolve()?;
            let output = ffmpeg::FfmpegOutput::Video(format);
            ffmpeg::encode(&executable, &temp, output, options, project, factory)
        }
    }?;
    pending.commit()?;
    Ok(summary)
}

fn validate_render_options(options: &RenderOptions) -> Result<()> {
    if !(8_000..=192_000).contains(&options.sample_rate) {
        return Err(ExportError::Invalid(format!(
            "sample rate {} is outside 8000..=192000 Hz",
            options.sample_rate
        )));
    }
    if !options.tail_seconds.is_finite() || !(0.0..=30.0).contains(&options.tail_seconds) {
        return Err(ExportError::Invalid(
            "tail must be between 0 and 30 seconds".into(),
        ));
    }
    Ok(())
}

fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

fn render_block<F>(
    engine: &mut EngineRt,
    block_l: &mut [f32],
    block_r: &mut [f32],
    peak: &mut f32,
    sink: &mut F,
) -> Result<()>
where
    F: FnMut(&[f32], &[f32]) -> Result<()>,
{
    let (left, right) = engine.render_planar(BLOCK_SIZE);
    for i in 0..BLOCK_SIZE {
        block_l[i] = finite(left[i]);
        block_r[i] = finite(right[i]);
        *peak = peak.max(block_l[i].abs()).max(block_r[i].abs());
    }
    sink(&block_l[..BLOCK_SIZE], &block_r[..BLOCK_SIZE])
}

/// Compatibility helper retained for callers that expect the original
/// project sample rate (48 kHz) when they did not specify one.
pub fn default_sample_rate() -> u32 {
    SAMPLE_RATE
}

/// A pattern's length in ticks, matching the sequencer schedule, or `None` if
/// the pattern is missing. Useful for UI duration labels.
pub fn pattern_length_ticks(project: &StudioProject, pattern_id: u32) -> Option<u64> {
    project
        .patterns
        .iter()
        .find(|pattern| pattern.id == pattern_id)
        .map(|pattern| pattern.length_steps as u64 * (PPQ as u64 / 4))
}

// ------------------------------------------------------------------- atomic output

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

struct PendingFile {
    temp: PathBuf,
    destination: PathBuf,
    committed: bool,
}

impl PendingFile {
    fn new(destination: &Path) -> Result<Self> {
        if destination.as_os_str().is_empty() {
            return Err(ExportError::Invalid("output path is empty".into()));
        }
        if destination.is_dir() {
            return Err(ExportError::Invalid(format!(
                "{} is a directory",
                destination.display()
            )));
        }
        let parent = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let name = destination
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("export");
        let extension = destination
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("tmp");
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(
            ".{name}.redraw-{}-{id}.{extension}",
            std::process::id()
        ));
        Ok(Self {
            temp,
            destination: destination.to_path_buf(),
            committed: false,
        })
    }

    fn temp_path(&self) -> &Path {
        &self.temp
    }

    fn commit(mut self) -> Result<()> {
        if !self.temp.is_file() {
            return Err(ExportError::Invalid(format!(
                "encoder did not create {}",
                self.temp.display()
            )));
        }
        match fs::rename(&self.temp, &self.destination) {
            Ok(()) => {}
            Err(first_error) => {
                // Windows refuses to replace an existing destination.
                if self.destination.exists()
                    && fs::remove_file(&self.destination).is_ok()
                    && fs::rename(&self.temp, &self.destination).is_ok()
                {
                } else {
                    let _ = fs::remove_file(&self.temp);
                    return Err(ExportError::Io(first_error));
                }
            }
        }
        self.committed = true;
        Ok(())
    }
}

impl Drop for PendingFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.temp);
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use redraw_shared::{ClipKind, ClipState, Note, Step};
    use std::sync::atomic::{AtomicU64, Ordering};

    pub(crate) fn test_project() -> StudioProject {
        let mut project = StudioProject::default();
        project.name = "Export Test".into();
        project.bpm = 120.0;
        project.patterns[0].length_steps = 4;
        let pattern = &mut project.patterns[0];
        let channel = pattern.channel_mut(1, 4);
        channel.steps[0] = Step {
            active: true,
            velocity: 100,
            nudge: 0,
        };
        channel.notes.push(Note {
            id: 1,
            key: 60,
            start: 0,
            length: PPQ as u64,
            velocity: 100,
            pan: 0,
            release: 64,
        });
        project.next_note_id = 2;
        project.playlist.clips.push(ClipState {
            id: 1,
            track: 0,
            start: 0,
            length: PPQ as u64 * 4,
            kind: ClipKind::Pattern,
            source_id: project.patterns[0].id,
            name: "Clip".into(),
            color: [90, 160, 120],
            looping: false,
            gain_db: 0.0,
            transpose: 0,
            stretch: 1.0,
            audio_path: None,
        });
        project.next_clip_id = 2;
        project
    }

    pub(crate) fn test_dir(tag: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-tmp")
            .join("export");
        let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = root.join(format!("{tag}-{}-{serial}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create export test directory");
        dir
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_project;
    use redraw_plugins::BuiltinFactory;

    fn render_vectors(options: RenderOptions) -> (ExportSummary, Vec<f32>, Vec<f32>) {
        let mut left = Vec::new();
        let mut right = Vec::new();
        let summary = render_project(
            &test_project(),
            &BuiltinFactory::new(),
            &options,
            |l, r| {
                left.extend_from_slice(l);
                right.extend_from_slice(r);
                Ok(())
            },
        )
        .unwrap();
        (summary, left, right)
    }

    #[test]
    fn pattern_render_has_frames_signal_and_tail() {
        let options = RenderOptions {
            scope: ExportScope::Pattern(1),
            sample_rate: 48_000,
            tail_seconds: 0.1,
        };
        let (summary, left, right) = render_vectors(options);
        assert!(summary.frames >= 24_000);
        assert_eq!(left.len(), summary.frames as usize);
        assert_eq!(right.len(), summary.frames as usize);
        assert!(summary.peak > 0.0);
        assert!(summary.duration_seconds >= 0.5);
        assert!(left.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn song_render_uses_playlist_extent() {
        let options = RenderOptions {
            scope: ExportScope::Song,
            sample_rate: 24_000,
            tail_seconds: 0.05,
        };
        let (summary, _, _) = render_vectors(options);
        assert!(summary.frames >= 24_000 * 2);
        assert!(summary.duration_seconds >= 2.0);
    }

    #[test]
    fn empty_song_and_bad_pattern_are_errors() {
        let mut project = test_project();
        project.playlist.clips.clear();
        let error = render_project(
            &project,
            &BuiltinFactory::new(),
            &RenderOptions::default(),
            |_, _| Ok(()),
        );
        assert!(matches!(error, Err(ExportError::NoAudio(_))));

        let error = render_project(
            &test_project(),
            &BuiltinFactory::new(),
            &RenderOptions {
                scope: ExportScope::Pattern(999),
                ..Default::default()
            },
            |_, _| Ok(()),
        );
        assert!(matches!(error, Err(ExportError::Invalid(_))));
    }

    #[test]
    fn render_options_are_validated() {
        let factory = BuiltinFactory::new();
        let project = test_project();
        for options in [
            RenderOptions {
                sample_rate: 100,
                ..Default::default()
            },
            RenderOptions {
                tail_seconds: -1.0,
                ..Default::default()
            },
        ] {
            let error = render_project(&project, &factory, &options, |_, _| Ok(()));
            assert!(matches!(error, Err(ExportError::Invalid(_))));
        }
    }

    #[test]
    fn pattern_length_helper_matches_step_length() {
        let project = test_project();
        assert_eq!(pattern_length_ticks(&project, 1), Some(4 * 240));
        assert_eq!(pattern_length_ticks(&project, 99), None);
        assert_eq!(default_sample_rate(), 48_000);
    }

    #[test]
    fn export_extension_and_video_flags_are_stable() {
        assert_eq!(ExportFormat::Wav(WavBitDepth::Pcm16).extension(), "wav");
        assert_eq!(ExportFormat::Aiff(AiffBitDepth::Pcm24).extension(), "aiff");
        assert!(ExportFormat::Mp4.is_video());
        assert!(!ExportFormat::Flac.is_video());
        assert_eq!(ExportFormat::default(), ExportFormat::Wav(WavBitDepth::Pcm24));
    }

    #[test]
    fn default_channel_and_playlist_types_are_available() {
        use redraw_shared::{ChannelInfo, Pattern, PlaylistState};
        let _ = ChannelInfo::new(1, "Test", "Aurora", [1, 2, 3]);
        let playlist = PlaylistState::default();
        assert_eq!(playlist.tracks.len(), 8);
        let pattern = Pattern::new(1, "P", [1, 2, 3], 4);
        assert_eq!(pattern.length_steps, 4);
    }
}
