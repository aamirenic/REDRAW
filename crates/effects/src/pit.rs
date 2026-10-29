//! Pitch and time effects: looping scrubs, two pitch correctors over a shared
//! autocorrelation core, granular shifting, frequency shifting, scratch
//! playback and the tape-brake machine. Buffers are fixed at construction.
#![forbid(unsafe_code)]

use crate::common::{db_to_gain, detect_pitch, impl_effect, ApfStage, EffectBase, PitchShiftCore};
use redraw_dsp::{tanh_dist, wrap01, DelayLine, Lfo, Rng, TWO_PI};
use redraw_shared::ParamSpec;

// =================================================================== TimeLoop

pub static TIMELOOP_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Buffer", 0.25, 2.0, 1.0, "s"),
    ParamSpec::new("Step Rate", 1.0, 32.0, 8.0, "Hz"),
    ParamSpec::new("Time Depth", 0.0, 1.0, 0.6, ""),
    ParamSpec::new("Gate Depth", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.6, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Ring buffer read at stepped varispeed multipliers with stepped gating.
pub struct TimeLoop {
    pub name: &'static str,
    pub p: [f32; TIMELOOP_PARAMS.len()],
    dl: DelayLine,
    rng: Rng,
    step_left: f32,
    read_off: f32,
    mult: f32,
    mult_t: f32,
    gate: f32,
    gate_t: f32,
    bypass: bool,
}

impl TimeLoop {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| TIMELOOP_PARAMS[i].default),
            dl: DelayLine::new(120_000),
            rng: Rng::new(0x71),
            step_left: 0.0,
            read_off: 960.0,
            mult: 1.0,
            mult_t: 1.0,
            gate: 1.0,
            gate_t: 1.0,
            bypass: false,
        }
    }
}

impl EffectBase for TimeLoop {
    const SPEC: &'static [ParamSpec] = TIMELOOP_PARAMS;
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
        let buffer_len = (self.p[0].clamp(0.25, 2.0) * sr).clamp(2.0, 95_000.0);
        let rate = self.p[1].clamp(1.0, 32.0);
        let time_depth = self.p[2].clamp(0.0, 1.0);
        let gate_depth = self.p[3].clamp(0.0, 1.0);
        let mix = self.p[4].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[5]);
        let step_samples = (sr / rate).max(1.0);
        let coef = (-1.0 / (0.02 * sr)).exp();
        let n = left.len().min(right.len());
        for i in 0..n {
            let mono = (left[i] + right[i]) * 0.5;
            self.dl.push(mono);
            self.step_left -= 1.0;
            if self.step_left <= 0.0 {
                self.step_left += step_samples;
                if self.step_left <= 0.0 {
                    self.step_left = step_samples;
                }
                let r = self.rng.next_f32();
                self.mult_t = (1.0 + (r * 2.0 - 1.0) * time_depth * 0.9).clamp(0.1, 2.5);
                let g = self.rng.next_f32();
                self.gate_t = if g > gate_depth * 0.7 { 1.0 } else { 0.25 };
            }
            self.mult = self.mult_t + (self.mult - self.mult_t) * coef;
            self.gate = self.gate_t + (self.gate - self.gate_t) * coef;
            self.read_off += self.mult;
            if self.read_off >= buffer_len {
                self.read_off -= buffer_len;
            }
            if self.read_off < 1.0 {
                self.read_off = 1.0;
            }
            let wet = self.dl.read(self.read_off) * self.gate;
            left[i] = (left[i] * (1.0 - mix) + wet * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + wet * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.dl.clear();
        self.rng = Rng::new(0x71);
        self.step_left = 0.0;
        self.read_off = 960.0;
        self.mult = 1.0;
        self.mult_t = 1.0;
        self.gate = 1.0;
        self.gate_t = 1.0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// =================================================================== LoopChop

pub static LOOPCHOP_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Loop", 0.25, 4.0, 1.0, "s"),
    ParamSpec::new("Scramble", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.6, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Captures the incoming audio and replays eight chopped segments, in order or
/// scrambled. Capture runs continuously; explicit record arming is host-level.
pub struct LoopChop {
    pub name: &'static str,
    pub p: [f32; LOOPCHOP_PARAMS.len()],
    dl: DelayLine,
    rng: Rng,
    step: usize,
    step_left: f32,
    play_head: f32,
    bypass: bool,
}

impl LoopChop {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| LOOPCHOP_PARAMS[i].default),
            dl: DelayLine::new(220_000),
            rng: Rng::new(0x72),
            step: 0,
            step_left: 0.0,
            play_head: 0.0,
            bypass: false,
        }
    }
}

