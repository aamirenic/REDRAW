//! Physical modelling engine: damped-loop string synth (Karplus-Strong) with
//! dual detuned strings, lowpassed pick excitation, body tone, sub oscillator
//! and release ramp. 8 preallocated voices.
#![forbid(unsafe_code)]

use redraw_dsp::{DelayLine, OnePole, Osc, OscKind, Rng};
use redraw_shared::{midi_note_hz as hz, pan_gains, ParamSpec, Synth};

pub const PLUCK_POLY: usize = 8;

#[rustfmt::skip]
pub static PLUCK_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Damping", 0.0, 1.0, 0.35, ""),
    ParamSpec::new("Decay Time", 0.2, 12.0, 4.0, "s"),
    ParamSpec::new("Brightness", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Pick Position", 0.0, 1.0, 0.35, ""),
    ParamSpec::new("Body Tone", 0.0, 1.0, 0.2, ""),
    ParamSpec::new("Sub Level", 0.0, 1.0, 0.05, ""),
    ParamSpec::new("Release", 0.05, 1.0, 0.3, "s"),
    ParamSpec::new("String Detune", 0.0, 30.0, 6.0, "cents"),
    ParamSpec::new("Body Size", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Gain", 0.0, 2.0, 1.1, ""),
];

struct PluckVoice {
    key: u8,
    vel: f32,
    gate: bool,
    done: bool,
    age: u64,
    period1: f32,
    period2: f32,
    loop_gain: f32,
    damp: f32,
    store1: f32,
    store2: f32,
    delay1: DelayLine,
    delay2: DelayLine,
    str_lp: OnePole,
    body_lp: OnePole,
    exc_lp: OnePole,
    excite_skip: u32,
    excite_left: u32,
    excite_len: u32,
    burst_left: u32,
    burst_len: u32,
    rel: f32,
    rel_step: f32,
    sub_env: f32,
    sub_lvl: f32,
    sub: Osc,
    rng: Rng,
}

impl PluckVoice {
    fn new() -> Self {
        Self {
            key: 0,
            vel: 0.0,
            gate: false,
            done: true,
            age: 0,
            period1: 400.0,
            period2: 400.0,
            loop_gain: 0.99,
            damp: 0.2,
            store1: 0.0,
            store2: 0.0,
            delay1: DelayLine::new(2048),
            delay2: DelayLine::new(2048),
            str_lp: OnePole::new(),
            body_lp: OnePole::new(),
            exc_lp: OnePole::new(),
            excite_skip: 0,
            excite_left: 0,
            excite_len: 600,
            burst_left: 0,
            burst_len: 601,
            rel: 0.0,
            rel_step: 0.0,
            sub_env: 0.0,
            sub_lvl: 0.0,
            sub: Osc::new(OscKind::Sine, 0x31),
            rng: Rng::new(0x33),
        }
    }
}

impl Default for PluckVoice {
    fn default() -> Self {
        Self::new()
    }
}

pub struct PluckEngine {
    pub name: &'static str,
    pub sr: f32,
    pub p: [f32; PLUCK_PARAMS.len()],
    voices: [PluckVoice; PLUCK_POLY],
    pub age: u64,
}

impl PluckEngine {
    pub fn with_params(name: &'static str, values: &[f32]) -> Self {
        let mut e = Self {
            name,
            sr: 48_000.0,
            p: std::array::from_fn(|i| PLUCK_PARAMS[i].default),
            voices: std::array::from_fn(|_| PluckVoice::default()),
            age: 0,
        };
        for (i, v) in values.iter().enumerate().take(e.p.len()) {
            e.set_param(i, *v);
        }
        e
    }

    fn take_voice(&mut self) -> usize {
        if let Some(pos) = (0..PLUCK_POLY).find(|&i| self.voices[i].done) {
            return pos;
        }
        (0..PLUCK_POLY)
            .min_by_key(|&i| self.voices[i].age)
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------- presets

impl PluckEngine {
    pub fn petal(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0.4, 4.0, 0.5, 0.35, 0.2, 0.05, 0.3, 6.0, 0.5, 1.1],
        )
    }
    pub fn string_storm(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0.25, 3.0, 0.8, 0.5, 0.35, 0.0, 0.25, 20.0, 0.6, 1.2],
        )
    }
    pub fn pluck_bass(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0.5, 2.6, 0.35, 0.2, 0.5, 0.8, 0.2, 0.0, 0.35, 1.2],
        )
    }
}

// ---------------------------------------------------------------- synth impl

