//! EQ-family effects: parametric/graphic banks, tilt-morph EQ, bass and sub
//! lifters, low-pass and motion filters, and a band splitter. Filter math runs
//! through the shared biquad/SVF cores. Band tools return mono sums — receives
//! for split bands arrive with the mixer's send routing (D-016 stand-in).

use crate::common::{db_to_gain, impl_effect, BandSplit, EffectBase, SBiquad};
use redraw_dsp::{tanh_dist, BiquadCoeffs, FilterKind, Lfo, Svf, SvfMode};
use redraw_shared::{ParamCurve, ParamSpec};

// ================================================================= ParaEq 7

pub static PARA_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Freq 1", 40.0, 18_000.0, 120.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Gain 1", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Q 1", 0.4, 8.0, 1.0, ""),
    ParamSpec::new("Freq 2", 40.0, 18_000.0, 500.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Gain 2", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Q 2", 0.4, 8.0, 1.0, ""),
    ParamSpec::new("Freq 3", 40.0, 18_000.0, 1800.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Gain 3", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Q 3", 0.4, 8.0, 1.0, ""),
    ParamSpec::new("Freq 4", 40.0, 18_000.0, 5000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Gain 4", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Q 4", 0.4, 8.0, 1.0, ""),
    ParamSpec::new("Freq 5", 40.0, 18_000.0, 10_000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Gain 5", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Q 5", 0.4, 8.0, 1.0, ""),
    ParamSpec::new("High Cut", 4000.0, 20_000.0, 20_000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("High Cut Q", 0.4, 8.0, 0.8, ""),
    ParamSpec::new("Low Cut", 20.0, 2000.0, 20.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Low Cut Q", 0.4, 8.0, 0.8, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct ParaEq7 {
    pub name: &'static str,
    pub p: [f32; PARA_PARAMS.len()],
    bands: [SBiquad; 5],
    hp: SBiquad,
    lp: SBiquad,
    bypass: bool,
    dirty: bool,
}

impl ParaEq7 {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| PARA_PARAMS[i].default),
            bands: std::array::from_fn(|_| SBiquad::new()),
            hp: SBiquad::new(),
            lp: SBiquad::new(),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        for (k, b) in self.bands.iter_mut().enumerate() {
            let f = self.p[k * 3];
            let g = self.p[k * 3 + 1];
            let q = self.p[k * 3 + 2];
            b.set(BiquadCoeffs::compute(FilterKind::Peak, f, q.max(0.1), g, sr));
        }
        // At the maximum the high cut passes audio above the corner.
        let top = (self.p[15] * 1.25).min(sr * 0.49);
        let lc = self.p[17].max(8.0);
        self.lp.set(BiquadCoeffs::compute(
            FilterKind::LowPass,
            top,
            self.p[16].max(0.4),
            0.0,
            sr,
        ));
        self.hp.set(BiquadCoeffs::compute(
            FilterKind::HighPass,
            lc,
            self.p[18].max(0.4),
            0.0,
            sr,
        ));
    }
}

