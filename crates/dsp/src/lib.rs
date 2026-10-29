//! REDRAW DSP toolkit: allocation-free primitives used by instruments and effects.
//! Everything here is written from scratch from standard public DSP technique.
#![forbid(unsafe_code)]

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

pub const TWO_PI: f32 = std::f32::consts::TAU;

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
pub fn wrap01(x: f32) -> f32 {
    x - x.floor()
}

#[inline]
pub fn flush_denorm(x: f32) -> f32 {
    if x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}

#[inline]
pub fn soft_clip(x: f32) -> f32 {
    x / (1.0 + x.abs())
}

#[inline]
pub fn tanh_dist(x: f32, drive: f32) -> f32 {
    (x * drive).tanh()
}

#[inline]
pub fn foldback(x: f32) -> f32 {
    let mut v = x;
    for _ in 0..4 {
        if v > 1.0 {
            v = 2.0 - v;
        } else if v < -1.0 {
            v = -2.0 - v;
        } else {
            break;
        }
    }
    v.clamp(-1.0, 1.0)
}

// ------------------------------------------------------------------ rng

#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 2],
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self {
            s: [seed.wrapping_mul(0x9E3779B97F4A7C15) | 1, 0xBF58476D1CE4E5B9],
        }
    }
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let s1 = self.s[0];
        let s0 = self.s[1];
        self.s[0] = s0;
        let mut x = s1.wrapping_add(s0);
        x ^= x << 23;
        self.s[1] = x ^ (s0 >> 17) ^ (x >> 26);
        self.s[0] ^ self.s[1]
    }
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
    }
    #[inline]
    pub fn unipolar(&mut self) -> f32 {
        (self.next_f32() + 1.0) * 0.5
    }
}

// ------------------------------------------------------------------ oscillator

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OscKind {
    Sine,
    Saw,
    Square,
    Triangle,
    Pulse,
    Noise,
}

