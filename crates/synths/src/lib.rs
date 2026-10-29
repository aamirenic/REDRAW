//! REDRAW synths: original DSP engines + registry of 49 instruments.
//! Instruments are tuned engine instances; capability mapping lives in PRODUCTION.md.
#![forbid(unsafe_code)]
#![allow(clippy::too_many_arguments)]

pub mod additive;
pub mod drums;
pub mod fm;
pub mod granular;
pub mod physical;
pub mod sampler_voice;
pub mod subtractive;
pub mod wavetable;

use redraw_shared::Synth;

pub const SYNTH_NAMES: &[&str] = &[
    "Aurora",
    "Hexad FM",
    "Triple Osc",
    "Prism",
    "Vowel",
    "Prism Lite",
    "Shapeshift",
    "Wildcard",
    "Forge",
    "Circuit Bass",
    "Ember FM",
    "DX Pair",
    "Orbit",
    "Orbit X",
    "Petal",
    "Sawline",
    "Zone",
    "Simplex",
    "Stinger",
    "Stinger XT",
    "MiniVee",
    "VoxForm",
    "Spectral Map",
    "Multiwave",
    "PadForge",
    "KickOne",
    "SubKick",
    "DrumCell",
    "Membrane",
    "DrumLab",
    "GrainMill",
    "ChopLab",
    "ChopLab 2",
    "SliceLab",
    "WarpTrail",
    "Sampler",
    "Soundfont Player",
    "KeyForge",
    "StringStorm",
    "PluckBass",
    "Groove Engine",
    "VocalDance",
    "AcidBox",
    "BassBox",
    "DrumBox",
    "DrumBox B",
    "PadBox",
    "PianoBox",
    "SynthBox",
];

pub fn synth_count() -> usize {
    SYNTH_NAMES.len()
}

pub fn engine_kind(name: &str) -> &'static str {
    match name {
        "Hexad FM" | "Ember FM" | "DX Pair" => "FM",
        "Prism" | "Prism Lite" | "Shapeshift" | "Vowel" | "VoxForm" | "Spectral Map"
        | "VocalDance" | "KeyForge" | "PianoBox" | "Forge" => "Additive",
        "Petal" | "StringStorm" | "PluckBass" => "Physical",
        "PadForge" | "KickOne" | "SubKick" | "DrumCell" | "Membrane" | "DrumLab" | "DrumBox"
        | "DrumBox B" => "Drum",
        "GrainMill" | "WarpTrail" => "Granular",
        "Aurora" | "Wildcard" | "PadBox" => "Wavetable",
        "Multiwave" | "ChopLab" | "ChopLab 2" | "SliceLab" | "Sampler" | "Soundfont Player" => {
            "Sampler"
        }
        _ => "Subtractive",
    }
}

