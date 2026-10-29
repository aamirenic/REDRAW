//! Drum/percussion engine: six synthesized voice types (kick, snare, hat, clap,
//! tom, tuned cell) with kit key-mapping, pitch envelopes, snap/click, drive
//! and tone tilt. 8 preallocated voices.
#![forbid(unsafe_code)]

use redraw_dsp::{wrap01, OnePole, Rng, TWO_PI};
use redraw_shared::{midi_note_hz as hz, pan_gains, ParamCurve, ParamSpec, Synth};

pub const DRUM_POLY: usize = 8;

#[rustfmt::skip]
pub static DRUM_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Slot", 0.0, 5.0, 0.0, ""),
    ParamSpec::new("Pitch", 20.0, 600.0, 120.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Pitch Drop", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Body Tone", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Decay", 0.02, 2.5, 0.4, "s"),
    ParamSpec::new("Snap", 0.0, 1.0, 0.2, ""),
    ParamSpec::new("Drive", 0.0, 8.0, 1.2, ""),
    ParamSpec::new("Tone Tilt", -1.0, 1.0, 0.0, ""),
    ParamSpec::new("Ring", 0.0, 1.0, 0.15, ""),
    ParamSpec::new("Level", 0.0, 2.0, 0.9, ""),
    ParamSpec::new("Click", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Tune 2", -12.0, 12.0, 0.0, "st"),
    ParamSpec::new("Noise Decay", 0.02, 1.0, 0.2, "s"),
    ParamSpec::new("Kit Mode", 0.0, 1.0, 0.0, ""),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrumKind {
    Kick,
    Snare,
    Hat,
    Clap,
    Tom,
    Cell,
}

struct DrumVoice {
    kind: DrumKind,
    pitch: f32,
    vel: f32,
    t: f32,
    decay: f32,
    ndecay: f32,
    phase: f32,
    phase2: f32,
    burst: f32,
    active: bool,
    age: u64,
    rng: Rng,
    hp_state: f32,
}

impl Default for DrumVoice {
    fn default() -> Self {
        Self {
            kind: DrumKind::Kick,
            pitch: 120.0,
            vel: 0.0,
            t: 0.0,
            decay: 0.4,
            ndecay: 0.2,
            phase: 0.0,
            phase2: 0.0,
            burst: 0.0,
            active: false,
            age: 0,
            rng: Rng::new(0x41),
            hp_state: 0.0,
        }
    }
}

pub struct DrumEngine {
    pub name: &'static str,
    pub sr: f32,
    pub p: [f32; DRUM_PARAMS.len()],
    voices: [DrumVoice; DRUM_POLY],
    pub age: u64,
    tilt_lp: OnePole,
}

impl DrumEngine {
    pub fn with_params(name: &'static str, values: &[f32]) -> Self {
        let mut e = Self {
            name,
            sr: 48_000.0,
            p: std::array::from_fn(|i| DRUM_PARAMS[i].default),
            voices: std::array::from_fn(|_| DrumVoice::default()),
            age: 0,
            tilt_lp: OnePole::new(),
        };
        for (i, v) in values.iter().enumerate().take(e.p.len()) {
            e.set_param(i, *v);
        }
        e
    }

    fn kit_slot(&self, key: u8) -> DrumKind {
        match key {
            35..=37 => DrumKind::Kick,
            38..=41 => DrumKind::Snare,
            42..=44 => DrumKind::Hat,
            45..=46 => DrumKind::Clap,
            47..=48 => DrumKind::Tom,
            _ => {
                let slot = (key as usize) % 6;
                match slot {
                    0 => DrumKind::Kick,
                    1 => DrumKind::Snare,
                    2 => DrumKind::Hat,
                    3 => DrumKind::Clap,
                    4 => DrumKind::Tom,
                    _ => DrumKind::Cell,
                }
            }
        }
    }

    fn take_voice(&mut self) -> usize {
        if let Some(pos) = (0..DRUM_POLY).find(|&i| !self.voices[i].active) {
            return pos;
        }
        (0..DRUM_POLY)
            .min_by_key(|&i| self.voices[i].age)
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------- presets

impl DrumEngine {
    pub fn kick_one(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0., 55., 0.7, 0.4, 0.55, 0.25, 2.2, -0.3, 0.05, 1.0, 0.5, 0., 0.08, 0.],
        )
    }
    pub fn subkick(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0., 45., 0.15, 0.9, 0.8, 0.05, 1.2, -0.6, 0.0, 1.1, 0.1, 0., 0.1, 0.],
        )
    }
    pub fn drum_cell(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[5., 220., 0.3, 0.2, 0.22, 0.4, 1.5, 0.2, 0.3, 0.8, 0.35, 7., 0.12, 0.],
        )
    }
    pub fn membrane(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[4., 130., 0.5, 0.8, 0.45, 0.3, 2.0, -0.2, 0.2, 0.95, 0.2, 7., 0.15, 0.],
        )
    }
    pub fn drum_lab(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[1., 190., 0.4, 0.4, 0.3, 0.6, 2.6, 0.1, 0.35, 0.9, 0.4, 0., 0.22, 0.],
        )
    }
    pub fn kit(name: &'static str, b: bool) -> Self {
        Self::with_params(
            name,
            &[0., if b { 210.0 } else { 150.0 }, 0.4, 0.5, 0.35, 0.45, 2.0, -0.1, 0.25, 0.95, 0.35, 0.,
              0.18, 1.0],
        )
    }
    pub fn padforge(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[5., 240., 0.2, 0.3, 1.2, 0.1, 1.4, 0.3, 0.55, 0.8, 0.05, 7., 0.3, 0.],
        )
    }
}