impl EffectBase for ParaEq7 {
    const SPEC: &'static [ParamSpec] = PARA_PARAMS;
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
        let out = db_to_gain(self.p[19]);
        let hp_on = self.p[17] > 25.0;
        let lp_on = self.p[15] < 21_000.0;
        let n = left.len().min(right.len());
        for i in 0..n {
            let mut l = left[i];
            let mut r = right[i];
            if hp_on {
                let t = self.hp.tick(l, r);
                l = t.0;
                r = t.1;
            }
            for b in self.bands.iter_mut() {
                let t = b.tick(l, r);
                l = t.0;
                r = t.1;
            }
            if lp_on {
                let t = self.lp.tick(l, r);
                l = t.0;
                r = t.1;
            }
            left[i] = l * out;
            right[i] = r * out;
        }
    }
    fn reset_fx(&mut self) {
        for b in self.bands.iter_mut() {
            b.reset();
        }
        self.hp.reset();
        self.lp.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================= Graphic 7

pub static GRAPH7_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("32 Hz", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("80 Hz", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("200 Hz", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("500 Hz", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("1250 Hz", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("3150 Hz", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("8000 Hz", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

const GRAPH7_HZ: [f32; 7] = [32.0, 80.0, 200.0, 500.0, 1250.0, 3150.0, 8000.0];

pub struct Graphic7 {
    pub name: &'static str,
    pub p: [f32; GRAPH7_PARAMS.len()],
    bands: [SBiquad; 7],
    bypass: bool,
    dirty: bool,
}

impl Graphic7 {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| GRAPH7_PARAMS[i].default),
            bands: std::array::from_fn(|_| SBiquad::new()),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        for (k, b) in self.bands.iter_mut().enumerate() {
            let f = GRAPH7_HZ[k].min(sr * 0.45);
            b.set(BiquadCoeffs::compute(
                FilterKind::Peak,
                f,
                1.4,
                self.p[k],
                sr,
            ));
        }
    }
}

impl EffectBase for Graphic7 {
    const SPEC: &'static [ParamSpec] = GRAPH7_PARAMS;
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
        let out = db_to_gain(self.p[7]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let mut l = left[i];
            let mut r = right[i];
            for b in self.bands.iter_mut() {
                let t = b.tick(l, r);
                l = t.0;
                r = t.1;
            }
            left[i] = l * out;
            right[i] = r * out;
        }
    }
    fn reset_fx(&mut self) {
        for b in self.bands.iter_mut() {
            b.reset();
        }
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================ Morph EQ

pub static MORPH_EQ_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Morph", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Low", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Mid", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("High", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Tilt", -1.0, 1.0, 0.0, ""),
    ParamSpec::new("Crossover 1", 100.0, 2000.0, 300.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Crossover 2", 1000.0, 12_000.0, 3000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Tilt/morph EQ over a complementary three-way split; output is summed mono.
pub struct MorphEq {
    pub name: &'static str,
    pub p: [f32; MORPH_EQ_PARAMS.len()],
    split: BandSplit,
    bypass: bool,
    dirty: bool,
}

impl MorphEq {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| MORPH_EQ_PARAMS[i].default),
            split: BandSplit::new(300.0, 3000.0),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        self.split.set_edges(self.p[5], self.p[6], sr);
    }
}

impl EffectBase for MorphEq {
    const SPEC: &'static [ParamSpec] = MORPH_EQ_PARAMS;
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
        if index >= 5 && index <= 6 {
            self.dirty = true;
        }
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, _tempo: f32) {
        if self.dirty {
            self.recompute(sr);
        }
        let morph = self.p[0].clamp(0.0, 1.0);
        let tilt = self.p[4].clamp(-1.0, 1.0);
        let gl = db_to_gain(self.p[1] * morph - tilt * 3.0);
        let gm = db_to_gain(self.p[2] * morph);
        let gh = db_to_gain(self.p[3] * morph + tilt * 3.0);
        let out = db_to_gain(self.p[7]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let (low, mid, high) = self.split.split(left[i], right[i]);
            let m = low * gl + mid * gm + high * gh;
            left[i] = m * out;
            right[i] = m * out;
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

// ================================================================ Bass Boost

pub static BASSB_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Boost", -12.0, 18.0, 6.0, "dB"),
    ParamSpec::new("Corner", 60.0, 500.0, 150.0, "Hz"),
    ParamSpec::new("Harmonics", 0.0, 1.0, 0.2, ""),
    ParamSpec::new("Tighten", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct BassBoost {
    pub name: &'static str,
    pub p: [f32; BASSB_PARAMS.len()],
    shelf: SBiquad,
    tighten: SBiquad,
    bypass: bool,
    dirty: bool,
}

impl BassBoost {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| BASSB_PARAMS[i].default),
            shelf: SBiquad::new(),
            tighten: SBiquad::new(),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        self.shelf.set(BiquadCoeffs::compute(
            FilterKind::LowShelf,
            self.p[1],
            0.8,
            self.p[0],
            sr,
        ));
        self.tighten.set(BiquadCoeffs::compute(
            FilterKind::HighPass,
            8.0 + self.p[3].clamp(0.0, 1.0) * 60.0,
            0.7,
            0.0,
            sr,
        ));
    }
}

impl EffectBase for BassBoost {
    const SPEC: &'static [ParamSpec] = BASSB_PARAMS;
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
        let harm = self.p[2].clamp(0.0, 1.0) * 0.08;
        let out = db_to_gain(self.p[4]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let t = self.tighten.tick(left[i], right[i]);
            let b = self.shelf.tick(t.0, t.1);
            let mut l = b.0;
            let mut r = b.1;
            if harm > 0.0 {
                // soft second harmonic over the content that got boosted
                let base = (t.0 + t.1) * 0.5;
                let h2 = base * base.abs() * harm;
                l += h2;
                r += h2;
            }
            left[i] = l * out;
            right[i] = r * out;
        }
    }
    fn reset_fx(&mut self) {
        self.shelf.reset();
        self.tighten.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================ Sub Lifter

pub static SUBLIFT_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Sub Level", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Sub Drive", 1.0, 6.0, 2.0, ""),
    ParamSpec::new("Corner", 60.0, 400.0, 120.0, "Hz"),
    ParamSpec::new("Shelf", -18.0, 12.0, 3.0, "dB"),
    ParamSpec::new("Blend", 0.0, 1.0, 0.4, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Extracts and saturates the low band, blends it under the dry signal and
/// adds a low shelf — low-end weight without mud.
pub struct SubLifter {
    pub name: &'static str,
    pub p: [f32; SUBLIFT_PARAMS.len()],
    band: SBiquad,
    shelf: SBiquad,
    bypass: bool,
    dirty: bool,
}

impl SubLifter {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| SUBLIFT_PARAMS[i].default),
            band: SBiquad::new(),
            shelf: SBiquad::new(),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        self.band.set(BiquadCoeffs::compute(
            FilterKind::LowPass,
            self.p[2].min(sr * 0.45),
            0.8,
            0.0,
            sr,
        ));
        self.shelf.set(BiquadCoeffs::compute(
            FilterKind::LowShelf,
            self.p[2],
            0.8,
            self.p[3],
            sr,
        ));
    }
}

impl EffectBase for SubLifter {
    const SPEC: &'static [ParamSpec] = SUBLIFT_PARAMS;
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
        let sub_amt = self.p[0].clamp(0.0, 1.0);
        let drive = self.p[1].clamp(1.0, 6.0);
        let blend = self.p[4].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[5]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let t = self.band.tick(left[i], right[i]);
            let sub = tanh_dist((t.0 + t.1) * 0.5, drive) * sub_amt;
            let l = left[i] * (1.0 - blend) + sub;
            let r = right[i] * (1.0 - blend) + sub;
            let sh = self.shelf.tick(l, r);
            left[i] = sh.0 * out;
            right[i] = sh.1 * out;
        }
    }
    fn reset_fx(&mut self) {
        self.band.reset();
        self.shelf.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================== Fast LP

pub static FAST_LP_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Cutoff", 20.0, 20_000.0, 8000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Resonance", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct FastLp {
    pub name: &'static str,
    pub p: [f32; FAST_LP_PARAMS.len()],
    lp: [Svf; 2],
    bypass: bool,
    dirty: bool,
}

impl FastLp {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| FAST_LP_PARAMS[i].default),
            lp: [Svf::new(), Svf::new()],
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        let fc = self.p[0].min(sr * 0.48);
        let q = 0.5 + self.p[1].clamp(0.0, 1.0) * 13.0;
        self.lp[0].set(fc, q, sr);
        self.lp[1].set(fc, q, sr);
    }
}

impl EffectBase for FastLp {
    const SPEC: &'static [ParamSpec] = FAST_LP_PARAMS;
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
        let out = db_to_gain(self.p[2]);
        let n = left.len().min(right.len());
        for i in 0..n {
            left[i] = self.lp[0].tick_mode(left[i], SvfMode::Low) * out;
            right[i] = self.lp[1].tick_mode(right[i], SvfMode::Low) * out;
        }
    }
    fn reset_fx(&mut self) {
        for f in self.lp.iter_mut() {
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

// =============================================================== MultiFilter

pub static MFILT_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Cutoff", 20.0, 20_000.0, 4000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Resonance", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Mode", 0.0, 3.0, 0.0, ""),
    ParamSpec::new("LFO Rate", 0.01, 8.0, 1.0, "Hz"),
    ParamSpec::new("LFO Depth", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

#[inline]
fn mode_of(p: f32) -> SvfMode {
    SvfMode::from_index(p.clamp(0.0, 3.0) as usize)
}

pub struct MultiFilter {
    pub name: &'static str,
    pub p: [f32; MFILT_PARAMS.len()],
    l: Svf,
    r: Svf,
    lfo: Lfo,
    bypass: bool,
    dirty: bool,
}

impl MultiFilter {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| MFILT_PARAMS[i].default),
            l: Svf::new(),
            r: Svf::new(),
            lfo: Lfo::new(0x91),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        self.lfo.set_rate(self.p[3].max(0.01));
        let _ = sr;
    }
}

impl EffectBase for MultiFilter {
    const SPEC: &'static [ParamSpec] = MFILT_PARAMS;
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
        let mode = mode_of(self.p[2]);
        let depth = self.p[4].clamp(0.0, 1.0);
        let base = self.p[0];
        let q = 0.5 + self.p[1].clamp(0.0, 1.0) * 13.0;
        let out = db_to_gain(self.p[5]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let lf = self.lfo.tick(sr);
            let fc = (base * 2f32.powf(lf * depth * 3.0)).clamp(20.0, sr * 0.48);
            self.l.set(fc, q, sr);
            self.r.set(fc, q, sr);
            left[i] = self.l.tick_mode(left[i], mode) * out;
            right[i] = self.r.tick_mode(right[i], mode) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.l.reset();
        self.r.reset();
        self.lfo.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ============================================================== MorphFilter

pub static MORPHF_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Cutoff", 20.0, 20_000.0, 4000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Resonance", 0.0, 1.0, 0.15, ""),
    ParamSpec::new("LFO Rate", 0.01, 8.0, 1.0, "Hz"),
    ParamSpec::new("LFO Depth", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Resonant low pass with an always-on morphing cutoff (LFO swept).
pub struct MorphFilter {
    pub name: &'static str,
    pub p: [f32; MORPHF_PARAMS.len()],
    l: Svf,
    r: Svf,
    lfo: Lfo,
    bypass: bool,
    dirty: bool,
}

impl MorphFilter {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| MORPHF_PARAMS[i].default),
            l: Svf::new(),
            r: Svf::new(),
            lfo: Lfo::new(0x92),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        self.lfo.set_rate(self.p[2].max(0.01));
        let _ = sr;
    }
}

impl EffectBase for MorphFilter {
    const SPEC: &'static [ParamSpec] = MORPHF_PARAMS;
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
        let depth = self.p[3].clamp(0.0, 1.0);
        let base = self.p[0];
        let q = 0.5 + self.p[1].clamp(0.0, 1.0) * 13.0;
        let out = db_to_gain(self.p[4]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let lf = self.lfo.tick(sr);
            let fc = (base * 2f32.powf(lf * depth * 3.0)).clamp(20.0, sr * 0.48);
            self.l.set(fc, q, sr);
            self.r.set(fc, q, sr);
            left[i] = self.l.tick_mode(left[i], SvfMode::Low) * out;
            right[i] = self.r.tick_mode(right[i], SvfMode::Low) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.l.reset();
        self.r.reset();
        self.lfo.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ============================================================= Band Splitter

pub static BSPLIT_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Split 1", 100.0, 2000.0, 250.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Split 2", 1000.0, 12_000.0, 2500.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Low Gain", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Mid Gain", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("High Gain", -18.0, 18.0, 0.0, "dB"),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Three-way complementary split with per-band gain; bands are summed mono
/// here, per-band routing lands with the mixer's sends (D-016 stand-in).
pub struct BandSplitter {
    pub name: &'static str,
    pub p: [f32; BSPLIT_PARAMS.len()],
    split: BandSplit,
    bypass: bool,
    dirty: bool,
}

impl BandSplitter {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| BSPLIT_PARAMS[i].default),
            split: BandSplit::new(250.0, 2500.0),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        self.split.set_edges(self.p[0], self.p[1], sr);
    }
}

impl EffectBase for BandSplitter {
    const SPEC: &'static [ParamSpec] = BSPLIT_PARAMS;
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
        let gl = db_to_gain(self.p[2]);
        let gm = db_to_gain(self.p[3]);
        let gh = db_to_gain(self.p[4]);
        let out = db_to_gain(self.p[5]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let (low, mid, high) = self.split.split(left[i], right[i]);
            let m = (low * gl + mid * gm + high * gh) * out;
            left[i] = m;
            right[i] = m;
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

impl_effect!(ParaEq7);
impl_effect!(Graphic7);
impl_effect!(MorphEq);
impl_effect!(BassBoost);
impl_effect!(SubLifter);
impl_effect!(FastLp);
impl_effect!(MultiFilter);
impl_effect!(MorphFilter);
impl_effect!(BandSplitter);
