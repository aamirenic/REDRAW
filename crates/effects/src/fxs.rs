//! Modulation effects: choruses, phasers, flanger and chorus-flanger, auto
//! pan, stereo image tools and the two-unit philter. Modulated voices share
//! the ModVoice/ApfStage cores; LFO state advances sample-major.
#![forbid(unsafe_code)]

use crate::common::{
    db_to_gain, impl_effect, mono_to_ms, ms_to_mono, ApfStage, BandSplit, EffectBase, ModVoice,
    SBiquad,
};
use redraw_dsp::{BiquadCoeffs, FilterKind, Lfo, LfoShape, Svf, SvfMode, TWO_PI, wrap01};
use redraw_shared::{pan_gains, ParamCurve, ParamSpec};

// ================================================================== Chorus 3

pub static CHORUS3_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Rate", 0.05, 8.0, 0.6, "Hz"),
    ParamSpec::new("Depth", 0.0, 1.0, 0.35, ""),
    ParamSpec::new("Spread", 0.0, 1.0, 0.7, ""),
    ParamSpec::new("Feedback", 0.0, 0.8, 0.0, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct Chorus3 {
    pub name: &'static str,
    pub p: [f32; CHORUS3_PARAMS.len()],
    voices: [ModVoice; 3],
    bypass: bool,
}

impl Chorus3 {
    pub fn new(name: &'static str) -> Self {
        let mut fx = Self {
            name,
            p: std::array::from_fn(|i| CHORUS3_PARAMS[i].default),
            voices: [ModVoice::new(0xC1), ModVoice::new(0xC2), ModVoice::new(0xC3)],
            bypass: false,
        };
        for v in fx.voices.iter_mut() {
            v.base_ms = 22.0;
        }
        fx
    }
}

impl EffectBase for Chorus3 {
    const SPEC: &'static [ParamSpec] = CHORUS3_PARAMS;
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
        let rate = self.p[0].clamp(0.05, 8.0);
        let depth_ms = self.p[1].clamp(0.0, 1.0) * 8.0;
        let spread = self.p[2].clamp(0.0, 1.0);
        let fb = self.p[3].clamp(0.0, 0.8);
        let mix = self.p[4].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[5]);
        let n = self.voices.len();
        for (i, v) in self.voices.iter_mut().enumerate() {
            v.rate = (rate * (1.0 + spread * (i as f32 - (n as f32 - 1.0) * 0.5) * 0.22)).max(0.02);
            v.depth_ms = depth_ms;
            v.feedback = fb;
        }
        let weight = 1.0 / (n as f32).sqrt();
        for i in 0..left.len().min(right.len()) {
            let mut wl = 0.0f32;
            let mut wr = 0.0f32;
            for (i2, v) in self.voices.iter_mut().enumerate() {
                let (w, _dry) = v.tick(left[i], right[i], sr);
                let pan = if i2 % 2 == 0 { 0.6 } else { -0.6 };
                let (pl, pr) = pan_gains(pan);
                wl += w * pl * weight;
                wr += w * pr * weight;
            }
            left[i] = (left[i] * (1.0 - mix) + wl * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + wr * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        for v in self.voices.iter_mut() {
            v.reset();
        }
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================== Chorus 8

pub static CHORUS8_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Rate", 0.05, 8.0, 0.35, "Hz"),
    ParamSpec::new("Depth", 0.0, 1.0, 0.45, ""),
    ParamSpec::new("Spread", 0.0, 1.0, 0.9, ""),
    ParamSpec::new("Feedback", 0.0, 0.8, 0.0, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.6, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct Chorus8 {
    pub name: &'static str,
    pub p: [f32; CHORUS8_PARAMS.len()],
    voices: [ModVoice; 8],
    bypass: bool,
}

impl Chorus8 {
    pub fn new(name: &'static str) -> Self {
        let mut fx = Self {
            name,
            p: std::array::from_fn(|i| CHORUS8_PARAMS[i].default),
            voices: std::array::from_fn(|i| ModVoice::new(0xD0 + i as u64)),
            bypass: false,
        };
        for v in fx.voices.iter_mut() {
            v.base_ms = 26.0;
        }
        fx
    }
}

impl EffectBase for Chorus8 {
    const SPEC: &'static [ParamSpec] = CHORUS8_PARAMS;
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
        let rate = self.p[0].clamp(0.05, 8.0);
        let depth_ms = self.p[1].clamp(0.0, 1.0) * 10.0;
        let spread = self.p[2].clamp(0.0, 1.0);
        let fb = self.p[3].clamp(0.0, 0.8);
        let mix = self.p[4].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[5]);
        let n = self.voices.len();
        for (i, v) in self.voices.iter_mut().enumerate() {
            v.rate = (rate * (1.0 + spread * (i as f32 - (n as f32 - 1.0) * 0.5) * 0.14)).max(0.02);
            v.depth_ms = depth_ms;
            v.feedback = fb;
        }
        let weight = 1.0 / (n as f32).sqrt();
        for i in 0..left.len().min(right.len()) {
            let mut wl = 0.0f32;
            let mut wr = 0.0f32;
            for (i2, v) in self.voices.iter_mut().enumerate() {
                let (w, _dry) = v.tick(left[i], right[i], sr);
                let pan = if i2 % 2 == 0 { 0.55 } else { -0.55 };
                let (pl, pr) = pan_gains(pan);
                wl += w * pl * weight;
                wr += w * pr * weight;
            }
            left[i] = (left[i] * (1.0 - mix) + wl * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + wr * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        for v in self.voices.iter_mut() {
            v.reset();
        }
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ============================================================ Vintage Chorus

pub static VCHORUS_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Rate", 0.05, 4.0, 0.3, "Hz"),
    ParamSpec::new("Depth", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.4, ""),
    ParamSpec::new("Era", 0.0, 1.0, 0.4, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct VintageChorus {
    pub name: &'static str,
    pub p: [f32; VCHORUS_PARAMS.len()],
    voices: [ModVoice; 4],
    tone: SBiquad,
    bypass: bool,
    dirty: bool,
}

impl VintageChorus {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| VCHORUS_PARAMS[i].default),
            voices: std::array::from_fn(|i| ModVoice::new(0xE0 + i as u64)),
            tone: SBiquad::new(),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        let era = self.p[3].clamp(0.0, 1.0);
        self.tone.set(BiquadCoeffs::compute(
            FilterKind::LowPass,
            16_000.0 - era * 11_000.0,
            0.7,
            0.0,
            sr,
        ));
        for v in self.voices.iter_mut() {
            v.base_ms = 30.0;
        }
    }
}

impl EffectBase for VintageChorus {
    const SPEC: &'static [ParamSpec] = VCHORUS_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn param_changed(&mut self, index: usize) {
        if index == 3 {
            self.dirty = true;
        }
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, _tempo: f32) {
        if self.dirty {
            self.recompute(sr);
        }
        let rate = self.p[0].clamp(0.05, 4.0);
        let depth_ms = self.p[1].clamp(0.0, 1.0) * 9.0;
        let mix = self.p[2].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[4]);
        for (i, v) in self.voices.iter_mut().enumerate() {
            v.rate = rate * (1.0 + i as f32 * 0.11);
            v.depth_ms = depth_ms;
        }
        let weight = 1.0 / (self.voices.len() as f32).sqrt();
        for i in 0..left.len().min(right.len()) {
            let mut wl = 0.0f32;
            let mut wr = 0.0f32;
            for (i2, v) in self.voices.iter_mut().enumerate() {
                let (w, _dry) = v.tick(left[i], right[i], sr);
                let pan = if i2 % 2 == 0 { 0.4 } else { -0.4 };
                let (pl, pr) = pan_gains(pan);
                wl += w * pl * weight;
                wr += w * pr * weight;
            }
            let m = self.tone.tick(wl, wr);
            left[i] = (left[i] * (1.0 - mix) + m.0 * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + m.1 * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        for v in self.voices.iter_mut() {
            v.reset();
        }
        self.tone.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================== Phaser 6

pub static PHASER6_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Rate", 0.05, 8.0, 0.5, "Hz"),
    ParamSpec::new("Depth", 0.0, 1.0, 0.7, ""),
    ParamSpec::new("Feedback", 0.0, 0.8, 0.2, ""),
    ParamSpec::new("Spread", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct Phaser6 {
    pub name: &'static str,
    pub p: [f32; PHASER6_PARAMS.len()],
    stages: [ApfStage; 6],
    fb: f32,
    phase: f32,
    bypass: bool,
}

impl Phaser6 {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| PHASER6_PARAMS[i].default),
            stages: [ApfStage::default(); 6],
            fb: 0.0,
            phase: 0.0,
            bypass: false,
        }
    }
}

impl EffectBase for Phaser6 {
    const SPEC: &'static [ParamSpec] = PHASER6_PARAMS;
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
        let rate = self.p[0].clamp(0.05, 8.0);
        let depth = self.p[1].clamp(0.0, 1.0);
        let fb = self.p[2].clamp(0.0, 0.8);
        let spread = self.p[3].clamp(0.0, 1.0);
        let mix = self.p[4].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[5]);
        let n = left.len().min(right.len());
        for i in 0..n {
            self.phase = wrap01(self.phase + rate / sr);
            let lf = (TWO_PI * self.phase).sin();
            let dryl = left[i];
            let dryr = right[i];
            let mut xl = dryl + self.fb * fb;
            let mut xr = dryr + self.fb * fb;
            let mut wl = 0.0;
            let mut wr = 0.0;
            for (k, st) in self.stages.iter_mut().enumerate() {
                let center = 260.0 * 2f32.powf(spread * k as f32 * 0.22);
                let fc = (center * 2f32.powf(lf * depth * 2.0)).clamp(70.0, sr * 0.45);
                let t = (std::f32::consts::PI * fc / sr).tan();
                st.a = (1.0 - t) / (1.0 + t);
                wl = st.tick(xl);
                wr = st.tick(xr);
                xl = wl;
                xr = wr;
            }
            self.fb = 0.5 * (wl + wr);
            left[i] = (dryl * (1.0 - mix) + wl * mix) * out;
            right[i] = (dryr * (1.0 - mix) + wr * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        for st in self.stages.iter_mut() {
            st.reset();
        }
        self.fb = 0.0;
        self.phase = 0.0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ============================================================ Vintage Phaser

pub static VPHASER_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Rate", 0.05, 8.0, 0.25, "Hz"),
    ParamSpec::new("Depth", 0.0, 1.0, 0.6, ""),
    ParamSpec::new("Feedback", 0.0, 0.8, 0.35, ""),
    ParamSpec::new("Stagger", 0.0, 1.0, 0.6, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct VintagePhaser {
    pub name: &'static str,
    pub p: [f32; VPHASER_PARAMS.len()],
    stages: [ApfStage; 8],
    fb: f32,
    phase: f32,
    bypass: bool,
}

impl VintagePhaser {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| VPHASER_PARAMS[i].default),
            stages: [ApfStage::default(); 8],
            fb: 0.0,
            phase: 0.0,
            bypass: false,
        }
    }
}

impl EffectBase for VintagePhaser {
    const SPEC: &'static [ParamSpec] = VPHASER_PARAMS;
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
        let rate = self.p[0].clamp(0.05, 8.0);
        let depth = self.p[1].clamp(0.0, 1.0);
        let fb_amt = self.p[2].clamp(0.0, 0.8);
        let stagger = self.p[3].clamp(0.0, 1.0);
        let mix = self.p[4].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[5]);
        let n = left.len().min(right.len());
        for i in 0..n {
            self.phase = wrap01(self.phase + rate / sr);
            let lf = (TWO_PI * self.phase).sin();
            // asymmetric sweep: saw-ish lean on the LFO for vintage character
            let lean = 0.5 - 0.5 * (TWO_PI * self.phase * 0.5).cos();
            let shape = lf * 0.85 + (lean * 2.0 - 1.0) * 0.15;
            let mut xl = left[i] + self.fb * fb_amt;
            let mut xr = right[i] + self.fb * fb_amt;
            let mut wl = 0.0;
            let mut wr = 0.0;
            for (k, st) in self.stages.iter_mut().enumerate() {
                let center = 220.0 * (1.0 + stagger * k as f32 * 0.13);
                let fc = (center * 2f32.powf(shape * depth * 2.2)).clamp(60.0, sr * 0.45);
                let t = (std::f32::consts::PI * fc / sr).tan();
                st.a = (1.0 - t) / (1.0 + t);
                wl = st.tick(xl);
                wr = st.tick(xr);
                xl = wl;
                xr = wr;
            }
            self.fb = 0.5 * (wl + wr);
            left[i] = (left[i] * (1.0 - mix) + wl * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + wr * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        for st in self.stages.iter_mut() {
            st.reset();
        }
        self.fb = 0.0;
        self.phase = 0.0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================ Flanger TZ

pub static FLTZ_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Rate", 0.05, 8.0, 0.2, "Hz"),
    ParamSpec::new("Depth", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Delay", 0.3, 15.0, 5.0, "ms"),
    ParamSpec::new("Feedback", 0.0, 0.9, 0.3, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Jet flanger whose modulated delay sweeps to (near) zero, standing in for
/// the dual-line through-zero crossing until the phase-split core expands.
pub struct FlangerTz {
    pub name: &'static str,
    pub p: [f32; FLTZ_PARAMS.len()],
    dl: redraw_dsp::DelayLine,
    damp: redraw_dsp::OnePole,
    phase: f32,
    bypass: bool,
}

impl FlangerTz {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| FLTZ_PARAMS[i].default),
            dl: redraw_dsp::DelayLine::new(1600),
            damp: redraw_dsp::OnePole::new(),
            phase: 0.0,
            bypass: false,
        }
    }
}

impl EffectBase for FlangerTz {
    const SPEC: &'static [ParamSpec] = FLTZ_PARAMS;
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
        let rate = self.p[0].clamp(0.05, 8.0);
        let depth = self.p[1].clamp(0.0, 1.0);
        let base = self.p[2].clamp(0.3, 15.0);
        let fb_amt = self.p[3].clamp(0.0, 0.9);
        let mix = self.p[4].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[5]);
        self.damp.set_lowpass(10_000.0, sr);
        let n = left.len().min(right.len());
        for i in 0..n {
            self.phase = wrap01(self.phase + rate / sr);
            let wob = (TWO_PI * self.phase).sin();
            let mono = (left[i] + right[i]) * 0.5;
            let d = (base * (1.0 - 0.92 * depth) * 0.5 + base * (0.5 + 0.5 * wob) * depth
                * 0.92)
                .max(0.03)
                * 0.001
                * sr;
            let d = d.clamp(1.0, 1590.0);
            let wet = self.dl.read(d);
            let fb_sig = self.damp.tick(wet) * fb_amt;
            self.dl.push(mono + fb_sig);
            left[i] = (left[i] * (1.0 - mix) + wet * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + wet * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.dl.clear();
        self.damp.reset();
        self.phase = 0.0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ============================================================ Flanger Chorus

pub static FLCH_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Flanger Rate", 0.05, 8.0, 0.3, "Hz"),
    ParamSpec::new("Flanger Depth", 0.0, 1.0, 0.4, ""),
    ParamSpec::new("Chorus Rate", 0.05, 8.0, 0.8, "Hz"),
    ParamSpec::new("Chorus Mix", 0.0, 1.0, 0.4, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.55, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct FlangerChorus {
    pub name: &'static str,
    pub p: [f32; FLCH_PARAMS.len()],
    dl: redraw_dsp::DelayLine,
    voices: [ModVoice; 3],
    fb: f32,
    phase: f32,
    bypass: bool,
}

impl FlangerChorus {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| FLCH_PARAMS[i].default),
            dl: redraw_dsp::DelayLine::new(1200),
            voices: std::array::from_fn(|i| ModVoice::new(0xF0 + i as u64)),
            fb: 0.0,
            phase: 0.0,
            bypass: false,
        }
    }
}

impl EffectBase for FlangerChorus {
    const SPEC: &'static [ParamSpec] = FLCH_PARAMS;
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
        let fr = self.p[0].clamp(0.05, 8.0);
        let fd = self.p[1].clamp(0.0, 1.0);
        let cr = self.p[2].clamp(0.05, 8.0);
        let cm = self.p[3].clamp(0.0, 1.0);
        let mix = self.p[4].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[5]);
        let weight = 1.0 / (self.voices.len() as f32).sqrt();
        for v in self.voices.iter_mut() {
            v.rate = cr;
            v.base_ms = 24.0;
            v.depth_ms = 4.0;
        }
        let n = left.len().min(right.len());
        for i in 0..n {
            self.phase = wrap01(self.phase + fr / sr);
            let wob = (TWO_PI * self.phase).sin();
            let mono = (left[i] + right[i]) * 0.5;
            let d = ((3.0 + fd * 3.0 * (0.5 + 0.5 * wob)) * 0.001 * sr).clamp(1.0, 1180.0);
            let fw = self.dl.read(d);
            self.fb = fw * 0.25;
            self.dl.push(mono + self.fb);
            let mut cl = 0.0f32;
            let mut crs = 0.0f32;
            for (i2, v) in self.voices.iter_mut().enumerate() {
                let (w, _dry) = v.tick(left[i], right[i], sr);
                let pan = if i2 % 2 == 0 { 0.5 } else { -0.5 };
                let (pl, pr) = pan_gains(pan);
                cl += w * pl * weight;
                crs += w * pr * weight;
            }
            left[i] = (left[i] * (1.0 - mix)
                + (fw * 0.35 + cl * cm) * mix)
                * out;
            right[i] = (right[i] * (1.0 - mix)
                + (fw * 0.35 + crs * cm) * mix)
                * out;
        }
    }
    fn reset_fx(&mut self) {
        self.dl.clear();
        for v in self.voices.iter_mut() {
            v.reset();
        }
        self.fb = 0.0;
        self.phase = 0.0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// =================================================================== Auto Pan

pub static AUTOPAN_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Rate", 0.05, 8.0, 1.0, "Hz"),
    ParamSpec::new("Depth", 0.0, 1.0, 0.7, ""),
    ParamSpec::new("Shape", 0.0, 4.0, 0.0, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct AutoPan {
    pub name: &'static str,
    pub p: [f32; AUTOPAN_PARAMS.len()],
    lfo: Lfo,
    bypass: bool,
    dirty: bool,
}

impl AutoPan {
    pub fn new(name: &'static str) -> Self {
        let mut lfo = Lfo::new(0xA1);
        lfo.shape = LfoShape::Sine;
        Self {
            name,
            p: std::array::from_fn(|i| AUTOPAN_PARAMS[i].default),
            lfo,
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self) {
        self.dirty = false;
        self.lfo.shape = LfoShape::from_index(self.p[2].clamp(0.0, 4.0) as usize);
    }
}

impl EffectBase for AutoPan {
    const SPEC: &'static [ParamSpec] = AUTOPAN_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn param_changed(&mut self, index: usize) {
        if index == 2 {
            self.dirty = true;
        }
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, _tempo: f32) {
        if self.dirty {
            self.recompute();
        }
        let rate = self.p[0].clamp(0.05, 8.0);
        let depth = self.p[1].clamp(0.0, 1.0);
        let mix = self.p[3].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[4]);
        self.lfo.set_rate(rate);
        let n = left.len().min(right.len());
        for i in 0..n {
            let lf = self.lfo.tick(sr);
            let pan = lf * depth;
            let (pl, pr) = pan_gains(pan.clamp(-1.0, 1.0) * 0.8);
            left[i] = (left[i] * (1.0 - mix) + left[i] * pl * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + right[i] * pr * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.lfo.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// =============================================================== Stereo Wide

pub static WIDE_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Width", 0.0, 2.0, 0.6, ""),
    ParamSpec::new("Haas", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct StereoWide {
    pub name: &'static str,
    pub p: [f32; WIDE_PARAMS.len()],
    dl_r: redraw_dsp::DelayLine,
    bypass: bool,
}

impl StereoWide {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| WIDE_PARAMS[i].default),
            dl_r: redraw_dsp::DelayLine::new(1300),
            bypass: false,
        }
    }
}

impl EffectBase for StereoWide {
    const SPEC: &'static [ParamSpec] = WIDE_PARAMS;
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
        let width = self.p[0].clamp(0.0, 2.0);
        let haas = self.p[1].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[2]);
        let haas_s = (haas * 0.02 * sr).clamp(1.0, 1280.0);
        let n = left.len().min(right.len());
        for i in 0..n {
            let (m, s) = mono_to_ms(left[i], right[i]);
            let s2 = s * width;
            let (l, r2) = ms_to_mono(m, s2);
            let r_delayed = self.dl_r.read(haas_s);
            self.dl_r.push(r2);
            let r = if haas > 0.001 { r_delayed } else { r2 };
            left[i] = l * out;
            right[i] = r * out;
        }
    }
    fn reset_fx(&mut self) {
        self.dl_r.clear();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ============================================================= Stereo Shaper

pub static SSHAPER_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Mode", 0.0, 3.0, 0.0, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct StereoShaper {
    pub name: &'static str,
    pub p: [f32; SSHAPER_PARAMS.len()],
    bypass: bool,
}

impl StereoShaper {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| SSHAPER_PARAMS[i].default),
            bypass: false,
        }
    }
}

impl EffectBase for StereoShaper {
    const SPEC: &'static [ParamSpec] = SSHAPER_PARAMS;
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
        let mode = (self.p[0] as u32) % 4;
        let mix = self.p[1].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[2]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let dl = left[i];
            let dr = right[i];
            let (l, r) = match mode {
                0 => (dl, dr),
                1 => (dr, dl),
                2 => (-dl, -dr),
                _ => (dl, -dr),
            };
            left[i] = (dl * (1.0 - mix) + l * mix) * out;
            right[i] = (dr * (1.0 - mix) + r * mix) * out;
        }
    }
    fn reset_fx(&mut self) {}
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ============================================================== Band Spreader

pub static BSPREAD_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Depth", 0.0, 1.0, 0.4, ""),
    ParamSpec::new("Split", 500.0, 8000.0, 3000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Mix", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Widens the high band only (mid/side expansion above the split point).
pub struct BandSpreader {
    pub name: &'static str,
    pub p: [f32; BSPREAD_PARAMS.len()],
    split: BandSplit,
    bypass: bool,
    dirty: bool,
}

impl BandSpreader {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| BSPREAD_PARAMS[i].default),
            split: BandSplit::new(3000.0, 6000.0),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        self.split.set_edges(
            self.p[1].min(sr * 0.42),
            (self.p[1] * 2.0).min(sr * 0.46),
            sr,
        );
    }
}

impl EffectBase for BandSpreader {
    const SPEC: &'static [ParamSpec] = BSPREAD_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn param_changed(&mut self, index: usize) {
        if index == 1 {
            self.dirty = true;
        }
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, _tempo: f32) {
        if self.dirty {
            self.recompute(sr);
        }
        let depth = self.p[0].clamp(0.0, 1.0);
        let mix = self.p[2].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[3]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let (bl, br) = self.split.split_stereo(left[i], right[i]);
            let (m, s) = ((bl[2] + br[2]) * 0.5, (bl[2] - br[2]) * 0.5);
            let hi_l = m + s * (1.0 + depth * 2.0);
            let hi_r = m - s * (1.0 + depth * 2.0);
            let l = bl[0] + bl[1] + hi_l;
            let r = br[0] + br[1] + hi_r;
            left[i] = (left[i] * (1.0 - mix) + l * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + r * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.split.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// =================================================================== FilterLoom

pub static PHILTER_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Cut A", 60.0, 8000.0, 500.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Cut B", 60.0, 8000.0, 1200.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Resonance", 0.0, 1.0, 0.8, ""),
    ParamSpec::new("Env Amount", 0.0, 1.0, 0.4, ""),
    ParamSpec::new("LFO Rate", 0.01, 8.0, 0.3, "Hz"),
    ParamSpec::new("LFO Depth", 0.0, 1.0, 0.2, ""),
    ParamSpec::new("Drive", 1.0, 8.0, 2.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Two self-resonant filter units (low-pass and notch) driven by an envelope
/// follower and the LFO together — the stand-in for an 8-unit philter bank.
pub struct FilterLoom {
    pub name: &'static str,
    pub p: [f32; PHILTER_PARAMS.len()],
    svf_a: Svf,
    svf_b: Svf,
    env: crate::common::EnvFollower,
    lfo: Lfo,
    bypass: bool,
}

impl FilterLoom {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| PHILTER_PARAMS[i].default),
            svf_a: Svf::new(),
            svf_b: Svf::new(),
            env: crate::common::EnvFollower {
                state: 0.0,
                attack_ms: 1.0,
                release_ms: 60.0,
            },
            lfo: Lfo::new(0xA9),
            bypass: false,
        }
    }
}

impl EffectBase for FilterLoom {
    const SPEC: &'static [ParamSpec] = PHILTER_PARAMS;
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
        let res = 0.5 + self.p[2].clamp(0.0, 1.0) * 11.0;
        let env_amt = self.p[3].clamp(0.0, 1.0);
        let depth = self.p[5].clamp(0.0, 1.0);
        let drive = self.p[6].clamp(1.0, 8.0);
        let out = db_to_gain(self.p[7]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let det = env_follower_tick(&mut self.env, left[i], right[i], sr);
            let lf = self.lfo.tick(sr);
            let base_a = self.p[0].min(sr * 0.45);
            let base_b = self.p[1].min(sr * 0.45);
            let fca = (base_a * 2f32.powf(det * env_amt * 3.0 + lf * depth * 2.0))
                .clamp(60.0, sr * 0.45);
            let fcb = (base_b * 2f32.powf(det * env_amt * 2.0 + lf * depth * 2.5))
                .clamp(60.0, sr * 0.45);
            self.svf_a.set(fca, res, sr);
            self.svf_b.set(fcb, res, sr);
            let m = (left[i] + right[i]) * 0.5;
            let a = self.svf_a.tick_mode(m, SvfMode::Low);
            let b = self.svf_b.tick_mode(m, SvfMode::Notch);
            let y = redraw_dsp::tanh_dist((a + b) * drive * 0.7, 1.2);
            left[i] = y * out;
            right[i] = y * out;
        }
    }
    fn reset_fx(&mut self) {
        self.svf_a.reset();
        self.svf_b.reset();
        self.env.reset();
        self.lfo.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

#[inline]
fn env_follower_tick(
    env: &mut crate::common::EnvFollower,
    l: f32,
    r: f32,
    sr: f32,
) -> f32 {
    env.tick((l + r) * 0.5, sr)
}

impl_effect!(Chorus3);
impl_effect!(Chorus8);
impl_effect!(VintageChorus);
impl_effect!(Phaser6);
impl_effect!(VintagePhaser);
impl_effect!(FlangerTz);
impl_effect!(FlangerChorus);
impl_effect!(AutoPan);
impl_effect!(StereoWide);
impl_effect!(StereoShaper);
impl_effect!(BandSpreader);
impl_effect!(FilterLoom);