impl OscKind {
    pub fn from_index(i: usize) -> Self {
        match i % 6 {
            0 => OscKind::Sine,
            1 => OscKind::Saw,
            2 => OscKind::Square,
            3 => OscKind::Triangle,
            4 => OscKind::Pulse,
            _ => OscKind::Noise,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            OscKind::Sine => "Sine",
            OscKind::Saw => "Saw",
            OscKind::Square => "Square",
            OscKind::Triangle => "Triangle",
            OscKind::Pulse => "Pulse",
            OscKind::Noise => "Noise",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Osc {
    pub kind: OscKind,
    pub phase: f32,
    pub pulse_width: f32,
    dt: f32,
    rng: Rng,
}

impl Osc {
    pub fn new(kind: OscKind, seed: u64) -> Self {
        Self {
            kind,
            phase: 0.0,
            pulse_width: 0.5,
            dt: 0.0,
            rng: Rng::new(seed),
        }
    }
    #[inline]
    pub fn set_freq(&mut self, freq: f32, sr: f32) {
        self.dt = (freq / sr).clamp(0.0, 0.45);
    }
    #[inline]
    fn blep(t: f32, dt: f32) -> f32 {
        if t < dt {
            let t = t / dt;
            t + t - t * t - 1.0
        } else if t > 1.0 - dt {
            let t = (t - 1.0) / dt;
            t * t + t + t + 1.0
        } else {
            0.0
        }
    }
    #[inline]
    pub fn next(&mut self) -> f32 {
        let t = self.phase;
        let dt = self.dt;
        let v = match self.kind {
            OscKind::Sine => (TWO_PI * t).sin(),
            OscKind::Saw => 2.0 * t - 1.0 - Self::blep(t, dt),
            OscKind::Square => {
                let mut v = if t < 0.5 { 1.0 } else { -1.0 };
                v += Self::blep(t, dt);
                v -= Self::blep(wrap01(t + 0.5), dt);
                v
            }
            OscKind::Triangle => 1.0 - 4.0 * (t - 0.5).abs(),
            OscKind::Pulse => {
                let pw = self.pulse_width.clamp(0.02, 0.98);
                let mut v = if t < pw { 1.0 } else { -1.0 };
                v += Self::blep(t, dt);
                v -= Self::blep(wrap01(t + (1.0 - pw)), dt);
                v
            }
            OscKind::Noise => self.rng.next_f32(),
        };
        self.phase = wrap01(t + dt);
        v
    }
    pub fn reset_phase(&mut self, phase: f32) {
        self.phase = wrap01(phase);
    }
}

// ------------------------------------------------------------------ wavetable

#[derive(Clone, Debug)]
pub struct Wavetable {
    pub table: Vec<f32>,
    phase: f32,
    dt: f32,
}

impl Wavetable {
    /// Build a band-limited table by additive synthesis (1/n rolloff up to max harmonics).
    pub fn from_shape(name: &str, harmonics: usize) -> Self {
        let size = 2048usize;
        let mut table = vec![0.0f32; size];
        for h in 1..=harmonics.max(1) {
            let amp = match name {
                "saw" => 1.0 / h as f32,
                "square" => {
                    if h % 2 == 0 {
                        0.0
                    } else {
                        1.0 / h as f32
                    }
                }
                "organ" => {
                    if [1, 2, 4, 8].contains(&h) {
                        1.0 / h as f32
                    } else {
                        0.0
                    }
                }
                "odd" => {
                    if h % 2 == 1 {
                        1.0 / (h * h) as f32
                    } else {
                        0.0
                    }
                }
                _ => 1.0 / (h * h) as f32, // "sine"-ish warm
            };
            for i in 0..size {
                let ph = i as f32 / size as f32;
                table[i] += amp * (TWO_PI * ph * h as f32).sin();
            }
        }
        let peak = table.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
        for v in &mut table {
            *v /= peak;
        }
        Self {
            table,
            phase: 0.0,
            dt: 0.0,
        }
    }
    pub fn sine_table() -> Self {
        let size = 2048;
        let table = (0..size)
            .map(|i| (TWO_PI * i as f32 / size as f32).sin())
            .collect();
        Self { table, phase: 0.0, dt: 0.0 }
    }
    #[inline]
    pub fn set_freq(&mut self, freq: f32, sr: f32) {
        self.dt = (freq / sr).clamp(0.0, 0.45);
    }
    #[inline]
    pub fn next(&mut self) -> f32 {
        let size = self.table.len();
        let pos = self.phase * size as f32;
        let i0 = pos as usize % size;
        let i1 = (i0 + 1) % size;
        let frac = pos - pos.floor();
        let v = lerp(self.table[i0], self.table[i1], frac);
        self.phase = wrap01(self.phase + self.dt);
        v
    }
    pub fn set_phase(&mut self, p: f32) {
        self.phase = wrap01(p);
    }
}

// ------------------------------------------------------------------ filters

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    LowPass,
    HighPass,
    BandPass,
    Notch,
    Peak,
    LowShelf,
    HighShelf,
}

impl FilterKind {
    pub fn from_index(i: usize) -> Self {
        match i % 7 {
            0 => FilterKind::LowPass,
            1 => FilterKind::HighPass,
            2 => FilterKind::BandPass,
            3 => FilterKind::Notch,
            4 => FilterKind::Peak,
            5 => FilterKind::LowShelf,
            _ => FilterKind::HighShelf,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            FilterKind::LowPass => "LP",
            FilterKind::HighPass => "HP",
            FilterKind::BandPass => "BP",
            FilterKind::Notch => "Notch",
            FilterKind::Peak => "Peak",
            FilterKind::LowShelf => "LowShelf",
            FilterKind::HighShelf => "HighShelf",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BiquadCoeffs {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl BiquadCoeffs {
    pub fn compute(kind: FilterKind, freq: f32, q: f32, gain_db: f32, sr: f32) -> Self {
        let f = freq.clamp(10.0, sr * 0.49);
        let w0 = TWO_PI * f / sr;
        let (sin_w, cos_w) = w0.sin_cos();
        let q = q.max(0.05);
        let alpha = sin_w / (2.0 * q);
        let a = 10f32.powf(gain_db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            FilterKind::LowPass => {
                let b0 = (1.0 - cos_w) / 2.0;
                (b0, 1.0 - cos_w, b0, 1.0 + alpha, -2.0 * cos_w, 1.0 - alpha)
            }
            FilterKind::HighPass => {
                let b0 = (1.0 + cos_w) / 2.0;
                (b0, -(1.0 + cos_w), b0, 1.0 + alpha, -2.0 * cos_w, 1.0 - alpha)
            }
            FilterKind::BandPass => (
                alpha,
                0.0,
                -alpha,
                1.0 + alpha,
                -2.0 * cos_w,
                1.0 - alpha,
            ),
            FilterKind::Notch => (
                1.0,
                -2.0 * cos_w,
                1.0,
                1.0 + alpha,
                -2.0 * cos_w,
                1.0 - alpha,
            ),
            FilterKind::Peak => (
                1.0 + alpha * a,
                -2.0 * cos_w,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos_w,
                1.0 - alpha / a,
            ),
            FilterKind::LowShelf => {
                let beta = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos_w + beta),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w),
                    a * ((a + 1.0) - (a - 1.0) * cos_w - beta),
                    (a + 1.0) + (a - 1.0) * cos_w + beta,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos_w),
                    (a + 1.0) + (a - 1.0) * cos_w - beta,
                )
            }
            FilterKind::HighShelf => {
                let beta = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos_w + beta),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w),
                    a * ((a + 1.0) + (a - 1.0) * cos_w - beta),
                    (a + 1.0) - (a - 1.0) * cos_w + beta,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos_w),
                    (a + 1.0) - (a - 1.0) * cos_w - beta,
                )
            }
        };
        let inv = 1.0 / a0;
        Self {
            b0: b0 * inv,
            b1: b1 * inv,
            b2: b2 * inv,
            a1: a1 * inv,
            a2: a2 * inv,
        }
    }
    pub fn bypass() -> Self {
        Self {
            b0: 1.0,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Biquad {
    pub c: BiquadCoeffs,
    z1: f32,
    z2: f32,
}

impl Biquad {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&mut self, c: BiquadCoeffs) {
        self.c = c;
    }
    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = self.c.b0 * x + self.z1;
        self.z1 = self.c.b1 * x - self.c.a1 * y + self.z2;
        self.z2 = self.c.b2 * x - self.c.a2 * y;
        flush_denorm(y)
    }
}

/// Topology-preserving state variable filter (Chamberlin/TPT).
#[derive(Clone, Debug)]
pub struct Svf {
    g: f32,
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    ic1: f32,
    ic2: f32,
    pub drive: f32,
}

impl Svf {
    pub fn new() -> Self {
        let mut f = Self {
            g: 0.0,
            k: 1.0,
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            ic1: 0.0,
            ic2: 0.0,
            drive: 1.0,
        };
        f.set(1000.0, 0.707, 48_000.0);
        f
    }
    pub fn set(&mut self, cutoff: f32, q: f32, sr: f32) {
        let f = cutoff.clamp(10.0, sr * 0.49);
        self.g = (std::f32::consts::PI * f / sr).tan();
        self.k = 1.0 / q.max(0.05);
        self.a1 = 1.0 / (1.0 + self.g * (self.g + self.k));
        self.a2 = self.g * self.a1;
        self.a3 = self.g * self.a2;
    }
    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }
    #[inline]
    pub fn tick(&mut self, x: f32) -> (f32, f32, f32) {
        let input = x * self.drive;
        let v3 = input - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        let low = v2;
        let band = v1;
        let high = input - self.k * v1 - v2;
        (low, band, high)
    }
}