impl EffectBase for LoopChop {
    const SPEC: &'static [ParamSpec] = LOOPCHOP_PARAMS;
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
        let loop_len = (self.p[0].clamp(0.25, 4.0) * sr).clamp(2.0, 191_000.0);
        let scramble = self.p[1].clamp(0.0, 1.0);
        let mix = self.p[2].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[3]);
        let seg = loop_len / 8.0;
        let n = left.len().min(right.len());
        for i in 0..n {
            let mono = (left[i] + right[i]) * 0.5;
            self.dl.push(mono);
            self.step_left -= 1.0;
            if self.step_left <= 0.0 {
                self.step_left += seg;
                if self.step_left <= 0.0 {
                    self.step_left = seg;
                }
                self.step = if self.rng.next_f32() < scramble {
                    (self.rng.next_f32() * 7.999) as usize
                } else {
                    (self.step + 1) % 8
                };
                self.play_head = self.step as f32 * seg;
            }
            self.play_head += 1.0;
            if self.play_head >= loop_len {
                self.play_head -= loop_len;
            }
            let delay = (loop_len - self.play_head).clamp(1.0, 219_000.0);
            let wet = self.dl.read(delay);
            left[i] = (left[i] * (1.0 - mix) + wet * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + wet * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.dl.clear();
        self.rng = Rng::new(0x72);
        self.step = 0;
        self.step_left = 0.0;
        self.play_head = 0.0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================== pitch-correction core

/// Shared monophonic correction engine: 2048-sample autocorrelation analysis
/// every half buffer, nearest-scale-note ratio, smoothed granular shifting.
struct CorrectorCore {
    hist: Vec<f32>,
    pos: usize,
    detected: f32,
    ratio: f32,
    sh_l: PitchShiftCore,
    sh_r: PitchShiftCore,
}

impl CorrectorCore {
    fn new() -> Self {
        Self {
            hist: vec![0.0; 2048],
            pos: 0,
            detected: 0.0,
            ratio: 1.0,
            sh_l: PitchShiftCore::new(),
            sh_r: PitchShiftCore::new(),
        }
    }
    fn reset(&mut self) {
        self.hist.fill(0.0);
        self.pos = 0;
        self.detected = 0.0;
        self.ratio = 1.0;
        self.sh_l.reset();
        self.sh_r.reset();
    }
    fn tick(
        &mut self,
        l: f32,
        r: f32,
        sr: f32,
        strength: f32,
        key: u32,
        speed_ms: f32,
    ) -> (f32, f32) {
        self.hist[self.pos] = (l + r) * 0.5;
        self.pos += 1;
        if self.pos >= self.hist.len() {
            if let Some(hz) = detect_pitch(&self.hist, sr) {
                self.detected = hz;
            }
            let half = self.hist.len() / 2;
            self.hist.copy_within(half.., 0);
            self.pos = half;
        }
        let mut target = 1.0f32;
        if self.detected > 40.0 {
            let midi = 69.0 + 12.0 * (self.detected / 440.0).log2();
            let snapped = snap_to_key(midi.round() as i32, key) as f32;
            let want = 440.0 * 2f32.powf((snapped - 69.0) / 12.0);
            let corr = (want / self.detected).clamp(0.5, 2.0);
            target = 1.0 + (corr - 1.0) * strength.clamp(0.0, 1.0);
        }
        let coef = (-1.0 / (speed_ms.clamp(0.5, 500.0) * 0.001 * sr)).exp();
        self.ratio = target + (self.ratio - target) * coef;
        let wl = self.sh_l.tick(l, self.ratio, sr);
        let wr = self.sh_r.tick(r, self.ratio, sr);
        (wl, wr)
    }
}

fn snap_to_key(midi: i32, key: u32) -> i32 {
    const MAJOR: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
    let mut best = midi;
    let mut best_d = i32::MAX;
    for cand in (midi - 6)..=(midi + 6) {
        let pc = (cand - key as i32).rem_euclid(12);
        if MAJOR.contains(&pc) {
            let d = (cand - midi).abs();
            if d < best_d {
                best_d = d;
                best = cand;
            }
        }
    }
    best
}

// ================================================================= TuneCanvas

pub static TUNE_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Strength", 0.0, 1.0, 0.8, ""),
    ParamSpec::new("Key", 0.0, 11.0, 0.0, ""),
    ParamSpec::new("Speed", 5.0, 200.0, 40.0, "ms"),
    ParamSpec::new("Mix", 0.0, 1.0, 0.8, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct TuneCanvas {
    pub name: &'static str,
    pub p: [f32; TUNE_PARAMS.len()],
    core: CorrectorCore,
    bypass: bool,
}

impl TuneCanvas {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| TUNE_PARAMS[i].default),
            core: CorrectorCore::new(),
            bypass: false,
        }
    }
}

