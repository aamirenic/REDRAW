//! Realtime audio ownership for the desktop shell.
//!
//! The app never renders samples itself. `AudioHost` owns the cpal stream and
//! the control-side `EngineHandle`; all mutation is sent through lock-free
//! command rings and the callback remains allocation-free.

use std::time::Instant;

use redraw_audio_engine::{
    build_schedule, start_audio, AudioSession, EngineGraph, EngineHandle, TransportConfig,
};
use redraw_shared::{PluginFactory, StudioProject};

/// Result of opening the default output device.
pub struct AudioHost {
    session: Option<AudioSession>,
    pub error: Option<String>,
    pub playlist_mode: bool,
    pub last_started: Option<Instant>,
    fallback_sample_rate: u32,
    fallback_block: u32,
}

impl AudioHost {
    pub fn new(
        project: &StudioProject,
        factory: &dyn PluginFactory,
        playlist_mode: bool,
    ) -> Self {
        let mut host = Self {
            session: None,
            error: None,
            playlist_mode,
            last_started: None,
            fallback_sample_rate: project.sample_rate.max(8_000),
            fallback_block: 512,
        };
        host.restart(project, factory);
        host
    }

    pub fn restart(&mut self, project: &StudioProject, factory: &dyn PluginFactory) {
        self.session = None;
        let graph = EngineGraph::from_project(project, factory, project.sample_rate as f32);
        let schedules = build_schedule(&project.patterns, project.swing);
        let cfg = transport_config(project, self.playlist_mode, false);
        match start_audio(graph, schedules, cfg) {
            Ok(mut session) => {
                session.handle.set_project(project, factory);
                session.handle.set_playlist_mode(self.playlist_mode);
                self.fallback_sample_rate = session.sample_rate;
                self.fallback_block = if session.block_size == 0 {
                    512
                } else {
                    session.block_size
                };
                self.error = None;
                self.last_started = Some(Instant::now());
                self.session = Some(session);
            }
            Err(error) => {
                self.error = Some(error);
                self.session = None;
            }
        }
    }

    pub fn is_running(&self) -> bool {
        self.session.is_some()
    }

    pub fn device_name(&self) -> &str {
        self.session
            .as_ref()
            .map(|s| s.device_name.as_str())
            .unwrap_or("No audio device")
    }

    pub fn sample_rate(&self) -> u32 {
        self.session
            .as_ref()
            .map(|s| s.sample_rate)
            .unwrap_or(self.fallback_sample_rate)
    }

    pub fn block_size(&self) -> u32 {
        self.session
            .as_ref()
            .map(|s| s.block_size)
            .filter(|v| *v > 0)
            .unwrap_or(self.fallback_block)
    }

    pub fn handle(&mut self) -> Option<&mut EngineHandle> {
        self.session.as_mut().map(|s| &mut s.handle)
    }

    pub fn set_project(&mut self, project: &StudioProject, factory: &dyn PluginFactory) {
        let playlist_mode = self.playlist_mode;
        if let Some(handle) = self.handle() {
            handle.set_project(project, factory);
            handle.set_playlist_mode(playlist_mode);
        }
    }

    pub fn set_playlist_mode(&mut self, on: bool) {
        self.playlist_mode = on;
        if let Some(handle) = self.handle() {
            handle.set_playlist_mode(on);
        }
    }

    pub fn play(&mut self) {
        if let Some(handle) = self.handle() {
            handle.play();
        }
    }

    pub fn stop(&mut self) {
        if let Some(handle) = self.handle() {
            handle.stop();
        }
    }

    pub fn toggle(&mut self) {
        if let Some(handle) = self.handle() {
            handle.toggle();
        }
    }

    pub fn play_from_start(&mut self) {
        if let Some(handle) = self.handle() {
            handle.seek(0);
            handle.play();
        }
    }

    pub fn seek(&mut self, tick: u64) {
        if let Some(handle) = self.handle() {
            handle.seek(tick);
        }
    }

    pub fn set_bpm(&mut self, bpm: f32) {
        if let Some(handle) = self.handle() {
            handle.set_bpm(bpm);
        }
    }

    pub fn set_swing(&mut self, swing: f32) {
        if let Some(handle) = self.handle() {
            handle.set_swing(swing);
        }
    }

    pub fn set_loop(&mut self, enabled: bool, start: u64, end: u64) {
        if let Some(handle) = self.handle() {
            handle.set_loop(enabled, start, end);
        }
    }

