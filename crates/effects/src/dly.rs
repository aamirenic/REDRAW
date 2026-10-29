//! Delay-family effects: dual-line stereo, ping-pong, tempo-synced, an 8-line
//! bank, per-band delays and the stepper transmitter stand-in. Feedback paths
//! are damped by one-pole low passes; line sizes are fixed at construction.
#![forbid(unsafe_code)]

use crate::common::{db_to_gain, impl_effect, BandSplit, DelayNet, EffectBase};
use redraw_dsp::{DelayLine, Rng};
use redraw_shared::{pan_gains, ParamCurve, ParamSpec};

// ================================================================== Delay One

pub static DELAY_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Time L", 5.0, 1000.0, 250.0, "ms").curve(ParamCurve::Exp),
    ParamSpec::new("Time R", 5.0, 1000.0, 250.0, "ms").curve(ParamCurve::Exp),
    ParamSpec::new("Feedback", 0.0, 0.95, 0.45, ""),
    ParamSpec::new("Damp", 0.0, 1.0, 0.2, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.35, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct DelayOne {
    pub name: &'static str,
    pub p: [f32; DELAY_PARAMS.len()],
    net: DelayNet,
    bypass: bool,
    dirty: bool,
}

impl DelayOne {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| DELAY_PARAMS[i].default),
            net: DelayNet::new(1100.0),
            bypass: false,
            dirty: true,
        }
    }
    fn refresh(&mut self, sr: f32) {
        self.dirty = false;
        let cut = 500.0 + (1.0 - self.p[3].clamp(0.0, 1.0)) * 19_000.0;
        self.net.lp_l.set_lowpass(cut, sr);
        self.net.lp_r.set_lowpass(cut, sr);
    }
}

impl EffectBase for DelayOne {
    const SPEC: &'static [ParamSpec] = DELAY_PARAMS;
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
            self.refresh(sr);
        }
        let t_l = (self.p[0].clamp(5.0, 1000.0) * 0.001 * sr).clamp(1.0, 65_000.0);
        let t_r = (self.p[1].clamp(5.0, 1000.0) * 0.001 * sr).clamp(1.0, 65_000.0);
        let fb = self.p[2].clamp(0.0, 0.95);
        let mix = self.p[4].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[5]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let dl = left[i];
            let dr = right[i];
            let wl = self.net.dl_l.read(t_l);
            let wr = self.net.dl_r.read(t_r);
            let fbl = self.net.lp_l.tick(wl) * fb;
            let fbr = self.net.lp_r.tick(wr) * fb;
            self.net.dl_l.push(dl + fbl);
            self.net.dl_r.push(dr + fbr);
            left[i] = (dl * (1.0 - mix) + wl * mix) * out;
            right[i] = (dr * (1.0 - mix) + wr * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
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

// ================================================================== Delay Two

/// Crossed feedback (ping-pong) stereo delay.
pub struct DelayTwo {
    pub name: &'static str,
    pub p: [f32; DELAY_PARAMS.len()],
    net: DelayNet,
    bypass: bool,
    dirty: bool,
}

impl DelayTwo {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| DELAY_PARAMS[i].default),
            net: DelayNet::new(1100.0),
            bypass: false,
            dirty: true,
        }
    }
}

impl EffectBase for DelayTwo {
    const SPEC: &'static [ParamSpec] = DELAY_PARAMS;
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
            self.dirty = false;
            let cut = 500.0 + (1.0 - self.p[3].clamp(0.0, 1.0)) * 19_000.0;
            self.net.lp_l.set_lowpass(cut, sr);
            self.net.lp_r.set_lowpass(cut, sr);
        }
        let t_l = (self.p[0].clamp(5.0, 1000.0) * 0.001 * sr).clamp(1.0, 65_000.0);
        let t_r = (self.p[1].clamp(5.0, 1000.0) * 0.001 * sr).clamp(1.0, 65_000.0);
        let fb = self.p[2].clamp(0.0, 0.95);
        let mix = self.p[4].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[5]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let dl = left[i];
            let dr = right[i];
            let wl = self.net.dl_l.read(t_l);
            let wr = self.net.dl_r.read(t_r);
            let fbl = self.net.lp_l.tick(wr) * fb;
            let fbr = self.net.lp_r.tick(wl) * fb;
            self.net.dl_l.push(dr + fbl);
            self.net.dl_r.push(dl + fbr);
            left[i] = (dl * (1.0 - mix) + wl * mix) * out;
            right[i] = (dr * (1.0 - mix) + wr * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
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

// ================================================================ Delay Three

pub static DELAY3_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Sync", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Time", 5.0, 1000.0, 300.0, "ms").curve(ParamCurve::Exp),
    ParamSpec::new("Beats", 0.25, 4.0, 1.0, "beat").curve(ParamCurve::Exp),
    ParamSpec::new("Feedback", 0.0, 0.95, 0.4, ""),
    ParamSpec::new("Damp", 0.0, 1.0, 0.25, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.35, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Delay with tempo-sync: beats mode derives its time from the transport.
pub struct DelayThree {
    pub name: &'static str,
    pub p: [f32; DELAY3_PARAMS.len()],
    net: DelayNet,
    bypass: bool,
    dirty: bool,
}

impl DelayThree {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| DELAY3_PARAMS[i].default),
            net: DelayNet::new(1100.0),
            bypass: false,
            dirty: true,
        }
    }
}

