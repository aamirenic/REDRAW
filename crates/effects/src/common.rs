//! Percent-common effect building blocks: gain/pan trims, stereo tools, delay
//! networks, modulated filter stages, granular pitch shifting, band splits and
//! envelope followers. Every core is allocation-free after construction.
#![forbid(unsafe_code)]

use redraw_dsp::{
    lerp, wrap01, Biquad, BiquadCoeffs, DelayLine, FilterKind, Lfo, LfoShape, OnePole, Svf,
    SvfMode, TWO_PI,
};
use redraw_shared::ParamSpec;

// ---------------------------------------------------------------- effect glue

/// Internal mechanics base: every concrete effect implements this; the
/// `impl_effect!` macro below adapts it to the shared `Effect` contract so
/// parameter clamping, bypass handling and controller plumbing live in one place.
pub trait EffectBase: Send {
    /// Static parameter table (same order as the instance's `p` array).
    const SPEC: &'static [ParamSpec];
    /// Registry name, also returned by `Effect::effect_type`.
    fn type_name(&self) -> &'static str;
    /// Live parameter values.
    fn p_read(&self) -> &[f32];
    /// Store a pre-clamped parameter value.
    fn p_write(&mut self, index: usize, value: f32);
    /// Fired after `set_param` stores a value (recompute flags etc).
    fn param_changed(&mut self, _index: usize) {}
    /// The DSP pass; only called when not bypassed.
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, tempo: f32);
    /// Clear all state back to construction values.
    fn reset_fx(&mut self);
    fn bypass_state(&self) -> bool;
    fn set_bypass_state(&mut self, bypass: bool);
    /// Modulator output for controller-type effects (Peak/XY/Formula mod).
    fn mod_value(&self) -> f32 {
        0.0
    }
}

/// Stamps the shared `Effect` contract for one `EffectBase` implementor.
/// Bypass is exact passthrough, also enforced here so callers that bypass the
/// mixer's gate still observe the contract.
macro_rules! impl_effect {
    ($ty:ty) => {
        impl redraw_shared::Effect for $ty {
            fn effect_type(&self) -> &'static str {
                crate::common::EffectBase::type_name(self)
            }
            fn reset(&mut self) {
                crate::common::EffectBase::reset_fx(self)
            }
            fn process(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, tempo: f32) {
                if crate::common::EffectBase::bypass_state(self) {
                    return;
                }
                crate::common::EffectBase::process_fx(self, left, right, sr, tempo);
            }
            fn params(&self) -> &'static [redraw_shared::ParamSpec] {
                <$ty as crate::common::EffectBase>::SPEC
            }
            fn set_param(&mut self, index: usize, value: f32) {
                let spec_list = <$ty as crate::common::EffectBase>::SPEC;
                if let Some(spec) = spec_list.get(index) {
                    let v = spec.denormalize(spec.normalize(value));
                    crate::common::EffectBase::p_write(self, index, v);
                    crate::common::EffectBase::param_changed(self, index);
                }
            }
            fn get_param(&self, index: usize) -> f32 {
                crate::common::EffectBase::p_read(self)
                    .get(index)
                    .copied()
                    .unwrap_or(0.0)
            }
            fn set_bypass(&mut self, bypass: bool) {
                crate::common::EffectBase::set_bypass_state(self, bypass);
            }
            fn is_bypassed(&self) -> bool {
                crate::common::EffectBase::bypass_state(self)
            }
            fn controller_value(&self) -> f32 {
                crate::common::EffectBase::mod_value(self)
            }
        }
    };
}
pub(crate) use impl_effect;

#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[inline]
pub fn gain_to_db(g: f32) -> f32 {
    20.0 * g.abs().max(1e-9).log10()
}

/// Stereo biquad pair ticking both channels through identical coefficients.
#[derive(Clone, Debug)]
pub struct SBiquad {
    pub l: Biquad,
    pub r: Biquad,
}

impl Default for SBiquad {
    fn default() -> Self {
        Self {
            l: Biquad::new(),
            r: Biquad::new(),
        }
    }
}

