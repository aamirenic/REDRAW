//! Additive/formant engine: 16-partial harmonic bank with brightness/tilt/blur
//! shaping, three-band formant filtering over vowel morphs, prism drift and
//! vibrato. 8 preallocated voices.
#![forbid(unsafe_code)]

use redraw_dsp::{wrap01, Adsr, Lfo, LfoShape, Rng, Svf, SvfMode, TWO_PI};
use redraw_shared::{midi_note_hz as hz, pan_gains, ParamSpec, Synth};

pub const ADD_POLY: usize = 8;
pub const ADD_PARTIALS: usize = 16;

#[rustfmt::skip]
pub static ADD_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Brightness", 0.0, 1.0, 0.6, ""),
    ParamSpec::new("Odd/Even Tilt", -1.0, 1.0, 0.0, ""),
    ParamSpec::new("Formant Vowel", 0.0, 4.0, 0.0, ""),
    ParamSpec::new("Formant Depth", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Prism Depth", 0.0, 1.0, 0.4, ""),
    ParamSpec::new("Prism Rate", 0.01, 8.0, 0.7, "Hz"),
    ParamSpec::new("Attack", 0.0005, 8.0, 0.008, "s"),
    ParamSpec::new("Decay", 0.0005, 8.0, 0.4, "s"),
    ParamSpec::new("Sustain", 0.0, 1.0, 0.75, ""),
    ParamSpec::new("Release", 0.0005, 8.0, 0.5, "s"),
    ParamSpec::new("Vibrato Rate", 0.01, 20.0, 5.0, "Hz"),
    ParamSpec::new("Vibrato Depth", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Blur", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Formant Q", 2.0, 14.0, 5.0, ""),
    ParamSpec::new("Formant Morph", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Gain", 0.0, 2.0, 0.85, ""),
];

/// Canonical vowel formant triads (F1, F2, F3 in Hz).
const VOWELS: [[f32; 3]; 5] = [
    [730.0, 1090.0, 2440.0], // A
    [530.0, 1840.0, 2480.0], // E
    [270.0, 2290.0, 3010.0], // I
    [570.0, 840.0, 2410.0],  // O
    [300.0, 870.0, 2240.0],  // U
];

struct AddVoice {
    key: u8,
    vel: f32,
    gate: bool,
    age: u64,
    amp: Adsr,
    phases: [f32; ADD_PARTIALS],
    freqs: [f32; ADD_PARTIALS],
    amps: [f32; ADD_PARTIALS],
    svfs: [Svf; 3],
    rng: Rng,
}

impl Default for AddVoice {
    fn default() -> Self {
        Self {
            key: 0,
            vel: 0.0,
            gate: false,
            age: 0,
            amp: Adsr::new(),
            phases: [0.0; ADD_PARTIALS],
            freqs: [0.0; ADD_PARTIALS],
            amps: [0.0; ADD_PARTIALS],
            svfs: std::array::from_fn(|i| {
                let mut f = Svf::new();
                f.reset();
                let _ = i;
                f
            }),
            rng: Rng::new(0x2B),
        }
    }
}

pub struct AdditiveEngine {
    pub name: &'static str,
    pub sr: f32,
    pub p: [f32; ADD_PARAMS.len()],
    voices: [AddVoice; ADD_POLY],
    pub age: u64,
    pub lfo: Lfo,
    pub prism_phase: f32,
}

impl AdditiveEngine {
    pub fn with_params(name: &'static str, values: &[f32]) -> Self {
        let mut e = Self {
            name,
            sr: 48_000.0,
            p: std::array::from_fn(|i| ADD_PARAMS[i].default),
            voices: std::array::from_fn(|_| AddVoice::default()),
            age: 0,
            lfo: Lfo::new(0x61),
            prism_phase: 0.0,
        };
        for (i, v) in values.iter().enumerate().take(e.p.len()) {
            e.set_param(i, *v);
        }
        e
    }

    fn take_voice(&mut self) -> usize {
        if let Some(pos) = (0..ADD_POLY).find(|&i| !self.voices[i].amp.is_active()) {
            return pos;
        }
        (0..ADD_POLY)
            .min_by_key(|&i| self.voices[i].age)
            .unwrap_or(0)
    }

    /// Interpolate the vowel triad between the selected and next vowel.
    fn formant_freqs(&self) -> [f32; 3] {
        let sel = (self.p[2] as usize).min(4);
        let nxt = (sel + 1) % 5;
        let m = self.p[14].clamp(0.0, 1.0) * 0.6;
        let a = VOWELS[sel];
        let b = VOWELS[nxt];
        [
            a[0] + (b[0] - a[0]) * m,
            a[1] + (b[1] - a[1]) * m,
            a[2] + (b[2] - a[2]) * m,
        ]
    }
}

// ---------------------------------------------------------------- presets

impl AdditiveEngine {
    pub fn prism(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0.75, 0.0, 0.0, 0.15, 0.5, 0.7, 0.01, 0.3, 0.8, 0.4, 5.5, 0.06, 0.1, 5.0, 0.1, 0.9],
        )
    }
    pub fn prism_lite(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0.5, 0.0, 0.0, 0.1, 0.2, 0.25, 0.005, 0.2, 0.8, 0.3, 4.5, 0.03, 0.05, 4.0, 0.0, 0.9],
        )
    }
    pub fn shapeshift(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0.65, 0.25, 1.0, 0.5, 0.95, 3.0, 0.03, 0.6, 0.85, 0.6, 6.0, 0.15, 0.5, 6.0, 0.5, 0.95],
        )
    }
    pub fn forge(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0.9, 0.6, 0.0, 0.0, 0.3, 8.0, 0.001, 1.5, 0.0, 1.5, 0.0, 0.0, 0.2, 9.0, 0.0, 1.0],
        )
    }
    pub fn vowel(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0.35, 0.0, 0.0, 0.95, 0.15, 0.3, 0.02, 0.2, 0.9, 0.2, 5.0, 0.05, 0.0, 8.0, 0.25, 1.0],
        )
    }
    pub fn vocal_dance(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0.45, 0.1, 0.0, 0.9, 0.2, 0.5, 0.04, 0.15, 0.95, 0.15, 5.5, 0.5, 0.1, 7.0, 0.15, 1.0],
        )
    }
    pub fn spectral_map(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[1.0, 0.0, 0.0, 0.2, 0.05, 0.15, 0.006, 0.25, 0.7, 0.35, 0.0, 0.0, 0.9, 4.0, 0.2, 0.9],
        )
    }
    pub fn keyforge(name: &'static str) -> Self {
        Self::with_params(
            name,
            &[0.55, -0.3, 0.0, 0.05, 0.1, 0.2, 0.002, 2.5, 0.3, 0.8, 0.0, 0.0, 0.05, 4.0, 0.0, 1.0],
        )
    }
}

