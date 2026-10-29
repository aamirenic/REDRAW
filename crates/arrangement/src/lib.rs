//! REDRAW playlist editing and arrangement playback.
//!
//! The arrangement is the absolute-tick layer above patterns: pattern clips are
//! placed on playlist tracks, optionally repeated, transposed and gain-trimmed,
//! and compiled into one sorted [`ArrangementSchedule`] for the engine. Events
//! are clipped to each clip's end so a shortened clip can never leave a hanging
//! note, and muted/soloed tracks are resolved at build time.
//!
//! Audio and MIDI clips are part of the shared model and are retained by the
//! editor, but do not produce note events here: audio playback arrives with the
//! sampler/UI phase and MIDI clips with the MIDI import phase (D-019).
#![forbid(unsafe_code)]

pub use redraw_shared as shared;

use redraw_sequencer::{
    build_schedule_with, EventKind, LoopRange, SchedEvent, ScheduleOptions, DEFAULT_GATE,
    PATTERN_COLORS,
};
use redraw_shared::{ClipKind, ClipState, Marker, StudioProject, TrackKind, TrackState};

pub const CRATE_PHASE: u8 = 10;

/// Hard ceiling for one compiled arrangement (about 273 bars of dense 16ths).
pub const MAX_ARRANGEMENT_EVENTS: usize = 262_144;
/// Safety ceiling for repeated pattern clips.
pub const MAX_CLIP_REPEATS: u64 = 4096;

/// Absolute-tick playlist events plus the song end used when looping is off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArrangementSchedule {
    pub length: u64,
    pub events: Vec<SchedEvent>,
}

/// Compile the playlist with the project's swing and the default step gate.
pub fn build_arrangement(project: &StudioProject) -> ArrangementSchedule {
    build_arrangement_with(
        project,
        ScheduleOptions::new(project.swing, DEFAULT_GATE),
    )
}

/// Compile the playlist. Pattern clips repeat when `looping` is set, clip
/// transpose shifts keys, and clip gain scales note velocity. Any track solo
/// restricts playback to soloed tracks; mute always wins.
pub fn build_arrangement_with(
    project: &StudioProject,
    options: ScheduleOptions,
) -> ArrangementSchedule {
    let length = playlist_length(project);
    let any_solo = project.playlist.tracks.iter().any(|t| t.solo);

    let mut pattern_events: Vec<(u32, u64, Vec<SchedEvent>)> = Vec::new();
    for pattern in &project.patterns {
        if let Some(sched) = build_schedule_with(std::slice::from_ref(pattern), options)
            .into_iter()
            .next()
        {
            pattern_events.push((pattern.id, sched.length, sched.events));
        }
    }

    let mut events = Vec::new();
    'clips: for clip in &project.playlist.clips {
        if clip.kind != ClipKind::Pattern || clip.length == 0 {
            continue;
        }
        let Some(track) = project.playlist.tracks.get(clip.track) else {
            continue;
        };
        if track.mute || (any_solo && !track.solo) {
            continue;
        }
        let Some((_, pattern_length, pattern_events)) = pattern_events
            .iter()
            .find(|(id, _, _)| *id == clip.source_id)
        else {
            continue;
        };
        let clip_end = clip.start.saturating_add(clip.length);
        let repeats = if clip.looping && *pattern_length > 0 {
            clip.length.div_ceil(*pattern_length).min(MAX_CLIP_REPEATS)
        } else {
            1
        };
        for repeat in 0..repeats {
            let base = repeat.saturating_mul(*pattern_length);
            if base >= clip.length {
                break;
            }
            for event in pattern_events {
                let absolute = clip
                    .start
                    .saturating_add(base)
                    .saturating_add(event.tick);
                let tick = match event.kind {
                    EventKind::NoteOn => {
                        if absolute >= clip_end {
                            continue;
                        }
                        absolute
                    }
                    EventKind::NoteOff => absolute.min(clip_end),
                };
                if tick < clip.start || tick > clip_end {
                    continue;
                }
                let velocity = match event.kind {
                    EventKind::NoteOn => scale_velocity(event.velocity, clip.gain_db),
                    EventKind::NoteOff => 0,
                };
                events.push(SchedEvent {
                    tick,
                    kind: event.kind,
                    channel: event.channel,
                    key: (event.key as i32 + clip.transpose).clamp(0, 127) as u8,
                    velocity,
                });
                if events.len() >= MAX_ARRANGEMENT_EVENTS {
                    break 'clips;
                }
            }
        }
    }

    events.sort_by(|a, b| {
        a.tick
            .cmp(&b.tick)
            .then_with(|| (a.kind == EventKind::NoteOn).cmp(&(b.kind == EventKind::NoteOn)))
            .then_with(|| a.channel.cmp(&b.channel))
            .then_with(|| a.key.cmp(&b.key))
    });
    ArrangementSchedule { length, events }
}