impl SBiquad {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&mut self, c: BiquadCoeffs) {
        self.l.set(c);
        self.r.set(c);
    }
    pub fn set_kind(&mut self, kind: FilterKind, freq: f32, q: f32, gain_db: f32, sr: f32) {
        self.set(BiquadCoeffs::compute(kind, freq, q, gain_db, sr));
    }
    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        (self.l.tick(l), self.r.tick(r))
    }
    pub fn reset(&mut self) {
        self.l.reset();
        self.r.reset();
    }
}

/// One chorus-style modulated delay voice.
#[derive(Clone, Debug)]
pub struct ModVoice {
    pub dl: DelayLine,
    pub phase: f32,
    pub rate: f32,
    pub base_ms: f32,
    pub depth_ms: f32,
    pub mix: f32,
    pub pan: f32,
    pub feedback: f32,
}

impl ModVoice {
    pub fn new(seed: u64) -> Self {
        let _ = seed;
        Self {
            dl: DelayLine::new(2400),
            phase: 0.0,
            rate: 0.6,
            base_ms: 20.0,
            depth_ms: 3.0,
            mix: 0.6,
            pan: 0.0,
            feedback: 0.0,
        }
    }
    /// Tick one sample through the modulated line; returns (wet, dry_mono).
    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, sr: f32) -> (f32, f32) {
        self.phase = wrap01(self.phase + self.rate / sr);
        let wob = 0.5 - 0.5 * (TWO_PI * self.phase).cos();
        let d = ((self.base_ms + self.depth_ms * wob) * 0.001 * sr).clamp(1.0, 2300.0);
        let mono = (l + r) * 0.5;
        let wet = self.dl.read(d);
        self.dl.push(mono + wet * self.feedback);
        (wet, mono)
    }
    pub fn reset(&mut self) {
        self.dl.clear();
        self.phase = 0.0;
    }
}

/// Stereo delay network: dual lines with per-side time/feedback and tone shaping.
#[derive(Clone, Debug)]
pub struct DelayNet {
    pub dl_l: DelayLine,
    pub dl_r: DelayLine,
    pub lp_l: OnePole,
    pub lp_r: OnePole,
    pub max_ms: f32,
    pub time_l_ms: f32,
    pub time_r_ms: f32,
    pub damp_l: f32,
    pub damp_r: f32,
}

impl DelayNet {
    pub fn new(max_ms: f32) -> Self {
        let max_ms = max_ms.clamp(50.0, 4000.0);
        Self {
            dl_l: DelayLine::new((max_ms * 60.0) as usize),
            dl_r: DelayLine::new((max_ms * 60.0) as usize),
            lp_l: OnePole::new(),
            lp_r: OnePole::new(),
            max_ms,
            time_l_ms: 250.0,
            time_r_ms: 250.0,
            damp_l: 0.2,
            damp_r: 0.2,
        }
    }
    pub fn max_ms(&self) -> f32 {
        self.max_ms
    }
    pub fn reset(&mut self) {
        self.dl_l.clear();
        self.dl_r.clear();
        self.lp_l.reset();
        self.lp_r.reset();
    }
}

/// Three-band phase-coherent split at two crossover points (complementary).
#[derive(Clone, Debug)]
pub struct BandSplit {
    pub low1: SBiquad,
    pub low2: SBiquad,
    pub mid1: SBiquad,
    pub mid2: SBiquad,
    pub split1: f32,
    pub split2: f32,
}

impl Default for BandSplit {
    fn default() -> Self {
        Self {
            low1: SBiquad::new(),
            low2: SBiquad::new(),
            mid1: SBiquad::new(),
            mid2: SBiquad::new(),
            split1: 200.0,
            split2: 2400.0,
        }
    }
}

impl BandSplit {
    pub fn new(f1: f32, f2: f32) -> Self {
        let mut s = BandSplit::default();
        s.split1 = f1;
        s.split2 = f2;
        s
    }
    pub fn set_edges(&mut self, f1: f32, f2: f32, sr: f32) {
        self.split1 = f1.max(20.0);
        self.split2 = f2.max(self.split1 * 1.2);
        let l1 = BiquadCoeffs::compute(FilterKind::LowPass, self.split1, 0.72, 0.0, sr);
        let l2 = l1;
        let m1 = BiquadCoeffs::compute(FilterKind::LowPass, self.split2, 0.72, 0.0, sr);
        let m2 = m1;
        self.low1.set(l1);
        self.low2.set(l2);
        self.mid1.set(m1);
        self.mid2.set(m2);
    }
    /// Returns (low, mid, high) as mono sums per band.
    #[inline]
    pub fn split(&mut self, l: f32, r: f32) -> (f32, f32, f32) {
        let (bl, br) = self.split_stereo(l, r);
        (bl[0] + br[0], bl[1] + br[1], bl[2] + br[2])
    }

