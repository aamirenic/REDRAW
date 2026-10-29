//! Granular engine: grain cloud with density/spray/pan-spread over an internal
//! tone+noise source pair (user sample loading arrives with the sampler phase).
#![forbid(unsafe_code)]

use redraw_dsp::{lerp, wrap01, Adsr, Rng, TWO_PI};
use redraw_shared::{pan_gains, ParamSpec, Synth};

pub const GRAIN_SLOTS: usize = 16;
pub const GRAIN_SRC: usize = 96_000;

#[rustfmt::skip]
pub static GRAN_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Grain Size", 10.0, 800.0, 120.0, "ms"),
    ParamSpec::new("Density", 1.0, 40.0, 12.0, "grains/s"),
    ParamSpec::new("Spray", 0.0, 1.0, 0.3, ""),
    ParamSpec::new("Pitch", -24.0, 24.0, 0.0, "st"),
    ParamSpec::new("Drift", -12.0, 12.0, 0.0, ""),
    ParamSpec::new("Spread", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Attack", 0.0, 2.0, 0.05, "s"),
    ParamSpec::new("Release", 0.0, 2.0, 0.4, "s"),
    ParamSpec::new("Window", 0.0, 3.0, 0.0, ""),
    ParamSpec::new("Source", 0.0, 1.0, 0.35, ""),
    ParamSpec::new("Loop Start", 0.0, 0.95, 0.1, ""),
    ParamSpec::new("Loop Length", 0.05, 1.0, 0.6, ""),
];

#[derive(Clone, Copy, Debug)]
struct Grain {
    active: bool,
    pos: f32,
    win_len: f32,
    age: f32,
    rate: f32,
    pan: f32,
    shape: u8,
}

pub struct GranularEngine {
    pub name: &'static str,
    pub sr: f32,
    pub p: [f32; GRAN_PARAMS.len()],
    pub env: Adsr,
    grains: [Grain; GRAIN_SLOTS],
    pub next_grain: f32,
    pub drift_t: f32,
    pub gate: bool,
    rng: Rng,
    tone: Box<[f32]>,
    noise: Box<[f32]>,
}

impl GranularEngine {
    pub fn with_params(name: &'static str, values: &[f32], bright: usize, dark_noise: f32) -> Self {
        let tone = tone_buf(bright, 1.0 - dark_noise);
        let noise = noise_buf(0.5 + dark_noise * 0.3);
        let mut e = Self {
            name,
            sr: 48_000.0,
            p: std::array::from_fn(|i| GRAN_PARAMS[i].default),
            env: Adsr::new(),
            grains: std::array::from_fn(|_| Grain {
                active: false,
                pos: 0.0,
                win_len: 2400.0,
                age: 0.0,
                rate: 1.0,
                pan: 0.0,
                shape: 0,
            }),
            next_grain: 0.0,
            drift_t: 0.0,
            gate: false,
            rng: Rng::new(0x59),
            tone,
            noise,
        };
        for (i, v) in values.iter().enumerate().take(e.p.len()) {
            e.set_param(i, *v);
        }
        e
    }

    pub fn grainmill(name: &'static str) -> Self {
        Self::with_params(name, &[120., 14., 0.3, 0., 0., 0.5, 0.05, 0.4, 0., 0.35, 0.1, 0.6], 20, 0.25)
    }
    pub fn warptrail(name: &'static str) -> Self {
        Self::with_params(name, &[260., 9., 0.7, -2., -3., 0.8, 0.1, 0.5, 1., 0.65, 0.05, 0.85], 10, 0.6)
    }

    fn spawn(&mut self, sr: f32) {
        if let Some(slot) = self.grains.iter_mut().find(|g| !g.active) {
            let start_f = self.p[10].clamp(0.0, 0.95);
            let len_f = self.p[11].clamp(0.05, 1.0);
            let drift = ((self.drift_t * self.p[4] / 12.0 * 0.02) % 1.0).abs();
            let center = (start_f + drift * (1.0 - len_f)) * GRAIN_SRC as f32;
            let spray = self.p[2].clamp(0.0, 1.0);
            let jitter = self.rng.unipolar() * spray * len_f * GRAIN_SRC as f32 * 0.5;
            let gsize = self.p[0].clamp(10.0, 800.0) * 0.001 * sr;
            slot.active = true;
            slot.pos = (center + jitter) % GRAIN_SRC as f32;
            slot.win_len = gsize.max(64.0);
            slot.age = 0.0;
            slot.rate = 2f32.powf((self.p[3] + (self.rng.next_f32() * self.p[2] * 2.0)) / 12.0);
            slot.pan = (self.rng.next_f32() * self.p[5].clamp(0.0, 1.0) * 1.6 - 0.8)
                * (self.p[5].clamp(0.0, 1.0));
            slot.shape = (self.p[8] as u8 % 4) + 1;
        }
    }
}

