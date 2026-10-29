//! Reverb effects: two algorithmic rooms over the dsp cores and a synthetic-IR
//! convolution unit. All four factory impulses are built at construction and
//! processed through hop staging, so the audio path never allocates.
#![forbid(unsafe_code)]

use crate::common::{db_to_gain, impl_effect, EffectBase, HopStager, SBiquad};
use redraw_dsp::{BiquadCoeffs, Convolver, DelayLine, FdnReverb, FilterKind, Reverb, Rng};
use redraw_shared::{ParamCurve, ParamSpec};

// ================================================================ Silver Verb

pub static SILVER_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Room", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Damp", 0.0, 1.0, 0.4, ""),
    ParamSpec::new("Width", 0.0, 1.0, 0.8, ""),
    ParamSpec::new("Predelay", 0.0, 200.0, 12.0, "ms"),
    ParamSpec::new("Low Cut", 20.0, 1000.0, 80.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("High Cut", 1000.0, 20_000.0, 9000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Mix", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Freeverb-style stereo room with tone-sculpted tail.
pub struct SilverRoom {
    pub name: &'static str,
    pub p: [f32; SILVER_PARAMS.len()],
    rev: Reverb,
    bypass: bool,
}

impl SilverRoom {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| SILVER_PARAMS[i].default),
            rev: Reverb::new(48_000.0),
            bypass: false,
        }
    }
}

impl EffectBase for SilverRoom {
    const SPEC: &'static [ParamSpec] = SILVER_PARAMS;
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
        self.rev.room = self.p[0].clamp(0.0, 1.0);
        self.rev.damp = self.p[1].clamp(0.0, 1.0);
        self.rev.width = self.p[2].clamp(0.0, 1.0);
        self.rev.predelay_ms = self.p[3].clamp(0.0, 200.0);
        self.rev.lowcut_hz = self.p[4].max(10.0);
        self.rev.highcut_hz = self.p[5].max(1000.0);
        self.rev.mix = self.p[6].clamp(0.0, 1.0);
        self.rev.process(left, right, sr);
        let out = db_to_gain(self.p[7]);
        if out != 1.0 {
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                *l *= out;
                *r *= out;
            }
        }
    }
    fn reset_fx(&mut self) {
        self.rev.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ============================================================== Silver Verb 2

pub static SILVER2_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Room", 0.0, 1.0, 0.55, ""),
    ParamSpec::new("Damp", 0.0, 1.0, 0.4, ""),
    ParamSpec::new("Width", 0.0, 1.0, 0.9, ""),
    ParamSpec::new("Predelay", 0.0, 200.0, 15.0, "ms"),
    ParamSpec::new("Early Level", 0.0, 1.0, 0.25, ""),
    ParamSpec::new("Early Time", 5.0, 50.0, 17.0, "ms"),
    ParamSpec::new("Mix", 0.0, 1.0, 0.35, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Room reverb with an added pair of asymmetric early-reflection taps.
pub struct SilverRoom2 {
    pub name: &'static str,
    pub p: [f32; SILVER2_PARAMS.len()],
    rev: Reverb,
    early_l: DelayLine,
    early_r: DelayLine,
    bypass: bool,
}

impl SilverRoom2 {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| SILVER2_PARAMS[i].default),
            rev: Reverb::new(48_000.0),
            early_l: DelayLine::new(3600),
            early_r: DelayLine::new(3600),
            bypass: false,
        }
    }
}