// ---------------------------------------------------------------- synth impl

impl Synth for DrumEngine {
    fn synth_type(&self) -> &'static str {
        self.name
    }
    fn set_display_name(&mut self, name: &'static str) {
        self.name = name;
    }
    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            *v = DrumVoice::default();
        }
        self.age = 0;
        self.tilt_lp.reset();
    }
    fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr.max(8000.0);
        self.tilt_lp.set_lowpass(6000.0, sr.max(8000.0));
    }
    fn note_on(&mut self, key: u8, velocity: f32) {
        let kit = self.p[13] >= 0.5;
        let kind = if kit {
            self.kit_slot(key)
        } else {
            match (self.p[0] as usize) % 6 {
                0 => DrumKind::Kick,
                1 => DrumKind::Snare,
                2 => DrumKind::Hat,
                3 => DrumKind::Clap,
                4 => DrumKind::Tom,
                _ => DrumKind::Cell,
            }
        };
        let idx = self.take_voice();
        let v = &mut self.voices[idx];
        self.age = self.age.wrapping_add(1);
        v.age = self.age;
        // tuned slot keeps some key tracking; pitch param centers the sound
        let base = if kit {
            hz(key as f32).max(30.0)
        } else {
            self.p[1].clamp(20.0, 600.0)
        };
        let base = base * 2f32.powf(self.p[11] / 12.0);
        v.kind = kind;
        v.pitch = base;
        v.vel = velocity.clamp(0.05, 1.0);
        v.t = 0.0;
        v.decay = self.p[4].clamp(0.02, 2.5);
        v.ndecay = self.p[12].clamp(0.02, 1.0);
        v.phase = 0.0;
        v.phase2 = 0.0;
        v.burst = 0.0;
        v.active = true;
        v.rng = Rng::new((self.age % 97) + 3);
        v.hp_state = 0.0;
    }
    fn note_off(&mut self, _key: u8) {}
    fn all_notes_off(&mut self) {
        for v in self.voices.iter_mut() {
            v.active = false;
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
        let decay = self.p[4];
        let ndecay = self.p[12];
        let drop = self.p[2] * 3.0;
        let snap = self.p[5];
        let ring = self.p[8];
        let click = self.p[10];
        let body = self.p[3];
        let drive = self.p[6];
        let tilt = self.p[7].clamp(-1.0, 1.0);
        let level = self.p[9];
        let tune2 = 2f32.powf(self.p[11] / 12.0);
        let drive_gain = 0.8 + drive * 0.35;
        let drive_norm = 0.9 + drive * 0.12;
        let inv_sr = 1.0 / sr;
        for i in 0..n {
            let mut acc = 0.0f32;
            for v in self.voices.iter_mut() {
                if !v.active {
                    continue;
                }
                v.t += inv_sr;
                if v.t > decay * 6.0 {
                    v.active = false;
                    continue;
                }
                let dt = v.t;
                let amp = (-dt / decay.max(0.01)).exp();
                let nap = (-dt / ndecay.max(0.01)).exp();
                let out = match v.kind {
                    DrumKind::Kick => {
                        // pitch envelope drop toward the fundamental
                        let f = v.pitch * (1.0 + drop * (-dt / (decay * 0.3).max(0.02)).exp());
                        v.phase = wrap01(v.phase + f * inv_sr);
                        let sine = (TWO_PI * v.phase).sin();
                        let shaped = (sine * (1.0 + body * 1.4)).tanh();
                        let sn_hp = v.rng.next_f32() - v.hp_state;
                        v.hp_state = 0.15;
                        let sn = sn_hp * snap * (-dt / 0.045).exp();
                        let clk = v.rng.next_f32() * click * (-dt / 0.004).exp();
                        shaped * amp + sn * 0.6 + clk
                    }
                    DrumKind::Snare => {
                        let f = v.pitch * (1.0 + drop * 0.5 * (-dt / (decay * 0.2).max(0.02)).exp());
                        v.phase = wrap01(v.phase + f * inv_sr);
                        let tone = (TWO_PI * v.phase).sin() * (-dt / (decay * 0.5).max(0.01)).exp();
                        let noise = v.rng.next_f32() * nap;
                        let ring_v = (TWO_PI * v.phase * 3.3).sin() * ring * amp * 0.4;
                        tone * 0.45 * body + noise * 0.9 + ring_v
                    }
                    DrumKind::Hat => {
                        // bright noise transient + odd-harmonic metallic partials
                        let noise = v.rng.next_f32();
                        let hp = noise - v.hp_state;
                        v.hp_state = noise * 0.5;
                        let metal_env = (-dt / (decay * 0.8).max(0.01)).exp();
                        v.phase = wrap01(v.phase + v.pitch * inv_sr);
                        let r1 = (TWO_PI * v.phase * 8.0).sin();
                        let r2 = (TWO_PI * v.phase * 11.7).sin();
                        let r3 = (TWO_PI * v.phase * 19.3).sin();
                        let tuned =
                            (r1 + r2 * 0.6 + r3 * 0.4) * 0.3 * metal_env * ring.max(0.1);
                        hp * nap * 1.1 + tuned
                    }
                    DrumKind::Clap => {
                        let noise = v.rng.next_f32();
                        // four short bursts then a tail
                        let burst_env = if dt < 0.035 {
                            let k = (dt / 0.010) as usize;
                            let pos = dt - k as f32 * 0.010;
                            (-pos / 0.004).exp()
                        } else {
                            (-((dt - 0.035) / 0.06)).exp()
                        };
                        v.burst = burst_env;
                        noise * burst_env * 1.2
                    }
                    DrumKind::Tom => {
                        let f = v.pitch * (1.0 + drop * 1.2 * (-dt / (decay * 0.25).max(0.02)).exp());
                        v.phase = wrap01(v.phase + f * inv_sr);
                        let sine = (TWO_PI * v.phase).sin();
                        let shaped = (sine * (1.0 + body)).tanh();
                        let sn = v.rng.next_f32() * snap * (-dt / 0.01).exp();
                        shaped * amp + sn * 0.3
                    }
                    DrumKind::Cell => {
                        let f = v.pitch;
                        let f2 = f * 2.0 * tune2;
                        let f3 = f * 3.3;
                        v.phase = wrap01(v.phase + f * inv_sr);
                        v.phase2 = wrap01(v.phase2 + f2 * inv_sr);
                        let s1 = (TWO_PI * v.phase).sin();
                        let s2 = (TWO_PI * v.phase2).sin() * 0.5;
                        let s3 = (TWO_PI * wrap01(v.phase2 * f3 / f2).sin()).sin() * 0.25;
                        let clk = v.rng.next_f32() * click * (-dt / 0.006).exp();
                        (s1 + s2 * ring + s3 * ring * 2.0) * amp + clk
                    }
                };
                acc += out.clamp(-6.0, 6.0).tanh() * v.vel * level;
            }
            // engine drive + tilt shelving
            let mut o = (acc * drive_gain).tanh() / drive_norm;
            o = o * (1.0 + tilt.min(0.0) * 0.8) + self.tilt_lp.tick(o) * tilt.max(0.0) * 0.9
                + o * tilt.max(0.0);
            let (pl, pr) = pan_gains(0.0);
            left[i] = o * pl;
            right[i] = o * pr;
        }
    }
    fn params(&self) -> &'static [ParamSpec] {
        DRUM_PARAMS
    }
    fn set_param(&mut self, i: usize, value: f32) {
        if let Some(spec) = DRUM_PARAMS.get(i) {
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

impl Default for DrumEngine {
    fn default() -> Self {
        Self::kick_one("KickOne")
    }
}