#[inline]
fn grain_window(g: &Grain) -> f32 {
    let t = (g.age / g.win_len).clamp(0.0, 1.0);
    match g.shape {
        1 => (TWO_PI * t).sin(), // sine window
        2 => 0.5 - 0.5 * (TWO_PI * t).cos(),               // hann
        3 => if t < 0.5 { 2.0 * t } else { 2.0 - 2.0 * t }, // triangle
        _ => (TWO_PI * t).sin(),
    }
}

fn tone_buf(harmonics: usize, level: f32) -> Box<[f32]> {
    let mut buf = vec![0.0f32; GRAIN_SRC];
    let base = 110.0;
    for h in 1..=harmonics.max(1) {
        let amp = (1.0 / h as f32).powf(1.4);
        for (i, s) in buf.iter_mut().enumerate() {
            let t = i as f32 / 48_000.0;
            *s += amp * (TWO_PI * base * h as f32 * t).sin() * 0.5;
        }
    }
    let peak = buf.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
    for s in buf.iter_mut() {
        *s = *s / peak * (level.clamp(0.0, 1.0) * 0.9);
    }
    buf.into_boxed_slice()
}

fn noise_buf(level: f32) -> Box<[f32]> {
    let mut rng = Rng::new(0x5A);
    let buf: Vec<f32> = (0..GRAIN_SRC).map(|_| rng.next_f32() * 0.6 * level).collect();
    buf.into_boxed_slice()
}

// ---------------------------------------------------------------- synth impl

impl Synth for GranularEngine {
    fn synth_type(&self) -> &'static str {
        self.name
    }
    fn set_display_name(&mut self, name: &'static str) {
        self.name = name;
    }
    fn reset(&mut self) {
        for g in self.grains.iter_mut() {
            g.active = false;
        }
        self.env.reset();
        self.gate = false;
        self.next_grain = 0.0;
        self.drift_t = 0.0;
    }
    fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr.max(8000.0);
    }
    fn note_on(&mut self, _key: u8, velocity: f32) {
        self.gate = true;
        if self.env.level() <= 0.0 {
            self.env.reset();
        }
        self.env.set(self.p[6].max(0.001), 0.2, 1.0, self.p[7].max(0.001));
        self.env.gate_on(velocity.clamp(0.02, 1.0));
    }
    fn note_off(&mut self, _key: u8) {
        self.gate = false;
        self.env.gate_off();
    }
    fn all_notes_off(&mut self) {
        self.gate = false;
        self.env.gate_off();
    }
    fn render(&mut self, left: &mut [f32], right: &mut [f32], sr_in: f32) {
        let sr = if self.sr > 1000.0 { self.sr } else { sr_in };
        let n = left.len().min(right.len());
        left.fill(0.0);
        right.fill(0.0);
        if n == 0 {
            return;
        }
        let src_mix = self.p[9].clamp(0.0, 1.0);
        let density = self.p[1].clamp(1.0, 40.0);
        for i in 0..n {
            let env_v = self.env.tick(sr);
            self.drift_t += 1.0 / sr;
            // grain scheduling
            self.next_grain -= 1.0;
            if env_v > 0.001 && self.next_grain <= 0.0 {
                self.spawn(sr);
                self.next_grain = (sr / density) * (0.8 + 0.4 * self.rng.unipolar());
            }
            let mut sl = 0.0f32;
            let mut sr_ = 0.0f32;
            for g in self.grains.iter_mut() {
                if !g.active {
                    continue;
                }
                let w = grain_window(g) * env_v;
                // read source (tone bed blended with noise bed)
                let i0 = (g.pos as usize) % GRAIN_SRC;
                let i1 = (i0 + 1) % GRAIN_SRC;
                let frac = g.pos - g.pos.floor();
                let tone_v = lerp(self.tone[i0], self.tone[i1], frac);
                let noise_v = lerp(self.noise[i0], self.noise[i1], frac);
                let v = (tone_v * (1.0 - src_mix) + noise_v * src_mix) * w;
                let (pl, pr) = pan_gains(g.pan.clamp(-1.0, 1.0));
                sl += v * pl;
                sr_ += v * pr;
                g.pos = wrap01(g.pos + g.rate);
                g.age += 1.0;
                if g.age >= g.win_len {
                    g.active = false;
                }
            }
            left[i] = sl.clamp(-6.0, 6.0);
            right[i] = sr_.clamp(-6.0, 6.0);
        }
    }
    fn params(&self) -> &'static [ParamSpec] {
        GRAN_PARAMS
    }
    fn set_param(&mut self, i: usize, value: f32) {
        if let Some(spec) = GRAN_PARAMS.get(i) {
            self.p[i] = spec.denormalize(spec.normalize(value));
        }
    }
    fn get_param(&self, i: usize) -> f32 {
        self.p.get(i).copied().unwrap_or(0.0)
    }
    fn active_voices(&self) -> usize {
        self.grains.iter().filter(|g| g.active).count()
    }
}

impl Default for GranularEngine {
    fn default() -> Self {
        Self::grainmill("GrainMill")
    }
}