fn scale_velocity(velocity: u8, gain_db: f32) -> u8 {
    let gain = 10f32.powf(gain_db.clamp(-60.0, 12.0) / 20.0);
    (velocity as f32 * gain).round().clamp(1.0, 127.0) as u8
}

/// Song end in ticks (the furthest clip end, including muted clips).
pub fn playlist_length(project: &StudioProject) -> u64 {
    project
        .playlist
        .clips
        .iter()
        .map(|c| c.start.saturating_add(c.length))
        .max()
        .unwrap_or(0)
}

// ------------------------------------------------------------------ clip editing

fn alloc_clip_id(project: &mut StudioProject) -> u64 {
    let id = project.next_clip_id.max(1);
    project.next_clip_id = id.saturating_add(1);
    id
}

pub fn find_clip(project: &StudioProject, clip_id: u64) -> Option<&ClipState> {
    project.playlist.clips.iter().find(|c| c.id == clip_id)
}

pub fn clip_mut(project: &mut StudioProject, clip_id: u64) -> Option<&mut ClipState> {
    project
        .playlist
        .clips
        .iter_mut()
        .find(|c| c.id == clip_id)
}

/// Place a pattern on a pattern track; the clip repeats by default.
pub fn add_pattern_clip(
    project: &mut StudioProject,
    track: usize,
    pattern_index: usize,
    start: u64,
    length: u64,
) -> Option<u64> {
    let track_state = project.playlist.tracks.get(track)?;
    if track_state.kind != TrackKind::Pattern {
        return None;
    }
    let pattern = project.patterns.get(pattern_index)?;
    let (pattern_id, pattern_name, pattern_color) =
        (pattern.id, pattern.name.clone(), pattern.color);
    let id = alloc_clip_id(project);
    project.playlist.clips.push(ClipState {
        id,
        track,
        start,
        length: length.max(1),
        kind: ClipKind::Pattern,
        source_id: pattern_id,
        name: pattern_name,
        color: pattern_color,
        looping: true,
        gain_db: 0.0,
        transpose: 0,
        stretch: 1.0,
        audio_path: None,
    });
    Some(id)
}

/// Insert a fully described clip (used by MIDI/audio automation later).
/// Returns the assigned id, or `None` for an invalid track or duplicate id.
pub fn add_clip(project: &mut StudioProject, mut clip: ClipState) -> Option<u64> {
    if clip.track >= project.playlist.tracks.len() {
        return None;
    }
    if clip.id == 0 {
        clip.id = alloc_clip_id(project);
    } else if find_clip(project, clip.id).is_some() {
        return None;
    }
    clip.length = clip.length.max(1);
    let id = clip.id;
    project.playlist.clips.push(clip);
    Some(id)
}

pub fn move_clip(project: &mut StudioProject, clip_id: u64, track: usize, start: u64) -> bool {
    if track >= project.playlist.tracks.len() {
        return false;
    }
    match clip_mut(project, clip_id) {
        Some(clip) => {
            clip.track = track;
            clip.start = start;
            true
        }
        None => false,
    }
}

pub fn resize_clip(project: &mut StudioProject, clip_id: u64, length: u64) -> bool {
    match clip_mut(project, clip_id) {
        Some(clip) => {
            clip.length = length.max(1);
            true
        }
        None => false,
    }
}

pub fn delete_clip(project: &mut StudioProject, clip_id: u64) -> bool {
    let before = project.playlist.clips.len();
    project.playlist.clips.retain(|c| c.id != clip_id);
    project.playlist.clips.len() != before
}

