//! Analysis effects: a capture deck, scope, 31-band spectrum, dB meter, peak
//! controller and tuner. Analyzer buffers are fixed at construction; spectrum
//! analysis runs on a 1024-sample hop and never allocates. Meter/analysis
//! values are public for the UI layer.
#![forbid(unsafe_code)]

use crate::common::{db_to_gain, detect_pitch, impl_effect, EffectBase};
use redraw_dsp::{Analyzer, DelayLine};
use redraw_shared::ParamSpec;

// ==================================================================== WaveDeck

pub static WAVEDECK_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Position", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Speed", -2.0, 2.0, 1.0, "x"),
    ParamSpec::new("Mix", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

const DECK_CAP: usize = 288_000; // 6 s at 48 kHz

/// Six-second stereo capture ring with a scrub head. `Position` picks the
/// point in history (0 = oldest, 1 = newest); `Speed` drifts the head.
pub struct WaveDeck {
    pub name: &'static str,
    pub p: [f32; WAVEDECK_PARAMS.len()],
    dl_l: DelayLine,
    dl_r: DelayLine,
    scan: f32,
    bypass: bool,
}

impl WaveDeck {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| WAVEDECK_PARAMS[i].default),
            dl_l: DelayLine::new(DECK_CAP),
            dl_r: DelayLine::new(DECK_CAP),
            scan: 0.0,
            bypass: false,
        }
    }
}

impl EffectBase for WaveDeck {
    const SPEC: &'static [ParamSpec] = WAVEDECK_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], _sr: f32, _tempo: f32) {
        let pos = self.p[0].clamp(0.0, 1.0);
        let speed = self.p[1].clamp(-2.0, 2.0);
        let mix = self.p[2].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[3]);
        let base = (1.0 - pos) * (DECK_CAP - 2) as f32;
        let n = left.len().min(right.len());
        for i in 0..n {
            self.dl_l.push(left[i]);
            self.dl_r.push(right[i]);
            self.scan += speed - 1.0;
            self.scan = self.scan.clamp(-(DECK_CAP as f32 - 2.0), DECK_CAP as f32 - 2.0);
            let d = (base + self.scan).clamp(1.0, (DECK_CAP - 2) as f32);
            let wl = self.dl_l.read(d);
            let wr = self.dl_r.read(d);
            left[i] = (left[i] * (1.0 - mix) + wl * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + wr * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.dl_l.clear();
        self.dl_r.clear();
        self.scan = 0.0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================= ScopeViewer

pub static SCOPE_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Scale", 1.0, 16.0, 4.0, "x"),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

const SCOPE_POINTS: usize = 1024;

/// Passthrough scope: keeps a decimated ring of recent samples for the editor
/// view. `Scale` is display-only and does not touch audio.
pub struct ScopeViewer {
    pub name: &'static str,
    pub p: [f32; SCOPE_PARAMS.len()],
    pub scope: Vec<f32>,
    pub scope_pos: usize,
    counter: usize,
    bypass: bool,
}

impl ScopeViewer {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| SCOPE_PARAMS[i].default),
            scope: vec![0.0; SCOPE_POINTS],
            scope_pos: 0,
            counter: 0,
            bypass: false,
        }
    }
}

