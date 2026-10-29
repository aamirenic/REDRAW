//! Wavetable engine: two band-limited table oscillators with crossblend,
//! unison, SVF filter with envelope and vibrato LFO. Tables are generated once
//! per engine (setup time) and shared read-only by the per-voice phases.
#![forbid(unsafe_code)]

use redraw_dsp::{Adsr, Lfo, LfoShape, Svf, SvfMode, TWO_PI};
use redraw_shared::{midi_note_hz as hz, pan_gains, ParamCurve, ParamSpec, Synth};

pub const WT_POLY: usize = 12;
pub const WT_UNI: usize = 7;
pub const WT_TABLES: usize = 6;

#[rustfmt::skip]
pub static WT_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Table1", 0.0, 5.0, 0.0, ""),
    ParamSpec::new("Osc1 Octave", -3.0, 3.0, 0.0, "oct"),
    ParamSpec::new("Osc1 Detune", -100.0, 100.0, 0.0, "cents"),
    ParamSpec::new("Osc1 Level", 0.0, 1.0, 0.75, ""),
    ParamSpec::new("Table2", 0.0, 5.0, 3.0, ""),
    ParamSpec::new("Osc2 Octave", -3.0, 3.0, -1.0, "oct"),
    ParamSpec::new("Osc2 Detune", -100.0, 100.0, 9.0, "cents"),
    ParamSpec::new("Osc2 Level", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Crossblend", 0.0, 1.0, 0.4, ""),
    ParamSpec::new("Unison", 1.0, 7.0, 1.0, ""),
    ParamSpec::new("Unison Detune", 0.0, 60.0, 14.0, "cents"),
    ParamSpec::new("Cutoff", 20.0, 20000.0, 12000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Resonance", 0.15, 14.0, 0.8, ""),
    ParamSpec::new("Flt Env Amount", -6.0, 6.0, 1.5, "oct"),
    ParamSpec::new("Amp Attack", 0.0005, 8.0, 0.01, "s"),
    ParamSpec::new("Amp Decay", 0.0005, 8.0, 0.3, "s"),
    ParamSpec::new("Amp Sustain", 0.0, 1.0, 0.8, ""),
    ParamSpec::new("Amp Release", 0.0005, 8.0, 0.5, "s"),
    ParamSpec::new("LFO Rate", 0.01, 20.0, 5.0, "Hz"),
    ParamSpec::new("LFO Depth", 0.0, 1.0, 0.05, ""),
    ParamSpec::new("Gain", 0.0, 2.0, 0.9, ""),
];

struct WtVoice {
    key: u8,
    vel: f32,
    gate: bool,
    age: u64,
    env: Adsr,
    phase1: f32,
    phase2: f32,
    uni: [f32; WT_UNI],
    svf: Svf,
}

impl Default for WtVoice {
    fn default() -> Self {
        Self {
            key: 0,
            vel: 0.0,
            gate: false,
            age: 0,
            env: Adsr::new(),
            phase1: 0.0,
            phase2: 0.0,
            uni: [0.0; WT_UNI],
            svf: Svf::new(),
        }
    }
}

pub struct WavetableEngine {
    pub name: &'static str,
    pub sr: f32,
    pub p: [f32; WT_PARAMS.len()],
    voices: [WtVoice; WT_POLY],
    pub age: u64,
    pub lfo: Lfo,
    pub tables: [Box<[f32]>; WT_TABLES],
}

impl WavetableEngine {
    pub fn with_params(name: &'static str, values: &[f32]) -> Self {
        let mut e = Self {
            name,
            sr: 48_000.0,
            p: std::array::from_fn(|i| WT_PARAMS[i].default),
            voices: std::array::from_fn(|_| WtVoice::default()),
            age: 0,
            lfo: Lfo::new(0x81),
            tables: build_tables(),
        };
        for (i, v) in values.iter().enumerate().take(e.p.len()) {
            e.set_param(i, *v);
        }
        e
    }

    pub fn aurora(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0., 0., -4., 0.8, 1., 0., 12., 0.55, 0.45, 5., 16., 9000., 0.9, 1.5, 0.01, 0.3, 0.8, 0.5,
              5., 0.05, 0.9],
        )
    }
    pub fn wildcard(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[3., 1., 7., 0.7, 2., -1., -9., 0.5, 0.6, 3., 22., 4000., 2.2, 2.5, 0.012, 0.2, 0.6, 0.35,
              3.5, 0.12, 0.95],
        )
    }
    pub fn padbox(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[2., 0., -2., 0.7, 5., 0., 14., 0.5, 0.5, 6., 18., 3000., 1.1, 2.0, 0.45, 0.9, 0.75, 1.4,
              0.4, 0.04, 0.85],
        )
    }

    fn take_voice(&mut self) -> usize {
        if let Some(pos) = (0..WT_POLY).find(|&i| !self.voices[i].env.is_active()) {
            return pos;
        }
        (0..WT_POLY)
            .min_by_key(|&i| self.voices[i].age)
            .unwrap_or(0)
    }
}