/// Copy a clip directly after itself and return the new id.
pub fn duplicate_clip(project: &mut StudioProject, clip_id: u64) -> Option<u64> {
    let mut copy = find_clip(project, clip_id)?.clone();
    copy.id = alloc_clip_id(project);
    copy.start = copy.start.saturating_add(copy.length);
    copy.name = format!("{} copy", copy.name);
    let id = copy.id;
    project.playlist.clips.push(copy);
    Some(id)
}

pub fn set_clip_looping(project: &mut StudioProject, clip_id: u64, looping: bool) -> bool {
    match clip_mut(project, clip_id) {
        Some(clip) => {
            clip.looping = looping;
            true
        }
        None => false,
    }
}

pub fn set_clip_transpose(project: &mut StudioProject, clip_id: u64, semitones: i32) -> bool {
    match clip_mut(project, clip_id) {
        Some(clip) => {
            clip.transpose = semitones.clamp(-48, 48);
            true
        }
        None => false,
    }
}

pub fn set_clip_gain(project: &mut StudioProject, clip_id: u64, gain_db: f32) -> bool {
    match clip_mut(project, clip_id) {
        Some(clip) => {
            clip.gain_db = gain_db.clamp(-60.0, 12.0);
            true
        }
        None => false,
    }
}

/// Topmost clip under a playlist position (last inserted wins).
pub fn clip_at(project: &StudioProject, track: usize, tick: u64) -> Option<&ClipState> {
    project.playlist.clips.iter().rev().find(|c| {
        c.track == track
            && tick >= c.start
            && tick < c.start.saturating_add(c.length)
    })
}

// ----------------------------------------------------------------- track editing

pub fn add_track(
    project: &mut StudioProject,
    kind: TrackKind,
    name: impl Into<String>,
) -> usize {
    let index = project.playlist.tracks.len();
    let color = PATTERN_COLORS[index % PATTERN_COLORS.len()];
    project.playlist.tracks.push(TrackState {
        name: name.into(),
        color,
        kind,
        mute: false,
        solo: false,
        height: 74.0,
    });
    index
}

/// Remove a track and its clips, shifting later clip indices. The last track is
/// never removed.
pub fn remove_track(project: &mut StudioProject, index: usize) -> bool {
    if project.playlist.tracks.len() <= 1 || index >= project.playlist.tracks.len() {
        return false;
    }
    project.playlist.tracks.remove(index);
    project.playlist.clips.retain(|c| c.track != index);
    for clip in &mut project.playlist.clips {
        if clip.track > index {
            clip.track -= 1;
        }
    }
    true
}

pub fn set_track_mute(project: &mut StudioProject, index: usize, mute: bool) -> bool {
    match project.playlist.tracks.get_mut(index) {
        Some(track) => {
            track.mute = mute;
            true
        }
        None => false,
    }
}

pub fn set_track_solo(project: &mut StudioProject, index: usize, solo: bool) -> bool {
    match project.playlist.tracks.get_mut(index) {
        Some(track) => {
            track.solo = solo;
            true
        }
        None => false,
    }
}

// ------------------------------------------------------------------ loop/markers

/// Set the playlist loop. Enabling requires `end > start`; disabling keeps the
/// last valid range.
pub fn set_playlist_loop(
    project: &mut StudioProject,
    enabled: bool,
    start: u64,
    end: u64,
) -> bool {
    if enabled && end <= start {
        return false;
    }
    project.playlist.loop_enabled = enabled;
    if end > start {
        project.playlist.loop_start = start;
        project.playlist.loop_end = end;
    }
    true
}

pub fn playlist_loop(project: &StudioProject) -> LoopRange {
    LoopRange {
        enabled: project.playlist.loop_enabled,
        start: project.playlist.loop_start,
        end: project.playlist.loop_end,
    }
}

pub fn add_marker(project: &mut StudioProject, name: impl Into<String>, tick: u64) -> usize {
    let color = PATTERN_COLORS[project.playlist.markers.len() % PATTERN_COLORS.len()];
    project.playlist.markers.push(Marker {
        name: name.into(),
        color,
        pos: tick,
    });
    project.playlist.markers.len() - 1
}

pub fn remove_marker(project: &mut StudioProject, index: usize) -> bool {
    if index >= project.playlist.markers.len() {
        return false;
    }
    project.playlist.markers.remove(index);
    true
}

