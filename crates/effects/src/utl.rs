//! Utility effects: center removal, polarity, clock/notes displays, modulation
//! controllers, meters-adjacent tools, trims, ramped mute and an explicit send
//! tap. The VST wrapper stub is constructed by the Phase 8 host glue and is not
//! part of the built-in registry.
#![forbid(unsafe_code)]

use crate::common::{db_to_gain, impl_effect, EffectBase};
use redraw_shared::ParamSpec;

// ================================================================== CenterCut

pub static CENTERCUT_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Amount", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Mid-channel cancellation for vocal removal; `Amount` 0 is exact bypass.
pub struct CenterCut {
    pub name: &'static str,
    pub p: [f32; CENTERCUT_PARAMS.len()],
    bypass: bool,
}

impl CenterCut {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| CENTERCUT_PARAMS[i].default),
            bypass: false,
        }
    }
}

impl EffectBase for CenterCut {
    const SPEC: &'static [ParamSpec] = CENTERCUT_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], _sr: f32, _tempo: f32) {
        let amt = self.p[0].clamp(0.0, 1.0);
        let out = db_to_gain(self.p[1]);
        let n = left.len().min(right.len());
        for i in 0..n {
            let mid = (left[i] + right[i]) * 0.5;
            left[i] = (left[i] - mid * amt) * out;
            right[i] = (right[i] - mid * amt) * out;
        }
    }
    fn reset_fx(&mut self) {}
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// =============================================================== PolarityFlip

pub static POLARITY_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Invert L", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Invert R", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

pub struct PolarityFlip {
    pub name: &'static str,
    pub p: [f32; POLARITY_PARAMS.len()],
    bypass: bool,
}

impl PolarityFlip {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| POLARITY_PARAMS[i].default),
            bypass: false,
        }
    }
}

impl EffectBase for PolarityFlip {
    const SPEC: &'static [ParamSpec] = POLARITY_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], _sr: f32, _tempo: f32) {
        let gl = if self.p[0] > 0.5 { -1.0 } else { 1.0 };
        let gr = if self.p[1] > 0.5 { -1.0 } else { 1.0 };
        let out = db_to_gain(self.p[2]);
        for i in 0..left.len().min(right.len()) {
            left[i] = left[i] * gl * out;
            right[i] = right[i] * gr * out;
        }
    }
    fn reset_fx(&mut self) {}
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// =================================================================== BigClock

pub static CLOCK_PARAMS: &[ParamSpec] = &[ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB")];

/// Transport clock display: counts processed samples and derives 4/4 bar,
/// beat and 16th-step plus milliseconds at the current tempo.
pub struct BigClock {
    pub name: &'static str,
    pub p: [f32; CLOCK_PARAMS.len()],
    pub samples: u64,
    pub bar: u32,
    pub beat: u32,
    pub step: u32,
    pub ms: u32,
    bypass: bool,
}

impl BigClock {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| CLOCK_PARAMS[i].default),
            samples: 0,
            bar: 1,
            beat: 1,
            step: 1,
            ms: 0,
            bypass: false,
        }
    }
}

