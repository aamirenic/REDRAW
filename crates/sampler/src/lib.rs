//! REDRAW sample loading, slicing and multisample programs.
//!
//! Everything here is engine-rate agnostic: a [`Sample`] keeps its source rate
//! and the caller converts with [`Sample::resampled`] or folds the rate into
//! [`playback_ratio`]. Playback ([`Player`]) is allocation-free and safe to run
//! inside the audio callback; loading, slicing and resampling allocate and must
//! happen on control threads. 8-bit WAV is rejected on purpose; 16/24/32-bit
//! integer and 32-bit float WAV are supported (D-016: user-supplied content
//! only, no bundled samples).
#![forbid(unsafe_code)]

pub use redraw_shared as shared;

use std::fmt;
use std::path::Path;

pub const CRATE_PHASE: u8 = 7;

// ---------------------------------------------------------------------- errors

#[derive(Debug)]
pub enum SampleError {
    Io(std::io::Error),
    Wav(hound::Error),
    Unsupported(String),
}

impl fmt::Display for SampleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SampleError::Io(e) => write!(f, "I/O error: {e}"),
            SampleError::Wav(e) => write!(f, "WAV error: {e}"),
            SampleError::Unsupported(msg) => write!(f, "unsupported sample: {msg}"),
        }
    }
}

impl std::error::Error for SampleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SampleError::Io(e) => Some(e),
            SampleError::Wav(e) => Some(e),
            SampleError::Unsupported(_) => None,
        }
    }
}

impl From<std::io::Error> for SampleError {
    fn from(value: std::io::Error) -> Self {
        SampleError::Io(value)
    }
}

impl From<hound::Error> for SampleError {
    fn from(value: hound::Error) -> Self {
        SampleError::Wav(value)
    }
}

// ---------------------------------------------------------------------- sample

/// Decoded audio, interleaved f32 in `-1..=1`. Mono (`channels == 1`) or
/// stereo (`channels == 2`); anything else is rejected at load time.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub name: String,
    pub sample_rate: u32,
    pub channels: u8,
    /// Interleaved frames: mono `[l]`, stereo `[l, r]`.
    pub data: Vec<f32>,
}

impl Sample {
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels.max(1) as usize
    }

    pub fn duration(&self) -> f32 {
        self.frames() as f32 / self.sample_rate.max(1) as f32
    }

    pub fn peak(&self) -> f32 {
        self.data.iter().fold(0.0f32, |a, b| a.max(b.abs()))
    }

    pub fn silence(frames: usize, channels: u8, sample_rate: u32) -> Self {
        let ch = if channels >= 2 { 2u8 } else { 1u8 };
        Self {
            name: "silence".into(),
            sample_rate: sample_rate.max(1),
            channels: ch,
            data: vec![0.0; frames * ch as usize],
        }
    }

    /// Interpolatable stereo view of one frame; out-of-range reads are silent.
    #[inline]
    pub fn frame(&self, index: usize) -> (f32, f32) {
        if self.channels >= 2 {
            let i = index * 2;
            match (self.data.get(i), self.data.get(i + 1)) {
                (Some(l), Some(r)) => (*l, *r),
                _ => (0.0, 0.0),
            }
        } else {
            match self.data.get(index) {
                Some(v) => (*v, *v),
                None => (0.0, 0.0),
            }
        }
    }

    /// Linear-interpolation resample; allocates, so control thread only.
    pub fn resampled(&self, target_rate: u32) -> Sample {
        let target = target_rate.max(1);
        if target == self.sample_rate || self.frames() == 0 {
            return self.clone();
        }
        let ratio = target as f64 / self.sample_rate.max(1) as f64;
        let frames = ((self.frames() as f64) * ratio).round().max(1.0) as usize;
        let last = self.frames() - 1;
        let mut data = Vec::with_capacity(frames * self.channels.max(1) as usize);
        for i in 0..frames {
            let src = (i as f64 / ratio).min(last as f64);
            let i0 = src.floor() as usize;
            let i1 = (i0 + 1).min(last);
            let frac = (src - i0 as f64) as f32;
            let (l0, r0) = self.frame(i0);
            let (l1, r1) = self.frame(i1);
            let l = l0 + (l1 - l0) * frac;
            let r = r0 + (r1 - r0) * frac;
            if self.channels >= 2 {
                data.push(l);
                data.push(r);
            } else {
                data.push(l);
            }
        }
        Sample {
            name: self.name.clone(),
            sample_rate: target,
            channels: self.channels,
            data,
        }
    }
}