/// Linear-interpolated table lookup. Tables are shared read-only; per-voice
/// phases advance in the render loop.
#[inline]
fn wt_read(tables: &[Box<[f32]>], idx: usize, phase: f32) -> f32 {
    let t = &tables[idx % WT_TABLES];
    let size = t.len() as f32;
    let pos = phase * size;
    let i0 = (pos as usize) % t.len();
    let i1 = (i0 + 1) % t.len();
    let frac = pos - pos.floor();
    t[i0] * (1.0 - frac) + t[i1] * frac
}

fn build_tables() -> [Box<[f32]>; WT_TABLES] {
    let mut list: Vec<Box<[f32]>> = Vec::with_capacity(WT_TABLES);
    for (idx, shape) in ["saw", "square", "organ", "odd", "warm", "pulses"].iter().enumerate() {
        let _ = idx;
        list.push(match *shape {
            "warm" => sine_table(),
            names => redraw_dsp::Wavetable::from_shape(names, 32).table.into_boxed_slice(),
        });
    }
    [
        list.remove(0),
        list.remove(0),
        list.remove(0),
        list.remove(0),
        list.remove(0),
        list.remove(0),
    ]
}

fn sine_table() -> Box<[f32]> {
    let size = 2048usize;
    (0..size)
        .map(|i| (TWO_PI * i as f32 / size as f32).sin())
        .collect::<Vec<f32>>()
        .into_boxed_slice()
}

// ---------------------------------------------------------------- synth impl

