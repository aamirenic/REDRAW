//! Sample playback engine: pitch-mapped voices over a user sample buffer or an
//! internal synthesized fallback bed. Slice/chop mapping for ChopLab/SliceLab,
//! loop modes, filter with envelope. 12 preallocated voices.
//!
//! `set_sample` is an inherent method (non-trait) so control code can install
//! decoded buffers; the registry rebuilds instruments when samples change.
#![forbid(unsafe_code)]

use redraw_dsp::{Adsr, Rng, Svf, SvfMode, TWO_PI};
use redraw_shared::{midi_note_hz as hz, pan_gains, ParamCurve, ParamSpec, Synth};

pub const SMP_POLY: usize = 12;
pub const SMP_SRC: usize = 96_000;

#[rustfmt::skip]
pub static SMP_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Start", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("End", 0.05, 1.0, 1.0, ""),
    ParamSpec::new("Loop Mode", 0.0, 2.0, 1.0, ""),
    ParamSpec::new("Pitch", -36.0, 36.0, 0.0, "st"),
    ParamSpec::new("Gain", 0.0, 2.0, 1.0, ""),
    ParamSpec::new("Attack", 0.0005, 2.0, 0.004, "s"),
    ParamSpec::new("Decay", 0.0005, 4.0, 0.35, "s"),
    ParamSpec::new("Sustain", 0.0, 1.0, 0.8, ""),
    ParamSpec::new("Release", 0.0005, 4.0, 0.3, "s"),
    ParamSpec::new("Cutoff", 20.0, 20000.0, 16000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Resonance", 0.15, 14.0, 0.7, ""),
    ParamSpec::new("Flt Env Amount", -6.0, 6.0, 0.0, "oct"),
    ParamSpec::new("Flt Attack", 0.0005, 2.0, 0.005, "s"),
    ParamSpec::new("Flt Decay", 0.0005, 4.0, 0.3, "s"),
    ParamSpec::new("Slices", 1.0, 64.0, 1.0, ""),
    ParamSpec::new("Key Base", 24.0, 96.0, 60.0, "note"),
];

struct SmpVoice {
    key: u8,
    vel: f32,
    gate: bool,
    active: bool,
    age: u64,
    env: Adsr,
    fenv: Adsr,
    filter: Svf,
    pos: f32,
    start: f32,
    end: f32,
    rate: f32,
    loop_on: bool,
    wrap: bool,
}

impl Default for SmpVoice {
    fn default() -> Self {
        Self {
            key: 0,
            vel: 0.0,
            gate: false,
            active: false,
            age: 0,
            env: Adsr::new(),
            fenv: Adsr::new(),
            filter: Svf::new(),
            pos: 0.0,
            start: 0.0,
            end: 0.0,
            rate: 1.0,
            loop_on: false,
            wrap: false,
        }
    }
}

pub struct SamplerVoiceEngine {
    pub name: &'static str,
    pub sr: f32,
    pub p: [f32; SMP_PARAMS.len()],
    voices: [SmpVoice; SMP_POLY],
    pub age: u64,
    pub user: Option<Box<[f32]>>,
    pub user_sr: u32,
    pub bed: Box<[f32]>,
}

fn bed_buf(harmonics: usize, seconds: f32, noise_amt: f32) -> Box<[f32]> {
    let len = (SMP_SRC as f32 * seconds).min(SMP_SRC as f32) as usize;
    let base = hz(60.0);
    let mut buf = vec![0.0f32; len.max(1024)];
    for h in 1..=harmonics.max(1) {
        let amp = 1.0 / h as f32;
        for (i, s) in buf.iter_mut().enumerate() {
            let t = i as f32 / 48_000.0;
            *s += amp * (TWO_PI * base * h as f32 * t).sin() * 0.45;
        }
    }
    let mut rng = Rng::new(0x71);
    if noise_amt > 0.0 {
        let buf_len = buf.len() as f32;
        for (i, s) in buf.iter_mut().enumerate() {
            *s += (rng.next_f32() * 0.35 + 0.0) * noise_amt * (1.0 - i as f32 / buf_len);
        }
    }
    // tail fade for loop click suppression
    let fade = buf.len() / 40;
    for k in 0..fade {
        let g = 0.5 + 0.5 * (k as f32 / fade.max(1) as f32);
        buf[k] *= g;
        let end = buf.len() - 1 - k;
        buf[end] *= g;
    }
    let peak = buf.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
    for s in buf.iter_mut() {
        *s /= peak;
    }
    buf.into_boxed_slice()
}

impl SamplerVoiceEngine {
    pub fn with_params(name: &'static str, values: &[f32], harmonics: usize, noise: f32) -> Self {
        let mut e = Self {
            name,
            sr: 48_000.0,
            p: std::array::from_fn(|i| SMP_PARAMS[i].default),
            voices: std::array::from_fn(|_| SmpVoice::default()),
            age: 0,
            user: None,
            user_sr: 48_000,
            bed: bed_buf(harmonics, 2.0, noise),
        };
        for (i, v) in values.iter().enumerate().take(e.p.len()) {
            e.set_param(i, *v);
        }
        e
    }

    pub fn multiwave(name: &'static str) -> Self {
        Self::with_params(name, &[], 24, 0.04)
    }
    pub fn slice16(name: &'static str) -> Self {
        let mut e = Self::with_params(name, &[], 14, 0.35);
        e.set_param(14, 16.0); // 16-slice chop
        e
    }
    pub fn slice32(name: &'static str) -> Self {
        let mut e = Self::with_params(name, &[], 14, 0.4);
        e.set_param(14, 32.0);
        e
    }
    pub fn basic(name: &'static str) -> Self {
        Self::with_params(name, &[], 1, 0.0)
    }

