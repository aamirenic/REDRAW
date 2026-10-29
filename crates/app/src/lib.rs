//! REDRAW desktop application.
//!
//! The UI is a control surface over the shared project model, the realtime
//! engine and the file/export crates. All audio rendering happens in the engine
//! callback or in the offline exporter; this crate only sends commands and reads
//! atomic meters.

#![forbid(unsafe_code)]

mod app;
mod audio;
mod automation_editor;
mod browser;
mod dialogs;
mod inspector;
mod mixer;
mod ops;
mod piano;
mod playlist;
mod plugin_editor;
mod rack;
mod theme;
mod widgets;

pub use app::{
    app_recent_is_empty, default_vst_roots, load_preferences, load_recent, sanitize_file_stem,
    theme_from_key, theme_key, workspace_root, AutoDrag, BrowserState, ClipDrag, ClipDragMode,
    DialogKind, ExportState, FxStrip, History, LayoutState, LowerTab, NoteDrag, NoteDragMode,
    PianoView, PlaylistView, PluginTarget, Preferences, RedrawApp, SelectionState, StepPaint,
    Toast,
};
pub use theme::{ThemeKind, PATTERN_COLORS};

/// Application name used in the title bar and About dialog.
pub const APP_NAME: &str = redraw_shared::APP_NAME;

#[cfg(test)]
mod tests {
    #[test]
    fn legacy_type_aliases_target_live_plugins() {
        use redraw_shared::PluginFactory;

        let factory = redraw_plugins::BuiltinFactory::new();
        for &(old, new) in redraw_fileio::LEGACY_EFFECT_ALIASES {
            assert!(
                factory.create_effect(new).is_some(),
                "effect alias {old} -> {new} has no live target"
            );
            assert!(
                factory.create_synth(new).is_none(),
                "effect alias target {new} is also an instrument"
            );
        }
        for &(old, new) in redraw_fileio::LEGACY_SYNTH_ALIASES {
            assert!(
                factory.create_synth(new).is_some(),
                "instrument alias {old} -> {new} has no live target"
            );
            assert!(
                factory.create_effect(new).is_none(),
                "instrument alias target {new} is also an effect"
            );
        }
    }

    #[test]
    fn production_reference_is_shipped() {
        let doc = include_str!("../../../PRODUCTION.md");
        for section in [
            "## Capability map (1:1)",
            "## Instruments (49)",
            "## Effects (74)",
            "## Clean-room provenance",
            "## Phase 16 acceptance record",
        ] {
            assert!(doc.contains(section), "PRODUCTION.md is missing {section}");
        }
    }
}