impl Default for Svf {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvfMode {
    Low,
    Band,
    High,
    Notch,
}

impl SvfMode {
    pub fn from_index(i: usize) -> Self {
        match i % 4 {
            0 => SvfMode::Low,
            1 => SvfMode::Band,
            2 => SvfMode::High,
            _ => SvfMode::Notch,
        }
    }
}

impl Svf {
    #[inline]
    pub fn tick_mode(&mut self, x: f32, mode: SvfMode) -> f32 {
        let (l, b, h) = self.tick(x);
        match mode {
            SvfMode::Low => l,
            SvfMode::Band => b,
            SvfMode::High => h,
            SvfMode::Notch => l + h,
        }
    }
}

// ------------------------------------------------------------------ envelopes

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnvStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Debug)]
pub struct Adsr {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    stage: EnvStage,
    level: f32,
    vel: f32,
    pub shape: f32, // 0 linear-ish, 1 exponential-ish
}

impl Adsr {
    pub fn new() -> Self {
        Self {
            attack: 0.005,
            decay: 0.15,
            sustain: 0.7,
            release: 0.2,
            stage: EnvStage::Idle,
            level: 0.0,
            vel: 1.0,
            shape: 0.8,
        }
    }
    pub fn set(&mut self, a: f32, d: f32, s: f32, r: f32) {
        self.attack = a.max(0.0005);
        self.decay = d.max(0.0005);
        self.sustain = s.clamp(0.0, 1.0);
        self.release = r.max(0.0005);
    }
    pub fn gate_on(&mut self, velocity: f32) {
        self.vel = velocity.clamp(0.0, 1.0);
        self.stage = EnvStage::Attack;
    }
    pub fn gate_off(&mut self) {
        if self.level <= 0.0 {
            self.stage = EnvStage::Idle;
        } else {
            self.stage = EnvStage::Release;
        }
    }
    pub fn reset(&mut self) {
        self.stage = EnvStage::Idle;
        self.level = 0.0;
    }
    pub fn is_active(&self) -> bool {
        !matches!(self.stage, EnvStage::Idle)
    }
    pub fn level(&self) -> f32 {
        self.level * self.vel
    }
    #[inline]
    pub fn tick(&mut self, sr: f32) -> f32 {
        let shape = self.shape.clamp(0.0, 1.0);
        match self.stage {
            EnvStage::Idle => {}
            EnvStage::Attack => {
                let inc = 1.0 / (self.attack * sr).max(1.0);
                self.level += inc;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = EnvStage::Decay;
                }
            }
            EnvStage::Decay => {
                let target = self.sustain;
                let coef = 1.0 - (-1.0 / (self.decay * sr * 0.25).max(1.0)).exp();
                self.level += (target - self.level) * (1.0 - shape + shape * coef);
                if (self.level - target).abs() < 0.0005 {
                    self.level = target;
                    self.stage = EnvStage::Sustain;
                }
            }
            EnvStage::Sustain => {
                self.level = self.sustain;
            }
            EnvStage::Release => {
                let coef = 1.0 - (-1.0 / (self.release * sr * 0.25).max(1.0)).exp();
                self.level += (0.0 - self.level) * (1.0 - shape + shape * coef);
                if self.level <= 0.0004 {
                    self.level = 0.0;
                    self.stage = EnvStage::Idle;
                }
            }
        }
        self.level * self.vel
    }
}

impl Default for Adsr {
    fn default() -> Self {
        Self::new()
    }
}

// ------------------------------------------------------------------ LFO

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfoShape {
    Sine,
    Triangle,
    Saw,
    Square,
    SampleHold,
}

