//! REDRAW effect processors: 74 DSP units over shared engine cores.
//! Registry and factory live at the crate root; typed structs per module.
#![forbid(unsafe_code)]

pub use redraw_shared as shared;

pub mod anl;
pub mod common;
pub mod dist;
pub mod dly;
pub mod dynamics;
pub mod eq;
pub mod fxs;
pub mod pit;
pub mod rev;
pub mod utl;
pub mod voc;

pub const CRATE_PHASE: u8 = 6;

use redraw_shared::Effect;

/// Built-in effect names in `spec/03-effects.md` row order (rows 1..=74).
/// Row 75 (the plugin wrapper) is host glue and intentionally absent.
pub const EFFECT_NAMES: [&str; 74] = [
    // PIT
    "Time Loom",
    "Wave Bench",
    "Loop Chisel",
    "Tone Trace",
    "Pitch Anchor",
    "Pitch Drift",
    // V
    "Vocal Grid",
    "Vocal Prism",
    // MOD / EQ / REV / DLY (spec order)
    "Filter Loom",
    "Curve EQ",
    "Graphic Seven",
    "Morph EQ",
    "Impulse Room",
    "Silver Room",
    "Silver Room II",
    "Velvet Hall",
    "Echo One",
    "Echo Two",
    "Echo Three",
    "Echo Bank",
    "Band Echo",
    // DYN
    "Glue Comp",
    "Band Comp",
    "Peak Guard",
    "Triad Guard",
    "Focus Comp",
    "Spectral Tilt",
    // DST / DYN
    "Soft Clip",
    "Attack Shape",
    "Bit Stream",
    "Macro Glow",
    "Redline Drive",
    "Quick Dist",
    "Dist Stack",
    "Pedal Board",
    "Crush Box",
    "Curve Shaper",
    // MOD
    "Chorus Three",
    "Chorus Eight",
    "Analog Chorus",
    "Phase Six",
    "Analog Phaser",
    "Flange TZ",
    "Flange Chorus",
    "Auto Pan",
    "Stereo Width",
    "Stereo Matrix",
    "Band Spreader",
    // UTL / EQ / PIT
    "Center Extract",
    "Polarity Flip",
    "Bass Lift",
    "Sub Lift",
    "Freq Shift",
    "Band Split",
    "Vinyl Scratch",
    "Tape Stop",
    // ANL / UTL
    "Signal Scope",
    "Spectrum 31",
    "Level Meter",
    "Session Clock",
    "Rich Notes",
    "Plain Notes",
    "Formula Mod",
    "Peak Follower",
    "XY Pad",
    "Stage FX",
    "Visual Host",
    "Tuner",
    "Balance Trim",
    "Soft Mute",
    "Send Tap",
    "Fast Lowpass",
    "Multi Filter",
    "Morph Filter",
];

