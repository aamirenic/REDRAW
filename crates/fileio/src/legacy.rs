//! Compatibility for projects written before the Phase 16 naming pass.
//!
//! The project schema stores plugin type names as strings. Phase 16 replaced
//! the 73 legacy effect labels and the `Toxic FM` instrument label with the
//! original REDRAW names documented in `PRODUCTION.md`. Projects saved before
//! that change still contain the old strings, so they are resolved here once
//! during load, before the engine asks the plugin factory to instantiate
//! anything.
//!
//! This is intentionally an exact-match alias table, not a schema migration:
//! the on-disk format, version number and field layout are unchanged. Unknown
//! names are left alone so the existing factory fallback still applies.

use redraw_shared::{FxSlotState, StudioProject};

/// Instrument labels renamed in Phase 16.
pub const LEGACY_SYNTH_ALIASES: &[(&str, &str)] = &[("Toxic FM", "Ember FM")];

/// Effect labels renamed in Phase 16. `Tuner` kept its name and is absent.
pub const LEGACY_EFFECT_ALIASES: &[(&str, &str)] = &[
    ("Gross Beat", "Time Loom"),
    ("Edison", "Wave Bench"),
    ("Newtime", "Loop Chisel"),
    ("Newtone", "Tone Trace"),
    ("Pitcher", "Pitch Anchor"),
    ("Pitch Shifter", "Pitch Drift"),
    ("Fruity Vocoder", "Vocal Grid"),
    ("Vocodex", "Vocal Prism"),
    ("Fruity Love Philter", "Filter Loom"),
    ("Fruity Parametric EQ 2", "Curve EQ"),
    ("Fruity 7 Band EQ", "Graphic Seven"),
    ("EQUO", "Morph EQ"),
    ("Fruity Convolver", "Impulse Room"),
    ("Fruity Reeverb", "Silver Room"),
    ("Fruity Reeverb 2", "Silver Room II"),
    ("LuxeVerb", "Velvet Hall"),
    ("Fruity Delay", "Echo One"),
    ("Fruity Delay 2", "Echo Two"),
    ("Fruity Delay 3", "Echo Three"),
    ("Fruity Delay Bank", "Echo Bank"),
    ("Multiband Delay", "Band Echo"),
    ("Fruity Compressor", "Glue Comp"),
    ("Fruity Multiband Compressor", "Band Comp"),
    ("Fruity Limiter", "Peak Guard"),
    ("Maximus", "Triad Guard"),
    ("Emphasis", "Focus Comp"),
    ("Emphsizer", "Spectral Tilt"),
    ("Fruity Soft Clipper", "Soft Clip"),
    ("Transient Processor", "Attack Shape"),
    ("Transmitter", "Bit Stream"),
    ("Soundgoodizer", "Macro Glow"),
    ("Fruity Blood Overdrive", "Redline Drive"),
    ("Fruity Fast Dist", "Quick Dist"),
    ("Distructor", "Dist Stack"),
    ("Hardcore", "Pedal Board"),
    ("Fruity Squeeze", "Crush Box"),
    ("Fruity WaveShaper", "Curve Shaper"),
    ("Fruity Chorus", "Chorus Three"),
    ("Hyper Chorus", "Chorus Eight"),
    ("Vintage Chorus", "Analog Chorus"),
    ("Fruity Phaser", "Phase Six"),
    ("Vintage Phaser", "Analog Phaser"),
    ("Fruity Flanger", "Flange TZ"),
    ("Fruity Flangus", "Flange Chorus"),
    ("Fruity Pan-O-Matic", "Auto Pan"),
    ("Fruity Stereo Enhancer", "Stereo Width"),
    ("Fruity Stereo Shaper", "Stereo Matrix"),
    ("Spreader", "Band Spreader"),
    ("Fruity Center", "Center Extract"),
    ("Fruity Phase Inverter", "Polarity Flip"),
    ("Fruity Bass Boost", "Bass Lift"),
    ("Low Lifter", "Sub Lift"),
    ("Frequency Shifter", "Freq Shift"),
    ("Frequency Splitter", "Band Split"),
    ("Fruity Scratcher", "Vinyl Scratch"),
    ("Transporter", "Tape Stop"),
    ("Wave Candy", "Signal Scope"),
    ("Fruity Spectroman", "Spectrum 31"),
    ("Fruity dB Meter", "Level Meter"),
    ("Fruity Big Clock", "Session Clock"),
    ("Fruity HTML Notebook", "Rich Notes"),
    ("Fruity Notebook", "Plain Notes"),
    ("Fruity Formula Controller", "Formula Mod"),
    ("Fruity Peak Controller", "Peak Follower"),
    ("Fruity X-Y Controller", "XY Pad"),
    ("Effector", "Stage FX"),
    ("ZGameEditor", "Visual Host"),
    ("Fruity Balance", "Balance Trim"),
    ("Fruity Mute 2", "Soft Mute"),
    ("Fruity Send", "Send Tap"),
    ("Fruity Fast LP", "Fast Lowpass"),
    ("Fruity Filter", "Multi Filter"),
    ("Fruity Free Filter", "Morph Filter"),
];