impl LfoShape {
    pub fn from_index(i: usize) -> Self {
        match i % 5 {
            0 => LfoShape::Sine,
            1 => LfoShape::Triangle,
            2 => LfoShape::Saw,
            3 => LfoShape::Square,
            _ => LfoShape::SampleHold,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Lfo {
    pub shape: LfoShape,
    pub rate_hz: f32,
    pub phase: f32,
    hold_value: f32,
    rng: Rng,
}

impl Lfo {
    pub fn new(seed: u64) -> Self {
        Self {
            shape: LfoShape::Sine,
            rate_hz: 1.0,
            phase: 0.0,
            hold_value: 0.0,
            rng: Rng::new(seed),
        }
    }
    pub fn set_rate(&mut self, hz: f32) {
        self.rate_hz = hz.clamp(0.01, 60.0);
    }
    pub fn reset(&mut self) {
        self.phase = 0.0;
    }
    #[inline]
    pub fn value(&self) -> f32 {
        match self.shape {
            LfoShape::Sine => (TWO_PI * self.phase).sin(),
            LfoShape::Triangle => 1.0 - 4.0 * (self.phase - 0.5).abs(),
            LfoShape::Saw => 2.0 * self.phase - 1.0,
            LfoShape::Square => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoShape::SampleHold => self.hold_value,
        }
    }
    #[inline]
    pub fn tick(&mut self, sr: f32) -> f32 {
        let v = self.value();
        self.phase = wrap01(self.phase + self.rate_hz / sr);
        if matches!(self.shape, LfoShape::SampleHold) && self.phase < self.rate_hz / sr {
            self.hold_value = self.rng.next_f32();
        }
        v
    }
}

// ------------------------------------------------------------------ delay

#[derive(Clone, Debug)]
pub struct DelayLine {
    buf: Vec<f32>,
    write: usize,
    mask: usize,
}

impl DelayLine {
    pub fn new(max_samples: usize) -> Self {
        let size = (max_samples + 2).next_power_of_two();
        Self {
            buf: vec![0.0; size],
            write: 0,
            mask: size - 1,
        }
    }
    pub fn clear(&mut self) {
        self.buf.fill(0.0);
        self.write = 0;
    }
    #[inline]
    pub fn push(&mut self, x: f32) {
        self.buf[self.write] = x;
        self.write = (self.write + 1) & self.mask;
    }
    #[inline]
    pub fn read(&self, delay: f32) -> f32 {
        let size = self.buf.len();
        let d = delay.clamp(1.0, (size - 2) as f32);
        let pos = self.write as f32 - d;
        let i0 = pos.floor();
        let frac = pos - i0;
        let i0 = (i0 as isize).rem_euclid(size as isize) as usize;
        let i1 = (i0 + 1) % size;
        lerp(self.buf[i0], self.buf[i1], frac)
    }
    #[inline]
    pub fn read_int(&self, delay: usize) -> f32 {
        let size = self.buf.len();
        let d = delay.min(size - 2);
        self.buf[(self.write + size - d) % size]
    }
    pub fn max_len(&self) -> usize {
        self.buf.len() - 2
    }
}

// ------------------------------------------------------------------ reverb

#[derive(Clone, Debug)]
struct Comb {
    line: DelayLine,
    feedback: f32,
    damp: f32,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self {
            line: DelayLine::new(len + 2),
            feedback: 0.84,
            damp: 0.2,
            store: 0.0,
        }
    }
    #[inline]
    fn tick(&mut self, input: f32, len: usize) -> f32 {
        let out = self.line.read_int(len);
        self.store = out * (1.0 - self.damp) + self.store * self.damp;
        self.line.push(input + self.store * self.feedback);
        flush_denorm(out)
    }
    fn clear(&mut self) {
        self.line.clear();
        self.store = 0.0;
    }
}

#[derive(Clone, Debug)]
struct Allpass {
    line: DelayLine,
    feedback: f32,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self {
            line: DelayLine::new(len + 2),
            feedback: 0.5,
        }
    }
    #[inline]
    fn tick(&mut self, input: f32, len: usize) -> f32 {
        let buffered = self.line.read_int(len);
        let out = -input + buffered;
        self.line.push(input + buffered * self.feedback);
        out
    }
    fn clear(&mut self) {
        self.line.clear();
    }
}

const COMB_L: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_L: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;

/// Freeverb-style stereo reverb with predelay, tone and modulation-free stability.
#[derive(Clone, Debug)]
pub struct Reverb {
    combs_l: Vec<Comb>,
    combs_r: Vec<Comb>,
    aps_l: Vec<Allpass>,
    aps_r: Vec<Allpass>,
    predelay: DelayLine,
    lowcut_l: OnePole,
    lowcut_r: OnePole,
    highcut_l: OnePole,
    highcut_r: OnePole,
    pub room: f32,
    pub damp: f32,
    pub width: f32,
    pub mix: f32,
    pub predelay_ms: f32,
    pub lowcut_hz: f32,
    pub highcut_hz: f32,
    sized_for: f32,
}

impl Reverb {
    pub fn new(sr: f32) -> Self {
        let scale = sr / 44_100.0;
        let combs_l = COMB_L.iter().map(|&l| Comb::new((l as f32 * scale) as usize)).collect();
        let combs_r = COMB_L
            .iter()
            .map(|&l| Comb::new(((l + STEREO_SPREAD) as f32 * scale) as usize))
            .collect();
        let aps_l = ALLPASS_L.iter().map(|&l| Allpass::new((l as f32 * scale) as usize)).collect();
        let aps_r = ALLPASS_L
            .iter()
            .map(|&l| Allpass::new(((l + STEREO_SPREAD) as f32 * scale) as usize))
            .collect();
        let mut r = Self {
            combs_l,
            combs_r,
            aps_l,
            aps_r,
            predelay: DelayLine::new((sr * 0.25) as usize),
            lowcut_l: OnePole::new(),
            lowcut_r: OnePole::new(),
            highcut_l: OnePole::new(),
            highcut_r: OnePole::new(),
            room: 0.7,
            damp: 0.35,
            width: 1.0,
            mix: 0.25,
            predelay_ms: 0.0,
            lowcut_hz: 80.0,
            highcut_hz: 12_000.0,
            sized_for: sr,
        };
        r.set_sample_rate(sr);
        r
    }
    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sized_for = sr;
        self.lowcut_l.set_lowpass(self.lowcut_hz, sr);
        self.lowcut_r.set_lowpass(self.lowcut_hz, sr);
        self.highcut_l.set_lowpass(self.highcut_hz, sr);
        self.highcut_r.set_lowpass(self.highcut_hz, sr);
    }
    pub fn reset(&mut self) {
        for c in self.combs_l.iter_mut().chain(self.combs_r.iter_mut()) {
            c.clear();
        }
        for a in self.aps_l.iter_mut().chain(self.aps_r.iter_mut()) {
            a.clear();
        }
        self.predelay.clear();
        self.lowcut_l.reset();
        self.lowcut_r.reset();
        self.highcut_l.reset();
        self.highcut_r.reset();
    }
    fn update_coeffs(&mut self) {
        let fb = 0.7 + self.room.clamp(0.0, 1.0) * 0.28;
        let damp = self.damp.clamp(0.0, 1.0) * 0.8;
        for c in self.combs_l.iter_mut().chain(self.combs_r.iter_mut()) {
            c.feedback = fb;
            c.damp = damp;
        }
        self.lowcut_l.set_lowpass(self.lowcut_hz, self.sized_for);
        self.lowcut_r.set_lowpass(self.lowcut_hz, self.sized_for);
        self.highcut_l.set_lowpass(self.highcut_hz, self.sized_for);
        self.highcut_r.set_lowpass(self.highcut_hz, self.sized_for);
    }
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], sr: f32) {
        if (sr - self.sized_for).abs() > 1.0 {
            self.set_sample_rate(sr);
        }
        self.update_coeffs();
        let pre = (self.predelay_ms * 0.001 * sr) as f32;
        let scale = sr / 44_100.0;
        let input_gain = 0.015;
        let wet1 = self.width * 0.5 + 0.5;
        let wet2 = (1.0 - self.width) * 0.5;
        for i in 0..l.len().min(r.len()) {
            let dry_l = l[i];
            let dry_r = r[i];
            let mono = (dry_l + dry_r) * input_gain;
            self.predelay.push(mono);
            let input = if pre > 0.0 { self.predelay.read(pre) } else { mono };
            let mut acc_l = 0.0;
            let mut acc_r = 0.0;
            for (idx, c) in self.combs_l.iter_mut().enumerate() {
                let base = (COMB_L[idx] as f32 * scale) as usize;
                acc_l += c.tick(input, base);
            }
            for (idx, c) in self.combs_r.iter_mut().enumerate() {
                let base = ((COMB_L[idx] + STEREO_SPREAD) as f32 * scale) as usize;
                acc_r += c.tick(input, base);
            }
            for (idx, a) in self.aps_l.iter_mut().enumerate() {
                let base = (ALLPASS_L[idx] as f32 * scale) as usize;
                acc_l = a.tick(acc_l, base);
            }
            for (idx, a) in self.aps_r.iter_mut().enumerate() {
                let base = ((ALLPASS_L[idx] + STEREO_SPREAD) as f32 * scale) as usize;
                acc_r = a.tick(acc_r, base);
            }
            acc_l = self.highcut_l.tick(acc_l);
            acc_r = self.highcut_r.tick(acc_r);
            acc_l = acc_l - self.lowcut_l.tick(acc_l);
            acc_r = acc_r - self.lowcut_r.tick(acc_r);
            let wet_l = acc_l * wet1 + acc_r * wet2;
            let wet_r = acc_r * wet1 + acc_l * wet2;
            l[i] = dry_l * (1.0 - self.mix) + wet_l * self.mix * 3.0;
            r[i] = dry_r * (1.0 - self.mix) + wet_r * self.mix * 3.0;
        }
    }
}