impl EffectBase for SilverRoom2 {
    const SPEC: &'static [ParamSpec] = SILVER2_PARAMS;
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
        self.rev.room = self.p[0].clamp(0.0, 1.0);
        self.rev.damp = self.p[1].clamp(0.0, 1.0);
        self.rev.width = self.p[2].clamp(0.0, 1.0);
        self.rev.predelay_ms = self.p[3].clamp(0.0, 200.0);
        let early = self.p[4].clamp(0.0, 1.0);
        let t1 = (self.p[5].clamp(5.0, 50.0) * 0.001 * sr).clamp(1.0, 3400.0);
        let t2 = (t1 * 1.37).min(3400.0);
        self.rev.mix = self.p[6].clamp(0.0, 1.0);
        self.rev.process(left, right, sr);
        let out = db_to_gain(self.p[7]);
        let n = left.len().min(right.len());
        for i in 0..n {
            self.early_l.push(left[i]);
            self.early_r.push(right[i]);
            let el = self.early_l.read(t1) * early;
            let er = self.early_r.read(t2) * early;
            left[i] = (left[i] + el) * out;
            right[i] = (right[i] + er) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.rev.reset();
        self.early_l.clear();
        self.early_r.clear();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// =================================================================== Lux Verb

pub static LUX_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Size", 0.1, 2.0, 1.0, ""),
    ParamSpec::new("Decay", 0.1, 0.999, 0.85, ""),
    ParamSpec::new("Damp", 0.0, 1.0, 0.35, ""),
    ParamSpec::new("Predelay", 0.0, 200.0, 20.0, "ms"),
    ParamSpec::new("Freeze", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Feedback-delay-network hall; the freeze control parks the decay near unity
/// for infinite tails.
pub struct VelvetHall {
    pub name: &'static str,
    pub p: [f32; LUX_PARAMS.len()],
    fdn: FdnReverb,
    bypass: bool,
}

impl VelvetHall {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| LUX_PARAMS[i].default),
            fdn: FdnReverb::new(48_000.0),
            bypass: false,
        }
    }
}

