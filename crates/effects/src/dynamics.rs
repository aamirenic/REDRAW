//! Dynamics effects: stereo-linked compression, multiband dynamics, look-ahead
//! limiting, transient shaping and preset glue busses. Detection runs through
//! the dsp `Compressor` core; blocking effects call their core once per block.
#![forbid(unsafe_code)]

use crate::common::{db_to_gain, impl_effect, BandSplit, EffectBase, SBiquad};
use redraw_dsp::{soft_clip, tanh_dist, Compressor, Limiter};
use redraw_shared::{ParamCurve, ParamSpec};

// =========================================================== Compressor One

pub static COMP1_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Threshold", -60.0, 0.0, -18.0, "dB"),
    ParamSpec::new("Ratio", 1.0, 20.0, 4.0, ":1"),
    ParamSpec::new("Attack", 0.5, 100.0, 8.0, "ms"),
    ParamSpec::new("Release", 10.0, 1000.0, 120.0, "ms"),
    ParamSpec::new("Knee", 0.0, 12.0, 6.0, "dB"),
    ParamSpec::new("Makeup", 0.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Mix", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct CompressorUnit {
    pub name: &'static str,
    pub p: [f32; COMP1_PARAMS.len()],
    comp: Compressor,
    bypass: bool,
}

impl CompressorUnit {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| COMP1_PARAMS[i].default),
            comp: Compressor::new(),
            bypass: false,
        }
    }
    fn sync(&mut self) {
        self.comp.threshold_db = self.p[0];
        self.comp.ratio = self.p[1].max(1.0);
        self.comp.attack_ms = self.p[2].max(0.1);
        self.comp.release_ms = self.p[3].max(1.0);
        self.comp.knee_db = self.p[4].max(0.0);
        self.comp.makeup_db = self.p[5];
        self.comp.mix = self.p[6].clamp(0.0, 1.0);
    }
}

impl EffectBase for CompressorUnit {
    const SPEC: &'static [ParamSpec] = COMP1_PARAMS;
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
        self.sync();
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, _tempo: f32) {
        self.sync();
        let out = db_to_gain(self.p[7]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let mut l = left[i];
            let mut r = right[i];
            self.comp.process_stereo(&mut l, &mut r, sr);
            left[i] = l * out;
            right[i] = r * out;
        }
    }
    fn reset_fx(&mut self) {
        self.comp.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================ Multi Comp

pub static MCOMP_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Split 1", 100.0, 2000.0, 200.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Split 2", 1000.0, 12_000.0, 2000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Threshold", -60.0, 0.0, -24.0, "dB"),
    ParamSpec::new("Ratio", 1.0, 12.0, 3.0, ":1"),
    ParamSpec::new("Attack", 0.5, 50.0, 10.0, "ms"),
    ParamSpec::new("Release", 10.0, 500.0, 80.0, "ms"),
    ParamSpec::new("Low Gain", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Mid Gain", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("High Gain", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Three stereo-linked band compressors over a phase-coherent split.
pub struct MultiComp {
    pub name: &'static str,
    pub p: [f32; MCOMP_PARAMS.len()],
    split: BandSplit,
    comps: [Compressor; 3],
    bypass: bool,
    dirty: bool,
}

impl MultiComp {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| MCOMP_PARAMS[i].default),
            split: BandSplit::new(200.0, 2000.0),
            comps: [Compressor::new(), Compressor::new(), Compressor::new()],
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        self.split.set_edges(self.p[0], self.p[1], sr);
        for c in self.comps.iter_mut() {
            c.threshold_db = self.p[2];
            c.ratio = self.p[3].max(1.0);
            c.attack_ms = self.p[4].max(0.1);
            c.release_ms = self.p[5].max(1.0);
            c.knee_db = 3.0;
            c.makeup_db = 0.0;
            c.mix = 1.0;
        }
    }
}