/// 4-line feedback delay network reverb (Hadamard mixing) for hall/plate character.
#[derive(Clone, Debug)]
pub struct FdnReverb {
    lines: [DelayLine; 4],
    damp: [f32; 4],
    lens: [usize; 4],
    pub decay: f32,
    pub size: f32,
    pub damp_amount: f32,
    pub mix: f32,
    pub predelay_ms: f32,
    predelay: DelayLine,
    lp: OnePole,
    sr: f32,
}

impl FdnReverb {
    pub fn new(sr: f32) -> Self {
        let base = [1557usize, 1617, 1491, 1422];
        let lens = base.map(|l| (l as f32 * sr / 44_100.0) as usize);
        let mut lines = [
            DelayLine::new(lens[0] + 2),
            DelayLine::new(lens[1] + 2),
            DelayLine::new(lens[2] + 2),
            DelayLine::new(lens[3] + 2),
        ];
        for l in &mut lines {
            l.clear();
        }
        Self {
            lines,
            damp: [0.0; 4],
            lens,
            decay: 0.8,
            size: 1.0,
            damp_amount: 0.3,
            mix: 0.3,
            predelay_ms: 10.0,
            predelay: DelayLine::new((sr * 0.25) as usize),
            lp: OnePole::new(),
            sr,
        }
    }
    pub fn reset(&mut self) {
        for l in &mut self.lines {
            l.clear();
        }
        self.damp = [0.0; 4];
        self.predelay.clear();
        self.lp.reset();
    }
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], sr: f32) {
        if (sr - self.sr).abs() > 1.0 {
            self.sr = sr;
            for i in 0..4 {
                self.lens[i] = ([1557usize, 1617, 1491, 1422][i] as f32 * sr / 44_100.0) as usize;
            }
        }
        self.lp.set_lowpass(9000.0, sr);
        let mix = self.mix.clamp(0.0, 1.0);
        let pre = (self.predelay_ms * 0.001 * sr).min((self.predelay.max_len() - 2) as f32);
        let size = self.size.clamp(0.1, 2.0);
        for i in 0..l.len().min(r.len()) {
            let in_mono = (l[i] + r[i]) * 0.5;
            self.predelay.push(in_mono);
            let inp = if pre > 0.0 { self.predelay.read(pre) } else { in_mono };
            let mut taps = [0.0f32; 4];
            for k in 0..4 {
                taps[k] = self.lines[k].read_int((self.lens[k] as f32 * size) as usize);
            }
            // Hadamard 4x4
            let a = taps[0] + taps[1] + taps[2] + taps[3];
            let b = taps[0] - taps[1] + taps[2] - taps[3];
            let c = taps[0] + taps[1] - taps[2] - taps[3];
            let d = taps[0] - taps[1] - taps[2] + taps[3];
            let mixed = [a * 0.5, b * 0.5, c * 0.5, d * 0.5];
            for k in 0..4 {
                self.damp[k] = mixed[k] * (1.0 - self.damp_amount) + self.damp[k] * self.damp_amount;
                let fb = self.decay.clamp(0.0, 0.999);
                self.lines[k].push(inp * 0.3 + self.damp[k] * fb);
            }
            let wet_l = mixed[0] * 0.6 + mixed[2] * 0.4;
            let wet_r = mixed[1] * 0.6 + mixed[3] * 0.4;
            l[i] = l[i] * (1.0 - mix) + self.lp.tick(wet_l) * mix;
            r[i] = r[i] * (1.0 - mix) + self.lp.tick(wet_r) * mix;
        }
    }
}

// ------------------------------------------------------------------ dynamics