impl EffectBase for ScopeViewer {
    const SPEC: &'static [ParamSpec] = SCOPE_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], _sr: f32, _tempo: f32) {
        let out = db_to_gain(self.p[1]);
        let n = left.len().min(right.len());
        for i in 0..n {
            self.counter += 1;
            if self.counter % 64 == 0 {
                self.scope[self.scope_pos] = (left[i] + right[i]) * 0.5;
                self.scope_pos = (self.scope_pos + 1) % SCOPE_POINTS;
            }
            left[i] *= out;
            right[i] *= out;
        }
    }
    fn reset_fx(&mut self) {
        self.scope.fill(0.0);
        self.scope_pos = 0;
        self.counter = 0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================= Spectrum31

pub static SPECTRUM_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Smoothing", 0.0, 0.99, 0.6, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

const SPEC_BANDS: usize = 31;
const SPEC_FFT: usize = 2048;

/// 31 log bands from 20 Hz to 20 kHz over a 2048-point Hann FFT.
pub struct Spectrum31 {
    pub name: &'static str,
    pub p: [f32; SPECTRUM_PARAMS.len()],
    an: Analyzer,
    hist: Vec<f32>,
    pos: usize,
    blo: [usize; SPEC_BANDS],
    bhi: [usize; SPEC_BANDS],
    edges_sr: f32,
    pub bands: [f32; SPEC_BANDS],
    bypass: bool,
}

impl Spectrum31 {
    pub fn new(name: &'static str) -> Self {
        let mut fx = Self {
            name,
            p: std::array::from_fn(|i| SPECTRUM_PARAMS[i].default),
            an: Analyzer::new(SPEC_FFT),
            hist: vec![0.0; SPEC_FFT],
            pos: 0,
            blo: [0; SPEC_BANDS],
            bhi: [1; SPEC_BANDS],
            edges_sr: 0.0,
            bands: [0.0; SPEC_BANDS],
            bypass: false,
        };
        fx.edges(48_000.0);
        fx
    }
    fn edges(&mut self, sr: f32) {
        if (sr - self.edges_sr).abs() < 1.0 {
            return;
        }
        self.edges_sr = sr;
        let lo = 20.0f32;
        let hi = 20_000.0f32;
        let ratio = (hi / lo).powf(1.0 / SPEC_BANDS as f32);
        let bin_hz = sr / SPEC_FFT as f32;
        for b in 0..SPEC_BANDS {
            let f0 = lo * ratio.powf(b as f32);
            let f1 = lo * ratio.powf(b as f32 + 1.0);
            let lo_bin = (f0 / bin_hz).floor() as usize;
            let hi_bin = (f1 / bin_hz).ceil() as usize;
            self.blo[b] = lo_bin.min(SPEC_FFT / 2 - 1);
            self.bhi[b] = hi_bin.max(self.blo[b] + 1).min(SPEC_FFT / 2);
        }
    }
}

impl EffectBase for Spectrum31 {
    const SPEC: &'static [ParamSpec] = SPECTRUM_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, _tempo: f32) {
        self.edges(sr);
        let smooth = self.p[0].clamp(0.0, 0.99);
        let out = db_to_gain(self.p[1]);
        let n = left.len().min(right.len());
        for i in 0..n {
            self.hist[self.pos] = (left[i] + right[i]) * 0.5;
            self.pos += 1;
            if self.pos >= SPEC_FFT {
                let mags = self.an.analyze(&self.hist);
                for b in 0..SPEC_BANDS {
                    let lo = self.blo[b];
                    let hi = self.bhi[b];
                    let mut m = 0.0f32;
                    for k in lo..hi {
                        if let Some(v) = mags.get(k) {
                            if *v > m {
                                m = *v;
                            }
                        }
                    }
                    self.bands[b] = m + (self.bands[b] - m) * smooth;
                }
                let half = SPEC_FFT / 2;
                self.hist.copy_within(half.., 0);
                self.pos = half;
            }
            left[i] *= out;
            right[i] *= out;
        }
    }
    fn reset_fx(&mut self) {
        self.hist.fill(0.0);
        self.pos = 0;
        self.bands = [0.0; SPEC_BANDS];
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ===================================================================== MeterDb

pub static METER_PARAMS: &[ParamSpec] = &[ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB")];

/// Peak/RMS meter with public values for the mixer and editor displays.
pub struct MeterDb {
    pub name: &'static str,
    pub p: [f32; METER_PARAMS.len()],
    pub peak_l: f32,
    pub peak_r: f32,
    pub rms_l: f32,
    pub rms_r: f32,
    bypass: bool,
}

impl MeterDb {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| METER_PARAMS[i].default),
            peak_l: 0.0,
            peak_r: 0.0,
            rms_l: 0.0,
            rms_r: 0.0,
            bypass: false,
        }
    }
}

