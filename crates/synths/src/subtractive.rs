//! Subtractive polysynth engine: 3 oscillators + noise, TPT SVF with drive, two
//! ADSRs, two LFOs, unison, glide, pan. 16 preallocated voices, no RT allocation.
#![forbid(unsafe_code)]

use redraw_dsp::{Adsr, Lfo, LfoShape, Osc, OscKind, Rng, Svf, SvfMode};
use redraw_shared::{midi_note_hz as hz, pan_gains, ParamCurve, ParamSpec, Synth};

pub const SUB_POLY: usize = 16;
pub const SUB_UNI: usize = 7;

#[rustfmt::skip]
pub static SUB_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Osc1 Wave", 0.0, 5.0, 1.0, ""),
    ParamSpec::new("Osc1 Octave", -3.0, 3.0, 0.0, "oct"),
    ParamSpec::new("Osc1 Detune", -100.0, 100.0, 0.0, "cents"),
    ParamSpec::new("Osc1 Level", 0.0, 1.0, 0.8, ""),
    ParamSpec::new("Osc2 Wave", 0.0, 5.0, 1.0, ""),
    ParamSpec::new("Osc2 Octave", -3.0, 3.0, 0.0, "oct"),
    ParamSpec::new("Osc2 Detune", -100.0, 100.0, 7.0, "cents"),
    ParamSpec::new("Osc2 Level", 0.0, 1.0, 0.6, ""),
    ParamSpec::new("Osc3 Wave", 0.0, 5.0, 0.0, ""),
    ParamSpec::new("Osc3 Octave", -3.0, 3.0, -1.0, "oct"),
    ParamSpec::new("Osc3 Detune", -100.0, 100.0, 0.0, "cents"),
    ParamSpec::new("Osc3 Level", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Noise Level", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Cutoff", 20.0, 20000.0, 12000.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Resonance", 0.15, 14.0, 0.7, ""),
    ParamSpec::new("Filter Type", 0.0, 3.0, 0.0, ""),
    ParamSpec::new("Filter Env Amount", -6.0, 6.0, 1.5, "oct"),
    ParamSpec::new("Filter Drive", 0.5, 8.0, 1.0, ""),
    ParamSpec::new("Amp Attack", 0.0005, 8.0, 0.004, "s"),
    ParamSpec::new("Amp Decay", 0.0005, 8.0, 0.2, "s"),
    ParamSpec::new("Amp Sustain", 0.0, 1.0, 0.75, ""),
    ParamSpec::new("Amp Release", 0.0005, 8.0, 0.25, "s"),
    ParamSpec::new("Flt Attack", 0.0005, 8.0, 0.003, "s"),
    ParamSpec::new("Flt Decay", 0.0005, 8.0, 0.35, "s"),
    ParamSpec::new("Flt Sustain", 0.0, 1.0, 0.2, ""),
    ParamSpec::new("Flt Release", 0.0005, 8.0, 0.4, "s"),
    ParamSpec::new("LFO1 Rate", 0.01, 20.0, 4.5, "Hz"),
    ParamSpec::new("LFO1 Target", 0.0, 3.0, 0.0, ""),
    ParamSpec::new("LFO1 Depth", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("LFO2 Rate", 0.01, 20.0, 0.8, "Hz"),
    ParamSpec::new("LFO2 Target", 0.0, 3.0, 0.0, ""),
    ParamSpec::new("LFO2 Depth", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Unison Voices", 1.0, 7.0, 1.0, ""),
    ParamSpec::new("Unison Detune", 0.0, 60.0, 12.0, "cents"),
    ParamSpec::new("Glide", 0.0, 2.0, 0.0, "s"),
    ParamSpec::new("Pan", -1.0, 1.0, 0.0, ""),
];

struct SubVoice {
    key: u8,
    vel: f32,
    gate: bool,
    age: u64,
    target: f32,
    cur: f32,
    amp: Adsr,
    fenv: Adsr,
    oscs: [Osc; 3],
    uni: [Osc; SUB_UNI],
    noise: Rng,
    svf: Svf,
}

impl Default for SubVoice {
    fn default() -> Self {
        Self {
            key: 0,
            vel: 0.0,
            gate: false,
            age: 0,
            target: 0.0,
            cur: 0.0,
            amp: Adsr::new(),
            fenv: Adsr::new(),
            oscs: std::array::from_fn(|i| Osc::new(OscKind::Saw, i as u64 + 11)),
            uni: std::array::from_fn(|i| Osc::new(OscKind::Saw, i as u64 + 71)),
            noise: Rng::new(0x17),
            svf: Svf::new(),
        }
    }
}

pub struct SubtractiveEngine {
    pub name: &'static str,
    pub sr: f32,
    pub p: [f32; SUB_PARAMS.len()],
    voices: [SubVoice; SUB_POLY],
    pub lfo1: Lfo,
    pub lfo2: Lfo,
    pub age: u64,
    pub last_freq: f32,
}

impl SubtractiveEngine {
    pub fn with_params(name: &'static str, values: &[f32]) -> Self {
        let mut e = Self {
            name,
            sr: 48_000.0,
            p: std::array::from_fn(|i| SUB_PARAMS[i].default),
            voices: std::array::from_fn(|_| SubVoice::default()),
            lfo1: Lfo::new(0x51),
            lfo2: Lfo::new(0x52),
            age: 0,
            last_freq: 0.0,
        };
        for (i, v) in values.iter().enumerate().take(e.p.len()) {
            e.set_param(i, *v);
        }
        e
    }

    #[inline]
    fn wave(p: f32) -> OscKind {
        match p as u32 {
            0 => OscKind::Sine,
            1 => OscKind::Saw,
            2 => OscKind::Square,
            3 => OscKind::Triangle,
            4 => OscKind::Pulse,
            _ => OscKind::Noise,
        }
    }

    #[inline]
    fn mode(p: f32) -> SvfMode {
        match p as u32 {
            0 => SvfMode::Low,
            1 => SvfMode::High,
            2 => SvfMode::Band,
            _ => SvfMode::Notch,
        }
    }

    fn take_voice(&mut self) -> usize {
        if let Some(pos) = (0..SUB_POLY).find(|&i| !self.voices[i].amp.is_active()) {
            return pos;
        }
        (0..SUB_POLY)
            .min_by_key(|&i| self.voices[i].age)
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------- presets

impl SubtractiveEngine {
    pub fn triple_osc(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[1., 0., 0., 0.9, 1., 0., 8., 0.7, 0., -1., 0., 0.35, 0., 14000., 0.7, 0., 0., 1.,
              0.003, 0.12, 0.8, 0.15, 0.003, 0.1, 0.2, 0.1, 4.5, 0., 0., 0.8, 0., 0., 1., 12., 0., 0.],
        )
    }
    pub fn circuit_bass(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[1., 0., 0., 0.95, 0., -1., 0., 0.6, 1., 1., 0., 0., 0., 650., 3.4, 0., 4.2, 3.2,
              0.001, 0.08, 0.12, 0.05, 0.001, 0.06, 0.18, 0.05, 0., 0., 0., 0., 0., 0., 1., 0., 0.06, 0.],
        )
    }
    pub fn orbit(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[1., 1., -5., 0.55, 1., 0., 9., 0.8, 1., -1., 4., 0.4, 0., 2400., 1.1, 0., 2., 2.,
              0.01, 0.25, 0.6, 0.5, 0.02, 0.3, 0.25, 0.4, 3.1, 0., 0.12, 0.6, 2., 0.15, 3., 10., 0., 0.],
        )
    }
    pub fn orbit_x(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[1., 1., -7., 0.5, 2., 0., 12., 0.7, 1., -1., 6., 0.45, 0.1, 1800., 1.6, 2., 3., 2.4,
              0.012, 0.3, 0.5, 0.7, 0.02, 0.35, 0.2, 0.5, 4., 0., 0.3, 5., 2., 0.2, 5., 14., 0.02, -0.2],
        )
    }
    pub fn sawline(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[1., 0., 0., 0.6, 0., 0., 0., 0.15, 0., 1., 0., 0.25, 0., 1200., 0.9, 0., 1.4, 2.6,
              0.003, 0.2, 0.85, 0.4, 0.004, 0.25, 0.3, 0.4, 1.4, 2., 0.25, 0., 0., 0., 6., 18., 0., 0.],
        )
    }
    pub fn zone(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0., 1., 0., 0.75, 0., 1., 6., 0.5, 1., 0., -3., 0.5, 0.06, 5200., 2.2, 3., 3., 2.,
              0.002, 0.1, 0.5, 0.2, 0.01, 0.2, 0.35, 0.2, 6., 1., 0.45, 0., 0., 0., 4., 9., 0., 0.1],
        )
    }
    pub fn simplex(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[1., 0., 0., 0.8, 1., 0., 10., 0.65, 0., 0., 0., 0.4, 0., 9000., 0.7, 0., 1., 1.6,
              0.006, 0.18, 0.7, 0.35, 0.006, 0.2, 0.3, 0.3, 0., 0., 0., 2., 0., 0., 2., 8., 0.09, 0.],
        )
    }
    pub fn stinger(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[4., 0., 0., 0.9, 1., 1., 11., 0.5, 0., -2., 0., 0.3, 0.12, 3200., 4.6, 0., 3.5, 4.,
              0.001, 0.09, 0.35, 0.03, 0.002, 0.12, 0.25, 0.06, 0., 0., 0., 0., 0., 0., 1., 0., 0.03, 0.2],
        )
    }
    pub fn stinger_xt(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[2., 0., 0., 0.8, 1., 1., 14., 0.55, 1., -1., 5., 0.35, 0.2, 4200., 5., 2., 4., 5.,
              0.001, 0.1, 0.3, 0.04, 0.002, 0.14, 0.3, 0.08, 7., 2., 0.4, 0., 0., 0., 3., 12., 0.04, 0.1],
        )
    }
    pub fn minivee(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[1., 0., 0., 0.85, 1., 0., -6., 0.55, 0., -1., 0., 0.3, 0.05, 1600., 1.3, 0., 2.2, 2.8,
              0.004, 0.22, 0.8, 0.3, 0.004, 0.25, 0.35, 0.35, 2.2, 0., 0.18, 0., 0., 0., 2., 7., 0.12, 0.],
        )
    }
    pub fn groove(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[1., 0., -4., 0.7, 1., 0., 7., 0.6, 3., 0., 3., 0.3, 0.08, 2800., 3.5, 0., 2.6, 3.,
              0.002, 0.15, 0.6, 0.12, 0.002, 0.2, 0.3, 0.15, 5., 0., 0.3, 3., 2., 0.2, 4., 11., 0., 0.],
        )
    }
    pub fn acidbox(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[1., -1., 0., 1., 0., -2., 0., 0., 0., 0., 0., 0., 0., 320., 9., 0., 4.4, 5.,
              0.001, 0.1, 0.05, 0.03, 0.001, 0.1, 0.25, 0.04, 0., 0., 0., 0., 0., 0., 1., 0., 0.07, 0.],
        )
    }
    pub fn bassbox(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[1., -1., 0., 0.9, 0., -1., 0., 0.5, 0., 0., 0., 0., 0., 760., 1.6, 0., 1.2, 2.2,
              0.003, 0.14, 0.8, 0.07, 0.003, 0.18, 0.4, 0.1, 0., 0., 0., 0., 0., 0., 1., 0., 0.04, 0.],
        )
    }
    pub fn synthbox(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[3., 0., 0., 0.7, 1., 1., 9., 0.6, 0., 0., 6., 0.3, 0., 6000., 1., 0., 1.5, 1.8,
              0.008, 0.25, 0.8, 0.4, 0.008, 0.3, 0.3, 0.4, 1.8, 0., 0.2, 0., 0., 0., 3., 15., 0., 0.],
        )
    }
}