#[derive(Clone, Debug)]
pub struct Compressor {
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub knee_db: f32,
    pub makeup_db: f32,
    pub mix: f32,
    env_db: f32,
    pub reduction_db: f32,
}

impl Compressor {
    pub fn new() -> Self {
        Self {
            threshold_db: -18.0,
            ratio: 4.0,
            attack_ms: 8.0,
            release_ms: 120.0,
            knee_db: 6.0,
            makeup_db: 0.0,
            mix: 1.0,
            env_db: 0.0,
            reduction_db: 0.0,
        }
    }
    pub fn reset(&mut self) {
        self.env_db = 0.0;
        self.reduction_db = 0.0;
    }
    /// Stereo-linked processing. Returns applied linear gain.
    #[inline]
    pub fn process_stereo(&mut self, l: &mut f32, r: &mut f32, sr: f32) -> f32 {
        let det = l.abs().max(r.abs()).max(1e-6);
        let det_db = 20.0 * det.log10();
        let over = det_db - self.threshold_db;
        let slope = 1.0 - 1.0 / self.ratio.max(1.0);
        let target_gr = if self.knee_db > 0.0 && over > -self.knee_db * 0.5 && over < self.knee_db * 0.5 {
            let x = over + self.knee_db * 0.5;
            -slope * x * x / (2.0 * self.knee_db)
        } else if over > 0.0 {
            -slope * over
        } else {
            0.0
        };
        let attack = (-1.0 / (self.attack_ms.max(0.05) * 0.001 * sr)).exp();
        let release = (-1.0 / (self.release_ms.max(1.0) * 0.001 * sr)).exp();
        let coef = if target_gr < self.reduction_db { attack } else { release };
        self.reduction_db = target_gr + (self.reduction_db - target_gr) * coef;
        let makeup = 10f32.powf(self.makeup_db / 20.0);
        let gain = 10f32.powf(self.reduction_db / 20.0) * makeup;
        let wet = self.mix.clamp(0.0, 1.0);
        let gl = 1.0 - wet + gain * wet;
        *l *= gl;
        *r *= gl;
        self.env_db = det_db;
        gain
    }
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct Limiter {
    look_l: DelayLine,
    look_r: DelayLine,
    look_samples: usize,
    pub ceiling_db: f32,
    pub release_ms: f32,
    gain: f32,
}

impl Limiter {
    pub fn new(sr: f32) -> Self {
        let look = (0.0015 * sr) as usize;
        Self {
            look_l: DelayLine::new(look + 2),
            look_r: DelayLine::new(look + 2),
            look_samples: look,
            ceiling_db: -0.3,
            release_ms: 80.0,
            gain: 1.0,
        }
    }
    pub fn reset(&mut self) {
        self.look_l.clear();
        self.look_r.clear();
        self.gain = 1.0;
    }
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], sr: f32) {
        let ceiling = 10f32.powf(self.ceiling_db / 20.0);
        let release = (-1.0 / (self.release_ms.max(1.0) * 0.001 * sr)).exp();
        for i in 0..l.len().min(r.len()) {
            let dl = l[i];
            let dr = r[i];
            self.look_l.push(dl);
            self.look_r.push(dr);
            let peak = dl.abs().max(dr.abs());
            let target = if peak > ceiling { ceiling / peak.max(1e-9) } else { 1.0 };
            if target < self.gain {
                self.gain = target;
            } else {
                self.gain = target + (self.gain - target) * release;
            }
            let gl = self.look_l.read(self.look_samples as f32) * self.gain;
            let gr = self.look_r.read(self.look_samples as f32) * self.gain;
            l[i] = gl.clamp(-ceiling, ceiling);
            r[i] = gr.clamp(-ceiling, ceiling);
        }
    }
}

// ------------------------------------------------------------------ one-pole

#[derive(Clone, Debug)]
pub struct OnePole {
    a: f32,
    z: f32,
    pub enabled: bool,
}

impl OnePole {
    pub fn new() -> Self {
        Self {
            a: 0.1,
            z: 0.0,
            enabled: true,
        }
    }
    pub fn set_lowpass(&mut self, cutoff: f32, sr: f32) {
        let w = TWO_PI * cutoff.clamp(1.0, sr * 0.49) / sr;
        let a = w.sin() / (w.sin() + w.cos());
        self.a = a.clamp(0.0001, 0.9999);
    }
    pub fn reset(&mut self) {
        self.z = 0.0;
    }
    pub fn tick(&mut self, x: f32) -> f32 {
        self.z += self.a * (x - self.z);
        flush_denorm(self.z)
    }
    pub fn hp(&mut self, x: f32) -> f32 {
        x - self.tick(x)
    }
}

impl Default for OnePole {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct DcBlocker {
    x1: f32,
    y1: f32,
}

impl DcBlocker {
    pub fn new() -> Self {
        Self { x1: 0.0, y1: 0.0 }
    }
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + 0.995 * self.y1;
        self.x1 = x;
        self.y1 = y;
        flush_denorm(y)
    }
}

impl Default for DcBlocker {
    fn default() -> Self {
        Self::new()
    }
}

// ------------------------------------------------------------------ smoother

#[derive(Clone, Debug)]
pub struct Smoother {
    current: f32,
    target: f32,
    coef: f32,
}

impl Smoother {
    pub fn new(value: f32) -> Self {
        Self {
            current: value,
            target: value,
            coef: 0.01,
        }
    }
    pub fn set_time(&mut self, seconds: f32, sr: f32) {
        let s = seconds.max(0.0001);
        self.coef = 1.0 - (-1.0 / (s * sr)).exp();
    }
    pub fn set_target(&mut self, v: f32) {
        self.target = v;
    }
    pub fn snap(&mut self, v: f32) {
        self.current = v;
        self.target = v;
    }
    #[inline]
    pub fn next(&mut self) -> f32 {
        self.current += (self.target - self.current) * self.coef;
        self.current
    }
    pub fn current(&self) -> f32 {
        self.current
    }
}

