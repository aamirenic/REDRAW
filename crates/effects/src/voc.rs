//! Vocoder effects. Both units share one log-spaced band bank; the 32-band
//! model simply halves the spacing. The modulator is the input signal and the
//! carrier is an internal detuned saw pair plus a little dry bleed (D-016
//! stand-in until sidechain routing exists).
#![forbid(unsafe_code)]

use crate::common::{db_to_gain, impl_effect, EffectBase};
use redraw_dsp::{wrap01, Rng, Svf, SvfMode};
use redraw_shared::{ParamCurve, ParamSpec};

pub static VOCODER_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Attack", 1.0, 100.0, 10.0, "ms").curve(ParamCurve::Exp),
    ParamSpec::new("Release", 10.0, 500.0, 90.0, "ms").curve(ParamCurve::Exp),
    ParamSpec::new("Unvoiced", 0.0, 1.0, 0.15, ""),
    ParamSpec::new("Formant", 0.5, 1.5, 1.0, ""),
    ParamSpec::new("Carrier", 50.0, 500.0, 110.0, "Hz").curve(ParamCurve::Exp),
    ParamSpec::new("Mix", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Log-spaced band vocoder, `N` bands from 100 Hz to 8 kHz.
pub struct VocBank<const N: usize> {
    mods: [Svf; N],
    env: [f32; N],
    centers: [f32; N],
    qs: [f32; N],
    phase_a: f32,
    phase_b: f32,
    rng: Rng,
}

impl<const N: usize> VocBank<N> {
    fn new() -> Self {
        let mut centers = [0.0f32; N];
        let mut qs = [0.0f32; N];
        let lo = 100.0f32;
        let hi = 8000.0f32;
        let ratio = (hi / lo).powf(1.0 / N as f32);
        for k in 0..N {
            let c = lo * ratio.powf(k as f32 + 0.5);
            centers[k] = c;
            let bw = c * (ratio - 1.0);
            qs[k] = (c / bw).max(0.5);
        }
        Self {
            mods: std::array::from_fn(|_| Svf::new()),
            env: [0.0; N],
            centers,
            qs,
            phase_a: 0.0,
            phase_b: 0.0,
            rng: Rng::new(0x81),
        }
    }
    fn reset(&mut self) {
        for f in self.mods.iter_mut() {
            f.reset();
        }
        self.env = [0.0; N];
        self.phase_a = 0.0;
        self.phase_b = 0.0;
        self.rng = Rng::new(0x81);
    }
    fn process(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, p: &[f32]) {
        let atk_ms = p[0].clamp(1.0, 100.0);
        let rel_ms = p[1].clamp(10.0, 500.0);
        let unvoiced = p[2].clamp(0.0, 1.0);
        let formant = p[3].clamp(0.5, 1.5);
        let carrier_hz = p[4].clamp(50.0, 500.0);
        let mix = p[5].clamp(0.0, 1.0);
        let out = db_to_gain(p[6]);
        for k in 0..N {
            let c = (self.centers[k] * formant).min(sr * 0.45);
            self.mods[k].set(c, self.qs[k], sr);
        }
        let atk = (-1.0 / (atk_ms * 0.001 * sr).max(1.0)).exp();
        let rel = (-1.0 / (rel_ms * 0.001 * sr).max(1.0)).exp();
        let band_gain = 1.6 / (N as f32).sqrt();
        let n = left.len().min(right.len());
        for i in 0..n {
            let dl = left[i];
            let dr = right[i];
            let mono = (dl + dr) * 0.5;
            self.phase_a = wrap01(self.phase_a + carrier_hz / sr);
            self.phase_b = wrap01(self.phase_b + carrier_hz * 2.0 / sr);
            let carrier = (2.0 * self.phase_a - 1.0) * 0.8
                + (2.0 * self.phase_b - 1.0) * 0.2
                + mono * 0.1;
            let mut wet = 0.0f32;
            for k in 0..N {
                let b = self.mods[k].tick_mode(mono, SvfMode::Band);
                let x = b.abs();
                let coef = if x > self.env[k] { atk } else { rel };
                self.env[k] = x + (self.env[k] - x) * coef;
                wet += carrier * self.env[k];
            }
            wet *= band_gain;
            let top = (self.env[N - 4] + self.env[N - 3] + self.env[N - 2] + self.env[N - 1])
                * 0.25;
            let noise = self.rng.next_f32() * 2.0 - 1.0;
            wet += noise * unvoiced * top * 0.7;
            left[i] = (dl * (1.0 - mix) + wet * mix) * out;
            right[i] = (dr * (1.0 - mix) + wet * mix) * out;
        }
    }
}

macro_rules! vocoder_fx {
    ($ty:ident, $bands:expr) => {
        pub struct $ty {
            pub name: &'static str,
            pub p: [f32; VOCODER_PARAMS.len()],
            bank: VocBank<$bands>,
            bypass: bool,
        }
        impl $ty {
            pub fn new(name: &'static str) -> Self {
                Self {
                    name,
                    p: std::array::from_fn(|i| VOCODER_PARAMS[i].default),
                    bank: VocBank::<$bands>::new(),
                    bypass: false,
                }
            }
        }
        impl EffectBase for $ty {
            const SPEC: &'static [ParamSpec] = VOCODER_PARAMS;
            fn type_name(&self) -> &'static str {
                self.name
            }
            fn p_read(&self) -> &[f32] {
                &self.p
            }
            fn p_write(&mut self, index: usize, value: f32) {
                self.p[index] = value;
            }
            fn process_fx(
                &mut self,
                left: &mut [f32],
                right: &mut [f32],
                sr: f32,
                _tempo: f32,
            ) {
                let p = self.p;
                self.bank.process(left, right, sr, &p);
            }
            fn reset_fx(&mut self) {
                self.bank.reset();
            }
            fn bypass_state(&self) -> bool {
                self.bypass
            }
            fn set_bypass_state(&mut self, bypass: bool) {
                self.bypass = bypass;
            }
        }
        impl_effect!($ty);
    };
}

vocoder_fx!(Vocode16, 16);
vocoder_fx!(Vocode32, 32);