/// Latest marker at or before `tick`.
pub fn marker_at(project: &StudioProject, tick: u64) -> Option<&Marker> {
    project
        .playlist
        .markers
        .iter()
        .filter(|m| m.pos <= tick)
        .max_by_key(|m| m.pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use redraw_shared::{Note, Pattern};

    fn project_with_pattern(length_steps: u32, active_step: usize) -> StudioProject {
        let mut project = StudioProject::default();
        project.patterns.truncate(1);
        project.patterns[0] = Pattern::new(1, "P1", [100, 150, 100], length_steps);
        let pc = project.patterns[0].channel_mut(1, length_steps);
        pc.steps[active_step].active = true;
        project.playlist.tracks.truncate(1);
        project.playlist.clips.clear();
        project.playlist.markers.clear();
        project.next_clip_id = 1;
        project
    }

    fn note_ons(schedule: &ArrangementSchedule) -> Vec<u64> {
        schedule
            .events
            .iter()
            .filter(|e| e.kind == EventKind::NoteOn)
            .map(|e| e.tick)
            .collect()
    }

    #[test]
    fn looping_pattern_clip_repeats_and_clips() {
        let mut project = project_with_pattern(4, 0);
        let clip = add_pattern_clip(&mut project, 0, 0, 480, 2400).expect("clip");
        let schedule = build_arrangement(&project);
        assert_eq!(schedule.length, 2880);
        assert_eq!(note_ons(&schedule), vec![480, 1440, 2400]);
        let offs: Vec<u64> = schedule
            .events
            .iter()
            .filter(|e| e.kind == EventKind::NoteOff)
            .map(|e| e.tick)
            .collect();
        assert_eq!(offs, vec![660, 1620, 2580]);

        assert!(set_clip_looping(&mut project, clip, false));
        let once = build_arrangement(&project);
        assert_eq!(note_ons(&once), vec![480]);
    }

    #[test]
    fn clip_transpose_and_gain_apply_to_notes() {
        let mut project = project_with_pattern(4, 0);
        let clip = add_pattern_clip(&mut project, 0, 0, 0, 480).expect("clip");
        assert!(set_clip_transpose(&mut project, clip, 12));
        assert!(set_clip_gain(&mut project, clip, -6.0206));
        let schedule = build_arrangement(&project);
        let on = schedule
            .events
            .iter()
            .find(|e| e.kind == EventKind::NoteOn)
            .expect("note on");
        assert_eq!(on.key, 72);
        assert!((on.velocity as i32 - 50).abs() <= 1, "velocity {}", on.velocity);
        assert!(set_clip_transpose(&mut project, clip, 999));
        assert_eq!(find_clip(&project, clip).unwrap().transpose, 48);
    }

    #[test]
    fn mute_and_solo_filter_tracks() {
        let mut project = project_with_pattern(4, 0);
        let track1 = add_track(&mut project, TrackKind::Pattern, "Track 2");
        add_pattern_clip(&mut project, 0, 0, 0, 480).unwrap();
        add_pattern_clip(&mut project, track1, 0, 960, 480).unwrap();
        assert_eq!(note_ons(&build_arrangement(&project)), vec![0, 960]);

        assert!(set_track_mute(&mut project, 0, true));
        assert_eq!(note_ons(&build_arrangement(&project)), vec![960]);
        assert!(set_track_mute(&mut project, 0, false));

        assert!(set_track_solo(&mut project, 0, true));
        assert_eq!(note_ons(&build_arrangement(&project)), vec![0]);
        assert!(set_track_solo(&mut project, 1, true));
        assert_eq!(note_ons(&build_arrangement(&project)), vec![0, 960]);

        assert!(set_track_solo(&mut project, 0, false));
        assert!(set_track_solo(&mut project, 1, false));
        assert!(set_track_mute(&mut project, 1, true));
        assert_eq!(note_ons(&build_arrangement(&project)), vec![0]);
    }

    #[test]
    fn clip_editing_and_tracks() {
        let mut project = project_with_pattern(4, 0);
        let clip = add_pattern_clip(&mut project, 0, 0, 0, 960).expect("clip");
        assert_eq!(project.playlist.clips.len(), 1);
        assert!(move_clip(&mut project, clip, 0, 480));
        assert!(resize_clip(&mut project, clip, 1200));
        let copy = duplicate_clip(&mut project, clip).expect("copy");
        assert_ne!(copy, clip);
        let copy_state = find_clip(&project, copy).unwrap();
        assert_eq!(copy_state.start, 1680);
        assert_eq!(copy_state.length, 1200);
        assert_eq!(project.playlist.clips.len(), 2);
        assert!(delete_clip(&mut project, clip));
        assert!(find_clip(&project, clip).is_none());
        assert!(!delete_clip(&mut project, clip));
        assert!(delete_clip(&mut project, copy));

        let audio = add_track(&mut project, TrackKind::Audio, "Audio 1");
        assert_eq!(audio, 1);
        let other = add_pattern_clip(&mut project, audio, 0, 0, 480);
        assert!(other.is_none(), "pattern clips require a pattern track");

        let audio_clip = add_clip(
            &mut project,
            ClipState {
                id: 0,
                track: audio,
                start: 0,
                length: 960,
                kind: ClipKind::Audio,
                source_id: 0,
                name: "Loop".into(),
                color: [100, 100, 100],
                looping: true,
                gain_db: 0.0,
                transpose: 0,
                stretch: 1.0,
                audio_path: None,
            },
        )
        .expect("audio clip");
        assert_eq!(playlist_length(&project), 960);
        assert!(build_arrangement(&project).events.is_empty());
        assert!(remove_track(&mut project, audio));
        assert!(find_clip(&project, audio_clip).is_none());
        assert!(!remove_track(&mut project, 0), "last track is kept");
    }

    #[test]
    fn clip_boundaries_release_notes() {
        let mut project = project_with_pattern(16, 0);
        let pc = project.patterns[0].channel_mut(2, 16);
        pc.notes.push(Note {
            id: 1,
            key: 64,
            start: 0,
            length: 960,
            velocity: 100,
            pan: 0,
            release: 64,
        });
        let clip = add_pattern_clip(&mut project, 0, 0, 100, 480).expect("clip");
        assert!(set_clip_looping(&mut project, clip, false));
        let schedule = build_arrangement(&project);
        let events: Vec<(u64, EventKind, u8)> = schedule
            .events
            .iter()
            .map(|e| (e.tick, e.kind, e.key))
            .collect();
        assert_eq!(
            events,
            vec![
                (100, EventKind::NoteOn, 60),
                (100, EventKind::NoteOn, 64),
                (280, EventKind::NoteOff, 60),
                (580, EventKind::NoteOff, 64),
            ]
        );
    }

    #[test]
    fn playlist_loop_and_markers() {
        let mut project = project_with_pattern(4, 0);
        assert!(!set_playlist_loop(&mut project, true, 960, 960));
        assert!(set_playlist_loop(&mut project, true, 480, 1920));
        assert_eq!(
            playlist_loop(&project),
            LoopRange {
                enabled: true,
                start: 480,
                end: 1920
            }
        );
        assert!(set_playlist_loop(&mut project, false, 0, 0));
        assert!(!playlist_loop(&project).enabled);
        assert_eq!(playlist_loop(&project).start, 480);

        let intro = add_marker(&mut project, "Intro", 0);
        add_marker(&mut project, "Drop", 1920);
        assert_eq!(marker_at(&project, 1000).unwrap().name, "Intro");
        assert_eq!(marker_at(&project, 1920).unwrap().name, "Drop");
        assert!(marker_at(&project, 0).is_some());
        assert!(remove_marker(&mut project, intro));
        assert!(marker_at(&project, 0).is_none());
    }

    #[test]
    fn events_are_sorted_and_unique_ids_allocated() {
        let mut project = project_with_pattern(4, 0);
        let a = add_pattern_clip(&mut project, 0, 0, 480, 960).unwrap();
        let b = add_pattern_clip(&mut project, 0, 0, 0, 960).unwrap();
        assert_ne!(a, b);
        let schedule = build_arrangement(&project);
        assert!(schedule
            .events
            .windows(2)
            .all(|w| w[0].tick <= w[1].tick));
        assert_eq!(project.next_clip_id, 3);
        assert_eq!(clip_at(&project, 0, 0).unwrap().id, b);
        assert!(clip_at(&project, 0, 2000).is_none());
    }
}