// ------------------------------------------------------------------ FFT analyzer

pub struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    buffer: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    window: Vec<f32>,
    mags: Vec<f32>,
    size: usize,
}

impl Analyzer {
    pub fn new(size: usize) -> Self {
        let size = size.max(64).next_power_of_two();
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(size);
        let scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len().max(1)];
        let window = (0..size)
            .map(|i| {
                let x = i as f32 / (size - 1) as f32;
                0.5 - 0.5 * (TWO_PI * x).cos()
            })
            .collect();
        Self {
            fft,
            buffer: vec![Complex::new(0.0, 0.0); size],
            scratch,
            window,
            mags: vec![0.0; size / 2],
            size,
        }
    }
    pub fn size(&self) -> usize {
        self.size
    }
    /// Analyze exactly `size` samples; returns magnitudes for bins 0..size/2.
    pub fn analyze(&mut self, samples: &[f32]) -> &[f32] {
        let size = self.size;
        let start = samples.len().saturating_sub(size);
        for i in 0..size {
            let s = samples.get(start + i).copied().unwrap_or(0.0) * self.window[i];
            self.buffer[i] = Complex::new(s, 0.0);
        }
        self.fft.process_with_scratch(&mut self.buffer, &mut self.scratch);
        let norm = 2.0 / size as f32;
        for (i, mag) in self.mags.iter_mut().enumerate() {
            *mag = self.buffer[i].norm() * norm;
        }
        &self.mags
    }
}

// ------------------------------------------------------------------ convolver

/// Uniformly-partitioned FFT convolution (overlap-add), mono IR.
pub struct Convolver {
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    n: usize,
    hop: usize,
    ir_specs: Vec<Vec<Complex<f32>>>,
    in_specs: Vec<Vec<Complex<f32>>>,
    head: usize,
    accum: Vec<Complex<f32>>,
    time: Vec<Complex<f32>>,
    overlap: Vec<f32>,
    scratch: Vec<Complex<f32>>,
    ir_dirty: bool,
}