impl EffectBase for MeterDb {
    const SPEC: &'static [ParamSpec] = METER_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], _sr: f32, _tempo: f32) {
        let out = db_to_gain(self.p[0]);
        let n = left.len().min(right.len());
        let mut pl = 0.0f32;
        let mut pr = 0.0f32;
        let mut sl = 0.0f32;
        let mut sr2 = 0.0f32;
        for i in 0..n {
            pl = pl.max(left[i].abs());
            pr = pr.max(right[i].abs());
            sl += left[i] * left[i];
            sr2 += right[i] * right[i];
            left[i] *= out;
            right[i] *= out;
        }
        let inv = if n > 0 { 1.0 / n as f32 } else { 0.0 };
        self.peak_l = pl;
        self.peak_r = pr;
        self.rms_l = (sl * inv).sqrt();
        self.rms_r = (sr2 * inv).sqrt();
    }
    fn reset_fx(&mut self) {
        self.peak_l = 0.0;
        self.peak_r = 0.0;
        self.rms_l = 0.0;
        self.rms_r = 0.0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ==================================================================== PeakMod

pub static PEAKMOD_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Base", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Gain", 0.0, 4.0, 1.0, "x"),
    ParamSpec::new("Attack", 1.0, 200.0, 15.0, "ms"),
    ParamSpec::new("Release", 10.0, 1000.0, 200.0, "ms"),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Envelope follower exposing its level as a modulation source through
/// `Effect::controller_value`; audio passes through.
pub struct PeakMod {
    pub name: &'static str,
    pub p: [f32; PEAKMOD_PARAMS.len()],
    pub env: f32,
    bypass: bool,
}

impl PeakMod {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| PEAKMOD_PARAMS[i].default),
            env: 0.0,
            bypass: false,
        }
    }
}

impl EffectBase for PeakMod {
    const SPEC: &'static [ParamSpec] = PEAKMOD_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, _tempo: f32) {
        let atk = (-1.0 / (self.p[2].clamp(1.0, 200.0) * 0.001 * sr).max(1.0)).exp();
        let rel = (-1.0 / (self.p[3].clamp(10.0, 1000.0) * 0.001 * sr).max(1.0)).exp();
        let out = db_to_gain(self.p[4]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let x = (left[i].abs() + right[i].abs()) * 0.5;
            let coef = if x > self.env { atk } else { rel };
            self.env = x + (self.env - x) * coef;
            left[i] *= out;
            right[i] *= out;
        }
    }
    fn reset_fx(&mut self) {
        self.env = 0.0;
    }
    fn mod_value(&self) -> f32 {
        (self.p[0] + self.env * self.p[1]).clamp(0.0, 1.0)
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ======================================================================= Tuner

pub static TUNER_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Reference", 415.0, 466.0, 440.0, "Hz"),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Autocorrelation tuner with public detected frequency and cents error for
/// the editor display. Passes audio through.
pub struct Tuner {
    pub name: &'static str,
    pub p: [f32; TUNER_PARAMS.len()],
    hist: Vec<f32>,
    pos: usize,
    pub detected_hz: f32,
    pub cents: f32,
    bypass: bool,
}

impl Tuner {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| TUNER_PARAMS[i].default),
            hist: vec![0.0; 2048],
            pos: 0,
            detected_hz: 0.0,
            cents: 0.0,
            bypass: false,
        }
    }
}

impl EffectBase for Tuner {
    const SPEC: &'static [ParamSpec] = TUNER_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, _tempo: f32) {
        let out = db_to_gain(self.p[1]);
        let n = left.len().min(right.len());
        for i in 0..n {
            self.hist[self.pos] = (left[i] + right[i]) * 0.5;
            self.pos += 1;
            if self.pos >= self.hist.len() {
                if let Some(hz) = detect_pitch(&self.hist, sr) {
                    self.detected_hz = hz;
                    let midi = 69.0 + 12.0 * (hz / self.p[0].max(1.0)).log2();
                    let note = midi.round();
                    let ref_hz = self.p[0].max(1.0) * 2f32.powf((note - 69.0) / 12.0);
                    self.cents = (1200.0 * (hz / ref_hz).log2()).clamp(-50.0, 50.0);
                }
                let half = self.hist.len() / 2;
                self.hist.copy_within(half.., 0);
                self.pos = half;
            }
            left[i] *= out;
            right[i] *= out;
        }
    }
    fn reset_fx(&mut self) {
        self.hist.fill(0.0);
        self.pos = 0;
        self.detected_hz = 0.0;
        self.cents = 0.0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

impl_effect!(WaveDeck);
impl_effect!(ScopeViewer);
impl_effect!(Spectrum31);
impl_effect!(MeterDb);
impl_effect!(PeakMod);
impl_effect!(Tuner);