impl EffectBase for TuneCanvas {
    const SPEC: &'static [ParamSpec] = TUNE_PARAMS;
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
        let strength = self.p[0].clamp(0.0, 1.0);
        let key = self.p[1].clamp(0.0, 11.0) as u32;
        let speed = self.p[2].clamp(5.0, 200.0);
        let mix = self.p[3].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[4]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let (wl, wr) = self.core.tick(left[i], right[i], sr, strength, key, speed);
            left[i] = (left[i] * (1.0 - mix) + wl * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + wr * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.core.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// =============================================================== PitchCorrect

pub static PCORR_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Strength", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Key", 0.0, 11.0, 0.0, ""),
    ParamSpec::new("Speed", 1.0, 100.0, 15.0, "ms"),
    ParamSpec::new("Mix", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Live variant of the corrector: faster smoothing, full strength by default.
pub struct PitchCorrect {
    pub name: &'static str,
    pub p: [f32; PCORR_PARAMS.len()],
    core: CorrectorCore,
    bypass: bool,
}

impl PitchCorrect {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| PCORR_PARAMS[i].default),
            core: CorrectorCore::new(),
            bypass: false,
        }
    }
}

impl EffectBase for PitchCorrect {
    const SPEC: &'static [ParamSpec] = PCORR_PARAMS;
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
        let strength = self.p[0].clamp(0.0, 1.0);
        let key = self.p[1].clamp(0.0, 11.0) as u32;
        let speed = self.p[2].clamp(1.0, 100.0);
        let mix = self.p[3].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[4]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let (wl, wr) = self.core.tick(left[i], right[i], sr, strength, key, speed);
            left[i] = (left[i] * (1.0 - mix) + wl * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + wr * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.core.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================= PitchShift

pub static PSHIFT_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Semitones", -24.0, 24.0, 0.0, "st"),
    ParamSpec::new("Mix", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct PitchShiftFx {
    pub name: &'static str,
    pub p: [f32; PSHIFT_PARAMS.len()],
    sh_l: PitchShiftCore,
    sh_r: PitchShiftCore,
    bypass: bool,
}

impl PitchShiftFx {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| PSHIFT_PARAMS[i].default),
            sh_l: PitchShiftCore::new(),
            sh_r: PitchShiftCore::new(),
            bypass: false,
        }
    }
}