/// Output bit depth for [`save_wav`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitDepth {
    Int16,
    Float32,
}

/// Load a WAV file into an interleaved [`Sample`].
pub fn load_wav(path: impl AsRef<Path>) -> Result<Sample, SampleError> {
    let path = path.as_ref();
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.channels > 2 {
        return Err(SampleError::Unsupported(format!(
            "{} channels (only mono/stereo)",
            spec.channels
        )));
    }
    let mut data: Vec<f32> = Vec::with_capacity(reader.len() as usize);
    match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Int, 16) => {
            let scale = 1.0f32 / 32_768.0;
            for s in reader.samples::<i16>() {
                data.push(s? as f32 * scale);
            }
        }
        (hound::SampleFormat::Int, 24) => {
            let scale = 1.0f32 / 8_388_608.0;
            for s in reader.samples::<i32>() {
                data.push(s? as f32 * scale);
            }
        }
        (hound::SampleFormat::Int, 32) => {
            let scale = 1.0f32 / 2_147_483_648.0;
            for s in reader.samples::<i32>() {
                data.push(s? as f32 * scale);
            }
        }
        (hound::SampleFormat::Float, 32) => {
            for s in reader.samples::<f32>() {
                data.push(s?);
            }
        }
        (fmt, bits) => {
            return Err(SampleError::Unsupported(format!(
                "{bits}-bit {fmt:?} WAV"
            )))
        }
    }
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("sample")
        .to_string();
    Ok(Sample {
        name,
        sample_rate: spec.sample_rate.max(1),
        channels: spec.channels as u8,
        data,
    })
}

/// Write a [`Sample`] as 16-bit integer or 32-bit float WAV.
pub fn save_wav(
    path: impl AsRef<Path>,
    sample: &Sample,
    depth: BitDepth,
) -> Result<(), SampleError> {
    let spec = hound::WavSpec {
        channels: sample.channels.clamp(1, 2) as u16,
        sample_rate: sample.sample_rate.max(1),
        bits_per_sample: if depth == BitDepth::Int16 { 16 } else { 32 },
        sample_format: if depth == BitDepth::Int16 {
            hound::SampleFormat::Int
        } else {
            hound::SampleFormat::Float
        },
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for v in &sample.data {
        match depth {
            BitDepth::Int16 => {
                writer.write_sample((v.clamp(-1.0, 1.0) * 32_767.0).round() as i16)?;
            }
            BitDepth::Float32 => writer.write_sample(*v)?,
        }
    }
    writer.finalize()?;
    Ok(())
}

/// Original synthesized demo sample (four inharmonic decaying partials), used
/// when no user content is installed. Never derived from third-party material.
pub fn demo_pluck(sample_rate: u32) -> Sample {
    let rate = sample_rate.max(8_000);
    let sr = rate as f32;
    let frames = (sr * 1.2) as usize;
    let base = 220.0f32;
    let partials = [
        (1.0f32, 0.55f32, 2.2f32),
        (2.01, 0.25, 3.4),
        (3.03, 0.14, 5.1),
        (4.97, 0.08, 7.9),
    ];
    let mut data = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let t = i as f32 / sr;
        let mut l = 0.0f32;
        let mut r = 0.0f32;
        for (ratio, amp, decay) in partials {
            let env = (-t * decay).exp();
            let phase = std::f32::consts::TAU * base * ratio * t;
            l += phase.sin() * amp * env;
            r += (phase + 0.15 * ratio).sin() * amp * env;
        }
        let attack = (t * 200.0).min(1.0);
        let release = (((frames - i) as f32) / (sr * 0.05)).min(1.0);
        let gain = 0.5 * attack * release;
        data.push(l * gain);
        data.push(r * gain);
    }
    Sample {
        name: "pluck".into(),
        sample_rate: rate,
        channels: 2,
        data,
    }
}