impl EffectBase for DelayThree {
    const SPEC: &'static [ParamSpec] = DELAY3_PARAMS;
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
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, tempo: f32) {
        if self.dirty {
            self.dirty = false;
            let cut = 500.0 + (1.0 - self.p[4].clamp(0.0, 1.0)) * 19_000.0;
            self.net.lp_l.set_lowpass(cut, sr);
            self.net.lp_r.set_lowpass(cut, sr);
        }
        let sync = self.p[0] > 0.5;
        let t_ms = if sync {
            (60.0 / tempo.max(20.0)) * self.p[2].clamp(0.25, 4.0) * 1000.0
        } else {
            self.p[1].clamp(5.0, 1000.0)
        };
        let t = (t_ms * 0.001 * sr).clamp(1.0, 65_000.0);
        let fb = self.p[3].clamp(0.0, 0.95);
        let mix = self.p[5].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[6]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let dl = left[i];
            let dr = right[i];
            let wl = self.net.dl_l.read(t);
            let wr = self.net.dl_r.read(t);
            let fbl = self.net.lp_l.tick(wl) * fb;
            let fbr = self.net.lp_r.tick(wr) * fb;
            self.net.dl_l.push(dl + fbl);
            self.net.dl_r.push(dr + fbr);
            left[i] = (dl * (1.0 - mix) + wl * mix) * out;
            right[i] = (dr * (1.0 - mix) + wr * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
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

// =============================================================== Delay Bank 8

pub static DBANK_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Time", 10.0, 800.0, 120.0, "ms").curve(ParamCurve::Exp),
    ParamSpec::new("Feedback", 0.0, 0.92, 0.4, ""),
    ParamSpec::new("Spread", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.4, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Eight short lines spread across the stereo field for dense spatial taps.
pub struct DelayBank8 {
    pub name: &'static str,
    pub p: [f32; DBANK_PARAMS.len()],
    lines: [DelayLine; 8],
    bypass: bool,
}

impl DelayBank8 {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| DBANK_PARAMS[i].default),
            lines: std::array::from_fn(|_| DelayLine::new(48_000)),
            bypass: false,
        }
    }
}

impl EffectBase for DelayBank8 {
    const SPEC: &'static [ParamSpec] = DBANK_PARAMS;
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
        let base = self.p[0].clamp(10.0, 800.0);
        let fb = self.p[1].clamp(0.0, 0.92);
        let spread = self.p[2].clamp(0.0, 1.0);
        let mix = self.p[3].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[4]);
        let weight = 1.0 / 8f32.sqrt();
        let n = left.len().min(right.len());
        for i in 0..n {
            let mono = (left[i] + right[i]) * 0.5;
            let mut wl = 0.0f32;
            let mut wr = 0.0f32;
            for (k, line) in self.lines.iter_mut().enumerate() {
                let mul = 0.5 + spread * (k as f32) * 0.125;
                let t = (base * mul * 0.001 * sr).clamp(1.0, 47_900.0);
                let w = line.read(t);
                line.push(mono + w * fb);
                let pan = spread * ((k as f32 / 7.0) * 2.0 - 1.0) * 0.8;
                let (pl, pr) = pan_gains(pan);
                wl += w * pl * weight;
                wr += w * pr * weight;
            }
            left[i] = (left[i] * (1.0 - mix) + wl * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + wr * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        for line in self.lines.iter_mut() {
            line.clear();
        }
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ============================================================= Multi Band Delay

pub static MBD_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Split 1", 100.0, 2000.0, 250.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Split 2", 1000.0, 12_000.0, 2500.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Time Low", 5.0, 800.0, 200.0, "ms").curve(ParamCurve::Exp),
    ParamSpec::new("Time High", 5.0, 800.0, 60.0, "ms").curve(ParamCurve::Exp),
    ParamSpec::new("Feedback", 0.0, 0.9, 0.35, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Three-band delay with independent low/high times; bands are summed mono
/// until per-band routing arrives with the mixer sends (D-016 stand-in).
pub struct MultiBandDelay {
    pub name: &'static str,
    pub p: [f32; MBD_PARAMS.len()],
    split: BandSplit,
    lines: [DelayLine; 3],
    bypass: bool,
    dirty: bool,
}

impl MultiBandDelay {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| MBD_PARAMS[i].default),
            split: BandSplit::new(250.0, 2500.0),
            lines: std::array::from_fn(|_| DelayLine::new(48_000)),
            bypass: false,
            dirty: true,
        }
    }
    fn recompute(&mut self, sr: f32) {
        self.dirty = false;
        self.split.set_edges(self.p[0], self.p[1], sr);
    }
}