impl Synth for PluckEngine {
    fn synth_type(&self) -> &'static str {
        self.name
    }
    fn set_display_name(&mut self, name: &'static str) {
        self.name = name;
    }
    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            *v = PluckVoice::default();
        }
        self.age = 0;
    }
    fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr.max(8000.0);
    }
    fn note_on(&mut self, key: u8, velocity: f32) {
        for v in self.voices.iter_mut() {
            if v.gate && v.key == key {
                v.gate = false;
                v.rel = 0.0;
                v.rel_step = -1.0 / (self.p[6].max(0.01) * self.sr.max(1.0));
            }
        }
        let idx = self.take_voice();
        let mut v = std::mem::take(&mut self.voices[idx]);
        v.key = key;
        v.vel = velocity.clamp(0.02, 1.0);
        v.gate = true;
        v.done = false;
        self.age = self.age.wrapping_add(1);
        v.age = self.age;
        let freq = hz(key as f32).max(55.0);
        let sr = if self.sr > 1000.0 { self.sr } else { 48_000.0 };
        let det = self.p[7].clamp(0.0, 30.0) / 1200.0;
        v.period1 = (sr / freq).max(40.0).min(1900.0);
        v.period2 = v.period1 * 2f32.powf(det);
        let decay = self.p[1].clamp(0.2, 12.0);
        // loop gain so the string reaches -60 dB after `decay` seconds
        v.loop_gain = 0.001f32.powf(1.0 / (freq * decay)).clamp(0.05, 0.9999);
        v.damp = self.p[0].clamp(0.0, 0.95) * 0.6;
        let bright = self.p[2].clamp(0.0, 1.0);
        v.exc_lp.set_lowpass(400.0 + bright * 9000.0, sr);
        v.body_lp.set_lowpass(18_000.0 - 17_600.0 * self.p[4].clamp(0.0, 1.0) * 0.9, sr);
        let plen = v.period1 as u32;
        // a soft pick delays the burst; a sharp pick starts immediately
        v.excite_skip = ((1.0 - self.p[3].clamp(0.0, 1.0)) * plen as f32 * 0.4) as u32;
        v.excite_len = (plen * 2).clamp(64, 2400);
        v.excite_left = v.excite_len;
        v.burst_len = v.excite_len;
        v.burst_left = v.burst_len;
        v.delay1.clear();
        v.delay2.clear();
        v.store1 = 0.0;
        v.store2 = 0.0;
        v.rel = 1.0;
        v.rel_step = 0.0;
        v.sub_env = 1.0;
        v.sub_lvl = self.p[5].clamp(0.0, 1.0);
        v.sub.reset_phase(0.0);
        v.sub.set_freq(freq, sr);
        self.voices[idx] = v;
    }
    fn note_off(&mut self, key: u8) {
        for v in self.voices.iter_mut() {
            if v.key == key && v.gate {
                v.gate = false;
                v.rel = 0.0;
                v.rel_step = -1.0 / (self.p[6].max(0.01) * self.sr.max(1.0));
            }
        }
    }
    fn all_notes_off(&mut self) {
        for v in self.voices.iter_mut() {
            if v.gate {
                v.gate = false;
                v.rel = 0.0;
                v.rel_step = -1.0 / (self.p[6].max(0.01) * self.sr.max(1.0));
            }
        }
    }
    fn render(&mut self, left: &mut [f32], right: &mut [f32], _sr_in: f32) {
        let n = left.len().min(right.len());
        let gain = self.p[9].clamp(0.0, 2.0);
        let tone = self.p[4].clamp(0.0, 1.0);
        let tone_mix = tone * 0.6;
        for i in 0..n {
            let mut sl = 0.0f32;
            for v in self.voices.iter_mut() {
                if v.done {
                    continue;
                }
                // excitation burst
                let mut input = 0.0f32;
                if v.excite_skip > 0 {
                    v.excite_skip -= 1;
                } else if v.excite_left > 0 {
                    v.excite_left -= 1;
                    let shape = 1.0 - v.burst_left.max(1) as f32 / v.burst_len.max(1) as f32;
                    v.burst_left = v.burst_left.saturating_sub(1);
                    input = v.exc_lp.tick(v.rng.next_f32()) * shape * v.vel * 1.4;
                }
                // string 1: damped loop
                let d1 = v.delay1.read(v.period1);
                let filtered1 = d1 * (1.0 - v.damp) + v.store1 * v.damp;
                v.store1 = v.str_lp.tick(filtered1);
                v.delay1.push(input + filtered1 * v.loop_gain);
                // string 2 (detuned)
                let d2 = v.delay2.read(v.period2);
                let filtered2 = d2 * (1.0 - v.damp) + v.store2 * v.damp;
                v.store2 = filtered2;
                v.delay2.push(input + filtered2 * v.loop_gain);
                // body + sub
                let str_out = (d1 + d2) * 0.5 * v.vel * gain;
                let body = str_out * (1.0 - tone_mix) + v.body_lp.tick(str_out) * tone_mix;
                v.sub_env *= 0.99955;
                let sub_v = v.sub.next() * v.sub_lvl * v.sub_env * 0.8 * v.vel;
                // release ramp
                if !v.gate {
                    v.rel = (v.rel + v.rel_step).max(0.0);
                    if v.rel <= 0.0001 {
                        v.rel = 1.0;
                        v.rel_step = 0.0;
                        v.done = true;
                    }
                }
                let out = (body + sub_v) * v.rel;
                let out = out.clamp(-8.0, 8.0);
                sl += out;
            }
            let (pl, _pr) = pan_gains(0.0);
            left[i] = sl * pl;
            right[i] = sl * _pr;
        }
    }
    fn params(&self) -> &'static [ParamSpec] {
        PLUCK_PARAMS
    }
    fn set_param(&mut self, i: usize, value: f32) {
        if let Some(spec) = PLUCK_PARAMS.get(i) {
            self.p[i] = spec.denormalize(spec.normalize(value));
        }
    }
    fn get_param(&self, i: usize) -> f32 {
        self.p.get(i).copied().unwrap_or(0.0)
    }
    fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| !v.done).count()
    }
}

impl Default for PluckEngine {
    fn default() -> Self {
        Self::petal("Petal")
    }
}