impl EffectBase for MultiComp {
    const SPEC: &'static [ParamSpec] = MCOMP_PARAMS;
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
        let gl = db_to_gain(self.p[6]);
        let gm = db_to_gain(self.p[7]);
        let gh = db_to_gain(self.p[8]);
        let out = db_to_gain(self.p[9]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let (mut bl, mut br) = self.split.split_stereo(left[i], right[i]);
            for k in 0..3 {
                self.comps[k].process_stereo(&mut bl[k], &mut br[k], sr);
            }
            left[i] = (bl[0] * gl + bl[1] * gm + bl[2] * gh) * out;
            right[i] = (br[0] * gl + br[1] * gm + br[2] * gh) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.split.reset();
        for c in self.comps.iter_mut() {
            c.reset();
        }
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// =============================================================== Limiter One

pub static LIM1_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Gain", -12.0, 12.0, 0.0, "dB"),
    ParamSpec::new("Ceiling", -18.0, 0.0, -0.3, "dB"),
    ParamSpec::new("Release", 10.0, 1000.0, 60.0, "ms"),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct LimiterOne {
    pub name: &'static str,
    pub p: [f32; LIM1_PARAMS.len()],
    lim: Limiter,
    bypass: bool,
}

impl LimiterOne {
    pub fn new(name: &'static str) -> Self {
        let mut lim = Limiter::new(48_000.0);
        lim.ceiling_db = -0.3;
        lim.release_ms = 60.0;
        Self {
            name,
            p: std::array::from_fn(|i| LIM1_PARAMS[i].default),
            lim,
            bypass: false,
        }
    }
}

impl EffectBase for LimiterOne {
    const SPEC: &'static [ParamSpec] = LIM1_PARAMS;
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
            self.lim.release_ms = self.p[2].max(1.0);
        }
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, _tempo: f32) {
        self.lim.ceiling_db = self.p[1];
        let in_gain = db_to_gain(self.p[0]);
        let out = db_to_gain(self.p[3]);
        if in_gain != 1.0 {
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                *l *= in_gain;
                *r *= in_gain;
            }
        }
        self.lim.process(left, right, sr);
        if out != 1.0 {
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                *l *= out;
                *r *= out;
            }
        }
    }
    fn reset_fx(&mut self) {
        self.lim.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ============================================================== Max 3 Band

pub static MAX3_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Split 1", 100.0, 2000.0, 180.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Split 2", 1000.0, 12_000.0, 1800.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Threshold", -60.0, 0.0, -20.0, "dB"),
    ParamSpec::new("Ratio", 1.0, 12.0, 4.0, ":1"),
    ParamSpec::new("Attack", 0.5, 50.0, 5.0, "ms"),
    ParamSpec::new("Release", 10.0, 500.0, 100.0, "ms"),
    ParamSpec::new("Low Gain", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Mid Gain", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("High Gain", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Per-band compression with a soft ceiling on each band (loudness bus).
pub struct TriadGuard {
    pub name: &'static str,
    pub p: [f32; MAX3_PARAMS.len()],
    split: BandSplit,
    comps: [Compressor; 3],
    bypass: bool,
    dirty: bool,
}

impl TriadGuard {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| MAX3_PARAMS[i].default),
            split: BandSplit::new(180.0, 1800.0),
            comps: [Compressor::new(), Compressor::new(), Compressor::new()],
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        self.split.set_edges(self.p[0], self.p[1], sr);
        for (k, c) in self.comps.iter_mut().enumerate() {
            // slightly gentler on low, harder on high for the loudness push
            c.threshold_db = self.p[2] + k as f32 * 2.0;
            c.ratio = self.p[3].max(1.0);
            c.attack_ms = self.p[4].max(0.1) + k as f32 * 2.0;
            c.release_ms = self.p[5].max(1.0);
            c.knee_db = 4.0;
            c.makeup_db = 0.0;
            c.mix = 1.0;
        }
    }
}