// ---------------------------------------------------------------- synth impl

impl Synth for SubtractiveEngine {
    fn synth_type(&self) -> &'static str {
        self.name
    }
    fn set_display_name(&mut self, name: &'static str) {
        self.name = name;
    }
    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            *v = SubVoice::default();
        }
        self.lfo1.reset();
        self.lfo2.reset();
        self.age = 0;
        self.last_freq = 0.0;
    }
    fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr.max(8000.0);
    }
    fn note_on(&mut self, key: u8, velocity: f32) {
        // retrigger: release the old voice on this key first
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
        v.vel = velocity.clamp(0.02, 1.0);
        v.gate = true;
        self.age = self.age.wrapping_add(1);
        v.age = self.age;
        let f = hz(key as f32);
        let glide = self.p[34];
        v.cur = if glide <= 0.0 {
            f
        } else if self.last_freq > 20.0 {
            self.last_freq
        } else {
            f
        };
        v.target = f;
        v.oscs[0] = Osc::new(Self::wave(self.p[0]), (idx as u64) * 5 + 31);
        v.oscs[1] = Osc::new(Self::wave(self.p[4]), (idx as u64) * 5 + 61);
        v.oscs[2] = Osc::new(Self::wave(self.p[8]), (idx as u64) * 5 + 93);
        v.uni =
            std::array::from_fn(|i| Osc::new(Self::wave(self.p[0]), (idx as u64) * 13 + 7 + i as u64));
        v.amp = Adsr::new();
        v.amp.set(self.p[18], self.p[19], self.p[20], self.p[21]);
        v.amp.gate_on(v.vel);
        v.fenv = Adsr::new();
        v.fenv.set(self.p[22], self.p[23], self.p[24], self.p[25]);
        v.fenv.gate_on(1.0);
        v.svf = Svf::new();
        self.voices[idx] = v;
        self.last_freq = f;
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
        // hoisted params
        let l1_r = self.p[26].max(0.01);
        let l2_r = self.p[29].max(0.01);
        let l1_d = self.p[28];
        let l2_d = self.p[31];
        let l1_t = self.p[27] as u32;
        let l2_t = self.p[30] as u32;
        let uni_n = (self.p[32] as usize).min(SUB_UNI);
        let uni_det = self.p[33];
        let waves = [Self::wave(self.p[0]), Self::wave(self.p[4]), Self::wave(self.p[8])];
        let octs = [self.p[1], self.p[5], self.p[9]];
        let dets = [self.p[2], self.p[6], self.p[10]];
        let lvls = [self.p[3], self.p[7], self.p[11]];
        let noise_lvl = self.p[12];
        let cut = self.p[13];
        let q = self.p[14];
        let mode = Self::mode(self.p[15]);
        let env_amt = self.p[16];
        let drive = self.p[17];
        let glide = self.p[34];
        let base_pan = self.p[35];
        let glide_c = if glide > 0.0 {
            1.0 - (-1.0 / (glide * sr).max(1.0)).exp()
        } else {
            1.0
        };
        self.lfo1.shape = LfoShape::Sine;
        self.lfo2.shape = LfoShape::Sine;
        for i in 0..n {
            // engine-level LFOs, ticked once per sample
            self.lfo1.set_rate(l1_r);
            self.lfo2.set_rate(l2_r);
            let lf1 = self.lfo1.tick(sr);
            let lf2 = self.lfo2.tick(sr);
            // pitch modulation (semitones)
            let mut pitch_mod = 0.0;
            if l1_t == 0 {
                pitch_mod += l1_d * lf1 * 12.0;
            }
            if l2_t == 0 {
                pitch_mod += l2_d * lf2 * 12.0;
            }
            let pitch = 2f32.powf(pitch_mod / 12.0);
            // pan (modulated)
            let pan = base_pan
                + if l1_t == 3 { lf1 * l1_d * 0.5 } else { 0.0 }
                + if l2_t == 3 { lf2 * l2_d * 0.5 } else { 0.0 };
            let (pl, pr) = pan_gains(pan.clamp(-1.0, 1.0));
            let mut sl = 0.0f32;
            let mut sr_ = 0.0f32;
            for v in self.voices.iter_mut() {
                if !v.amp.is_active() {
                    continue;
                }
                let a = v.amp.tick(sr) * v.vel;
                let fe = v.fenv.tick(sr);
                // glide toward target
                if glide > 0.0 {
                    v.cur += (v.target - v.cur) * glide_c;
                    if (v.cur - v.target).abs() < 0.02 {
                        v.cur = v.target;
                    }
                } else {
                    v.cur = v.target;
                }
                // oscillators
                let mut m = 0.0f32;
                for k in 0..3 {
                    if lvls[k] <= 0.0 {
                        continue;
                    }
                    let freq = (v.cur * pitch * 2f32.powf(octs[k]) * 2f32.powf(dets[k] / 1200.0))
                        .max(8.0);
                    let osc = &mut v.oscs[k];
                    osc.set_freq(freq, sr);
                    let s = if waves[k] == OscKind::Noise {
                        v.noise.next_f32()
                    } else {
                        osc.next()
                    };
                    m += s * a * lvls[k];
                }
                // unison on osc1
                if uni_n > 1 {
                    let weight = 1.0 / (uni_n as f32).sqrt();
                    for u in 0..uni_n {
                        let spread = ((u as f32 / (uni_n.max(2) - 1) as f32) - 0.5)
                            * 2.0
                            * uni_det
                            / 1200.0;
                        let freq = (v.cur * pitch * 2f32.powf(spread)).max(8.0);
                        let osc = &mut v.uni[u];
                        osc.set_freq(freq, sr);
                        let s = if waves[0] == OscKind::Noise {
                            v.noise.next_f32()
                        } else {
                            osc.next()
                        };
                        m += s * a * lvls[0] * weight;
                    }
                }
                if noise_lvl > 0.0 {
                    m += v.noise.next_f32() * a * noise_lvl;
                }
                // filter
                let mut fc_mod = fe * env_amt;
                if l1_t == 1 {
                    fc_mod += l1_d * lf1 * 4.0;
                }
                if l2_t == 1 {
                    fc_mod += l2_d * lf2 * 4.0;
                }
                let fc = (cut * 2f32.powf(fc_mod)).clamp(20.0, sr * 0.47);
                v.svf.drive = drive.max(0.2);
                v.svf.set(fc, q, sr);
                let filtered = v.svf.tick_mode(m, mode);
                // amplitude LFO (multiplicative, unipolar-weighted)
                let mut amp_mod = 1.0;
                if l1_t == 2 {
                    amp_mod += l1_d * lf1 * 0.5;
                }
                if l2_t == 2 {
                    amp_mod += l2_d * lf2 * 0.5;
                }
                let out = (filtered * amp_mod.abs()).clamp(-4.0, 4.0);
                sl += out * pl;
                sr_ += out * pr;
            }
            left[i] = sl;
            right[i] = sr_;
        }
    }
    fn params(&self) -> &'static [ParamSpec] {
        SUB_PARAMS
    }
    fn set_param(&mut self, i: usize, value: f32) {
        if let Some(spec) = SUB_PARAMS.get(i) {
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

impl Default for SubtractiveEngine {
    fn default() -> Self {
        Self::triple_osc("Triple Osc")
    }
}