    /// Per-channel three-way split; returns ([low, mid, high] for L, same for R).
    #[inline]
    pub fn split_stereo(&mut self, l: f32, r: f32) -> ([f32; 3], [f32; 3]) {
        let la = self.low1.l.tick(l);
        let low_l = self.low2.l.tick(la);
        let ra = self.low1.r.tick(r);
        let low_r = self.low2.r.tick(ra);
        let hl = l - low_l;
        let hr = r - low_r;
        let ml = self.mid1.l.tick(hl);
        let mid_l = self.mid2.l.tick(ml);
        let mr = self.mid1.r.tick(hr);
        let mid_r = self.mid2.r.tick(mr);
        ([low_l, mid_l, hl - mid_l], [low_r, mid_r, hr - mid_r])
    }
    pub fn reset(&mut self) {
        self.low1.reset();
        self.low2.reset();
        self.mid1.reset();
        self.mid2.reset();
    }
}

/// Mid/side utilities.
#[inline]
pub fn mono_to_ms(l: f32, r: f32) -> (f32, f32) {
    ((l + r) * 0.5, (l - r) * 0.5)
}
#[inline]
pub fn ms_to_mono(m: f32, s: f32) -> (f32, f32) {
    (m + s, m - s)
}

/// First-order modulated allpass stage for phasers.
#[derive(Clone, Copy, Debug, Default)]
pub struct ApfStage {
    pub a: f32,
    pub x1: f32,
    pub y1: f32,
}

impl ApfStage {
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let y = self.a * x + self.x1 - self.a * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }
}

/// Dual-head granular pitch shifter: two delay ramps a quarter grain apart,
/// equal-power crossfaded so each head jumps while hidden.
#[derive(Clone, Debug)]
pub struct PitchShiftCore {
    pub dl: DelayLine,
    pub pa: f32,
    pub grain: f32,
}

impl PitchShiftCore {
    pub fn new() -> Self {
        Self {
            dl: DelayLine::new(9600),
            pa: 0.5,
            grain: 0.09,
        }
    }
    /// `ratio` = 2^(semitones/12). Returns the shifted sample.
    #[inline]
    pub fn tick(&mut self, x: f32, ratio: f32, sr: f32) -> f32 {
        self.dl.push(x);
        let seg = (self.grain * 0.5 * sr).clamp(128.0, 4000.0);
        if (ratio - 1.0).abs() < 0.0025 {
            return self.dl.read(seg);
        }
        let drift = (1.0 - ratio).clamp(-0.6, 0.6) / seg;
        self.pa = wrap01(self.pa + drift);
        let d_a = 32.0 + self.pa * (seg - 32.0);
        let d_b = 32.0 + wrap01(self.pa + 0.25) * (seg - 32.0);
        let s = (TWO_PI * self.pa).sin();
        let wa = s * s;
        self.dl.read(d_a) * wa + self.dl.read(d_b) * (1.0 - wa)
    }
    pub fn reset(&mut self) {
        self.dl.clear();
        self.pa = 0.5;
    }
}

/// Envelope follower with attack/release smoothing (linear gain domain).
#[derive(Clone, Copy, Debug)]
pub struct EnvFollower {
    pub state: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
}

impl Default for EnvFollower {
    fn default() -> Self {
        Self {
            state: 0.0,
            attack_ms: 5.0,
            release_ms: 120.0,
        }
    }
}

impl EnvFollower {
    #[inline]
    pub fn tick(&mut self, x: f32, sr: f32) -> f32 {
        let a = (-1.0 / (self.attack_ms.max(0.05) * 0.001 * sr)).exp();
        let r = (-1.0 / (self.release_ms.max(1.0) * 0.001 * sr)).exp();
        let target = x.abs();
        let c = if target > self.state { a } else { r };
        self.state = target + (self.state - target) * c;
        self.state
    }
    pub fn reset(&mut self) {
        self.state = 0.0;
    }
}