impl EffectBase for PitchShiftFx {
    const SPEC: &'static [ParamSpec] = PSHIFT_PARAMS;
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
        let ratio = 2f32.powf(self.p[0].clamp(-24.0, 24.0) / 12.0);
        let mix = self.p[1].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[2]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let wl = self.sh_l.tick(left[i], ratio, sr);
            let wr = self.sh_r.tick(right[i], ratio, sr);
            left[i] = (left[i] * (1.0 - mix) + wl * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + wr * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.sh_l.reset();
        self.sh_r.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================== FreqShift

pub static FSHIFT_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Shift", -2000.0, 2000.0, 200.0, "Hz"),
    ParamSpec::new("Mix", 0.0, 1.0, 0.6, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Single-sideband frequency shifter: two allpass quadrature chains feed a
/// complex rotation. The 2+2 stage pair approximates 90 degrees across the
/// voice band (D-016 simplification of a longer Hilbert network).
pub struct FreqShift {
    pub name: &'static str,
    pub p: [f32; FSHIFT_PARAMS.len()],
    chain_i: [ApfStage; 2],
    chain_q: [ApfStage; 2],
    phase: f32,
    bypass: bool,
}

impl FreqShift {
    pub fn new(name: &'static str) -> Self {
        let mut chain_i = [ApfStage::default(); 2];
        chain_i[0].a = 0.479_400_9;
        chain_i[1].a = 0.876_171_6;
        let mut chain_q = [ApfStage::default(); 2];
        chain_q[0].a = 0.976_222_4;
        chain_q[1].a = 0.998_289_0;
        Self {
            name,
            p: std::array::from_fn(|i| FSHIFT_PARAMS[i].default),
            chain_i,
            chain_q,
            phase: 0.0,
            bypass: false,
        }
    }
}

impl EffectBase for FreqShift {
    const SPEC: &'static [ParamSpec] = FSHIFT_PARAMS;
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
        let shift = self.p[0].clamp(-2000.0, 2000.0);
        let mix = self.p[1].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[2]);
        let step = shift / sr;
        let n = left.len().min(right.len());
        for i in 0..n {
            let mono = (left[i] + right[i]) * 0.5;
            let mut xi = mono;
            for st in self.chain_i.iter_mut() {
                xi = st.tick(xi);
            }
            let mut xq = mono;
            for st in self.chain_q.iter_mut() {
                xq = st.tick(xq);
            }
            self.phase = wrap01(self.phase + step);
            let (sn, cs) = (TWO_PI * self.phase).sin_cos();
            let wet = xi * cs - xq * sn;
            left[i] = (left[i] * (1.0 - mix) + wet * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + wet * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        for st in self.chain_i.iter_mut().chain(self.chain_q.iter_mut()) {
            st.reset();
        }
        self.phase = 0.0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================ ScratchDeck

pub static SCRATCH_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Speed", 0.0, 2.0, 1.0, "x"),
    ParamSpec::new("Wobble Rate", 0.05, 8.0, 1.0, "Hz"),
    ParamSpec::new("Wobble Depth", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Continuous capture with a drifting read head: speed above/below 1 scrubs
/// the recent history; wobble adds hand-on-vinyl pitch sway.
pub struct ScratchDeck {
    pub name: &'static str,
    pub p: [f32; SCRATCH_PARAMS.len()],
    dl_l: DelayLine,
    dl_r: DelayLine,
    drift: f32,
    lfo: Lfo,
    bypass: bool,
}

impl ScratchDeck {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| SCRATCH_PARAMS[i].default),
            dl_l: DelayLine::new(150_000),
            dl_r: DelayLine::new(150_000),
            drift: 0.0,
            lfo: Lfo::new(0x73),
            bypass: false,
        }
    }
}

impl EffectBase for ScratchDeck {
    const SPEC: &'static [ParamSpec] = SCRATCH_PARAMS;
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
        let speed = self.p[0].clamp(0.0, 2.0);
        let wob_rate = self.p[1].clamp(0.05, 8.0);
        let wob_depth = self.p[2].clamp(0.0, 1.0);
        let mix = self.p[3].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[4]);
        self.lfo.set_rate(wob_rate);
        let base = (0.4 * sr).clamp(2.0, 30_000.0);
        let wob_ms = wob_depth * 0.006 * sr;
        let n = left.len().min(right.len());
        for i in 0..n {
            self.dl_l.push(left[i]);
            self.dl_r.push(right[i]);
            self.drift += 1.0 - speed;
            self.drift = self.drift.clamp(0.0, base - 200.0);
            let lf = self.lfo.tick(sr);
            let d = (base - self.drift + lf * wob_ms).clamp(1.0, 149_000.0);
            let wl = self.dl_l.read(d);
            let wr = self.dl_r.read(d);
            left[i] = (left[i] * (1.0 - mix) + wl * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + wr * mix) * out;
        }
    }
    fn reset_fx(&mut self) {
        self.dl_l.clear();
        self.dl_r.clear();
        self.drift = 0.0;
        self.lfo.reset();
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// =================================================================== TapeWarp

pub static TAPEWARP_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Brake", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Time", 0.05, 4.0, 1.2, "s"),
    ParamSpec::new("Wobble", 0.0, 1.0, 0.2, ""),
    ParamSpec::new("Mix", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Tape-stop machine: a rising Brake edge slews playback to a crawl, the
/// falling edge spins back up faster. Drift is capped by the ring length.
pub struct TapeWarp {
    pub name: &'static str,
    pub p: [f32; TAPEWARP_PARAMS.len()],
    dl_l: DelayLine,
    dl_r: DelayLine,
    rate: f32,
    drift: f32,
    phase: f32,
    bypass: bool,
}

impl TapeWarp {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| TAPEWARP_PARAMS[i].default),
            dl_l: DelayLine::new(240_000),
            dl_r: DelayLine::new(240_000),
            rate: 1.0,
            drift: 0.0,
            phase: 0.0,
            bypass: false,
        }
    }
}