impl Synth for WavetableEngine {
    fn synth_type(&self) -> &'static str {
        self.name
    }
    fn set_display_name(&mut self, name: &'static str) {
        self.name = name;
    }
    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            *v = WtVoice::default();
        }
        self.lfo.reset();
        self.age = 0;
    }
    fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr.max(8000.0);
    }
    fn note_on(&mut self, key: u8, velocity: f32) {
        for v in self.voices.iter_mut() {
            if v.gate && v.key == key {
                v.gate = false;
                v.env.gate_off();
            }
        }
        let idx = self.take_voice();
        let mut v = std::mem::take(&mut self.voices[idx]);
        v.key = key;
        v.vel = velocity.clamp(0.02, 1.0);
        v.gate = true;
        self.age = self.age.wrapping_add(1);
        v.age = self.age;
        v.phase1 = 0.0;
        v.phase2 = 0.0;
        v.uni = std::array::from_fn(|i| (i as f32) * 0.137);
        v.env = Adsr::new();
        v.env.set(self.p[14], self.p[15], self.p[16], self.p[17]);
        v.env.gate_on(v.vel);
        v.svf = Svf::new();
        self.voices[idx] = v;
    }
    fn note_off(&mut self, key: u8) {
        for v in self.voices.iter_mut() {
            if v.key == key && v.gate {
                v.gate = false;
                v.env.gate_off();
            }
        }
    }
    fn all_notes_off(&mut self) {
        for v in self.voices.iter_mut() {
            v.gate = false;
            v.env.gate_off();
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
        let t1 = (self.p[0] as usize).min(WT_TABLES - 1);
        let t2 = (self.p[4] as usize).min(WT_TABLES - 1);
        let o1_o = self.p[1];
        let o1_d = self.p[2];
        let o1_l = self.p[3];
        let o2_o = self.p[5];
        let o2_d = self.p[6];
        let o2_l = self.p[7];
        let cross = self.p[8].clamp(0.0, 1.0);
        let uni_n = (self.p[9] as usize).min(WT_UNI).max(1);
        let uni_det = self.p[10];
        let cut = self.p[11];
        let q = self.p[12];
        let env_amt = self.p[13];
        let lfo_rate = self.p[18].max(0.01);
        let lfo_depth = self.p[19].clamp(0.0, 1.0) * 1.0;
        let gain = self.p[20];
        let tables: &[Box<[f32]>] = &self.tables;
        self.lfo.shape = LfoShape::Sine;
        for i in 0..n {
            self.lfo.set_rate(lfo_rate);
            let lf = self.lfo.tick(sr);
            let pitch_lf = 2f32.powf(lf * lfo_depth / 12.0);
            let mut sl = 0.0f32;
            let mut sr_ = 0.0f32;
            for v in self.voices.iter_mut() {
                if !v.env.is_active() {
                    continue;
                }
                let a = v.env.tick(sr) * v.vel;
                let mut m = 0.0f32;
                // osc 1: crossblend morphs between the two tables, unison on top
                {
                    let f1 = (hz(v.key as f32) * 2f32.powf(o1_o) * 2f32.powf(o1_d / 1200.0)
                        * pitch_lf)
                        .max(8.0);
                    v.phase1 += f1 / sr;
                    v.uni[0] += f1 / sr;
                    let s1 = wt_read(tables, t1, v.phase1) * (1.0 - cross)
                        + wt_read(tables, t2, v.phase1) * cross;
                    m += s1 * a * o1_l;
                }
                if uni_n > 1 {
                    let weight = 1.0 / (uni_n as f32).sqrt();
                    for u in 1..uni_n {
                        let spread = ((u as f32 / (uni_n.max(2) - 1) as f32) - 0.5) * uni_det
                            / 1200.0;
                        let f = (hz(v.key as f32) * pitch_lf * 2f32.powf(spread)).max(8.0);
                        v.uni[u] += f / sr;
                        let s = wt_read(tables, t1, v.uni[u]);
                        m += s * a * o1_l * weight;
                    }
                }
                // osc 2 (harmonic support at its own octave)
                {
                    let f2 = (hz(v.key as f32)
                        * 2f32.powf(o2_o)
                        * 2f32.powf(o2_d / 1200.0)
                        * pitch_lf)
                        .max(8.0);
                    v.phase2 += f2 / sr;
                    let m2 = wt_read(tables, t2, v.phase2) * a * o2_l;
                    m += m2;
                }
                v.phase1 = wrap_f(v.phase1);
                v.phase2 = wrap_f(v.phase2);
                for u in v.uni.iter_mut() {
                    *u = wrap_f(*u);
                }
                // filter envelope follows the amp envelope (pad style)
                let fc = (cut * 2f32.powf(a * env_amt)).clamp(20.0, sr * 0.47);
                v.svf.set(fc, q, sr);
                let out = v.svf.tick_mode(m * gain, SvfMode::Low).clamp(-4.0, 4.0);
                sl += out;
                sr_ += out;
            }
            let (pl, pr) = pan_gains(0.0);
            left[i] = sl * pl;
            right[i] = sr_ * pr;
        }
    }
    fn params(&self) -> &'static [ParamSpec] {
        WT_PARAMS
    }
    fn set_param(&mut self, i: usize, value: f32) {
        if let Some(spec) = WT_PARAMS.get(i) {
            self.p[i] = spec.denormalize(spec.normalize(value));
        }
    }
    fn get_param(&self, i: usize) -> f32 {
        self.p.get(i).copied().unwrap_or(0.0)
    }
    fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.env.is_active()).count()
    }
}

impl Default for WavetableEngine {
    fn default() -> Self {
        Self::aurora("Aurora")
    }
}

#[inline]
fn wrap_f(x: f32) -> f32 {
    x - x.floor()
}