/// Small shared LFO holder with per-sample tick used by mod effects.
#[derive(Clone, Debug)]
pub struct ModLfo {
    pub lfo: Lfo,
    pub rate: f32,
    pub depth: f32,
}

impl ModLfo {
    pub fn sine(rate: f32, depth: f32) -> Self {
        let mut lfo = Lfo::new(0x77);
        lfo.shape = LfoShape::Sine;
        Self { lfo, rate, depth }
    }
}

/// Bit/decimate crusher.
#[derive(Clone, Copy, Debug, Default)]
pub struct Crusher {
    pub hold: f32,
    pub hold_n: u32,
    pub bits: f32,
    pub rate_div: u32,
}

impl Crusher {
    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        if self.rate_div <= 1 {
            self.hold = x;
        } else if self.hold_n == 0 {
            self.hold = x;
            self.hold_n = self.rate_div;
        } else {
            self.hold_n -= 1;
        }
        let levels = 2f32.powf(self.bits.clamp(1.0, 16.0) - 1.0);
        (self.hold * levels).round() / levels
    }
}

/// Editable polyline waveshaper over a fixed 64-point curve domain (-1..1).
#[derive(Clone, Debug)]
pub struct CurveShaper {
    pub pts: [f32; 64],
}

impl Default for CurveShaper {
    fn default() -> Self {
        Self::new()
    }
}

impl CurveShaper {
    pub fn new() -> Self {
        Self {
            pts: identity_curve(),
        }
    }
    #[inline]
    pub fn shape(&self, x: f32) -> f32 {
        let t = ((x + 1.0) * 0.5).clamp(0.0, 1.0) * 63.0;
        let i0 = t as usize;
        let frac = t - i0 as f32;
        let a = self.pts[i0.min(63)];
        let b = self.pts[(i0 + 1).min(63)];
        lerp(a, b, frac)
    }
}

#[rustfmt::skip]
fn identity_curve() -> [f32; 64] {
    let mut p = [0.0f32; 64];
    for (i, v) in p.iter_mut().enumerate() {
        let x = i as f32 / 63.0;
        *v = x * 2.0 - 1.0;
    }
    p
}

/// Build a 64-point curve through five control heights (each 0..1 -> -1..1).
#[rustfmt::skip]
pub fn curve_from_points(y0: f32, y25: f32, y50: f32, y75: f32, y100: f32) -> [f32; 64] {
    let mut p = [0.0f32; 64];
    for (i, v) in p.iter_mut().enumerate() {
        let x = i as f32 / 63.0;
        let (ya, yb, t) = if x < 0.25 {
            (y0, y25, x / 0.25)
        } else if x < 0.5 {
            (y25, y50, (x - 0.25) / 0.25)
        } else if x < 0.75 {
            (y50, y75, (x - 0.5) / 0.25)
        } else {
            (y75, y100, (x - 0.75) / 0.25)
        };
        *v = (ya + (yb - ya) * t).clamp(0.0, 1.0) * 2.0 - 1.0;
    }
    p
}

#[inline]
pub fn pan_offset(l: f32, r: f32, pan: f32) -> (f32, f32) {
    let (pl, pr) = redraw_shared::pan_gains(pan.clamp(-1.0, 1.0) * 0.7);
    (l * pl, r * pr)
}

/// Reusable HP/LP tone stack (share one coefficient set per channel pair).
#[derive(Clone, Debug)]
pub struct ToneStack {
    pub hp: SBiquad,
    pub lp: SBiquad,
}

impl Default for ToneStack {
    fn default() -> Self {
        Self {
            hp: SBiquad::new(),
            lp: SBiquad::new(),
        }
    }
}