impl Convolver {
    pub fn new(max_ir_samples: usize, hop: usize) -> Self {
        let hop = hop.max(64).next_power_of_two();
        let n = hop * 2;
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n);
        let ifft = planner.plan_fft_inverse(n);
        let scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len().max(ifft.get_inplace_scratch_len())];
        let parts = max_ir_samples / hop + 1;
        Self {
            fft,
            ifft,
            n,
            hop,
            ir_specs: Vec::new(),
            in_specs: vec![vec![Complex::new(0.0, 0.0); n]; parts],
            head: 0,
            accum: vec![Complex::new(0.0, 0.0); n],
            time: vec![Complex::new(0.0, 0.0); n],
            overlap: vec![0.0; hop],
            scratch,
            ir_dirty: true,
        }
    }
    /// Set the impulse response; allocation happens here (setup time only).
    pub fn set_ir(&mut self, ir: &[f32]) {
        let hop = self.hop;
        let n = self.n;
        let parts = ir.len() / hop + 1;
        self.ir_specs.clear();
        self.ir_specs.resize_with(parts, || vec![Complex::new(0.0, 0.0); n]);
        for p in 0..parts {
            let mut buf = vec![Complex::new(0.0, 0.0); n];
            for i in 0..hop {
                let idx = p * hop + i;
                if idx < ir.len() {
                    buf[i] = Complex::new(ir[idx], 0.0);
                }
            }
            self.fft.process_with_scratch(&mut buf, &mut self.scratch);
            self.ir_specs[p] = buf;
        }
        self.in_specs = vec![vec![Complex::new(0.0, 0.0); n]; parts];
        self.head = 0;
        self.ir_dirty = false;
    }
    pub fn reset(&mut self) {
        for s in &mut self.in_specs {
            s.fill(Complex::new(0.0, 0.0));
        }
        self.overlap.fill(0.0);
        self.head = 0;
    }
    /// Convolve one hop-sized block in place.
    pub fn process_block(&mut self, block: &mut [f32]) {
        if self.ir_dirty || self.ir_specs.is_empty() {
            return;
        }
        let hop = self.hop;
        let n = self.n;
        let parts = self.ir_specs.len();
        debug_assert_eq!(block.len(), hop);
        for (i, s) in self.time.iter_mut().enumerate() {
            *s = if i < hop {
                Complex::new(block[i], 0.0)
            } else {
                Complex::new(0.0, 0.0)
            };
        }
        self.fft.process_with_scratch(&mut self.time, &mut self.scratch);
        self.head = (self.head + 1) % parts;
        let slot = self.head;
        self.in_specs[slot].copy_from_slice(&self.time);
        self.accum.fill(Complex::new(0.0, 0.0));
        for p in 0..parts {
            let ispec = &self.in_specs[(slot + parts - p) % parts];
            let irspec = &self.ir_specs[p];
            for i in 0..n {
                let re = self.accum[i].re + ispec[i].re * irspec[i].re - ispec[i].im * irspec[i].im;
                let im = self.accum[i].im + ispec[i].re * irspec[i].im + ispec[i].im * irspec[i].re;
                self.accum[i] = Complex::new(re, im);
            }
        }
        self.ifft.process_with_scratch(&mut self.accum, &mut self.scratch);
        let scale = 1.0 / n as f32;
        for i in 0..hop {
            block[i] = self.overlap[i] + self.accum[i].re * scale;
        }
        for i in 0..hop {
            self.overlap[i] = self.accum[hop + i].re * scale;
        }
    }
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oscillator_frequency() {
        let sr = 48_000.0;
        let mut osc = Osc::new(OscKind::Sine, 1);
        osc.set_freq(440.0, sr);
        let mut crossings = 0;
        let mut prev = osc.next();
        for _ in 1..48_000 {
            let v = osc.next();
            if prev <= 0.0 && v > 0.0 {
                crossings += 1;
            }
            prev = v;
        }
        assert!((crossings as i32 - 440).abs() <= 2, "crossings={crossings}");
    }

    #[test]
    fn saw_and_square_bounded() {
        let sr = 48_000.0;
        for kind in [OscKind::Saw, OscKind::Square, OscKind::Triangle, OscKind::Pulse] {
            let mut osc = Osc::new(kind, 7);
            osc.set_freq(220.0, sr);
            let mut peak = 0.0f32;
            for _ in 0..48_000 {
                peak = peak.max(osc.next().abs());
            }
            assert!(peak < 1.25, "{kind:?} peak {peak}");
        }
    }

    #[test]
    fn wavetable_saw_harmonics() {
        let mut wt = Wavetable::from_shape("saw", 64);
        wt.set_freq(100.0, 48_000.0);
        let mut buf = vec![0.0; 4096];
        for b in &mut buf {
            *b = wt.next();
        }
        let mut an = Analyzer::new(4096);
        let mags = an.analyze(&buf);
        let bin = |hz: f32| mags[(hz / 48_000.0 * 4096.0) as usize];
        assert!(bin(100.0) > bin(200.0));
        assert!(bin(200.0) > bin(300.0));
    }

    #[test]
    fn biquad_lowpass_dc_gain() {
        let mut f = Biquad::new();
        f.set(BiquadCoeffs::compute(FilterKind::LowPass, 1000.0, 0.707, 0.0, 48_000.0));
        let mut y = 0.0;
        for _ in 0..48_000 {
            y = f.tick(1.0);
        }
        assert!((y - 1.0).abs() < 1e-2, "y={y}");
        let mut nan = Biquad::new();
        nan.set(BiquadCoeffs::compute(FilterKind::HighPass, 100.0, 0.9, 0.0, 48_000.0));
        let out = nan.tick(f32::NAN);
        assert!(out.is_nan()); // NaN propagation is the caller's guard duty
    }

    #[test]
    fn svf_stable() {
        let mut f = Svf::new();
        f.set(500.0, 2.0, 48_000.0);
        let mut peak = 0.0f32;
        for i in 0..10_000 {
            let x = if i % 100 == 0 { 1.0 } else { 0.0 };
            let (l, b, h) = f.tick(x);
            assert!(l.is_finite() && b.is_finite() && h.is_finite());
            peak = peak.max(l.abs());
        }
        assert!(peak < 100.0);
    }

    #[test]
    fn adsr_shape() {
        let mut env = Adsr::new();
        env.set(0.01, 0.05, 0.5, 0.1);
        env.gate_on(1.0);
        let mut peak = 0.0f32;
        for _ in 0..4800 {
            peak = peak.max(env.tick(48_000.0));
        }
        assert!(peak > 0.8);
        env.gate_off();
        let mut last = 1.0;
        for _ in 0..48_000 {
            last = env.tick(48_000.0);
        }
        assert!(last < 0.001, "last={last}");
        assert!(!env.is_active());
    }

    #[test]
    fn delay_reads_delayed_sample() {
        let mut d = DelayLine::new(1024);
        d.push(1.0);
        for _ in 1..100 {
            d.push(0.0);
        }
        assert!((d.read(100.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn reverb_tail_decays() {
        let mut rv = Reverb::new(48_000.0);
        rv.room = 0.5;
        rv.mix = 1.0;
        rv.highcut_hz = 20_000.0;
        let n = 48_000;
        let mut l = vec![0.0; n];
        let mut r = vec![0.0; n];
        for i in 0..64 {
            l[i] = 0.5;
            r[i] = 0.5;
        }
        rv.process(&mut l, &mut r, 48_000.0);
        let tail: f32 = l[9600..10_000].iter().map(|x| x.abs()).sum();
        assert!(tail.is_finite() && tail < 60.0);
    }

    #[test]
    fn compressor_reduces_loud_signal() {
        let mut c = Compressor::new();
        c.threshold_db = -20.0;
        c.ratio = 10.0;
        c.attack_ms = 1.0;
        let sr = 48_000.0;
        let mut l = vec![0.5f32; 4800];
        let mut r = vec![0.5f32; 4800];
        for i in 0..4800 {
            c.process_stereo(&mut l[i], &mut r[i], sr);
        }
        let out_db = 20.0 * l[4799].abs().log10();
        assert!(out_db < -8.0, "out_db={out_db}");
    }

    #[test]
    fn convolver_matches_direct() {
        let hop = 512;
        let mut conv = Convolver::new(hop * 2, hop);
        let ir = [1.0f32, 0.5, 0.25, -0.1];
        conv.set_ir(&ir);
        let mut block = vec![0.0f32; hop];
        block[0] = 1.0;
        conv.process_block(&mut block);
        for i in 0..ir.len() {
            assert!((block[i] - ir[i]).abs() < 1e-4, "i={i} got {} want {}", block[i], ir[i]);
        }
        for i in ir.len()..hop {
            assert!(block[i].abs() < 1e-4);
        }
    }

    #[test]
    fn fft_peak_correct() {
        let sr = 48_000.0;
        let size = 4096;
        let buf: Vec<f32> = (0..size)
            .map(|i| (TWO_PI * 1000.0 * i as f32 / sr).sin())
            .collect();
        let mut an = Analyzer::new(size);
        let mags = an.analyze(&buf);
        let peak_bin = mags
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        let hz = peak_bin as f32 * sr / size as f32;
        assert!((hz - 1000.0).abs() < 20.0, "hz={hz}");
    }
}