// ---------------------------------------------------------------------- slices

/// Slice boundaries in frames; each boundary starts a slice that runs to the
/// next boundary (or the end of the sample).
#[derive(Clone, Debug, PartialEq)]
pub struct SliceMap {
    pub boundaries: Vec<usize>,
}

impl SliceMap {
    /// Evenly spaced slices; the first boundary is always frame 0.
    pub fn fixed(frames: usize, slices: usize) -> Self {
        let frames = frames.max(1);
        let n = slices.clamp(1, 128).min(frames);
        let mut boundaries = Vec::with_capacity(n);
        for k in 0..n {
            boundaries.push(k * frames / n);
        }
        Self { boundaries }
    }

    /// Transient detection over a 128-frame energy envelope. An onset is a hop
    /// whose energy exceeds both an absolute fraction of the loudest hop and
    /// twice the previous hop, with at least 30 ms between slices.
    ///
    /// `sensitivity` 0..1 lowers the absolute gate (higher = more slices);
    /// `max_slices` caps the result (16 for ChopLab, up to 128 for SliceLab).
    pub fn detect(sample: &Sample, sensitivity: f32, max_slices: usize) -> Self {
        let frames = sample.frames();
        let max_slices = max_slices.clamp(1, 128);
        if frames < 1024 {
            return Self::fixed(frames, 1);
        }
        let hop = 128usize;
        let hops = frames / hop;
        let mut energy = Vec::with_capacity(hops);
        for h in 0..hops {
            let start = h * hop;
            let end = (start + hop).min(frames);
            let mut sum = 0.0f32;
            for i in start..end {
                let (l, r) = sample.frame(i);
                sum += l * l + r * r;
            }
            energy.push((sum / ((end - start).max(1) as f32 * 2.0)).sqrt());
        }
        let peak = energy.iter().fold(0.0f32, |a, b| a.max(*b)).max(1e-9);
        let sens = sensitivity.clamp(0.0, 1.0);
        let threshold = peak * (0.03 - 0.025 * sens);
        let min_gap = (((0.030 * sample.sample_rate.max(1) as f32) / hop as f32).ceil() as usize)
            .max(2);
        let mut boundaries = vec![0usize];
        let mut last_hop = 0usize;
        for h in 1..hops {
            if boundaries.len() >= max_slices {
                break;
            }
            let e = energy[h];
            let prev = energy[h - 1].max(1e-9);
            if e > threshold && e > prev * 2.0 && h - last_hop >= min_gap {
                boundaries.push(h * hop);
                last_hop = h;
            }
        }
        Self { boundaries }
    }

    pub fn len(&self) -> usize {
        self.boundaries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.boundaries.is_empty()
    }

    /// Frame range of slice `index`, clamped to the sample length.
    pub fn range(&self, index: usize, frames: usize) -> Option<(usize, usize)> {
        let start = *self.boundaries.get(index)?;
        let end = self.boundaries.get(index + 1).copied().unwrap_or(frames);
        let end = end.min(frames);
        if end > start {
            Some((start, end))
        } else {
            None
        }
    }

    /// Map a MIDI key to `(slice index, frame range)`, with `base` playing
    /// slice 0. Slices above key 127 are ignored.
    pub fn slice_for_key(
        &self,
        key: u8,
        base: u8,
        frames: usize,
    ) -> Option<(usize, (usize, usize))> {
        let index = key.checked_sub(base)? as usize;
        if index >= self.len() {
            return None;
        }
        self.range(index, frames).map(|r| (index, r))
    }
}

// ------------------------------------------------------------ multisample data

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopMode {
    Off,
    Forward,
    PingPong,
}