impl ToneStack {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_bands(&mut self, hp_freq: f32, lp_freq: f32, q: f32, sr: f32) {
        let hp =
            BiquadCoeffs::compute(FilterKind::HighPass, hp_freq.clamp(20.0, sr * 0.45), q, 0.0, sr);
        let lp =
            BiquadCoeffs::compute(FilterKind::LowPass, lp_freq.clamp(200.0, sr * 0.49), q, 0.0, sr);
        self.hp.set(hp);
        self.lp.set(lp);
    }
    pub fn reset(&mut self) {
        self.hp.reset();
        self.lp.reset();
    }
}

#[inline]
pub fn mix_wet(dry: f32, wet: f32, mix: f32) -> f32 {
    let m = mix.clamp(0.0, 1.0);
    dry * (1.0 - m) + wet * m
}

/// Stereo SVF holder (left+right share one coefficient set).
#[derive(Clone, Debug)]
pub struct SvfStereo {
    pub l: Svf,
    pub r: Svf,
    pub mode: SvfMode,
}

impl SvfStereo {
    pub fn new(mode: SvfMode) -> Self {
        Self {
            l: Svf::new(),
            r: Svf::new(),
            mode,
        }
    }
    pub fn set(&mut self, cutoff: f32, q: f32, sr: f32) {
        self.l.set(cutoff, q, sr);
        self.r.set(cutoff, q, sr);
    }
    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        (self.l.tick_mode(l, self.mode), self.r.tick_mode(r, self.mode))
    }
}

// ---------------------------------------------------------------- block staging

/// Adapts a strict hop-sized block processor (FFT convolver) to arbitrary
/// sample-slice lengths with no allocation after construction. Latency is one
/// hop; the output ring holds two hops so read/write never alias.
pub struct HopStager {
    stage: Vec<f32>,
    ring: Vec<f32>,
    head: usize,
    rpos: usize,
    wpos: usize,
    count: usize,
}

impl HopStager {
    pub fn new(hop: usize) -> Self {
        Self {
            stage: vec![0.0; hop],
            ring: vec![0.0; hop * 2],
            head: 0,
            rpos: 0,
            wpos: 0,
            count: 0,
        }
    }
    pub fn reset(&mut self) {
        self.stage.fill(0.0);
        self.ring.fill(0.0);
        self.head = 0;
        self.rpos = 0;
        self.wpos = 0;
        self.count = 0;
    }
    /// Push one input sample; returns one output sample (0 while filling the
    /// first hop). `process` runs in place on exactly `hop` samples.
    #[inline]
    pub fn tick<F: FnOnce(&mut [f32])>(&mut self, x: f32, process: F) -> f32 {
        self.stage[self.head] = x;
        self.head += 1;
        if self.head == self.stage.len() {
            self.head = 0;
            process(&mut self.stage);
            let len = self.ring.len();
            for k in 0..self.stage.len() {
                self.ring[self.wpos] = self.stage[k];
                self.wpos = (self.wpos + 1) % len;
                self.count += 1;
            }
        }
        if self.count > 0 {
            let y = self.ring[self.rpos];
            self.rpos = (self.rpos + 1) % self.ring.len();
            self.count -= 1;
            y
        } else {
            0.0
        }
    }
}

// ------------------------------------------------------------------ pitch detect

/// Difference-autocorrelation pitch estimate over one full window. Returns
/// `None` when the window is too quiet or not periodic enough (diff energy
/// above ~1.8x the window energy). Cost is O(window * lag range); callers run
/// it once per analysis hop, never per sample.
pub fn detect_pitch(h: &[f32], sr: f32) -> Option<f32> {
    let n = h.len();
    let energy: f32 = h.iter().map(|x| x * x).sum::<f32>() / n as f32;
    if energy < 1e-7 {
        return None;
    }
    let min_lag = (sr / 1200.0) as usize;
    let max_lag = ((sr / 60.0) as usize).min(n / 2);
    if max_lag <= min_lag {
        return None;
    }
    let mut best = f32::MAX;
    let mut best_lag = 0usize;
    for lag in min_lag..=max_lag {
        let len = n - lag;
        let mut d = 0.0f32;
        for i in 0..len {
            let diff = h[i] - h[i + lag];
            d += diff * diff;
        }
        d /= len as f32;
        if d < best {
            best = d;
            best_lag = lag;
        }
    }
    if best_lag == 0 || best > energy * 1.8 {
        return None;
    }
    Some(sr / best_lag as f32)
}