    pub fn set_pattern(&mut self, index: usize) {
        if let Some(handle) = self.handle() {
            handle.set_pattern(index);
        }
    }

    pub fn position(&self) -> f64 {
        self.session
            .as_ref()
            .map(|s| s.handle.position())
            .unwrap_or(0.0)
    }

    pub fn is_playing(&self) -> bool {
        self.session
            .as_ref()
            .map(|s| s.handle.is_playing())
            .unwrap_or(false)
    }

    pub fn cpu_permille(&self) -> u32 {
        self.session
            .as_ref()
            .map(|s| s.handle.meters().cpu_permille.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(0)
    }

    pub fn voices(&self) -> usize {
        self.session
            .as_ref()
            .map(|s| s.handle.meters().voice_count())
            .unwrap_or(0)
    }

    pub fn strip_peak(&self, strip: usize) -> f32 {
        self.session
            .as_ref()
            .map(|s| s.handle.meters().peak(strip))
            .unwrap_or(0.0)
    }

    pub fn master_peak(&self) -> (f32, f32) {
        self.session
            .as_ref()
            .map(|s| s.handle.meters().master())
            .unwrap_or((0.0, 0.0))
    }

    pub fn collect_garbage(&mut self) {
        if let Some(handle) = self.handle() {
            handle.collect_garbage();
        }
    }

    pub fn note_on(&mut self, channel: u32, key: u8, velocity: f32) {
        if let Some(handle) = self.handle() {
            handle.note_on(channel, key, velocity);
        }
    }

    pub fn note_off(&mut self, channel: u32, key: u8) {
        if let Some(handle) = self.handle() {
            handle.note_off(channel, key);
        }
    }

    pub fn all_notes_off(&mut self) {
        if let Some(handle) = self.handle() {
            handle.all_notes_off();
        }
    }

    pub fn set_synth_param(&mut self, channel: u32, param: u32, value: f32) {
        if let Some(handle) = self.handle() {
            handle.set_synth_param(channel, param, value);
        }
    }

    pub fn set_channel_gain(&mut self, channel: u32, db: f32) {
        if let Some(handle) = self.handle() {
            handle.set_channel_gain(channel, db);
        }
    }

    pub fn set_channel_pan(&mut self, channel: u32, pan: f32) {
        if let Some(handle) = self.handle() {
            handle.set_channel_pan(channel, pan);
        }
    }

    pub fn set_channel_mute(&mut self, channel: u32, mute: bool) {
        if let Some(handle) = self.handle() {
            handle.set_channel_mute(channel, mute);
        }
    }

    pub fn set_channel_solo(&mut self, channel: u32, solo: bool) {
        if let Some(handle) = self.handle() {
            handle.set_channel_solo(channel, solo);
        }
    }

    pub fn set_send(&mut self, channel: u32, send: u8, level_db: f32, pre: bool, target: u8) {
        if let Some(handle) = self.handle() {
            handle.set_send(channel, send, level_db, pre, target);
        }
    }

    pub fn set_fx_param(&mut self, channel: u32, slot: u8, param: u32, value: f32) {
        if let Some(handle) = self.handle() {
            handle.set_fx_param(channel, slot, param, value);
        }
    }

    pub fn set_fx_bypass(&mut self, channel: u32, slot: u8, bypass: bool) {
        if let Some(handle) = self.handle() {
            handle.set_fx_bypass(channel, slot, bypass);
        }
    }

    pub fn set_master_gain(&mut self, db: f32) {
        if let Some(handle) = self.handle() {
            handle.set_master_gain(db);
        }
    }
}

pub fn transport_config(project: &StudioProject, playlist_mode: bool, playing: bool) -> TransportConfig {
    TransportConfig {
        bpm: project.bpm.clamp(10.0, 999.0),
        time_sig: project.time_sig,
        swing: project.swing.clamp(0.0, 0.75),
        loop_enabled: project.playlist.loop_enabled,
        loop_start: project.playlist.loop_start,
        loop_end: project.playlist.loop_end,
        pattern_index: project.current_pattern,
        playing,
        playlist_mode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_config_clamps_and_copies_project_state() {
        let mut project = StudioProject::default();
        project.bpm = 5.0;
        project.swing = 9.0;
        project.current_pattern = 0;
        let cfg = transport_config(&project, true, false);
        assert_eq!(cfg.bpm, 10.0);
        assert_eq!(cfg.swing, 0.75);
        assert!(cfg.playlist_mode);
        assert!(!cfg.playing);
        assert_eq!(cfg.loop_start, project.playlist.loop_start);
    }
}
