//! Distortion effects: soft clip, overdrive, foldback, staged destruction,
//! pedal-chain emulation, crush, an editable wave-shaper curve and a live
//! performance stomp. Shapes come from the dsp wave functions plus the
//! CurveShaper/Crusher cores in common.
#![forbid(unsafe_code)]

use crate::common::{
    curve_from_points, db_to_gain, impl_effect, Crusher as CrusherCore, CurveShaper, EffectBase,
    EnvFollower, ModVoice, SBiquad,
};
use redraw_dsp::{foldback, soft_clip, tanh_dist, BiquadCoeffs, Compressor, FilterKind, Svf, SvfMode};
use redraw_shared::{ParamCurve, ParamSpec};

// ================================================================== SoftClip

pub static SOFTC_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Drive", 1.0, 12.0, 2.0, ""),
    ParamSpec::new("Ceiling", -18.0, 0.0, -0.5, "dB"),
    ParamSpec::new("Mix", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct SoftClipFx {
    pub name: &'static str,
    pub p: [f32; SOFTC_PARAMS.len()],
    bypass: bool,
}

impl SoftClipFx {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| SOFTC_PARAMS[i].default),
            bypass: false,
        }
    }
}

impl EffectBase for SoftClipFx {
    const SPEC: &'static [ParamSpec] = SOFTC_PARAMS;
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
        let drive = self.p[0].clamp(1.0, 12.0);
        let ceil = db_to_gain(self.p[1]).max(0.01);
        let mix = self.p[2].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[3]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let dc = left[i];
            let dr = right[i];
            let mut l = soft_clip(dc * drive);
            let mut r = soft_clip(dr * drive);
            l = l.clamp(-ceil, ceil);
            r = r.clamp(-ceil, ceil);
            left[i] = (dc * (1.0 - mix) + l * mix) * out;
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

// ================================================================== OverDrive

pub static OD_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Drive", 1.0, 12.0, 3.0, ""),
    ParamSpec::new("Symmetry", 0.0, 1.0, 0.2, ""),
    ParamSpec::new("Tone", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Asymmetric diode-flavored engine clothed as overdrive: HP input, saturate,
/// even-harmonic lift, tone LP.
pub struct OverDrive {
    pub name: &'static str,
    pub p: [f32; OD_PARAMS.len()],
    hp: SBiquad,
    tone: SBiquad,
    bypass: bool,
    dirty: bool,
}

impl OverDrive {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| OD_PARAMS[i].default),
            hp: SBiquad::new(),
            tone: SBiquad::new(),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        self.hp.set(BiquadCoeffs::compute(
            FilterKind::HighPass,
            50.0,
            0.7,
            0.0,
            sr,
        ));
        let tone = self.p[2].clamp(0.0, 1.0);
        self.tone.set(BiquadCoeffs::compute(
            FilterKind::LowPass,
            3000.0 + tone * 13_000.0,
            0.8,
            0.0,
            sr,
        ));
    }
}

impl EffectBase for OverDrive {
    const SPEC: &'static [ParamSpec] = OD_PARAMS;
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
            self.recompute(sr);
        }
        let drive = self.p[0].clamp(1.0, 12.0);
        let sym = self.p[1].clamp(0.0, 1.0);
        let mix = self.p[3].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[4]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let t = self.hp.tick(left[i], right[i]);
            let mut l = tanh_dist(t.0 * drive, 1.0) + sym * 0.4 * tanh_dist(t.0 * drive * 1.7 + 0.3, 1.0);
            let mut r = tanh_dist(t.1 * drive, 1.0) + sym * 0.4 * tanh_dist(t.1 * drive * 1.7 + 0.3, 1.0);
            let o = self.tone.tick(l, r);
            l = o.0 / (1.0 + sym * 0.4);
            r = o.1 / (1.0 + sym * 0.4);
            left[i] = (left[i] * (1.0 - mix) + l * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + r * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.hp.reset();
        self.tone.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// =================================================================== FastDist

pub static FASTD_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Drive", 1.0, 8.0, 2.0, ""),
    ParamSpec::new("Fold", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// One stage of tanh saturation blended with wave folding.
pub struct FastDist {
    pub name: &'static str,
    pub p: [f32; FASTD_PARAMS.len()],
    bypass: bool,
}

impl FastDist {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| FASTD_PARAMS[i].default),
            bypass: false,
        }
    }
}