impl EffectBase for TapeWarp {
    const SPEC: &'static [ParamSpec] = TAPEWARP_PARAMS;
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
        let braking = self.p[0] >= 0.5;
        let time_s = self.p[1].clamp(0.05, 4.0);
        let wob = self.p[2].clamp(0.0, 1.0);
        let mix = self.p[3].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[4]);
        let down_step = 0.98 / (time_s * sr).max(1.0);
        let up_step = 0.98 / (time_s * 0.45 * sr).max(1.0);
        let target = if braking {
            0.02
        } else if self.drift > 240.0 {
            1.6
        } else {
            1.0
        };
        let wob_ms = wob * 0.004 * sr;
        let n = left.len().min(right.len());
        for i in 0..n {
            self.dl_l.push(left[i]);
            self.dl_r.push(right[i]);
            if self.rate < target {
                self.rate = (self.rate + up_step).min(target);
            } else if self.rate > target {
                self.rate = (self.rate - down_step).max(target);
            }
            self.drift += 1.0 - self.rate;
            self.drift = self.drift.clamp(0.0, 230_000.0);
            self.phase = wrap01(self.phase + 5.0 / sr);
            let sway = (TWO_PI * self.phase).sin() * wob_ms;
            let d = (self.drift + 2.0 + sway).clamp(1.0, 239_000.0);
            let wl = tanh_dist(self.dl_l.read(d), 1.5);
            let wr = tanh_dist(self.dl_r.read(d), 1.5);
            left[i] = (left[i] * (1.0 - mix) + wl * mix) * out;
            right[i] = (right[i] * (1.0 - mix) + wr * mix) * out;
        }
        // keep the ring anchored: drift alone bounds the read head
        if self.drift > 235_000.0 {
            self.drift = 235_000.0;
        }
    }
    fn reset_fx(&mut self) {
        self.dl_l.clear();
        self.dl_r.clear();
        self.rate = 1.0;
        self.drift = 0.0;
        self.phase = 0.0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

impl_effect!(TimeLoop);
impl_effect!(LoopChop);
impl_effect!(TuneCanvas);
impl_effect!(PitchCorrect);
impl_effect!(PitchShiftFx);
impl_effect!(FreqShift);
impl_effect!(ScratchDeck);
impl_effect!(TapeWarp);