/// One key/velocity layer backed by a sample and a trimmed, loopable region.
#[derive(Clone, Debug)]
pub struct Zone {
    /// Index into [`Program::samples`].
    pub sample: usize,
    pub lo_key: u8,
    pub hi_key: u8,
    pub lo_vel: u8,
    pub hi_vel: u8,
    pub root_key: u8,
    pub gain_db: f32,
    pub tune_cents: f32,
    pub start_frac: f32,
    pub end_frac: f32,
    pub loop_mode: LoopMode,
    pub reverse: bool,
}

impl Zone {
    pub fn new(sample: usize, lo_key: u8, hi_key: u8, root_key: u8) -> Self {
        Self {
            sample,
            lo_key,
            hi_key,
            lo_vel: 1,
            hi_vel: 127,
            root_key,
            gain_db: 0.0,
            tune_cents: 0.0,
            start_frac: 0.0,
            end_frac: 1.0,
            loop_mode: LoopMode::Off,
            reverse: false,
        }
    }

    pub fn with_velocity(mut self, lo: u8, hi: u8) -> Self {
        self.lo_vel = lo;
        self.hi_vel = hi;
        self
    }

    pub fn with_range(mut self, start: f32, end: f32) -> Self {
        self.start_frac = start;
        self.end_frac = end;
        self
    }

    pub fn with_loop(mut self, mode: LoopMode, reverse: bool) -> Self {
        self.loop_mode = mode;
        self.reverse = reverse;
        self
    }

    pub fn contains(&self, key: u8, velocity: u8) -> bool {
        key >= self.lo_key
            && key <= self.hi_key
            && velocity >= self.lo_vel
            && velocity <= self.hi_vel
    }

    /// Linear gain from the layer trim.
    pub fn gain(&self) -> f32 {
        10f32.powf(self.gain_db / 20.0)
    }
}

/// A multisample instrument: samples plus key/velocity zones. Zone order is
/// priority order: the first matching zone wins, so put specific layers first.
#[derive(Clone, Debug, Default)]
pub struct Program {
    pub name: String,
    pub samples: Vec<Sample>,
    pub zones: Vec<Zone>,
}

impl Program {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    pub fn add_sample(&mut self, sample: Sample) -> usize {
        self.samples.push(sample);
        self.samples.len() - 1
    }

    pub fn add_zone(&mut self, zone: Zone) {
        self.zones.push(zone);
    }

    pub fn zone_for(&self, key: u8, velocity: u8) -> Option<&Zone> {
        self.zones.iter().find(|z| z.contains(key, velocity))
    }

    pub fn sample_for(&self, zone: &Zone) -> Option<&Sample> {
        self.samples.get(zone.sample)
    }
}

/// Build a one-shot slice program: slice `n` plays from key `base + n`, each
/// zone referencing the trimmed frame range. Mirrors the ChopLab/SliceLab key
/// mapping while keeping original DSP.
pub fn slice_program(
    name: impl Into<String>,
    sample: Sample,
    map: &SliceMap,
    base_key: u8,
) -> Program {
    let mut program = Program::new(name);
    let frames = sample.frames().max(1);
    program.add_sample(sample);
    for index in 0..map.len() {
        if let Some((start, end)) = map.range(index, frames) {
            let key = (base_key as u16 + index as u16).min(127u16) as u8;
            let zone = Zone::new(0, key, key, key).with_range(
                start as f32 / frames as f32,
                end as f32 / frames as f32,
            );
            program.add_zone(zone);
        }
    }
    program
}

/// Playback ratio for a note versus zone root, including source-rate conversion
/// into the engine rate (48 kHz in REDRAW).
pub fn playback_ratio(note: f32, root_note: f32, sample_rate: u32, engine_rate: f32) -> f64 {
    let semitones = (note - root_note) as f64 / 12.0;
    2f64.powf(semitones) * sample_rate.max(1) as f64 / engine_rate.max(1.0) as f64
}

// ---------------------------------------------------------------------- player