pub fn create_synth(name: &str) -> Option<Box<dyn Synth>> {
    use drums::DrumEngine;
    use fm::FmEngine;
    use granular::GranularEngine;
    use physical::PluckEngine;
    use sampler_voice::SamplerVoiceEngine;
    use subtractive::SubtractiveEngine;
    use wavetable::WavetableEngine;

    // Each arm passes the exact static registry name into its constructor, so
    // synth_type() already equals the lookup key (asserted by the tests).
    let s: Box<dyn Synth> = match name {
        // ---------------------------------------------------------------- additive
        "Prism" => Box::new(additive::AdditiveEngine::prism("Prism")),
        "Prism Lite" => Box::new(additive::AdditiveEngine::prism_lite("Prism Lite")),
        "Shapeshift" => Box::new(additive::AdditiveEngine::shapeshift("Shapeshift")),
        "Forge" => Box::new(additive::AdditiveEngine::forge("Forge")),
        "Vowel" => Box::new(additive::AdditiveEngine::vowel("Vowel")),
        "VoxForm" => Box::new(additive::AdditiveEngine::vowel("VoxForm")),
        "VocalDance" => Box::new(additive::AdditiveEngine::vocal_dance("VocalDance")),
        "Spectral Map" => Box::new(additive::AdditiveEngine::spectral_map("Spectral Map")),
        "KeyForge" => Box::new(additive::AdditiveEngine::keyforge("KeyForge")),
        "PianoBox" => Box::new(additive::AdditiveEngine::keyforge("PianoBox")),
        // ---------------------------------------------------------------- FM
        "Hexad FM" => Box::new(FmEngine::hexad("Hexad FM")),
        "Ember FM" => Box::new(FmEngine::ember("Ember FM")),
        "DX Pair" => Box::new(FmEngine::dx_pair("DX Pair")),
        // ---------------------------------------------------------------- physical
        "Petal" => Box::new(PluckEngine::petal("Petal")),
        "StringStorm" => Box::new(PluckEngine::string_storm("StringStorm")),
        "PluckBass" => Box::new(PluckEngine::pluck_bass("PluckBass")),
        // ---------------------------------------------------------------- drums
        "PadForge" => Box::new(DrumEngine::padforge("PadForge")),
        "KickOne" => Box::new(DrumEngine::kick_one("KickOne")),
        "SubKick" => Box::new(DrumEngine::subkick("SubKick")),
        "DrumCell" => Box::new(DrumEngine::drum_cell("DrumCell")),
        "Membrane" => Box::new(DrumEngine::membrane("Membrane")),
        "DrumLab" => Box::new(DrumEngine::drum_lab("DrumLab")),
        "DrumBox" => Box::new(DrumEngine::kit("DrumBox", false)),
        "DrumBox B" => Box::new(DrumEngine::kit("DrumBox B", true)),
        // ---------------------------------------------------------------- granular
        "GrainMill" => Box::new(GranularEngine::grainmill("GrainMill")),
        "WarpTrail" => Box::new(GranularEngine::warptrail("WarpTrail")),
        // ---------------------------------------------------------------- wavetable
        "Aurora" => Box::new(WavetableEngine::aurora("Aurora")),
        "Wildcard" => Box::new(WavetableEngine::wildcard("Wildcard")),
        "PadBox" => Box::new(WavetableEngine::padbox("PadBox")),
        // ---------------------------------------------------------------- sampler
        "Multiwave" => Box::new(SamplerVoiceEngine::multiwave("Multiwave")),
        "ChopLab" => Box::new(SamplerVoiceEngine::slice16("ChopLab")),
        "ChopLab 2" => Box::new(SamplerVoiceEngine::slice32("ChopLab 2")),
        "SliceLab" => Box::new(SamplerVoiceEngine::slice16("SliceLab")),
        "Sampler" => Box::new(SamplerVoiceEngine::basic("Sampler")),
        "Soundfont Player" => Box::new(SamplerVoiceEngine::basic("Soundfont Player")),
        // ---------------------------------------------------------------- subtractive
        "Triple Osc" => Box::new(SubtractiveEngine::triple_osc("Triple Osc")),
        "Circuit Bass" => Box::new(SubtractiveEngine::circuit_bass("Circuit Bass")),
        "Orbit" => Box::new(SubtractiveEngine::orbit("Orbit")),
        "Orbit X" => Box::new(SubtractiveEngine::orbit_x("Orbit X")),
        "Sawline" => Box::new(SubtractiveEngine::sawline("Sawline")),
        "Zone" => Box::new(SubtractiveEngine::zone("Zone")),
        "Simplex" => Box::new(SubtractiveEngine::simplex("Simplex")),
        "Stinger" => Box::new(SubtractiveEngine::stinger("Stinger")),
        "Stinger XT" => Box::new(SubtractiveEngine::stinger_xt("Stinger XT")),
        "MiniVee" => Box::new(SubtractiveEngine::minivee("MiniVee")),
        "Groove Engine" => Box::new(SubtractiveEngine::groove("Groove Engine")),
        "AcidBox" => Box::new(SubtractiveEngine::acidbox("AcidBox")),
        "BassBox" => Box::new(SubtractiveEngine::bassbox("BassBox")),
        "SynthBox" => Box::new(SubtractiveEngine::synthbox("SynthBox")),
        _ => return None,
    };
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use redraw_shared::Synth;

    #[test]
    fn registry_uses_original_names() {
        const LEGACY: &[&str] = &[
            "Toxic",
            "Fruity",
            "Sytrus",
            "Harmor",
            "Sakura",
            "Sawer",
            "PoiZone",
            "SimSynth",
            "Wasp",
            "MiniSynth",
            "DirectWave",
            "Slicex",
            "Drumaxx",
            "Granulizer",
            "Slicer",
            "SoundFont",
            "FL Keys",
            "FL Slayer",
            "BooBass",
            "Groove Machine",
            "FLEX",
            "Kepler",
            "Transistor Bass",
            "Speech Synthesizer",
            "BeepMap",
            "FPC",
            "Fruit Kick",
            "BassDrum",
            "Drumpad",
            "DrumSynth",
            "Wave Traveller",
            "Channel Sampler",
        ];
        for name in SYNTH_NAMES {
            for legacy in LEGACY {
                assert!(
                    !name.contains(legacy),
                    "{name} still contains legacy name {legacy}"
                );
            }
        }
    }

    #[test]
    fn registry_complete() {
        assert_eq!(synth_count(), 49);
        for name in SYNTH_NAMES {
            let s = create_synth(name)
                .unwrap_or_else(|| panic!("missing synth {name}"));
            assert_eq!(s.synth_type(), *name);
        }
        assert!(create_synth("Not A Plugin").is_none());
    }

    fn render_burst(s: &mut dyn Synth) -> f32 {
        s.note_on(60, 0.9);
        let mut l = vec![0.0f32; 4800];
        let mut r = vec![0.0f32; 4800];
        s.render(&mut l, &mut r, 48_000.0);
        l.iter().map(|x| x.abs()).fold(0.0f32, f32::max)
    }

    #[test]
    fn every_synth_renders_and_survives_param_sweep() {
        for name in SYNTH_NAMES {
            let mut s = create_synth(name).unwrap();
            s.set_sample_rate(48_000.0);
            // parameter sweep must not panic/NaN
            for p in 0..s.get_param_count() {
                s.set_param(p, 2000.0);
                s.set_param(p, s.get_param(p));
            }
            s.reset();
            let peak = render_burst(s.as_mut());
            assert!(
                peak.is_finite() && peak > 1e-4 && peak < 32.0,
                "{name} peak={peak}"
            );
            // note off releases
            s.note_off(60);
            let mut l = vec![0.0f32; 4800];
            let mut r = vec![0.0f32; 4800];
            s.render(&mut l, &mut r, 48_000.0);
            assert!(l.iter().all(|x| x.is_finite()));
            s.all_notes_off();
        }
    }

    #[test]
    fn polyphony_and_retrig() {
        for name in SYNTH_NAMES {
            let mut s = create_synth(name).unwrap();
            s.set_sample_rate(48_000.0);
            for k in 0..80u8 {
                s.note_on(24 + k % 60, 1.0);
                let _ = k;
            }
            let mut l = vec![0.0f32; 512];
            let mut r = vec![0.0f32; 512];
            s.render(&mut l, &mut r, 48_000.0);
            assert!(l.iter().all(|x| x.is_finite()), "{name}");
            // same note retrig x3
            for _ in 0..3 {
                s.note_on(60, 0.5);
                s.note_off(60);
            }
            s.render(&mut l, &mut r, 48_000.0);
            assert!(l.iter().all(|x| x.is_finite()), "{name} retrig");
        }
    }
}