    /// Install a decoded mono sample (44.1k/48k). Control-thread only.
    pub fn set_sample(&mut self, mono: &[f32], sr: u32) {
        self.user = Some(mono.to_vec().into_boxed_slice());
        self.user_sr = if sr < 8000 { 48_000 } else { sr };
        self.all_notes_off();
    }
    pub fn clear_sample(&mut self) {
        self.user = None;
        self.all_notes_off();
    }

    fn source(&self) -> &[f32] {
        self.user.as_deref().unwrap_or(&self.bed)
    }

    fn take_voice(&mut self) -> usize {
        if let Some(pos) = (0..SMP_POLY).find(|&i| !self.voices[i].active) {
            return pos;
        }
        (0..SMP_POLY)
            .min_by_key(|&i| self.voices[i].age)
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------- synth impl

impl Synth for SamplerVoiceEngine {
    fn synth_type(&self) -> &'static str {
        self.name
    }
    fn set_display_name(&mut self, name: &'static str) {
        self.name = name;
    }
    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            *v = SmpVoice::default();
        }
        self.age = 0;
    }
    fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr.max(8000.0);
    }
    fn note_on(&mut self, key: u8, velocity: f32) {
        let idx = self.take_voice();
        let mut v = std::mem::take(&mut self.voices[idx]);
        v.key = key;
        v.vel = velocity.clamp(0.02, 1.0);
        v.gate = true;
        v.active = true;
        self.age = self.age.wrapping_add(1);
        v.age = self.age;
        let slices = self.p[14].max(1.0) as usize;
        let slice_len = 1.0 / slices as f32;
        let slice_offset = (key.saturating_sub(36) as usize) % slices;
        let start_f = self.p[0].clamp(0.0, 0.95);
        let end_f = self.p[1].clamp(start_f + 0.02, 1.0);
        let point = start_f + (end_f - start_f) * slice_offset as f32 * slice_len;
        let span = (end_f - start_f) * slice_len;
        v.start = point;
        v.end = (point + span).min(1.0);
        let src_len = self.source().len() as f32;
        v.pos = v.start * src_len;
        // pitch: key tracking relative to the bed's builtin tuning (key base param)
        let base = self.p[15];
        let sr_ratio = if self.user.is_some() {
            self.user_sr as f32 / 48_000.0
        } else {
            1.0
        };
        v.rate = 2f32.powf((key as f32 - base + self.p[3]) / 12.0) * sr_ratio;
        v.loop_on = (self.p[2] as usize) != 0;
        v.wrap = (self.p[2] as usize) == 1;
        v.env = Adsr::new();
        v.env.set(self.p[5], self.p[6], self.p[7], self.p[8]);
        v.env.gate_on(v.vel);
        v.fenv = Adsr::new();
        v.fenv.set(self.p[12], self.p[13], 0.35, 0.2);
        v.fenv.gate_on(1.0);
        v.filter = Svf::new();
        self.voices[idx] = v;
    }
    fn note_off(&mut self, key: u8) {
        for v in self.voices.iter_mut() {
            if v.key == key && v.gate {
                v.gate = false;
                v.env.gate_off();
                v.fenv.gate_off();
            }
        }
    }
    fn all_notes_off(&mut self) {
        for v in self.voices.iter_mut() {
            if v.gate {
                v.gate = false;
                v.env.gate_off();
                v.fenv.gate_off();
            }
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
        let cut = self.p[9];
        let q = self.p[10];
        let env_amt = self.p[11];
        let gain = self.p[4];
        let src: &[f32] = self.user.as_deref().unwrap_or(&self.bed);
        let src_len = src.len() as f32;
        for i in 0..n {
            let mut sl = 0.0f32;
            let mut sr_ = 0.0f32;
            for v in self.voices.iter_mut() {
                if !v.active {
                    continue;
                }
                let a = v.env.tick(sr);
                let fe = v.fenv.tick(sr) * env_amt;
                let i0 = (v.pos as usize).min(src.len() - 1);
                let i1 = (i0 + 1) % src.len();
                let frac = v.pos - v.pos.floor();
                let s = src[i0] * (1.0 - frac) + src[i1] * frac;
                v.pos += v.rate;
                if v.pos >= v.end * src_len {
                    if v.loop_on && (v.gate || !v.wrap) {
                        v.pos = v.start * src_len;
                    } else if v.pos >= src_len {
                        v.pos = (v.end * src_len - 1.0).max(0.0);
                    }
                }
                let fc = (cut * 2f32.powf(fe)).clamp(20.0, sr * 0.47);
                v.filter.set(fc, q, sr);
                let out = v.filter.tick_mode(s * gain * a, SvfMode::Low);
                sl += out.clamp(-6.0, 6.0);
                sr_ += out.clamp(-6.0, 6.0);
                if !v.gate && !v.env.is_active() {
                    v.active = false;
                }
            }
            let (pl, pr) = pan_gains(0.0);
            left[i] = sl * pl;
            right[i] = sr_ * pr;
        }
    }
    fn params(&self) -> &'static [ParamSpec] {
        SMP_PARAMS
    }
    fn set_param(&mut self, i: usize, value: f32) {
        if let Some(spec) = SMP_PARAMS.get(i) {
            self.p[i] = spec.denormalize(spec.normalize(value));
        }
    }
    fn get_param(&self, i: usize) -> f32 {
        self.p.get(i).copied().unwrap_or(0.0)
    }
    fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }
}

impl Default for SamplerVoiceEngine {
    fn default() -> Self {
        Self::basic("Sampler")
    }
}
