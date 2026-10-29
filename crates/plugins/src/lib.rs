//! Built-in plugin registry adapter for the control-thread engine/export layers.
//!
//! The audio callback never constructs plugins. `BuiltinFactory` is the shared
//! bridge between the project model and the original REDRAW synth/effect
//! registries, so the UI, offline export and tests all instantiate the same
//! instruments and processors.

#![forbid(unsafe_code)]

use redraw_shared::{Effect, PluginFactory, Synth};
use redraw_synths::{create_synth, SYNTH_NAMES};
use redraw_effects::{create_effect, EFFECT_NAMES};

pub const CRATE_PHASE: u8 = 14;

/// Factory for the in-tree REDRAW instruments and effects.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuiltinFactory;

impl BuiltinFactory {
    pub const fn new() -> Self {
        Self
    }
}

impl PluginFactory for BuiltinFactory {
    fn create_synth(&self, type_name: &str) -> Option<Box<dyn Synth>> {
        create_synth(type_name)
    }

    fn create_effect(&self, type_name: &str) -> Option<Box<dyn Effect>> {
        create_effect(type_name)
    }

    fn synth_names(&self) -> &'static [&'static str] {
        SYNTH_NAMES
    }

    fn effect_names(&self) -> &'static [&'static str] {
        &EFFECT_NAMES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_factory_exposes_both_registries() {
        let factory = BuiltinFactory::new();
        assert_eq!(factory.synth_names().len(), 49);
        assert_eq!(factory.effect_names().len(), 74);
        assert!(factory.create_synth("Aurora").is_some());
        assert!(factory.create_effect(&factory.effect_names()[0]).is_some());
        assert!(factory.create_synth("not a synth").is_none());
        assert!(factory.create_effect("not an effect").is_none());
    }
}