fn lookup(table: &'static [(&'static str, &'static str)], stored: &str) -> Option<&'static str> {
    table
        .iter()
        .find(|(old, _)| *old == stored)
        .map(|(_, new)| *new)
}

/// Resolve a stored type name to its current name. Unknown names are returned
/// unchanged.
pub fn resolve_legacy_type_name(stored: &str) -> &str {
    lookup(LEGACY_SYNTH_ALIASES, stored)
        .or_else(|| lookup(LEGACY_EFFECT_ALIASES, stored))
        .unwrap_or(stored)
}

fn resolve_in_place(name: &mut String) {
    if let Some(new) = lookup(LEGACY_SYNTH_ALIASES, name.as_str())
        .or_else(|| lookup(LEGACY_EFFECT_ALIASES, name.as_str()))
    {
        if new != name.as_str() {
            name.clear();
            name.push_str(new);
        }
    }
}

fn resolve_slots(slots: &mut [Option<FxSlotState>]) {
    for slot in slots.iter_mut().flatten() {
        resolve_in_place(&mut slot.effect_type);
    }
}

/// Rewrite every stored instrument and effect type name in a project.
///
/// Covers channel instruments plus the insert chains on channel strips, the
/// master strip and all four send/return buses.
pub fn migrate_legacy_type_names(project: &mut StudioProject) {
    for channel in &mut project.channels {
        resolve_in_place(&mut channel.synth_type);
    }
    resolve_slots(&mut project.mixer.master.inserts);
    for strip in &mut project.mixer.strips {
        resolve_slots(&mut strip.inserts);
    }
    for bus in &mut project.mixer.bus_inserts {
        resolve_slots(bus);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redraw_shared::{ChannelInfo, FxSlotState};

    fn slot(effect_type: &str) -> Option<FxSlotState> {
        Some(FxSlotState {
            effect_type: effect_type.into(),
            params: Vec::new(),
            bypass: false,
        })
    }

    #[test]
    fn alias_tables_are_complete_and_unique() {
        assert_eq!(LEGACY_SYNTH_ALIASES.len(), 1);
        assert_eq!(LEGACY_EFFECT_ALIASES.len(), 73);

        let mut olds = std::collections::HashSet::new();
        for (old, new) in LEGACY_SYNTH_ALIASES
            .iter()
            .chain(LEGACY_EFFECT_ALIASES.iter())
        {
            assert!(!old.is_empty() && !new.is_empty());
            assert_ne!(old, new);
            assert!(olds.insert(*old), "duplicate legacy name: {old}");
        }
    }

    #[test]
    fn every_alias_resolves_to_its_current_name() {
        for (old, new) in LEGACY_SYNTH_ALIASES {
            assert_eq!(resolve_legacy_type_name(old), *new);
        }
        for (old, new) in LEGACY_EFFECT_ALIASES {
            assert_eq!(resolve_legacy_type_name(old), *new);
        }
        assert_eq!(resolve_legacy_type_name("Tuner"), "Tuner");
        assert_eq!(resolve_legacy_type_name("Unknown Plugin"), "Unknown Plugin");
    }

    #[test]
    fn migration_covers_channels_strips_master_and_buses() {
        let mut project = StudioProject::default();
        project.channels.clear();
        project
            .channels
            .push(ChannelInfo::new(1, "Old Instrument", "Toxic FM", [1, 2, 3]));
        project.mixer.strips[0].inserts[0] = slot("Gross Beat");
        project.mixer.master.inserts[1] = slot("Fruity Compressor");
        project.mixer.bus_inserts[2][3] = slot("Edison");

        migrate_legacy_type_names(&mut project);

        assert_eq!(project.channels[0].synth_type, "Ember FM");
        assert_eq!(
            project.mixer.strips[0].inserts[0]
                .as_ref()
                .unwrap()
                .effect_type,
            "Time Loom"
        );
        assert_eq!(
            project.mixer.master.inserts[1]
                .as_ref()
                .unwrap()
                .effect_type,
            "Glue Comp"
        );
        assert_eq!(
            project.mixer.bus_inserts[2][3]
                .as_ref()
                .unwrap()
                .effect_type,
            "Wave Bench"
        );
    }

    #[test]
    fn migration_is_idempotent_and_preserves_unknown_names() {
        let mut project = StudioProject::default();
        project.channels.clear();
        project
            .channels
            .push(ChannelInfo::new(1, "A", "Toxic FM", [1, 2, 3]));
        project
            .channels
            .push(ChannelInfo::new(2, "B", "Unknown Synth", [1, 2, 3]));
        project.mixer.strips[0].inserts[0] = slot("Gross Beat");
        project.mixer.strips[0].inserts[1] = slot("Unknown Effect");

        migrate_legacy_type_names(&mut project);
        let first = bincode::serialize(&project).unwrap();
        migrate_legacy_type_names(&mut project);
        let second = bincode::serialize(&project).unwrap();

        assert_eq!(first, second);
        assert_eq!(project.channels[0].synth_type, "Ember FM");
        assert_eq!(project.channels[1].synth_type, "Unknown Synth");
        assert_eq!(
            project.mixer.strips[0].inserts[0]
                .as_ref()
                .unwrap()
                .effect_type,
            "Time Loom"
        );
        assert_eq!(
            project.mixer.strips[0].inserts[1]
                .as_ref()
                .unwrap()
                .effect_type,
            "Unknown Effect"
        );
    }
}