impl EffectBase for MultiBandDelay {
    const SPEC: &'static [ParamSpec] = MBD_PARAMS;
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
        let fb = self.p[4].clamp(0.0, 0.9);
        let mix = self.p[5].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[6]);
        let lo = self.p[2].clamp(5.0, 800.0);
        let hi = self.p[3].clamp(5.0, 800.0);
        let mid = (lo * hi).sqrt();
        let times = [lo, mid, hi];
        let n = left.len().min(right.len());
        for i in 0..n {
            let (b0, b1, b2) = self.split.split(left[i], right[i]);
            let bands = [b0, b1, b2];
            let mut wet = 0.0f32;
            for (k, line) in self.lines.iter_mut().enumerate() {
                let t = (times[k] * 0.001 * sr).clamp(1.0, 47_900.0);
                let w = line.read(t);
                line.push(bands[k] + w * fb);
                wet += w;
            }
            let y = wet * 0.6;
            left[i] = (left[i] * (1.0 - mix) + y * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + y * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        for line in self.lines.iter_mut() {
            line.clear();
        }
        self.split.reset();
        self.dirty = true;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ============================================================= BitTransmitter

pub static BITTX_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Density", 0.0, 1.0, 0.6, ""),
    ParamSpec::new("Steps", 1.0, 4.0, 4.0, "/beat"),
    ParamSpec::new("Mix", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Stepped gate pattern generated from a seeded RNG at 1..4 steps per beat.
/// The original transmitter forwards note events; this is the audio-domain
/// stand-in for its rhythmic stepping until the sequencer host ships.
pub struct BitTransmitter {
    pub name: &'static str,
    pub p: [f32; BITTX_PARAMS.len()],
    rng: Rng,
    step_left: f32,
    gate: f32,
    active: bool,
    bypass: bool,
}

impl BitTransmitter {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| BITTX_PARAMS[i].default),
            rng: Rng::new(0x51),
            step_left: 0.0,
            gate: 0.0,
            active: false,
            bypass: false,
        }
    }
}

impl EffectBase for BitTransmitter {
    const SPEC: &'static [ParamSpec] = BITTX_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, tempo: f32) {
        let density = self.p[0].clamp(0.0, 1.0);
        let steps = self.p[1].clamp(1.0, 4.0);
        let mix = self.p[2].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[3]);
        let step_samples = ((60.0 / tempo.max(20.0)) / steps * sr).max(1.0);
        let atk = (-1.0 / (0.002 * sr)).exp();
        let rel = (-1.0 / (0.030 * sr)).exp();
        let n = left.len().min(right.len());
        for i in 0..n {
            self.step_left -= 1.0;
            if self.step_left <= 0.0 {
                self.step_left += step_samples;
                if self.step_left <= 0.0 {
                    self.step_left = step_samples;
                }
                self.active = self.rng.next_f32() < density;
            }
            let target = if self.active { 1.0 } else { 0.0 };
            let coef = if target > self.gate { atk } else { rel };
            self.gate = target + (self.gate - target) * coef;
            left[i] = (left[i] * (1.0 - mix) + left[i] * self.gate * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + right[i] * self.gate * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.rng = Rng::new(0x51);
        self.step_left = 0.0;
        self.gate = 0.0;
        self.active = false;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

impl_effect!(DelayOne);
impl_effect!(DelayTwo);
impl_effect!(DelayThree);
impl_effect!(DelayBank8);
impl_effect!(MultiBandDelay);
impl_effect!(BitTransmitter);