// ---------------------------------------------------------------- synth impl

impl Synth for AdditiveEngine {
    fn synth_type(&self) -> &'static str {
        self.name
    }
    fn set_display_name(&mut self, name: &'static str) {
        self.name = name;
    }
    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            *v = AddVoice::default();
        }
        self.lfo.reset();
        self.prism_phase = 0.0;
        self.age = 0;
    }
    fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr.max(8000.0);
    }
    fn note_on(&mut self, key: u8, velocity: f32) {
        for v in self.voices.iter_mut() {
            if v.gate && v.key == key {
                v.gate = false;
                v.amp.gate_off();
            }
        }
        let idx = self.take_voice();
        let bright = self.p[0].clamp(0.0, 1.0);
        let tilt = self.p[1].clamp(-1.0, 1.0);
        let blur = self.p[12].clamp(0.0, 1.0);
        let prism = self.p[4].clamp(0.0, 1.0);
        let mut fv = std::mem::take(&mut self.voices[idx]);
        fv.key = key;
        fv.vel = velocity.clamp(0.02, 1.0);
        fv.gate = true;
        self.age = self.age.wrapping_add(1);
        fv.age = self.age;
        let base = hz(key as f32).max(30.0);
        let steep = (1.0 - bright) * 2.6 + 0.4;
        let mut peak = 0.0f32;
        for (h, hf) in fv.freqs.iter_mut().enumerate() {
            let harm = (h + 1) as f32;
            let mut amp = 1.0 / harm;
            amp = amp.powf(steep);
            amp *= if (h + 1) % 2 == 1 {
                1.0 + tilt * 0.4
            } else {
                1.0 - tilt * 0.4
            };
            amp = amp.max(0.0);
            amp *= 1.0 + prism * 0.2 * fv.rng.unipolar() * 0.5;
            *hf = base * harm * (1.0 + blur * 0.012 * fv.rng.next_f32());
            fv.amps[h] = amp;
            peak = peak.max(amp);
        }
        if peak > 0.0 {
            for a in fv.amps.iter_mut() {
                *a /= peak;
            }
        }
        for ph in fv.phases.iter_mut() {
            *ph = fv.rng.unipolar();
        }
        for f in fv.svfs.iter_mut() {
            *f = Svf::new();
        }
        fv.amp = Adsr::new();
        fv.amp.set(self.p[6], self.p[7], self.p[8], self.p[9]);
        fv.amp.gate_on(fv.vel);
        self.voices[idx] = fv;
    }
    fn note_off(&mut self, key: u8) {
        for v in self.voices.iter_mut() {
            if v.key == key && v.gate {
                v.gate = false;
                v.amp.gate_off();
            }
        }
    }
    fn all_notes_off(&mut self) {
        for v in self.voices.iter_mut() {
            v.gate = false;
            v.amp.gate_off();
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
        let formant_depth = self.p[3].clamp(0.0, 1.0);
        let f_q = self.p[13].max(1.0);
        let prism = self.p[4].clamp(0.0, 1.0);
        let prism_rate = self.p[5].max(0.01);
        let gain = self.p[15];
        let vib_rate = self.p[10].max(0.01);
        let vib_d = self.p[11].clamp(0.0, 1.0) * 0.5; // semitones
        let ff = self.formant_freqs();
        self.lfo.shape = LfoShape::Sine;
        let mut prism_lfo = self.prism_phase;
        for i in 0..n {
            self.lfo.set_rate(vib_rate);
            let vib = self.lfo.tick(sr);
            prism_lfo = wrap01(prism_lfo + prism_rate / sr);
            let pitch = 2f32.powf(vib * vib_d / 12.0);
            let mut sl = 0.0f32;
            let mut sr_ = 0.0f32;
            for v in self.voices.iter_mut() {
                if !v.amp.is_active() {
                    continue;
                }
                let a = v.amp.tick(sr) * v.vel;
                let mut dry = 0.0f32;
                for k in 0..ADD_PARTIALS {
                    // prism drifts the spectrum unevenly across partials
                    let mod_a = 1.0 + prism * 0.3 * (TWO_PI * (prism_lfo + k as f32 * 0.13)).sin();
                    v.phases[k] = wrap01(v.phases[k] + (v.freqs[k] * pitch).max(8.0) / sr);
                    dry += v.phases[k].sin() * v.amps[k] * mod_a;
                }
                // three-band formant chain (parallel bandpasses)
                let mut formant = 0.0f32;
                for (k, f) in v.svfs.iter_mut().enumerate() {
                    f.set(ff[k], f_q, sr);
                    let bp = f.tick_mode(dry, SvfMode::Band);
                    let w = [1.0, 0.7, 0.45][k];
                    formant += bp * w;
                }
                let out = (dry * (1.0 - formant_depth) + formant * formant_depth * 3.2)
                    * gain
                    * a;
                let out = out.clamp(-8.0, 8.0);
                sl += out;
                sr_ += out;
            }
            let (pl, pr) = pan_gains(0.0);
            left[i] = sl * pl;
            right[i] = sr_ * pr;
        }
        self.prism_phase = prism_lfo;
    }
    fn params(&self) -> &'static [ParamSpec] {
        ADD_PARAMS
    }
    fn set_param(&mut self, i: usize, value: f32) {
        if let Some(spec) = ADD_PARAMS.get(i) {
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

impl Default for AdditiveEngine {
    fn default() -> Self {
        Self::prism("Prism")
    }
}