impl EffectBase for TriadGuard {
    const SPEC: &'static [ParamSpec] = MAX3_PARAMS;
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
        let gains = [self.p[6], self.p[7], self.p[8]];
        let out = db_to_gain(self.p[9]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let (mut bl, mut br) = self.split.split_stereo(left[i], right[i]);
            for k in 0..3 {
                self.comps[k].process_stereo(&mut bl[k], &mut br[k], sr);
                let g = db_to_gain(gains[k]);
                bl[k] = soft_clip(bl[k] * g);
                br[k] = soft_clip(br[k] * g);
            }
            left[i] = (bl[0] + bl[1] + bl[2]) * out;
            right[i] = (br[0] + br[1] + br[2]) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.split.reset();
        for c in self.comps.iter_mut() {
            c.reset();
        }
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================ Emphasizer

pub static EMPH_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Focus", -12.0, 12.0, 6.0, "dB"),
    ParamSpec::new("Drive", 1.0, 8.0, 2.5, ""),
    ParamSpec::new("Corner", 200.0, 6000.0, 1500.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Mix", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Pre-emphasize, saturate, de-emphasize — a bright transient push with no
/// harsh noise floor.
pub struct Emphasizer {
    pub name: &'static str,
    pub p: [f32; EMPH_PARAMS.len()],
    pre: SBiquad,
    post: SBiquad,
    bypass: bool,
    dirty: bool,
}

impl Emphasizer {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| EMPH_PARAMS[i].default),
            pre: SBiquad::new(),
            post: SBiquad::new(),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        let g = self.p[0].clamp(-12.0, 12.0);
        let c = self.p[2].min(sr * 0.45);
        self.pre
            .set(redraw_dsp::BiquadCoeffs::compute(
                redraw_dsp::FilterKind::HighShelf,
                c,
                0.7,
                g,
                sr,
            ));
        self.post
            .set(redraw_dsp::BiquadCoeffs::compute(
                redraw_dsp::FilterKind::LowShelf,
                c,
                0.7,
                -g,
                sr,
            ));
    }
}

impl EffectBase for Emphasizer {
    const SPEC: &'static [ParamSpec] = EMPH_PARAMS;
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
        let drive = self.p[1].clamp(1.0, 8.0);
        let mix = self.p[3].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[4]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let t = self.pre.tick(left[i], right[i]);
            let mut l = t.0 * drive;
            let mut r = t.1 * drive;
            l = tanh_dist(l, 1.0);
            r = tanh_dist(r, 1.0);
            let d = self.post.tick(l, r);
            left[i] = (left[i] * (1.0 - mix) + d.0 * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + d.1 * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.pre.reset();
        self.post.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ============================================================= Spectral Push

pub static SPUSH_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Amount", -1.0, 1.0, 0.3, ""),
    ParamSpec::new("Split 1", 100.0, 2000.0, 250.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Split 2", 1000.0, 12_000.0, 2500.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Release", 10.0, 300.0, 60.0, "ms"),
    ParamSpec::new("Mix", 0.0, 1.0, 0.6, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Per-band fast/slow envelope shaping: pushes transients (positive amount) or
/// smooths attacks (negative).
pub struct SpectralPush {
    pub name: &'static str,
    pub p: [f32; SPUSH_PARAMS.len()],
    split: BandSplit,
    fast: [crate::common::EnvFollower; 3],
    slow: [crate::common::EnvFollower; 3],
    bypass: bool,
    dirty: bool,
}

impl SpectralPush {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| SPUSH_PARAMS[i].default),
            split: BandSplit::new(250.0, 2500.0),
            fast: std::array::from_fn(|_| crate::common::EnvFollower {
                state: 0.0,
                attack_ms: 1.0,
                release_ms: 30.0,
            }),
            slow: std::array::from_fn(|_| crate::common::EnvFollower {
                state: 0.0,
                attack_ms: 15.0,
                release_ms: 120.0,
            }),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        self.split.set_edges(self.p[1], self.p[2], sr);
        let rel = self.p[3].max(5.0);
        for f in self.slow.iter_mut() {
            f.release_ms = rel;
        }
    }
}

impl EffectBase for SpectralPush {
    const SPEC: &'static [ParamSpec] = SPUSH_PARAMS;
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
        if index == 1 || index == 2 || index == 3 {
            self.dirty = true;
        }
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, _tempo: f32) {
        if self.dirty {
            self.recompute(sr);
        }
        let amount = self.p[0].clamp(-1.0, 1.0);
        let mix = self.p[4].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[5]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let (mut bl, mut br) = self.split.split_stereo(left[i], right[i]);
            for k in 0..3 {
                let det = bl[k].abs().max(br[k].abs());
                let fa = self.fast[k].tick(det, sr);
                let sl = self.slow[k].tick(det, sr);
                let g = (1.0 + (fa - sl).clamp(-0.5, 2.0) * amount * 3.0).clamp(0.1, 4.0);
                bl[k] *= g;
                br[k] *= g;
            }
            let l = bl[0] + bl[1] + bl[2];
            let r = br[0] + br[1] + br[2];
            left[i] = (left[i] * (1.0 - mix) + l * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + r * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.split.reset();
        for f in self.fast.iter_mut().chain(self.slow.iter_mut()) {
            f.reset();
        }
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// =========================================================== Transient Shaper

pub static TRANSP_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Attack", 0.0, 12.0, 3.0, "dB"),
    ParamSpec::new("Sustain", 0.0, 12.0, 0.0, "dB"),
    ParamSpec::new("Mix", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Fast/slow envelope difference shapes hits up and tails down.
pub struct TransientShaper {
    pub name: &'static str,
    pub p: [f32; TRANSP_PARAMS.len()],
    fast: crate::common::EnvFollower,
    slow: crate::common::EnvFollower,
    bypass: bool,
}

impl TransientShaper {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| TRANSP_PARAMS[i].default),
            fast: crate::common::EnvFollower {
                state: 0.0,
                attack_ms: 1.0,
                release_ms: 25.0,
            },
            slow: crate::common::EnvFollower {
                state: 0.0,
                attack_ms: 12.0,
                release_ms: 120.0,
            },
            bypass: false,
        }
    }
}

impl EffectBase for TransientShaper {
    const SPEC: &'static [ParamSpec] = TRANSP_PARAMS;
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
        let att = self.p[0].clamp(0.0, 12.0) / 20.0;
        let sus = self.p[1].clamp(0.0, 12.0) / 20.0;
        let mix = self.p[2].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[3]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let det = left[i].abs().max(right[i].abs());
            let fa = self.fast.tick(det, sr);
            let sl = self.slow.tick(det, sr);
            let push = (fa - sl).clamp(0.0, 1.0) * att * 4.0;
            let shave = (sl - fa).clamp(0.0, 1.0) * sus * 4.0;
            let g = (1.0 + push - shave).clamp(0.2, 6.0);
            left[i] = (left[i] * (1.0 - mix) + left[i] * g * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + right[i] * g * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.fast.reset();
        self.slow.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================== MacroGlow

pub static GOOD_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Preset", 0.0, 3.0, 1.0, ""),
    ParamSpec::new("Amount", 0.0, 1.0, 0.6, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// One-knob glue bus: curated compressor + saturation presets per amount.
pub struct MacroGlow {
    pub name: &'static str,
    pub p: [f32; GOOD_PARAMS.len()],
    comp: Compressor,
    bypass: bool,
    dirty: bool,
}

impl MacroGlow {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| GOOD_PARAMS[i].default),
            comp: Compressor::new(),
            bypass: false,
            dirty: true,
        }
    }
    fn apply_preset(&mut self) {
        self.dirty = false;
        let sel = (self.p[0] as u32) % 4;
        let (th, ra, at, re) = match sel {
            0 => (-24.0, 2.0, 10.0, 180.0), // glue
            1 => (-18.0, 4.0, 6.0, 120.0),  // punch
            2 => (-12.0, 6.0, 3.0, 90.0),   // loud
            _ => (-8.0, 10.0, 1.0, 60.0),   // crank
        };
        self.comp.threshold_db = th;
        self.comp.ratio = ra;
        self.comp.attack_ms = at;
        self.comp.release_ms = re;
        self.comp.knee_db = 4.0;
        self.comp.makeup_db = 0.0;
        self.comp.mix = 1.0;
    }
}

impl EffectBase for MacroGlow {
    const SPEC: &'static [ParamSpec] = GOOD_PARAMS;
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
        if index == 0 {
            self.dirty = true;
        }
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, _tempo: f32) {
        if self.dirty {
            self.apply_preset();
        }
        let amount = self.p[1].clamp(0.0, 1.0);
        let drive = 1.0 + amount * 2.2;
        let lift = 1.0 - amount * 0.15; // keep perceived level near unity
        let makeup = 1.0 + amount * 0.5; // compensate compression power loss
        let out = db_to_gain(self.p[2]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let mut l = left[i];
            let mut r = right[i];
            self.comp.process_stereo(&mut l, &mut r, sr);
            let l2 = tanh_dist(l * makeup, drive) * lift;
            let r2 = tanh_dist(r * makeup, drive) * lift;
            left[i] = l2 * out;
            right[i] = r2 * out;
        }
    }
    fn reset_fx(&mut self) {
        self.comp.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

impl_effect!(CompressorUnit);
impl_effect!(MultiComp);
impl_effect!(LimiterOne);
impl_effect!(TriadGuard);
impl_effect!(Emphasizer);
impl_effect!(SpectralPush);
impl_effect!(TransientShaper);
impl_effect!(MacroGlow);