impl EffectBase for FastDist {
    const SPEC: &'static [ParamSpec] = FASTD_PARAMS;
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
        let drive = self.p[0].clamp(1.0, 8.0);
        let fold = self.p[1].clamp(0.0, 1.0);
        let mix = self.p[2].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[3]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let dc = left[i];
            let dr = right[i];
            let lt = tanh_dist(dc, drive) * (1.0 - fold) + foldback(dc * drive) * fold;
            let rt = tanh_dist(dr, drive) * (1.0 - fold) + foldback(dr * drive) * fold;
            left[i] = (dc * (1.0 - mix) + lt * mix) * out;
            right[i] = (dr * (1.0 - mix) + rt * mix) * out;
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

// ================================================================ DistStack 6

pub static DIST6_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Drive", 1.0, 12.0, 3.0, ""),
    ParamSpec::new("Crush Bits", 1.0, 16.0, 16.0, "bit"),
    ParamSpec::new("Crush Divide", 1.0, 64.0, 1.0, "x"),
    ParamSpec::new("Comp Mix", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Dub Time", 0.0, 250.0, 0.0, "ms"),
    ParamSpec::new("Dub Feedback", 0.0, 0.9, 0.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Six serial chips: tone HP, saturator, crusher, glue comp, dub delay, trim.
pub struct DistStack6 {
    pub name: &'static str,
    pub p: [f32; DIST6_PARAMS.len()],
    hp: SBiquad,
    cr_l: CrusherCore,
    cr_r: CrusherCore,
    comp: Compressor,
    net: crate::common::DelayNet,
    bypass: bool,
    dirty: bool,
}

impl DistStack6 {
    pub fn new(name: &'static str) -> Self {
        let mut fx = Self {
            name,
            p: std::array::from_fn(|i| DIST6_PARAMS[i].default),
            hp: SBiquad::new(),
            cr_l: CrusherCore::default(),
            cr_r: CrusherCore::default(),
            comp: Compressor::new(),
            net: crate::common::DelayNet::new(250.0),
            bypass: false,
            dirty: true,
        };
        fx.comp.threshold_db = -12.0;
        fx.comp.ratio = 4.0;
        fx.comp.attack_ms = 5.0;
        fx.comp.release_ms = 80.0;
        fx.comp.knee_db = 3.0;
        fx
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        self.hp.set(BiquadCoeffs::compute(
            FilterKind::HighPass,
            30.0,
            0.7,
            0.0,
            sr,
        ));
        self.comp.mix = self.p[3].clamp(0.0, 1.0);
        for cr in [&mut self.cr_l, &mut self.cr_r] {
            cr.bits = self.p[1].clamp(1.0, 16.0);
            cr.rate_div = self.p[2].clamp(1.0, 64.0) as u32;
        }
    }
}

impl EffectBase for DistStack6 {
    const SPEC: &'static [ParamSpec] = DIST6_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn param_changed(&mut self, _index: usize) {
        self.dirty = true;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, _tempo: f32) {
        if self.dirty {
            self.recompute(sr);
        }
        let drive = self.p[0].clamp(1.0, 12.0);
        let dub_t = (self.p[4].clamp(0.0, 240.0) * 0.001 * sr).max(1.0);
        let dub_fb = self.p[5].clamp(0.0, 0.9);
        let out = db_to_gain(self.p[6]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let t = self.hp.tick(left[i], right[i]);
            let mut l = tanh_dist(t.0, drive);
            let mut r = tanh_dist(t.1, drive);
            l = self.cr_l.tick(l);
            r = self.cr_r.tick(r);
            self.comp.process_stereo(&mut l, &mut r, sr);
            // dub: wet echo folded back into the signal
            let wl = self.net.dl_l.read(dub_t);
            let wr = self.net.dl_r.read(dub_t);
            self.net.dl_l.push(l + wl * dub_fb);
            self.net.dl_r.push(r + wr * dub_fb);
            let dl = self.net.lp_l.tick(wl) * 0.9;
            let dr2 = self.net.lp_r.tick(wr) * 0.9;
            left[i] = (l + dl) * out;
            right[i] = (r + dr2) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.hp.reset();
        self.cr_l = CrusherCore::default();
        self.cr_r = CrusherCore::default();
        self.comp.reset();
        self.net.reset();
        self.dirty = true;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================ PedalBoard

pub static PEDAL_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Boost", -12.0, 12.0, 0.0, "dB"),
    ParamSpec::new("OD Drive", 1.0, 10.0, 3.0, ""),
    ParamSpec::new("Chorus Rate", 0.05, 6.0, 0.5, "Hz"),
    ParamSpec::new("Chorus Depth", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Cabinet LP", 1000.0, 20_000.0, 5000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Boost into asymmetric drive, two-voice chorus, cabinet low pass.
pub struct PedalBoard {
    pub name: &'static str,
    pub p: [f32; PEDAL_PARAMS.len()],
    voices: [ModVoice; 2],
    cab: SBiquad,
    bypass: bool,
    dirty: bool,
}

impl PedalBoard {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| PEDAL_PARAMS[i].default),
            voices: [ModVoice::new(0xB1), ModVoice::new(0xB2)],
            cab: SBiquad::new(),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        self.cab.set(BiquadCoeffs::compute(
            FilterKind::LowPass,
            self.p[4].min(sr * 0.45),
            0.8,
            0.0,
            sr,
        ));
        let rate = self.p[2].clamp(0.05, 6.0);
        for (i, v) in self.voices.iter_mut().enumerate() {
            v.rate = rate * (1.0 + i as f32 * 0.07);
            v.depth_ms = 2.0 + self.p[3].clamp(0.0, 1.0) * 4.0;
            v.base_ms = 18.0;
        }
    }
}

impl EffectBase for PedalBoard {
    const SPEC: &'static [ParamSpec] = PEDAL_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn param_changed(&mut self, _index: usize) {
        self.dirty = true;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, _tempo: f32) {
        if self.dirty {
            self.recompute(sr);
        }
        let boost = db_to_gain(self.p[0]);
        let drive = self.p[1].clamp(1.0, 10.0);
        let out = db_to_gain(self.p[5]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let mut l = left[i] * boost;
            let mut r = right[i] * boost;
            l = tanh_dist(l * drive * 0.8, 1.6);
            r = tanh_dist(r * drive * 0.8, 1.6);
            let mut cl = 0.0f32;
            let mut cr = 0.0f32;
            for (i2, v) in self.voices.iter_mut().enumerate() {
                let (wl, wr) = v.tick(l, r, sr);
                let k = if i2 == 0 { 0.35 } else { -0.35 };
                cl += wl * k;
                cr += wr * -k;
            }
            l += cl;
            r += cr;
            let c = self.cab.tick(l, r);
            left[i] = c.0 * out;
            right[i] = c.1 * out;
        }
    }
    fn reset_fx(&mut self) {
        for v in self.voices.iter_mut() {
            v.reset();
        }
        self.cab.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ==================================================================== Crusher

pub static CRUSH_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Bits", 1.0, 16.0, 8.0, "bit"),
    ParamSpec::new("Divide", 1.0, 64.0, 4.0, "x"),
    ParamSpec::new("Boost", 0.0, 12.0, 0.0, "dB"),
    ParamSpec::new("Mix", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Sample-hold and quantize (bit/decimate) with a boost for gain staging.
pub struct CrusherFx {
    pub name: &'static str,
    pub p: [f32; CRUSH_PARAMS.len()],
    cr_l: CrusherCore,
    cr_r: CrusherCore,
    bypass: bool,
    dirty: bool,
}

impl CrusherFx {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| CRUSH_PARAMS[i].default),
            cr_l: CrusherCore::default(),
            cr_r: CrusherCore::default(),
            bypass: false,
            dirty: true,
        }
    }
}

impl EffectBase for CrusherFx {
    const SPEC: &'static [ParamSpec] = CRUSH_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn param_changed(&mut self, _index: usize) {
        self.dirty = true;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], _sr: f32, _tempo: f32) {
        if self.dirty {
            self.dirty = false;
            for cr in [&mut self.cr_l, &mut self.cr_r] {
                cr.bits = self.p[0].clamp(1.0, 16.0);
                cr.rate_div = self.p[1].clamp(1.0, 64.0) as u32;
            }
        }
        let boost = db_to_gain(self.p[2]);
        let mix = self.p[3].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[4]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let dc = left[i];
            let dr = right[i];
            let l = self.cr_l.tick(dc * boost);
            let r = self.cr_r.tick(dr * boost);
            left[i] = (dc * (1.0 - mix) + l * mix) * out;
            right[i] = (dr * (1.0 - mix) + r * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.cr_l = CrusherCore::default();
        self.cr_r = CrusherCore::default();
        self.dirty = true;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================ WaveShaper

pub static SHAPE_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Point A", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Point B", 0.0, 1.0, 0.25, ""),
    ParamSpec::new("Point C", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Point D", 0.0, 1.0, 0.75, ""),
    ParamSpec::new("Point E", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Pre", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Mix", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Five-point editable transfer curve over the -1..1 domain.
pub struct WaveShaperFx {
    pub name: &'static str,
    pub p: [f32; SHAPE_PARAMS.len()],
    shaper: CurveShaper,
    bypass: bool,
    dirty: bool,
}

impl WaveShaperFx {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| SHAPE_PARAMS[i].default),
            shaper: CurveShaper::new(),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self) {
        self.dirty = false;
        self.shaper.pts =
            curve_from_points(self.p[0], self.p[1], self.p[2], self.p[3], self.p[4]);
    }
}

impl EffectBase for WaveShaperFx {
    const SPEC: &'static [ParamSpec] = SHAPE_PARAMS;
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
        if index <= 4 {
            self.dirty = true;
        }
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], _sr: f32, _tempo: f32) {
        if self.dirty {
            self.recompute();
        }
        let pre = db_to_gain(self.p[5]);
        let mix = self.p[6].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[7]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let dc = left[i];
            let dr = right[i];
            let l = self.shaper.shape(dc * pre);
            let r = self.shaper.shape(dr * pre);
            left[i] = (dc * (1.0 - mix) + l * mix) * out;
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

// ================================================================== Stage FX

pub static STAGE_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Cut", 200.0, 20_000.0, 8000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Resonance", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Gate", -60.0, 0.0, -30.0, "dB"),
    ParamSpec::new("Boost", 0.0, 12.0, 0.0, "dB"),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Live stomp chain: resonant low pass into a smoothed noise gate and boost.
pub struct StageFx {
    pub name: &'static str,
    pub p: [f32; STAGE_PARAMS.len()],
    lp: Svf,
    follower: EnvFollower,
    level: f32,
    bypass: bool,
    dirty: bool,
}

impl StageFx {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| STAGE_PARAMS[i].default),
            lp: Svf::new(),
            follower: EnvFollower {
                state: 0.0,
                attack_ms: 1.0,
                release_ms: 20.0,
            },
            level: 0.0,
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        self.lp.set(
            self.p[0].min(sr * 0.48),
            0.5 + self.p[1].clamp(0.0, 1.0) * 11.0,
            sr,
        );
    }
}

impl EffectBase for StageFx {
    const SPEC: &'static [ParamSpec] = STAGE_PARAMS;
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
        if index <= 1 {
            self.dirty = true;
        }
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, _tempo: f32) {
        if self.dirty {
            self.recompute(sr);
        }
        let thresh = db_to_gain(self.p[2]).max(1e-5);
        let boost = db_to_gain(self.p[3]);
        let out = db_to_gain(self.p[4]);
        let atk = (-1.0 / (0.002 * sr)).exp();
        let rel = (-1.0 / (0.05 * sr)).exp();
        let n = left.len().min(right.len());
        for i in 0..n {
            let l = self.lp.tick_mode(left[i], SvfMode::Low);
            let r = self.lp.tick_mode(right[i], SvfMode::Low);
            let det = self.follower.tick(l.abs().max(r.abs()) * 8.0, sr);
            let target = if det > thresh { 1.0 } else { 0.0 };
            let coef = if target > self.level { atk } else { rel };
            self.level = target + (self.level - target) * coef;
            left[i] = l * self.level * boost * out;
            right[i] = r * self.level * boost * out;
        }
    }
    fn reset_fx(&mut self) {
        self.lp.reset();
        self.follower.reset();
        self.level = 0.0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

impl_effect!(SoftClipFx);
impl_effect!(OverDrive);
impl_effect!(FastDist);
impl_effect!(DistStack6);
impl_effect!(PedalBoard);
impl_effect!(CrusherFx);
impl_effect!(WaveShaperFx);
impl_effect!(StageFx);
