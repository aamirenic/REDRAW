//! 6-operator FM engine with four algorithm routings, per-operator ratios/levels,
//! feedback, filter and envelopes. 8 preallocated voices.
#![forbid(unsafe_code)]

use redraw_dsp::{wrap01, Adsr, Svf, SvfMode};
use redraw_shared::{midi_note_hz as hz, pan_gains, ParamCurve, ParamSpec, Synth};

pub const FM_POLY: usize = 8;
pub const FM_OPS: usize = 6;

#[rustfmt::skip]
pub static FM_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Op1 Level", 0.0, 1.0, 0.8, ""),
    ParamSpec::new("Op2 Level", 0.0, 1.0, 0.4, ""),
    ParamSpec::new("Op3 Level", 0.0, 1.0, 0.35, ""),
    ParamSpec::new("Op4 Level", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Op5 Level", 0.0, 1.0, 0.25, ""),
    ParamSpec::new("Op6 Level", 0.0, 1.0, 0.2, ""),
    ParamSpec::new("Op1 Ratio", 0.0625, 16.0, 1.0, "x").curve(ParamCurve::Exp),
    ParamSpec::new("Op2 Ratio", 0.0625, 16.0, 2.0, "x").curve(ParamCurve::Exp),
    ParamSpec::new("Op3 Ratio", 0.0625, 16.0, 3.0, "x").curve(ParamCurve::Exp),
    ParamSpec::new("Op4 Ratio", 0.0625, 16.0, 4.0, "x").curve(ParamCurve::Exp),
    ParamSpec::new("Op5 Ratio", 0.0625, 16.0, 5.0, "x").curve(ParamCurve::Exp),
    ParamSpec::new("Op6 Ratio", 0.0625, 16.0, 6.0, "x").curve(ParamCurve::Exp),
    ParamSpec::new("Algorithm", 0.0, 3.0, 0.0, ""),
    ParamSpec::new("Feedback", 0.0, 8.0, 1.5, ""),
    ParamSpec::new("Cutoff", 20.0, 20000.0, 12000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Resonance", 0.15, 14.0, 0.7, ""),
    ParamSpec::new("Flt Env Amount", -6.0, 6.0, 1.0, "oct"),
    ParamSpec::new("Amp Attack", 0.0005, 8.0, 0.004, "s"),
    ParamSpec::new("Amp Decay", 0.0005, 8.0, 0.25, "s"),
    ParamSpec::new("Amp Sustain", 0.0, 1.0, 0.7, ""),
    ParamSpec::new("Amp Release", 0.0005, 8.0, 0.3, "s"),
    ParamSpec::new("Flt Attack", 0.0005, 8.0, 0.004, "s"),
    ParamSpec::new("Flt Decay", 0.0005, 8.0, 0.3, "s"),
    ParamSpec::new("Flt Sustain", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Flt Release", 0.0005, 8.0, 0.4, "s"),
    ParamSpec::new("Detune", -200.0, 200.0, 0.0, "cents"),
    ParamSpec::new("Vel Sense", 0.0, 1.0, 0.6, ""),
    ParamSpec::new("Gain", 0.0, 2.0, 0.9, ""),
];

struct FmVoice {
    key: u8,
    gate: bool,
    age: u64,
    freq: f32,
    amp: Adsr,
    fenv: Adsr,
    phases: [f32; FM_OPS],
    fb_carrier: f32,
    svf: Svf,
}

impl Default for FmVoice {
    fn default() -> Self {
        Self {
            key: 0,
            gate: false,
            age: 0,
            freq: 0.0,
            amp: Adsr::new(),
            fenv: Adsr::new(),
            phases: [0.0; FM_OPS],
            fb_carrier: 0.0,
            svf: Svf::new(),
        }
    }
}

pub struct FmEngine {
    pub name: &'static str,
    pub sr: f32,
    pub p: [f32; FM_PARAMS.len()],
    voices: [FmVoice; FM_POLY],
    pub age: u64,
    pub last_freq: f32,
}

impl FmEngine {
    pub fn with_params(name: &'static str, values: &[f32]) -> Self {
        let mut e = Self {
            name,
            sr: 48_000.0,
            p: std::array::from_fn(|i| FM_PARAMS[i].default),
            voices: std::array::from_fn(|_| FmVoice::default()),
            age: 0,
            last_freq: 0.0,
        };
        for (i, v) in values.iter().enumerate().take(e.p.len()) {
            e.set_param(i, *v);
        }
        e
    }

    fn take_voice(&mut self) -> usize {
        if let Some(pos) = (0..FM_POLY).find(|&i| !self.voices[i].amp.is_active()) {
            return pos;
        }
        (0..FM_POLY)
            .min_by_key(|&i| self.voices[i].age)
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------- presets

impl FmEngine {
    pub fn hexad(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0.6, 0.35, 0.3, 0.25, 0.2, 0.25, 1., 2., 3., 5., 7., 9., 0., 2., 9000., 1., 1.5,
              0.005, 0.3, 0.7, 0.35, 0.004, 0.25, 0.3, 0.3, 6., 0.6, 0.9],
        )
    }
    pub fn ember(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0.7, 0.5, 0.4, 0.3, 0.2, 0.2, 1., 1.5, 2., 3., 4., 6., 2., 3.5, 6000., 3., 2.,
              0.002, 0.15, 0.6, 0.2, 0.002, 0.2, 0.25, 0.25, 0., 0.8, 0.9],
        )
    }
    pub fn dx_pair(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0.7, 0.45, 0.0, 0.55, 0.4, 0.0, 1., 3.0, 3., 1., 2., 5., 1., 1., 10000., 1.2, 1.,
              0.004, 0.2, 0.8, 0.25, 0.004, 0.2, 0.3, 0.3, 8., 0.5, 1.],
        )
    }
}