/// Instantiate a built-in effect by registry name. Returns `None` for unknown
/// names and for the plugin wrapper (created only by the host glue).
pub fn create_effect(name: &str) -> Option<Box<dyn Effect>> {
    let fx: Box<dyn Effect> = match name {
        "Time Loom" => Box::new(pit::TimeLoop::new("Time Loom")),
        "Wave Bench" => Box::new(anl::WaveDeck::new("Wave Bench")),
        "Loop Chisel" => Box::new(pit::LoopChop::new("Loop Chisel")),
        "Tone Trace" => Box::new(pit::TuneCanvas::new("Tone Trace")),
        "Pitch Anchor" => Box::new(pit::PitchCorrect::new("Pitch Anchor")),
        "Pitch Drift" => Box::new(pit::PitchShiftFx::new("Pitch Drift")),
        "Vocal Grid" => Box::new(voc::Vocode16::new("Vocal Grid")),
        "Vocal Prism" => Box::new(voc::Vocode32::new("Vocal Prism")),
        "Filter Loom" => Box::new(fxs::FilterLoom::new("Filter Loom")),
        "Curve EQ" => Box::new(eq::ParaEq7::new("Curve EQ")),
        "Graphic Seven" => Box::new(eq::Graphic7::new("Graphic Seven")),
        "Morph EQ" => Box::new(eq::MorphEq::new("Morph EQ")),
        "Impulse Room" => Box::new(rev::ConvolverFx::new("Impulse Room")),
        "Silver Room" => Box::new(rev::SilverRoom::new("Silver Room")),
        "Silver Room II" => Box::new(rev::SilverRoom2::new("Silver Room II")),
        "Velvet Hall" => Box::new(rev::VelvetHall::new("Velvet Hall")),
        "Echo One" => Box::new(dly::DelayOne::new("Echo One")),
        "Echo Two" => Box::new(dly::DelayTwo::new("Echo Two")),
        "Echo Three" => Box::new(dly::DelayThree::new("Echo Three")),
        "Echo Bank" => Box::new(dly::DelayBank8::new("Echo Bank")),
        "Band Echo" => Box::new(dly::MultiBandDelay::new("Band Echo")),
        "Glue Comp" => Box::new(dynamics::CompressorUnit::new("Glue Comp")),
        "Band Comp" => Box::new(dynamics::MultiComp::new("Band Comp")),
        "Peak Guard" => Box::new(dynamics::LimiterOne::new("Peak Guard")),
        "Triad Guard" => Box::new(dynamics::TriadGuard::new("Triad Guard")),
        "Focus Comp" => Box::new(dynamics::Emphasizer::new("Focus Comp")),
        "Spectral Tilt" => Box::new(dynamics::SpectralPush::new("Spectral Tilt")),
        "Soft Clip" => Box::new(dist::SoftClipFx::new("Soft Clip")),
        "Attack Shape" => Box::new(dynamics::TransientShaper::new("Attack Shape")),
        "Bit Stream" => Box::new(dly::BitTransmitter::new("Bit Stream")),
        "Macro Glow" => Box::new(dynamics::MacroGlow::new("Macro Glow")),
        "Redline Drive" => Box::new(dist::OverDrive::new("Redline Drive")),
        "Quick Dist" => Box::new(dist::FastDist::new("Quick Dist")),
        "Dist Stack" => Box::new(dist::DistStack6::new("Dist Stack")),
        "Pedal Board" => Box::new(dist::PedalBoard::new("Pedal Board")),
        "Crush Box" => Box::new(dist::CrusherFx::new("Crush Box")),
        "Curve Shaper" => Box::new(dist::WaveShaperFx::new("Curve Shaper")),
        "Chorus Three" => Box::new(fxs::Chorus3::new("Chorus Three")),
        "Chorus Eight" => Box::new(fxs::Chorus8::new("Chorus Eight")),
        "Analog Chorus" => Box::new(fxs::VintageChorus::new("Analog Chorus")),
        "Phase Six" => Box::new(fxs::Phaser6::new("Phase Six")),
        "Analog Phaser" => Box::new(fxs::VintagePhaser::new("Analog Phaser")),
        "Flange TZ" => Box::new(fxs::FlangerTz::new("Flange TZ")),
        "Flange Chorus" => Box::new(fxs::FlangerChorus::new("Flange Chorus")),
        "Auto Pan" => Box::new(fxs::AutoPan::new("Auto Pan")),
        "Stereo Width" => Box::new(fxs::StereoWide::new("Stereo Width")),
        "Stereo Matrix" => Box::new(fxs::StereoShaper::new("Stereo Matrix")),
        "Band Spreader" => Box::new(fxs::BandSpreader::new("Band Spreader")),
        "Center Extract" => Box::new(utl::CenterCut::new("Center Extract")),
        "Polarity Flip" => Box::new(utl::PolarityFlip::new("Polarity Flip")),
        "Bass Lift" => Box::new(eq::BassBoost::new("Bass Lift")),
        "Sub Lift" => Box::new(eq::SubLifter::new("Sub Lift")),
        "Freq Shift" => Box::new(pit::FreqShift::new("Freq Shift")),
        "Band Split" => Box::new(eq::BandSplitter::new("Band Split")),
        "Vinyl Scratch" => Box::new(pit::ScratchDeck::new("Vinyl Scratch")),
        "Tape Stop" => Box::new(pit::TapeWarp::new("Tape Stop")),
        "Signal Scope" => Box::new(anl::ScopeViewer::new("Signal Scope")),
        "Spectrum 31" => Box::new(anl::Spectrum31::new("Spectrum 31")),
        "Level Meter" => Box::new(anl::MeterDb::new("Level Meter")),
        "Session Clock" => Box::new(utl::BigClock::new("Session Clock")),
        "Rich Notes" => Box::new(utl::NotesRich::new("Rich Notes")),
        "Plain Notes" => Box::new(utl::NotesPlain::new("Plain Notes")),
        "Formula Mod" => Box::new(utl::FormulaMod::new("Formula Mod")),
        "Peak Follower" => Box::new(anl::PeakMod::new("Peak Follower")),
        "XY Pad" => Box::new(utl::XyMod::new("XY Pad")),
        "Stage FX" => Box::new(dist::StageFx::new("Stage FX")),
        "Visual Host" => Box::new(utl::VisualHost::new("Visual Host")),
        "Tuner" => Box::new(anl::Tuner::new("Tuner")),
        "Balance Trim" => Box::new(utl::BalanceTrim::new("Balance Trim")),
        "Soft Mute" => Box::new(utl::SoftMute::new("Soft Mute")),
        "Send Tap" => Box::new(utl::SendTap::new("Send Tap")),
        "Fast Lowpass" => Box::new(eq::FastLp::new("Fast Lowpass")),
        "Multi Filter" => Box::new(eq::MultiFilter::new("Multi Filter")),
        "Morph Filter" => Box::new(eq::MorphFilter::new("Morph Filter")),
        _ => return None,
    };
    Some(fx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const SR: f32 = 48_000.0;

    fn sine_block(n: usize, amp: f32) -> (Vec<f32>, Vec<f32>) {
        let mut l = Vec::with_capacity(n);
        let mut r = Vec::with_capacity(n);
        for i in 0..n {
            let s = (i as f32 * 0.13).sin() * amp;
            l.push(s);
            r.push(s * 0.7);
        }
        (l, r)
    }

    fn set_named(fx: &mut dyn Effect, name: &str, value: f32) {
        let idx = fx
            .params()
            .iter()
            .position(|s| s.name == name)
            .unwrap_or_else(|| panic!("parameter {name} not found"));
        fx.set_param(idx, value);
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0f32, |a, b| a.max(b.abs()))
    }

    #[test]
    fn registry_complete() {
        assert_eq!(EFFECT_NAMES.len(), 74);
        let unique: HashSet<&str> = EFFECT_NAMES.iter().copied().collect();
        assert_eq!(unique.len(), 74, "registry contains duplicate names");
        for name in EFFECT_NAMES {
            let fx = create_effect(name).unwrap_or_else(|| panic!("{name} is not constructible"));
            assert_eq!(fx.effect_type(), name, "registry name/type mismatch");
            assert!(!fx.params().is_empty(), "{name} has no parameters");
        }
        assert!(create_effect("Plugin Wrapper").is_none());
        assert!(create_effect("Not An Effect").is_none());
    }

    #[test]
    fn registry_uses_original_names() {
        const LEGACY: &[&str] = &[
            "Fruity",
            "FL Studio",
            "Image-Line",
            "Gross Beat",
            "Edison",
            "Newtime",
            "Newtone",
            "Pitcher",
            "Vocodex",
            "Maximus",
            "Soundgoodizer",
            "Hardcore",
            "Distructor",
            "Transporter",
            "Effector",
            "Wave Candy",
            "ZGameEditor",
            "LuxeVerb",
            "EQUO",
            "Emphsizer",
        ];
        for name in EFFECT_NAMES {
            for legacy in LEGACY {
                assert!(
                    !name.contains(legacy),
                    "{name} still contains legacy name {legacy}"
                );
            }
        }
    }

    #[test]
    fn bypass_is_exact_passthrough() {
        for name in EFFECT_NAMES {
            let mut fx = create_effect(name).unwrap();
            let (mut l, mut r) = sine_block(256, 0.5);
            let dry_l = l.clone();
            let dry_r = r.clone();
            fx.set_bypass(true);
            assert!(fx.is_bypassed());
            fx.process(&mut l, &mut r, SR, 120.0);
            assert_eq!(l, dry_l, "{name} altered left while bypassed");
            assert_eq!(r, dry_r, "{name} altered right while bypassed");
        }
    }

    #[test]
    fn every_effect_runs_and_survives_param_sweep() {
        for name in EFFECT_NAMES {
            let mut fx = create_effect(name).unwrap();
            let count = fx.params().len();
            for idx in 0..count {
                let original = fx.get_param(idx);
                fx.set_param(idx, 2000.0);
                let (mut l, mut r) = sine_block(256, 0.5);
                fx.process(&mut l, &mut r, SR, 120.0);
                for (ls, rs) in l.iter().zip(r.iter()) {
                    assert!(
                        ls.is_finite() && rs.is_finite(),
                        "{name} param {idx} produced non-finite output"
                    );
                    assert!(
                        ls.abs() < 64.0 && rs.abs() < 64.0,
                        "{name} param {idx} peak too high: {ls} / {rs}"
                    );
                }
                fx.set_param(idx, original);
            }
        }
    }

    #[test]
    fn reset_makes_deterministic() {
        for name in EFFECT_NAMES {
            let mut fx = create_effect(name).unwrap();
            let (mut l1, mut r1) = sine_block(512, 0.5);
            fx.process(&mut l1, &mut r1, SR, 120.0);
            fx.reset();
            let (mut l2, mut r2) = sine_block(512, 0.5);
            fx.process(&mut l2, &mut r2, SR, 120.0);
            assert_eq!(l1, l2, "{name} is not deterministic after reset (left)");
            assert_eq!(r1, r2, "{name} is not deterministic after reset (right)");
        }
    }

    #[test]
    fn balance_trim_applies_gain() {
        let mut fx = create_effect("Balance Trim").unwrap();
        set_named(&mut *fx, "Gain", 6.0);
        let mut l = vec![0.25f32; 256];
        let mut r = vec![0.25f32; 256];
        fx.process(&mut l, &mut r, SR, 120.0);
        let expected = 0.25 * 10f32.powf(6.0 / 20.0);
        for v in l.iter().chain(r.iter()) {
            assert!((v - expected).abs() < 2e-3, "gain trim: {v} vs {expected}");
        }
    }

    #[test]
    fn delay_one_round_trips_at_100ms() {
        let mut fx = create_effect("Echo One").unwrap();
        set_named(&mut *fx, "Time L", 100.0);
        set_named(&mut *fx, "Time R", 100.0);
        set_named(&mut *fx, "Feedback", 0.0);
        set_named(&mut *fx, "Damp", 0.0);
        set_named(&mut *fx, "Mix", 1.0);
        set_named(&mut *fx, "Output", 0.0);
        let n = 6000usize;
        let mut l = vec![0.0f32; n];
        let mut r = vec![0.0f32; n];
        l[0] = 0.5;
        r[0] = -0.5;
        fx.process(&mut l, &mut r, SR, 120.0);
        // 100 ms at 48 kHz = 4800 samples.
        assert!(l[4800].abs() > 0.45, "impulse missing at 100 ms: {}", l[4800]);
        assert!(r[4800] < -0.45, "right impulse missing at 100 ms: {}", r[4800]);
        assert!(l[4799].abs() < 0.05, "early output before tap");
        assert!(peak(&l[..4799]) < 0.05, "unexpected energy before tap");
    }

    #[test]
    fn compressor_tames_loud_signal() {
        let mut fx = create_effect("Glue Comp").unwrap();
        set_named(&mut *fx, "Threshold", -30.0);
        set_named(&mut *fx, "Ratio", 20.0);
        set_named(&mut *fx, "Attack", 0.5);
        set_named(&mut *fx, "Release", 100.0);
        set_named(&mut *fx, "Knee", 0.0);
        set_named(&mut *fx, "Makeup", 0.0);
        set_named(&mut *fx, "Mix", 1.0);
        set_named(&mut *fx, "Output", 0.0);
        let (mut l, mut r) = sine_block(9600, 1.0);
        fx.process(&mut l, &mut r, SR, 120.0);
        let tail = peak(&l[8576..]);
        assert!(tail < 0.45, "compressor left a {tail} peak");
    }

    #[test]
    fn limiter_respects_ceiling() {
        let mut fx = create_effect("Peak Guard").unwrap();
        set_named(&mut *fx, "Gain", 12.0);
        set_named(&mut *fx, "Ceiling", -6.0);
        set_named(&mut *fx, "Release", 60.0);
        set_named(&mut *fx, "Output", 0.0);
        let (mut l, mut r) = sine_block(19_200, 2.0);
        fx.process(&mut l, &mut r, SR, 120.0);
        let tail = peak(&l[18_176..]);
        assert!(tail < 0.55, "limiter exceeded ceiling: {tail}");
        assert!(tail > 0.1, "limiter muted the signal: {tail}");
    }

    #[test]
    fn reverb_tail_decays() {
        let mut fx = create_effect("Silver Room").unwrap();
        set_named(&mut *fx, "Room", 0.85);
        set_named(&mut *fx, "Damp", 0.2);
        set_named(&mut *fx, "Width", 1.0);
        set_named(&mut *fx, "Predelay", 0.0);
        set_named(&mut *fx, "Low Cut", 20.0);
        set_named(&mut *fx, "High Cut", 20_000.0);
        set_named(&mut *fx, "Mix", 1.0);
        set_named(&mut *fx, "Output", 0.0);
        let n = 96_000usize;
        let mut l = vec![0.0f32; n];
        let mut r = vec![0.0f32; n];
        l[0] = 1.0;
        r[0] = 1.0;
        fx.process(&mut l, &mut r, SR, 120.0);
        let energy = |a: usize, b: usize| -> f32 { l[a..b].iter().map(|x| x * x).sum() };
        let early = energy(9600, 14_400);
        let late = energy(72_000, 76_800);
        assert!(early > 1e-9, "no reverb tail detected");
        assert!(late < early * 0.5, "tail did not decay: {early} -> {late}");
    }

    #[test]
    fn soft_clip_bounds_output() {
        let mut fx = create_effect("Soft Clip").unwrap();
        set_named(&mut *fx, "Drive", 12.0);
        set_named(&mut *fx, "Ceiling", 0.0);
        set_named(&mut *fx, "Mix", 1.0);
        set_named(&mut *fx, "Output", 0.0);
        let (mut l, mut r) = sine_block(4800, 3.0);
        fx.process(&mut l, &mut r, SR, 120.0);
        let p = peak(&l).max(peak(&r));
        assert!(p < 1.2, "clipper exceeded bound: {p}");
    }

    #[test]
    fn peak_follower_reports_envelope() {
        let mut fx = create_effect("Peak Follower").unwrap();
        set_named(&mut *fx, "Base", 0.0);
        set_named(&mut *fx, "Gain", 1.0);
        set_named(&mut *fx, "Attack", 5.0);
        set_named(&mut *fx, "Release", 200.0);
        let mut l = vec![0.5f32; 4096];
        let mut r = vec![0.5f32; 4096];
        fx.process(&mut l, &mut r, SR, 120.0);
        let v = fx.controller_value();
        assert!(v > 0.3 && v <= 1.0, "peak envelope output: {v}");
    }
}