/// One allocation-free sample playhead with fractional read position, linear
/// interpolation, reverse playback, forward/ping-pong loops and one-shot stop.
#[derive(Clone, Copy, Debug)]
pub struct Player {
    pos: f64,
    start: f64,
    end: f64,
    rate: f64,
    dir: f64,
    mode: LoopMode,
    active: bool,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            pos: 0.0,
            start: 0.0,
            end: 0.0,
            rate: 1.0,
            dir: 1.0,
            mode: LoopMode::Off,
            active: false,
        }
    }
}

impl Player {
    pub fn new() -> Self {
        Self::default()
    }

    /// Arm the playhead. `start_frac`/`end_frac` select the region; `rate` is in
    /// samples per output frame (see [`playback_ratio`]).
    pub fn trigger(
        &mut self,
        frames: usize,
        start_frac: f32,
        end_frac: f32,
        rate: f64,
        reverse: bool,
        mode: LoopMode,
    ) {
        let total = frames.max(1) as f64;
        let start = ((start_frac.clamp(0.0, 1.0) as f64) * total).floor();
        let start = start.min(total - 1.0);
        let end = ((end_frac.clamp(0.0, 1.0) as f64) * total)
            .floor()
            .max(start + 1.0)
            .min(total);
        self.start = start;
        self.end = end;
        self.rate = rate.abs().max(1e-4);
        self.dir = if reverse { -1.0 } else { 1.0 };
        self.pos = if reverse { end - 1.0 } else { start };
        self.mode = mode;
        self.active = frames > 1;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn pos(&self) -> f64 {
        self.pos
    }

    pub fn mode(&self) -> LoopMode {
        self.mode
    }

    pub fn stop(&mut self) {
        self.active = false;
    }

    pub fn set_rate(&mut self, rate: f64) {
        self.rate = rate.abs().max(1e-4);
    }

    /// Next interpolated stereo frame, or `None` once a one-shot finishes.
    #[inline]
    pub fn next(&mut self, sample: &Sample) -> Option<(f32, f32)> {
        if !self.active {
            return None;
        }
        let frames = sample.frames();
        if frames == 0 {
            self.active = false;
            return None;
        }
        let frame_limit = (self.end.ceil() as usize).min(frames);
        let floor = self.pos.floor();
        let mut i0 = (floor.max(0.0) as usize).min(frames - 1);
        i0 = i0.max((self.start.floor() as usize).min(frames - 1));
        let i1 = (i0 + 1)
            .min(frame_limit.saturating_sub(1))
            .min(frames - 1);
        let frac = (self.pos - floor) as f32;
        let (l0, r0) = sample.frame(i0);
        let (l1, r1) = sample.frame(i1);
        let out = (l0 + (l1 - l0) * frac, r0 + (r1 - r0) * frac);
        self.advance();
        Some(out)
    }

    fn advance(&mut self) {
        let len = self.end - self.start;
        self.pos += self.dir * self.rate;
        match self.mode {
            LoopMode::Off => {
                if self.pos >= self.end || self.pos < self.start {
                    self.active = false;
                }
            }
            LoopMode::Forward => {
                if self.dir > 0.0 {
                    if self.pos >= self.end {
                        self.pos = self.start + (self.pos - self.end) % len;
                    }
                } else if self.pos < self.start {
                    self.pos = self.end - 1.0 - (self.start - self.pos) % len;
                }
            }
            LoopMode::PingPong => {
                if self.pos >= self.end {
                    self.pos = self.end - 1.0 - (self.pos - self.end);
                    self.dir = -1.0;
                }
                if self.pos < self.start {
                    self.pos = self.start + (self.start - self.pos);
                    self.dir = 1.0;
                }
                self.pos = self.pos.clamp(self.start, self.end - 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/test-tmp");
        std::fs::create_dir_all(&dir).expect("create test dir");
        dir.join(name)
    }

    fn ramp_sample(frames: usize) -> Sample {
        let mut data = Vec::with_capacity(frames);
        for i in 0..frames {
            data.push(i as f32 / frames.max(1) as f32);
        }
        Sample {
            name: "ramp".into(),
            sample_rate: 48_000,
            channels: 1,
            data,
        }
    }

    fn click_train() -> Sample {
        let frames = 24_000usize;
        let mut data = vec![0.0f32; frames * 2];
        for hit in 0..4 {
            let start = 2400 + hit * 4800;
            for i in 0..300usize {
                let idx = start + i;
                if idx * 2 + 1 < data.len() {
                    let v = (i as f32 * 0.3).sin() * (1.0 - i as f32 / 300.0) * 0.8;
                    data[idx * 2] = v;
                    data[idx * 2 + 1] = v;
                }
            }
        }
        Sample {
            name: "clicks".into(),
            sample_rate: 48_000,
            channels: 2,
            data,
        }
    }

    #[test]
    fn wav_round_trip() {
        let frames = 480usize;
        let mut data = vec![0.0f32; frames * 2];
        for i in 0..frames {
            data[i * 2] = (i as f32 / frames as f32) - 0.5;
            data[i * 2 + 1] = 0.25 * ((i % 17) as f32 - 8.0) / 8.0;
        }
        let sample = Sample {
            name: "roundtrip".into(),
            sample_rate: 44_100,
            channels: 2,
            data: data.clone(),
        };
        let path = tmp_path("sampler_roundtrip.wav");
        save_wav(&path, &sample, BitDepth::Int16).expect("save int16");
        let loaded = load_wav(&path).expect("load int16");
        assert_eq!(loaded.sample_rate, 44_100);
        assert_eq!(loaded.channels, 2);
        assert_eq!(loaded.frames(), frames);
        for (a, b) in sample.data.iter().zip(loaded.data.iter()) {
            assert!((a - b).abs() <= 1e-4, "int16 round trip: {a} vs {b}");
        }

        let float_path = tmp_path("sampler_roundtrip_f32.wav");
        save_wav(&float_path, &sample, BitDepth::Float32).expect("save f32");
        let loaded = load_wav(&float_path).expect("load f32");
        assert_eq!(loaded.data, sample.data);
        assert_eq!(loaded.name, "sampler_roundtrip_f32");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&float_path);
    }

    #[test]
    fn demo_pluck_is_original_and_finite() {
        let sample = demo_pluck(48_000);
        assert_eq!(sample.channels, 2);
        assert_eq!(sample.sample_rate, 48_000);
        assert!(sample.frames() >= 48_000);
        let peak = sample.peak();
        assert!(peak > 0.1 && peak <= 0.6, "demo peak {peak}");
        assert!(sample.data.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn slice_detects_click_train() {
        let sample = click_train();
        let map = SliceMap::detect(&sample, 0.8, 16);
        let expected = [0usize, 2400, 7200, 12_000, 16_800];
        assert!(
            map.len() >= expected.len(),
            "detected only {} slices: {:?}",
            map.len(),
            map.boundaries
        );
        for target in expected {
            assert!(
                map.boundaries
                    .iter()
                    .any(|b| (*b as i64 - target as i64).abs() <= 512),
                "no boundary near {target}: {:?}",
                map.boundaries
            );
        }
        for index in 0..map.len() {
            assert!(map.range(index, sample.frames()).is_some());
        }
    }

    #[test]
    fn fixed_slices_cover_sample() {
        let frames = 10_000usize;
        let map = SliceMap::fixed(frames, 8);
        assert_eq!(map.len(), 8);
        assert_eq!(map.boundaries[0], 0);
        for (i, b) in map.boundaries.iter().enumerate() {
            assert_eq!(*b, i * frames / 8);
        }
        for i in 0..8 {
            let (start, end) = map.range(i, frames).expect("slice range");
            assert!(end > start);
            let next_start = map
                .range(i + 1, frames)
                .map(|r| r.0)
                .unwrap_or(frames);
            assert_eq!(end, next_start);
        }
        assert!(map.range(8, frames).is_none());
        let (index, range) = map.slice_for_key(60, 60, frames).expect("key 60");
        assert_eq!(index, 0);
        assert_eq!(range, (0, 1250));
        assert!(map.slice_for_key(59, 60, frames).is_none());
        assert!(map.slice_for_key(68, 60, frames).is_none());
    }

    #[test]
    fn forward_loop_wraps_without_stopping() {
        let sample = ramp_sample(100);
        let mut player = Player::new();
        player.trigger(100, 0.0, 1.0, 0.5, false, LoopMode::Forward);
        let mut produced = 0;
        for _ in 0..600 {
            if let Some((l, _)) = player.next(&sample) {
                assert!(l.is_finite());
                assert!((0.0..1.0).contains(&l), "looped value out of range: {l}");
                produced += 1;
            }
        }
        assert_eq!(produced, 600);
        assert!(player.is_active());
    }

    #[test]
    fn one_shot_length_matches_region() {
        let sample = ramp_sample(100);
        let mut player = Player::new();
        player.trigger(100, 0.0, 1.0, 1.0, false, LoopMode::Off);
        let mut produced = 0;
        for _ in 0..1000 {
            if player.next(&sample).is_none() {
                break;
            }
            produced += 1;
        }
        assert_eq!(produced, 100);
        assert!(!player.is_active());
    }

    #[test]
    fn reverse_plays_backwards() {
        let sample = ramp_sample(100);
        let mut player = Player::new();
        player.trigger(100, 0.0, 1.0, 1.0, true, LoopMode::Off);
        let first = player.next(&sample).expect("first").0;
        let second = player.next(&sample).expect("second").0;
        let third = player.next(&sample).expect("third").0;
        assert!((first - 0.99).abs() < 0.02, "reverse start {first}");
        assert!(first > second && second > third);
    }

    #[test]
    fn resample_preserves_duration() {
        let sample = ramp_sample(100);
        let up = sample.resampled(96_000);
        assert_eq!(up.frames(), 200);
        assert_eq!(up.sample_rate, 96_000);
        assert!((up.frame(100).0 - 0.5).abs() < 0.01);
        assert!((up.duration() - sample.duration()).abs() < 1e-4);
        let down = sample.resampled(24_000);
        assert_eq!(down.frames(), 50);
        assert_eq!(down.sample_rate, 24_000);
    }

    #[test]
    fn program_selects_key_and_velocity_zones() {
        let mut program = Program::new("multisample");
        let low = program.add_sample(ramp_sample(64));
        let high = program.add_sample(ramp_sample(128));
        program.add_zone(Zone::new(low, 0, 59, 48).with_velocity(1, 63));
        program.add_zone(Zone::new(high, 0, 59, 48).with_velocity(64, 127));
        program.add_zone(Zone::new(high, 60, 127, 72));
        assert_eq!(program.zone_for(40, 30).expect("soft low").sample, low);
        assert_eq!(program.zone_for(40, 100).expect("hard low").sample, high);
        assert_eq!(program.zone_for(80, 30).expect("high key").sample, high);
        assert!(program.zone_for(40, 0).is_none());
        assert_eq!(
            program.sample_for(program.zone_for(80, 30).unwrap()).unwrap().frames(),
            128
        );
    }

    #[test]
    fn slice_program_maps_trimmed_zones() {
        let sample = ramp_sample(1000);
        let map = SliceMap::fixed(1000, 4);
        let program = slice_program("chopped", sample, &map, 60);
        assert_eq!(program.samples.len(), 1);
        assert_eq!(program.zones.len(), 4);
        let zone = program.zone_for(61, 100).expect("slice 1");
        assert_eq!(zone.root_key, 61);
        assert!((zone.start_frac - 0.25).abs() < 1e-6);
        assert!((zone.end_frac - 0.5).abs() < 1e-6);
        assert!(program.zone_for(64, 100).is_none());
    }

    #[test]
    fn playback_ratio_matches_octave_and_rate() {
        let octave_up = playback_ratio(72.0, 60.0, 48_000, 48_000.0);
        assert!((octave_up - 2.0).abs() < 1e-9);
        let doubled_rate = playback_ratio(60.0, 60.0, 96_000, 48_000.0);
        assert!((doubled_rate - 2.0).abs() < 1e-9);
    }
}