impl EffectBase for VelvetHall {
    const SPEC: &'static [ParamSpec] = LUX_PARAMS;
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
        self.fdn.size = self.p[0].clamp(0.1, 2.0);
        let freeze = self.p[4].clamp(0.0, 1.0);
        self.fdn.decay = if freeze > 0.5 {
            0.9995
        } else {
            self.p[1].clamp(0.1, 0.999)
        };
        self.fdn.damp_amount = self.p[2].clamp(0.0, 1.0);
        self.fdn.predelay_ms = self.p[3].clamp(0.0, 200.0);
        self.fdn.mix = self.p[5].clamp(0.0, 1.0);
        self.fdn.process(left, right, sr);
        let out = db_to_gain(self.p[6]);
        if out != 1.0 {
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                *l *= out;
                *r *= out;
            }
        }
    }
    fn reset_fx(&mut self) {
        self.fdn.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================= Convolver

pub static CONV_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Type", 0.0, 3.0, 0.0, ""),
    ParamSpec::new("Predelay", 0.0, 200.0, 10.0, "ms"),
    ParamSpec::new("Low Cut", 20.0, 1000.0, 60.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("High Cut", 1000.0, 20_000.0, 10_000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Mix", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

const CONV_HOP: usize = 512;
const CONV_MAX_IR: usize = 115_200;

/// Four built-in impulses (room, hall, plate, gate) convolved per channel.
/// Type changes select a prebuilt pair — nothing is allocated after `new`.
pub struct ConvolverFx {
    pub name: &'static str,
    pub p: [f32; CONV_PARAMS.len()],
    convs: [[Convolver; 2]; 4],
    stagers: [HopStager; 2],
    pre_l: DelayLine,
    pre_r: DelayLine,
    hp: SBiquad,
    lp: SBiquad,
    bypass: bool,
    dirty: bool,
}

impl ConvolverFx {
    pub fn new(name: &'static str) -> Self {
        let mut convs: [[Convolver; 2]; 4] = std::array::from_fn(|_| {
            [
                Convolver::new(CONV_MAX_IR, CONV_HOP),
                Convolver::new(CONV_MAX_IR, CONV_HOP),
            ]
        });
        for (k, pair) in convs.iter_mut().enumerate() {
            let ir = gen_ir(k, 48_000.0);
            pair[0].set_ir(&ir);
            pair[1].set_ir(&ir);
        }
        Self {
            name,
            p: std::array::from_fn(|i| CONV_PARAMS[i].default),
            convs,
            stagers: [HopStager::new(CONV_HOP), HopStager::new(CONV_HOP)],
            pre_l: DelayLine::new(9800),
            pre_r: DelayLine::new(9800),
            hp: SBiquad::new(),
            lp: SBiquad::new(),
            bypass: false,
            dirty: true,
        }
    }
}

impl EffectBase for ConvolverFx {
    const SPEC: &'static [ParamSpec] = CONV_PARAMS;
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
        if index == 2 || index == 3 {
            self.dirty = true;
        }
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, _tempo: f32) {
        if self.dirty {
            self.dirty = false;
            self.hp.set(BiquadCoeffs::compute(
                FilterKind::HighPass,
                self.p[2].max(10.0),
                0.7,
                0.0,
                sr,
            ));
            self.lp.set(BiquadCoeffs::compute(
                FilterKind::LowPass,
                self.p[3].min(sr * 0.45),
                0.7,
                0.0,
                sr,
            ));
        }
        let kind = (self.p[0].clamp(0.0, 3.0) as u32 % 4) as usize;
        let pre_s = (self.p[1].clamp(0.0, 200.0) * 0.001 * sr).clamp(1.0, 9600.0);
        let mix = self.p[4].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[5]);
        let n = left.len().min(right.len());
        let (pl, pr) = (&mut self.pre_l, &mut self.pre_r);
        let (s0, s1) = self.stagers.split_at_mut(1);
        let s0 = &mut s0[0];
        let s1 = &mut s1[0];
        let (c0, c1) = self.convs[kind].split_at_mut(1);
        let cl = &mut c0[0];
        let cr = &mut c1[0];
        for i in 0..n {
            let dl = left[i];
            let dr = right[i];
            pl.push(dl);
            pr.push(dr);
            let wl = s0.tick(pl.read(pre_s), |b| cl.process_block(b));
            let wr = s1.tick(pr.read(pre_s), |b| cr.process_block(b));
            let t = self.hp.tick(wl, wr);
            let f = self.lp.tick(t.0, t.1);
            left[i] = (dl * (1.0 - mix) + f.0 * mix) * out;
            right[i] = (dr * (1.0 - mix) + f.1 * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        for pair in self.convs.iter_mut() {
            pair[0].reset();
            pair[1].reset();
        }
        for st in self.stagers.iter_mut() {
            st.reset();
        }
        self.pre_l.clear();
        self.pre_r.clear();
        self.hp.reset();
        self.lp.reset();
        self.dirty = true;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

/// Deterministic synthetic impulses: decaying filtered noise with a small
/// early transient; the gated variant is hard-truncated.
fn gen_ir(kind: usize, sr: f32) -> Vec<f32> {
    let (secs, tau, bright) = match kind {
        0 => (0.35f32, 0.10f32, 0.55f32), // room
        1 => (2.20, 0.55, 0.30),          // hall
        2 => (1.40, 0.35, 0.75),          // plate
        _ => (0.18, 0.06, 0.45),          // gated
    };
    let len = (secs * sr) as usize;
    let mut rng = Rng::new(0x1000 + kind as u64);
    let mut out = Vec::with_capacity(len);
    let mut lp = 0.0f32;
    let coef = 0.15 + bright * 0.65;
    for i in 0..len {
        let t = i as f32 / sr;
        let env = (-t / tau).exp();
        let n = rng.next_f32() * 2.0 - 1.0;
        lp += (n - lp) * coef;
        let mut v = lp * env;
        if i < (0.004 * sr) as usize {
            v += env * 0.5;
        }
        out.push(v);
    }
    if kind == 3 {
        let fade = (0.008 * sr) as usize;
        let l = out.len();
        for i in l.saturating_sub(fade)..l {
            out[i] *= (l - i) as f32 / fade as f32;
        }
    }
    let peak = out.iter().fold(0.0f32, |a, b| a.max(b.abs())).max(1e-6);
    let g = 0.5 / peak;
    for v in out.iter_mut() {
        *v *= g;
    }
    out
}

impl_effect!(SilverRoom);
impl_effect!(SilverRoom2);
impl_effect!(VelvetHall);
impl_effect!(ConvolverFx);