impl EffectBase for BigClock {
    const SPEC: &'static [ParamSpec] = CLOCK_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], sr: f32, tempo: f32) {
        let n = left.len().min(right.len());
        self.samples += n as u64;
        let secs = self.samples as f64 / sr.max(1.0) as f64;
        let beats = secs * tempo.clamp(20.0, 300.0) as f64 / 60.0;
        self.bar = (beats / 4.0).floor() as u32 + 1;
        self.beat = (beats % 4.0).floor() as u32 + 1;
        self.step = ((beats % 1.0) * 4.0).floor() as u32 + 1;
        self.ms = ((secs % 1.0) * 1000.0) as u32;
        let out = db_to_gain(self.p[0]);
        for i in 0..n {
            left[i] *= out;
            right[i] *= out;
        }
    }
    fn reset_fx(&mut self) {
        self.samples = 0;
        self.bar = 1;
        self.beat = 1;
        self.step = 1;
        self.ms = 0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ============================================================ Notebook effects

pub static NOTES_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Page", 0.0, 2.0, 0.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

const RICH_PAGES: [&str; 3] = [
    "Rich notebook\n\nPage 1: arrangement notes.\nThe formatted editor arrives with the UI pass.",
    "Rich notebook\n\nPage 2: mix notes.\nUse the channel rack to keep levels tidy.",
    "Rich notebook\n\nPage 3: session log.\nAutosave keeps snapshots beside the project.",
];

const PLAIN_PAGES: [&str; 3] = [
    "Notes\n\nPage 1.\nPlain text scratchpad.",
    "Notes\n\nPage 2.\nTrack ideas and take numbers here.",
    "Notes\n\nPage 3.\nRemember to export a WAV reference.",
];

macro_rules! notebook_fx {
    ($ty:ident, $pages:expr) => {
        pub struct $ty {
            pub name: &'static str,
            pub p: [f32; NOTES_PARAMS.len()],
            pages: [&'static str; 3],
            bypass: bool,
        }
        impl $ty {
            pub fn new(name: &'static str) -> Self {
                Self {
                    name,
                    p: std::array::from_fn(|i| NOTES_PARAMS[i].default),
                    pages: $pages,
                    bypass: false,
                }
            }
            /// Text for the currently selected page.
            pub fn page_text(&self) -> &'static str {
                let idx = (self.p[0].clamp(0.0, 2.0) as usize).min(2);
                self.pages[idx]
            }
        }
        impl EffectBase for $ty {
            const SPEC: &'static [ParamSpec] = NOTES_PARAMS;
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
                _sr: f32,
                _tempo: f32,
            ) {
                let out = db_to_gain(self.p[1]);
                for i in 0..left.len().min(right.len()) {
                    left[i] *= out;
                    right[i] *= out;
                }
            }
            fn reset_fx(&mut self) {}
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

notebook_fx!(NotesRich, RICH_PAGES);
notebook_fx!(NotesPlain, PLAIN_PAGES);

// ================================================================= FormulaMod

pub static FORMULA_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Formula", 0.0, 5.0, 0.0, ""),
    ParamSpec::new("A", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("B", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Modulation controller over a fixed table of six safe expressions. The full
/// expression parser is deferred (D-016); this table covers the common shapes
/// without eval or allocation.
pub struct FormulaMod {
    pub name: &'static str,
    pub p: [f32; FORMULA_PARAMS.len()],
    bypass: bool,
}

impl FormulaMod {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| FORMULA_PARAMS[i].default),
            bypass: false,
        }
    }
    fn eval(&self) -> f32 {
        let a = self.p[1].clamp(0.0, 1.0);
        let b = self.p[2].clamp(0.0, 1.0);
        let v = match (self.p[0].round() as u32) % 6 {
            0 => a,
            1 => a * b,
            2 => 1.0 - a,
            3 => (a + b) * 0.5,
            4 => (a - b).abs(),
            _ => (a + b).min(1.0),
        };
        v.clamp(0.0, 1.0)
    }
}

impl EffectBase for FormulaMod {
    const SPEC: &'static [ParamSpec] = FORMULA_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], _sr: f32, _tempo: f32) {
        let out = db_to_gain(self.p[3]);
        for i in 0..left.len().min(right.len()) {
            left[i] *= out;
            right[i] *= out;
        }
    }
    fn reset_fx(&mut self) {}
    fn mod_value(&self) -> f32 {
        self.eval()
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ====================================================================== XyMod

pub static XY_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("X", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Y", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Two-axis controller: X drives the modulation output, Y is exposed for the
/// UI through `y()`.
pub struct XyMod {
    pub name: &'static str,
    pub p: [f32; XY_PARAMS.len()],
    bypass: bool,
}

impl XyMod {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| XY_PARAMS[i].default),
            bypass: false,
        }
    }
    pub fn y(&self) -> f32 {
        self.p[1].clamp(0.0, 1.0)
    }
}

impl EffectBase for XyMod {
    const SPEC: &'static [ParamSpec] = XY_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], _sr: f32, _tempo: f32) {
        let out = db_to_gain(self.p[2]);
        for i in 0..left.len().min(right.len()) {
            left[i] *= out;
            right[i] *= out;
        }
    }
    fn reset_fx(&mut self) {}
    fn mod_value(&self) -> f32 {
        self.p[0].clamp(0.0, 1.0)
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================== VisualHost

pub static VISUAL_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Intensity", 0.0, 1.0, 0.5, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Visualizer host placeholder: tracks frames processed and passes audio
/// through. The Phase 8 host attaches the renderer.
pub struct VisualHost {
    pub name: &'static str,
    pub p: [f32; VISUAL_PARAMS.len()],
    pub frame: u64,
    bypass: bool,
}

impl VisualHost {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| VISUAL_PARAMS[i].default),
            frame: 0,
            bypass: false,
        }
    }
}

impl EffectBase for VisualHost {
    const SPEC: &'static [ParamSpec] = VISUAL_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], _sr: f32, _tempo: f32) {
        self.frame += 1;
        let out = db_to_gain(self.p[1]);
        for i in 0..left.len().min(right.len()) {
            left[i] *= out;
            right[i] *= out;
        }
    }
    fn reset_fx(&mut self) {
        self.frame = 0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================ BalanceTrim

pub static BALANCE_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Gain", -24.0, 24.0, 0.0, "dB"),
    ParamSpec::new("Pan", -1.0, 1.0, 0.0, ""),
];

/// Balance-law gain/pan trim: pan 0 is unity on both channels.
pub struct BalanceTrim {
    pub name: &'static str,
    pub p: [f32; BALANCE_PARAMS.len()],
    bypass: bool,
}

impl BalanceTrim {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| BALANCE_PARAMS[i].default),
            bypass: false,
        }
    }
}

impl EffectBase for BalanceTrim {
    const SPEC: &'static [ParamSpec] = BALANCE_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], _sr: f32, _tempo: f32) {
        let g = db_to_gain(self.p[0].clamp(-24.0, 24.0));
        let pan = self.p[1].clamp(-1.0, 1.0);
        let gl = g * if pan > 0.0 { 1.0 - pan } else { 1.0 };
        let gr = g * if pan < 0.0 { 1.0 + pan } else { 1.0 };
        for i in 0..left.len().min(right.len()) {
            left[i] *= gl;
            right[i] *= gr;
        }
    }
    fn reset_fx(&mut self) {}
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// =================================================================== SoftMute