// ---------------------------------------------------------------- synth impl

impl Synth for FmEngine {
    fn synth_type(&self) -> &'static str {
        self.name
    }
    fn set_display_name(&mut self, name: &'static str) {
        self.name = name;
    }
    fn reset(&mut self) {
        for f in self.voices.iter_mut() {
            *f = FmVoice::default();
        }
        self.age = 0;
        self.last_freq = 0.0;
    }
    fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr.max(8000.0);
    }
    fn note_on(&mut self, key: u8, velocity: f32) {
        for v in self.voices.iter_mut() {
            if v.gate && v.key == key {
                v.gate = false;
                v.amp.gate_off();
                v.fenv.gate_off();
            }
        }
        let idx = self.take_voice();
        let mut v = std::mem::take(&mut self.voices[idx]);
        v.key = key;
        v.gate = true;
        self.age = self.age.wrapping_add(1);
        v.age = self.age;
        v.freq = hz(key as f32);
        v.phases = [0.0; FM_OPS];
        v.fb_carrier = 0.0;
        let sense = self.p[26];
        let vel = 1.0 - sense + sense * velocity.clamp(0.02, 1.0);
        v.amp = Adsr::new();
        v.amp.set(self.p[17], self.p[18], self.p[19], self.p[20]);
        v.amp.gate_on(vel);
        v.fenv = Adsr::new();
        v.fenv.set(self.p[21], self.p[22], self.p[23], self.p[24]);
        v.fenv.gate_on(1.0);
        v.svf = Svf::new();
        self.voices[idx] = v;
        self.last_freq = hz(key as f32);
    }
    fn note_off(&mut self, key: u8) {
        for v in self.voices.iter_mut() {
            if v.key == key && v.gate {
                v.gate = false;
                v.amp.gate_off();
                v.fenv.gate_off();
            }
        }
    }
    fn all_notes_off(&mut self) {
        for v in self.voices.iter_mut() {
            v.gate = false;
            v.amp.gate_off();
            v.fenv.gate_off();
        }
    }
    fn render(&mut self, left: &mut [f32], right: &mut [f32], sr_in: f32) {
        let sr = if self.sr > 1000.0 { self.sr } else { sr_in };
        let n = left.len().min(right.len());
        left.fill(0.0);
        right.fill(0.0);
        if n == 0 {
            return;
        }
        let lvls = [self.p[0], self.p[1], self.p[2], self.p[3], self.p[4], self.p[5]];
        let ratios = [self.p[6], self.p[7], self.p[8], self.p[9], self.p[10], self.p[11]];
        let algo = self.p[12] as usize % 4;
        let fb = self.p[13];
        let cut = self.p[14];
        let q = self.p[15];
        let env_amt = self.p[16];
        let det_c = self.p[25];
        let gain = self.p[27];
        let det_mul = [
            2f32.powf(-det_c / 1200.0),
            1.0,
            2f32.powf(det_c / 1200.0),
            1.0,
            2f32.powf(det_c / 1200.0),
            2f32.powf(-det_c / 1200.0),
        ];
        for i in 0..n {
            let mut sl = 0.0f32;
            let mut sr_ = 0.0f32;
            for v in self.voices.iter_mut() {
                if !v.amp.is_active() {
                    continue;
                }
                let a = v.amp.tick(sr);
                let fe = v.fenv.tick(sr) * env_amt;
                // advance phases (ratio folded into the increment)
                for (k, ph) in v.phases.iter_mut().enumerate() {
                    *ph = wrap01(*ph + v.freq * ratios[k] * det_mul[k] / sr);
                }
                let out = match algo {
                    // 0: full cascade op6 -> op5 -> ... -> op1 carrier, op2 self-feedback
                    0 => {
                        let m6 = (v.phases[5]).sin() * lvls[5];
                        let m5 = (v.phases[4] + m6).sin() * lvls[4];
                        let m4 = (v.phases[3] + m5).sin() * lvls[3];
                        let m3 = (v.phases[2] + m4).sin() * lvls[2];
                        let m2 = (v.phases[1] + m3 + fb * v.fb_carrier).sin() * lvls[1];
                        let c = (v.phases[0] + m2).sin() * lvls[0];
                        v.fb_carrier = c;
                        c
                    }
                    // 1: three carrier/modulator pairs
                    1 => {
                        let m1 = (v.phases[0] + fb * v.fb_carrier).sin() * lvls[0];
                        v.fb_carrier = m1;
                        let m3 = (v.phases[2]).sin() * lvls[2];
                        let m5 = (v.phases[4]).sin() * lvls[4];
                        (v.phases[1] + m1).sin() * lvls[1]
                            + (v.phases[3] + m3).sin() * lvls[3]
                            + (v.phases[5] + m5).sin() * lvls[5]
                    }
                    // 2: shared modulators + multiple carriers
                    2 => {
                        let m2 = (v.phases[1]).sin() * lvls[1];
                        let m3 = (v.phases[2]).sin() * lvls[2];
                        (v.phases[0] + m2 + m3).sin() * lvls[0]
                            + (v.phases[3] + m2 * 0.7 + m3).sin() * lvls[3]
                            + (v.phases[4] + m3 * 0.6).sin() * lvls[4]
                            + (v.phases[5]).sin() * lvls[5] * 0.8
                    }
                    // 3: two stacks + additive drone
                    _ => {
                        let m2 = (v.phases[1] + fb * v.fb_carrier).sin() * lvls[1];
                        v.fb_carrier = (v.phases[0] + m2).sin();
                        let m6 = (v.phases[5]).sin() * lvls[5];
                        (v.phases[0] + m2).sin() * lvls[0] * 0.8
                            + (v.phases[4] + m6).sin() * lvls[4] * 0.8
                            + (v.phases[2]).sin() * lvls[2] * 0.5
                            + (v.phases[3]).sin() * lvls[3] * 0.5
                    }
                };
                let fc = (cut * 2f32.powf(fe)).clamp(20.0, sr * 0.47);
                v.svf.set(fc, q, sr);
                let filtered = v.svf.tick_mode(out * gain * a, SvfMode::Low);
                sl += filtered.clamp(-8.0, 8.0);
                sr_ += filtered.clamp(-8.0, 8.0);
            }
            let (pl, pr) = pan_gains(0.0);
            left[i] = sl * pl;
            right[i] = sr_ * pr;
        }
    }
    fn params(&self) -> &'static [ParamSpec] {
        FM_PARAMS
    }
    fn set_param(&mut self, i: usize, value: f32) {
        if let Some(spec) = FM_PARAMS.get(i) {
            self.p[i] = spec.denormalize(spec.normalize(value));
        }
    }
    fn get_param(&self, i: usize) -> f32 {
        self.p.get(i).copied().unwrap_or(0.0)
    }
    fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.amp.is_active()).count()
    }
}

impl Default for FmEngine {
    fn default() -> Self {
        Self::hexad("Hexad FM")
    }
}