pub static MUTE_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Mute", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Ramp", 1.0, 1000.0, 20.0, "ms"),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Click-free mute with a linear ramp; the gain moves toward the target by one
/// ramp step per sample.
pub struct SoftMute {
    pub name: &'static str,
    pub p: [f32; MUTE_PARAMS.len()],
    gain: f32,
    bypass: bool,
}

impl SoftMute {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| MUTE_PARAMS[i].default),
            gain: 1.0,
            bypass: false,
        }
    }
}

impl EffectBase for SoftMute {
    const SPEC: &'static [ParamSpec] = MUTE_PARAMS;
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
        let target = if self.p[0] > 0.5 { 0.0 } else { 1.0 };
        let step = 1.0 / (self.p[1].clamp(1.0, 1000.0) * 0.001 * sr).max(1.0);
        let out = db_to_gain(self.p[2]);
        let n = left.len().min(right.len());
        for i in 0..n {
            if self.gain < target {
                self.gain = (self.gain + step).min(target);
            } else if self.gain > target {
                self.gain = (self.gain - step).max(target);
            }
            let g = self.gain * out;
            left[i] *= g;
            right[i] *= g;
        }
    }
    fn reset_fx(&mut self) {
        self.gain = 1.0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ==================================================================== SendTap

pub static SEND_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Send", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Mute", 0.0, 1.0, 0.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Explicit send tap: scales the signal by the send level with a short
/// de-click ramp so the tap can be muted without steps.
pub struct SendTap {
    pub name: &'static str,
    pub p: [f32; SEND_PARAMS.len()],
    gain: f32,
    bypass: bool,
}

impl SendTap {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| SEND_PARAMS[i].default),
            gain: 1.0,
            bypass: false,
        }
    }
    pub fn send_level(&self) -> f32 {
        self.gain
    }
}

impl EffectBase for SendTap {
    const SPEC: &'static [ParamSpec] = SEND_PARAMS;
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
        let mute = if self.p[1] > 0.5 { 0.0 } else { 1.0 };
        let target = self.p[0].clamp(0.0, 1.0) * mute;
        let step = 1.0 / (0.005 * sr).max(1.0);
        let out = db_to_gain(self.p[2]);
        let n = left.len().min(right.len());
        for i in 0..n {
            if self.gain < target {
                self.gain = (self.gain + step).min(target);
            } else if self.gain > target {
                self.gain = (self.gain - step).max(target);
            }
            let g = self.gain * out;
            left[i] *= g;
            right[i] *= g;
        }
    }
    fn reset_fx(&mut self) {
        self.gain = 1.0;
    }
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

// ================================================================== VstWrapper

pub static WRAPPER_PARAMS: &[ParamSpec] = &[
    ParamSpec::new("Mix", 0.0, 1.0, 1.0, ""),
    ParamSpec::new("Output", -12.0, 12.0, 0.0, "dB"),
];

/// Host-glue stub for third-party plugins. The Phase 8 VST host creates this
/// type directly (it is intentionally absent from `EFFECT_NAMES`) and wires
/// the loaded instance into `process_fx`; until then it is passthrough.
pub struct VstWrapper {
    pub name: &'static str,
    pub p: [f32; WRAPPER_PARAMS.len()],
    pub loaded: bool,
    pub plugin_path: Option<String>,
    bypass: bool,
}

impl VstWrapper {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            p: std::array::from_fn(|i| WRAPPER_PARAMS[i].default),
            loaded: false,
            plugin_path: None,
            bypass: false,
        }
    }
}

impl EffectBase for VstWrapper {
    const SPEC: &'static [ParamSpec] = WRAPPER_PARAMS;
    fn type_name(&self) -> &'static str {
        self.name
    }
    fn p_read(&self) -> &[f32] {
        &self.p
    }
    fn p_write(&mut self, index: usize, value: f32) {
        self.p[index] = value;
    }
    fn process_fx(&mut self, left: &mut [f32], right: &mut [f32], _sr: f32, _tempo: f32) {
        let out = db_to_gain(self.p[1]);
        if out != 1.0 {
            for i in 0..left.len().min(right.len()) {
                left[i] *= out;
                right[i] *= out;
            }
        }
    }
    fn reset_fx(&mut self) {}
    fn bypass_state(&self) -> bool {
        self.bypass
    }
    fn set_bypass_state(&mut self, bypass: bool) {
        self.bypass = bypass;
    }
}

impl_effect!(CenterCut);
impl_effect!(PolarityFlip);
impl_effect!(BigClock);
impl_effect!(FormulaMod);
impl_effect!(XyMod);
impl_effect!(VisualHost);
impl_effect!(BalanceTrim);
impl_effect!(SoftMute);
impl_effect!(SendTap);
impl_effect!(VstWrapper);
