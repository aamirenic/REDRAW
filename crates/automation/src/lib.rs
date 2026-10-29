//! REDRAW automation clips, envelopes, recording and compiled playback schedules.
//!
//! Automation points are always **normalized** to `0..1`. The target supplies the
//! real-world range (tempo BPM, fader dB, pan, a plugin parameter's `ParamSpec`),
//! so a clip can be retargeted without rewriting its envelope. The engine compiles
//! the project's clips into an [`AutomationSchedule`] on the control thread and
//! applies values at a fixed control rate; evaluation itself never allocates.
#![forbid(unsafe_code)]

pub use redraw_shared as shared;

use redraw_shared::{
    automation_value, AutoTarget, AutomationClipState, AutomationPoint, Interp, ParamSpec,
    StudioProject,
};

pub const CRATE_PHASE: u8 = 12;

/// Hard limits keep schedules bounded and the audio thread's work predictable.
pub const MAX_AUTOMATION_CLIPS: usize = 256;
pub const MAX_AUTOMATION_POINTS: usize = 16_384;
pub const MAX_SCHEDULE_POINTS: usize = 131_072;

pub const TEMPO_MIN_BPM: f32 = 10.0;
pub const TEMPO_MAX_BPM: f32 = 999.0;
pub const VOLUME_MIN_DB: f32 = -90.0;
pub const VOLUME_MAX_DB: f32 = 12.0;
pub const PAN_MIN: f32 = -1.0;
pub const PAN_MAX: f32 = 1.0;

/// Default recorder throttle: about 5 ms at 120 BPM (960 PPQ).
pub const DEFAULT_RECORD_INTERVAL: u64 = 10;
/// Changes smaller than this are treated as knob noise and not stored.
pub const DEFAULT_RECORD_TOLERANCE: f32 = 0.002;

// ---------------------------------------------------------------- target mapping

/// Real-world range and unit for an automation target. Plugin parameters use the
/// parameter's own [`ParamSpec`] instead because their range is instance-specific.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutomationRange {
    pub min: f32,
    pub max: f32,
    pub unit: &'static str,
}

impl AutomationRange {
    pub const fn new(min: f32, max: f32, unit: &'static str) -> Self {
        Self { min, max, unit }
    }
    pub fn normalize(&self, value: f32) -> f32 {
        let span = (self.max - self.min).max(1e-9);
        ((value.clamp(self.min, self.max) - self.min) / span).clamp(0.0, 1.0)
    }
    pub fn denormalize(&self, normalized: f32) -> f32 {
        self.min + (self.max - self.min) * normalized.clamp(0.0, 1.0)
    }
}

pub const TEMPO_RANGE: AutomationRange = AutomationRange::new(TEMPO_MIN_BPM, TEMPO_MAX_BPM, "BPM");
pub const VOLUME_RANGE: AutomationRange = AutomationRange::new(VOLUME_MIN_DB, VOLUME_MAX_DB, "dB");
pub const PAN_RANGE: AutomationRange = AutomationRange::new(PAN_MIN, PAN_MAX, "");
pub const SEND_RANGE: AutomationRange = VOLUME_RANGE;

/// Range for targets whose scale is fixed. Channel/FX parameters return `None`;
/// callers supply the plugin's [`ParamSpec`].
pub fn fixed_range(target: &AutoTarget) -> Option<AutomationRange> {
    match target {
        AutoTarget::Tempo => Some(TEMPO_RANGE),
        AutoTarget::MasterVolume => Some(VOLUME_RANGE),
        AutoTarget::ChannelVolume { .. } => Some(VOLUME_RANGE),
        AutoTarget::ChannelPan { .. } => Some(PAN_RANGE),
        AutoTarget::SendLevel { .. } => Some(SEND_RANGE),
        AutoTarget::ChannelParam { .. } | AutoTarget::FxParam { .. } => None,
    }
}

/// Convert a normalized value to the target's real-world value for fixed-range
/// targets. Parameter targets return `None` and must use `ParamSpec::denormalize`.
pub fn denormalize_target(target: &AutoTarget, normalized: f32) -> Option<f32> {
    fixed_range(target).map(|r| r.denormalize(normalized))
}

/// Convert a real-world value to `0..1` for fixed-range targets.
pub fn normalize_target(target: &AutoTarget, value: f32) -> Option<f32> {
    fixed_range(target).map(|r| r.normalize(value))
}

pub fn tempo_normalized(bpm: f32) -> f32 {
    TEMPO_RANGE.normalize(bpm)
}

pub fn tempo_bpm(normalized: f32) -> f32 {
    TEMPO_RANGE.denormalize(normalized)
}

pub fn volume_db(normalized: f32) -> f32 {
    VOLUME_RANGE.denormalize(normalized)
}

pub fn volume_normalized(db: f32) -> f32 {
    VOLUME_RANGE.normalize(db)
}

pub fn pan_value(normalized: f32) -> f32 {
    PAN_RANGE.denormalize(normalized)
}

pub fn pan_normalized(pan: f32) -> f32 {
    PAN_RANGE.normalize(pan)
}

/// Use a plugin parameter's own curve and range for automation.
pub fn param_value(spec: &ParamSpec, normalized: f32) -> f32 {
    spec.denormalize(normalized)
}

pub fn param_normalized(spec: &ParamSpec, value: f32) -> f32 {
    spec.normalize(value)
}

/// Human-readable target name for lane headers and default clip names.
pub fn target_label(target: &AutoTarget) -> String {
    match target {
        AutoTarget::Tempo => "Tempo".into(),
        AutoTarget::MasterVolume => "Master Volume".into(),
        AutoTarget::ChannelVolume { channel_id } => format!("Channel {channel_id} Volume"),
        AutoTarget::ChannelPan { channel_id } => format!("Channel {channel_id} Pan"),
        AutoTarget::ChannelParam { channel_id, param } => {
            format!("Channel {channel_id} Param {param}")
        }
        AutoTarget::FxParam {
            channel_id,
            slot,
            param,
        } => format!("Channel {channel_id} FX {slot} Param {param}"),
        AutoTarget::SendLevel { channel_id, send } => {
            format!("Channel {channel_id} Send {send}")
        }
    }
}

// ---------------------------------------------------------------- envelope editing

fn clean_point(mut point: AutomationPoint, length: u64) -> AutomationPoint {
    if !point.time.is_finite() {
        point.time = 0.0;
    }
    point.time = point.time.clamp(0.0, length.max(1) as f64);
    if !point.value.is_finite() {
        point.value = 0.0;
    }
    point.value = point.value.clamp(0.0, 1.0);
    if !point.tension.is_finite() {
        point.tension = 0.0;
    }
    point.tension = point.tension.clamp(-1.0, 1.0);
    point
}

/// Sort, clamp and collapse duplicate timestamps (keeping the newest value).
/// Returns the resulting point count.
pub fn clean_points(points: &mut Vec<AutomationPoint>, length: u64) -> usize {
    for p in points.iter_mut() {
        *p = clean_point(*p, length);
    }
    points.sort_by(|a, b| a.time.total_cmp(&b.time));
    let mut write = 0usize;
    for read in 0..points.len() {
        if write > 0 && (points[write - 1].time - points[read].time).abs() <= 1e-9 {
            points[write - 1] = points[read];
        } else {
            points[write] = points[read];
            write += 1;
        }
    }
    points.truncate(write.min(MAX_AUTOMATION_POINTS));
    points.len()
}

/// Replace a clip's envelope with an unsorted input. Values are clamped to `0..1`
/// and timestamps to the clip length, then sorted and deduplicated.
pub fn set_points(clip: &mut AutomationClipState, points: &[AutomationPoint]) -> usize {
    clip.points.clear();
    clip.points.extend(points.iter().copied().take(MAX_AUTOMATION_POINTS));
    clean_points(&mut clip.points, clip.length)
}

/// Add or replace a point at `time`. Returns the point's index, or `None` when the
/// clip already holds the maximum number of points.
pub fn add_point(
    clip: &mut AutomationClipState,
    time: f64,
    value: f32,
    tension: f32,
) -> Option<usize> {
    let time = if time.is_finite() { time } else { 0.0 }
        .clamp(0.0, clip.length.max(1) as f64);
    if let Some(index) = clip
        .points
        .iter()
        .position(|p| (p.time - time).abs() <= 1e-9)
    {
        clip.points[index].value = value.clamp(0.0, 1.0);
        clip.points[index].tension = if tension.is_finite() {
            tension.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        return Some(index);
    }
    if clip.points.len() >= MAX_AUTOMATION_POINTS {
        return None;
    }
    clip.points.push(AutomationPoint {
        time,
        value: value.clamp(0.0, 1.0),
        tension: if tension.is_finite() {
            tension.clamp(-1.0, 1.0)
        } else {
            0.0
        },
    });
    clean_points(&mut clip.points, clip.length);
    clip.points
        .iter()
        .position(|p| (p.time - time).abs() <= 1e-9)
}

pub fn remove_point(clip: &mut AutomationClipState, index: usize) -> bool {
    if index < clip.points.len() {
        clip.points.remove(index);
        true
    } else {
        false
    }
}

/// Move an existing point, clamping it into the clip. Returns its new index.
pub fn move_point(
    clip: &mut AutomationClipState,
    mut index: usize,
    time: f64,
    value: f32,
) -> Option<usize> {
    if index >= clip.points.len() {
        return None;
    }
    let time = if time.is_finite() { time } else { 0.0 }
        .clamp(0.0, clip.length.max(1) as f64);
    clip.points[index].time = time;
    clip.points[index].value = value.clamp(0.0, 1.0);
    // The moved point wins over any point it lands on.
    let mut i = 0usize;
    while i < clip.points.len() {
        if i != index && (clip.points[i].time - time).abs() <= 1e-9 {
            clip.points.remove(i);
            if i < index {
                index -= 1;
            }
        } else {
            i += 1;
        }
    }
    clean_points(&mut clip.points, clip.length);
    clip.points
        .iter()
        .position(|p| (p.time - time).abs() <= 1e-9)
}

pub fn set_point_tension(clip: &mut AutomationClipState, index: usize, tension: f32) -> bool {
    if let Some(p) = clip.points.get_mut(index) {
        p.tension = if tension.is_finite() {
            tension.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        true
    } else {
        false
    }
}

pub fn remove_points_in_range(clip: &mut AutomationClipState, start: f64, end: f64) -> usize {
    let (lo, hi) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    let before = clip.points.len();
    clip.points.retain(|p| p.time < lo || p.time > hi);
    before - clip.points.len()
}

pub fn clear_points(clip: &mut AutomationClipState) {
    clip.points.clear();
}

/// Change a clip's length. Points past the new end are clamped to it (and then
/// deduplicated), so an envelope never addresses time outside its clip.
pub fn set_clip_length(clip: &mut AutomationClipState, length: u64) {
    clip.length = length.max(1);
    clean_points(&mut clip.points, clip.length);
}

pub fn set_clip_interp(clip: &mut AutomationClipState, interp: Interp) {
    clip.interp = interp;
}

pub fn set_clip_enabled(clip: &mut AutomationClipState, enabled: bool) {
    clip.enabled = enabled;
}

pub fn set_clip_name(clip: &mut AutomationClipState, name: impl Into<String>) {
    clip.name = name.into();
}

pub fn set_clip_target(clip: &mut AutomationClipState, target: AutoTarget) {
    clip.target = target;
}

// ---------------------------------------------------------------- project clips

pub fn find_clip(project: &StudioProject, id: u64) -> Option<&AutomationClipState> {
    project.automation.iter().find(|c| c.id == id)
}

pub fn find_clip_mut(project: &mut StudioProject, id: u64) -> Option<&mut AutomationClipState> {
    project.automation.iter_mut().find(|c| c.id == id)
}

fn next_clip_id(project: &mut StudioProject) -> u64 {
    let mut id = project.next_automation_id.max(1);
    while project.automation.iter().any(|c| c.id == id) {
        id = id.saturating_add(1);
        if id == u64::MAX {
            break;
        }
    }
    project.next_automation_id = id.saturating_add(1);
    id
}

/// Create a clip with a constant envelope at `initial_value` (already normalized).
/// Returns `None` when the project already holds [`MAX_AUTOMATION_CLIPS`] lanes.
pub fn add_clip(
    project: &mut StudioProject,
    target: AutoTarget,
    length: u64,
    initial_value: f32,
    color: [u8; 3],
) -> Option<u64> {
    let name = format!("{} Automation", target_label(&target));
    add_named_clip(project, name, target, length, initial_value, color)
}

pub fn add_named_clip(
    project: &mut StudioProject,
    name: impl Into<String>,
    target: AutoTarget,
    length: u64,
    initial_value: f32,
    color: [u8; 3],
) -> Option<u64> {
    if project.automation.len() >= MAX_AUTOMATION_CLIPS {
        return None;
    }
    let length = length.max(1);
    let value = if initial_value.is_finite() {
        initial_value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let id = next_clip_id(project);
    project.automation.push(AutomationClipState {
        id,
        name: name.into(),
        color,
        target,
        points: vec![
            AutomationPoint {
                time: 0.0,
                value,
                tension: 0.0,
            },
            AutomationPoint {
                time: length as f64,
                value,
                tension: 0.0,
            },
        ],
        interp: Interp::Linear,
        enabled: true,
        length,
    });
    Some(id)
}

pub fn duplicate_clip(project: &mut StudioProject, id: u64) -> Option<u64> {
    if project.automation.len() >= MAX_AUTOMATION_CLIPS {
        return None;
    }
    let mut copy = find_clip(project, id)?.clone();
    copy.id = next_clip_id(project);
    copy.name = format!("{} Copy", copy.name);
    let new_id = copy.id;
    project.automation.push(copy);
    Some(new_id)
}

pub fn remove_clip(project: &mut StudioProject, id: u64) -> bool {
    let before = project.automation.len();
    project.automation.retain(|c| c.id != id);
    project.automation.len() != before
}

/// Return every enabled lane's value at `tick`, in project order. Multiple lanes
/// may target the same parameter; playback applies them in this order, so the
/// last lane wins.
pub fn values_at(project: &StudioProject, tick: f64) -> Vec<(AutoTarget, f32)> {
    AutomationSchedule::from_project(project).values_at(tick)
}

// ---------------------------------------------------------------- playback schedule

/// One immutable envelope copied out of the project for real-time playback.
#[derive(Clone, Debug)]
pub struct AutomationLane {
    pub id: u64,
    pub target: AutoTarget,
    pub points: Vec<AutomationPoint>,
    pub interp: Interp,
    pub length: u64,
}

impl AutomationLane {
    pub fn value_at(&self, tick: f64) -> f32 {
        let value = automation_value(&self.points, self.interp, tick);
        if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// Allocation-free (after construction) evaluation table installed in the audio
/// engine. Disabled and empty clips are omitted.
#[derive(Clone, Debug, Default)]
pub struct AutomationSchedule {
    pub lanes: Vec<AutomationLane>,
    pub has_tempo: bool,
}

impl AutomationSchedule {
    pub fn from_project(project: &StudioProject) -> Self {
        Self::from_clips(&project.automation)
    }

    pub fn from_clips(clips: &[AutomationClipState]) -> Self {
        let mut lanes = Vec::with_capacity(clips.len().min(MAX_AUTOMATION_CLIPS));
        let mut total_points = 0usize;
        let mut has_tempo = false;
        for clip in clips {
            if !clip.enabled || clip.points.is_empty() {
                continue;
            }
            if lanes.len() >= MAX_AUTOMATION_CLIPS
                || total_points.saturating_add(clip.points.len()) > MAX_SCHEDULE_POINTS
            {
                break;
            }
            let mut points = clip.points.clone();
            clean_points(&mut points, clip.length);
            if points.is_empty() {
                continue;
            }
            total_points += points.len();
            has_tempo |= matches!(clip.target, AutoTarget::Tempo);
            lanes.push(AutomationLane {
                id: clip.id,
                target: clip.target.clone(),
                points,
                interp: clip.interp,
                length: clip.length.max(1),
            });
        }
        Self { lanes, has_tempo }
    }

    pub fn is_empty(&self) -> bool {
        self.lanes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.lanes.len()
    }

    /// Last lane wins when several lanes share a target.
    pub fn value_at(&self, target: &AutoTarget, tick: f64) -> Option<f32> {
        self.lanes
            .iter()
            .rev()
            .find(|lane| &lane.target == target)
            .map(|lane| lane.value_at(tick))
    }

    pub fn values_at(&self, tick: f64) -> Vec<(AutoTarget, f32)> {
        self.lanes
            .iter()
            .map(|lane| (lane.target.clone(), lane.value_at(tick)))
            .collect()
    }

    /// Iterate values without allocating. Callers apply targets in lane order.
    pub fn for_each_at(&self, tick: f64, mut f: impl FnMut(&AutoTarget, f32)) {
        for lane in &self.lanes {
            f(&lane.target, lane.value_at(tick));
        }
    }
}

// ---------------------------------------------------------------- recording

/// Captures a stream of normalized parameter values into an envelope. The
/// recorder is deliberately simple: it throttles by tick interval and drops
/// changes below `tolerance`, which keeps knob jitter from flooding the clip.
#[derive(Clone, Debug)]
pub struct AutomationRecorder {
    pub target: AutoTarget,
    pub points: Vec<AutomationPoint>,
    pub min_interval: u64,
    pub tolerance: f32,
    pub active: bool,
    last_tick: u64,
    last_value: f32,
}

impl AutomationRecorder {
    pub fn new(target: AutoTarget) -> Self {
        Self {
            target,
            points: Vec::new(),
            min_interval: DEFAULT_RECORD_INTERVAL,
            tolerance: DEFAULT_RECORD_TOLERANCE,
            active: false,
            last_tick: 0,
            last_value: 0.0,
        }
    }

    pub fn with_settings(target: AutoTarget, min_interval: u64, tolerance: f32) -> Self {
        let mut recorder = Self::new(target);
        recorder.min_interval = min_interval.min(MAX_AUTOMATION_POINTS as u64);
        recorder.tolerance = if tolerance.is_finite() {
            tolerance.clamp(0.0, 1.0)
        } else {
            0.0
        };
        recorder
    }

    pub fn begin(&mut self, tick: u64, value: f32) {
        self.points.clear();
        self.active = true;
        self.last_tick = tick;
        self.last_value = value.clamp(0.0, 1.0);
        self.points.push(AutomationPoint {
            time: tick as f64,
            value: self.last_value,
            tension: 0.0,
        });
    }

    /// Record a value if the throttle interval elapsed and it differs from the
    /// previous value. Returns whether a point was added.
    pub fn capture(&mut self, tick: u64, value: f32) -> bool {
        if !self.active {
            return false;
        }
        let value = value.clamp(0.0, 1.0);
        if tick < self.last_tick {
            return false;
        }
        if tick == self.last_tick {
            if let Some(last) = self.points.last_mut() {
                last.value = value;
            }
            self.last_value = value;
            return false;
        }
        if tick - self.last_tick < self.min_interval {
            return false;
        }
        if (value - self.last_value).abs() <= self.tolerance {
            return false;
        }
        if self.points.len() >= MAX_AUTOMATION_POINTS {
            return false;
        }
        self.points.push(AutomationPoint {
            time: tick as f64,
            value,
            tension: 0.0,
        });
        self.last_tick = tick;
        self.last_value = value;
        true
    }

    /// Stop recording and take the captured points.
    pub fn finish(&mut self) -> Vec<AutomationPoint> {
        self.active = false;
        std::mem::take(&mut self.points)
    }

    /// Stop recording and replace a clip's envelope with the captured points.
    /// The clip length and interpolation mode are preserved.
    pub fn finish_into_clip(&mut self, clip: &mut AutomationClipState) -> usize {
        let points = self.finish();
        set_points(clip, &points)
    }
}

/// Append one live parameter value to a clip envelope. Repeating the current
/// timestamp replaces its value (last event wins). Intended for UI knobs that
/// already throttle; use [`AutomationRecorder`] for continuous streams.
pub fn record_point(clip: &mut AutomationClipState, tick: u64, value: f32) -> bool {
    let time = tick as f64;
    if let Some(last) = clip.points.last_mut() {
        if time <= last.time {
            last.value = value.clamp(0.0, 1.0);
            return true;
        }
    }
    add_point(clip, time, value, 0.0).is_some()
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use redraw_shared::ParamCurve;

    fn point(time: f64, value: f32) -> AutomationPoint {
        AutomationPoint {
            time,
            value,
            tension: 0.0,
        }
    }

    #[test]
    fn target_ranges_map_both_ways() {
        assert!((tempo_bpm(0.0) - TEMPO_MIN_BPM).abs() < 0.001);
        assert!((tempo_bpm(1.0) - TEMPO_MAX_BPM).abs() < 0.001);
        assert!((tempo_bpm(tempo_normalized(140.0)) - 140.0).abs() < 0.01);
        assert!((volume_db(0.0) - VOLUME_MIN_DB).abs() < 0.001);
        assert!((volume_db(1.0) - VOLUME_MAX_DB).abs() < 0.001);
        assert!((volume_db(volume_normalized(-6.0)) + 6.0).abs() < 0.01);
        assert!((pan_value(0.0) + 1.0).abs() < 0.001);
        assert!((pan_value(1.0) - 1.0).abs() < 0.001);
        assert!((pan_normalized(0.0) - 0.5).abs() < 0.001);

        let target = AutoTarget::ChannelVolume { channel_id: 3 };
        assert_eq!(denormalize_target(&target, 1.0), Some(12.0));
        assert_eq!(
            normalize_target(&target, 12.0),
            Some(1.0)
        );
        let param = AutoTarget::ChannelParam {
            channel_id: 1,
            param: 2,
        };
        assert_eq!(denormalize_target(&param, 0.5), None);

        let spec = ParamSpec::new("cutoff", 20.0, 20_000.0, 1_000.0, "Hz")
            .curve(ParamCurve::Exp);
        assert!((param_value(&spec, 0.0) - 20.0).abs() < 0.01);
        assert!((param_value(&spec, 1.0) - 20_000.0).abs() < 1.0);
        let geometric_mid = (20.0f32 * 20_000.0).sqrt();
        assert!((param_value(&spec, 0.5) - geometric_mid).abs() < 0.1);
        assert!((param_normalized(&spec, geometric_mid) - 0.5).abs() < 0.001);
    }

    #[test]
    fn clip_creation_and_envelope_editing() {
        let mut project = StudioProject::default();
        let id = add_clip(
            &mut project,
            AutoTarget::Tempo,
            1_920,
            tempo_normalized(140.0),
            [1, 2, 3],
        )
        .unwrap();
        assert_eq!(project.next_automation_id, 2);
        {
            let clip = find_clip_mut(&mut project, id).unwrap();
            assert_eq!(clip.points.len(), 2);
            assert!((clip.points[0].value - tempo_normalized(140.0)).abs() < 1e-6);
            assert_eq!(add_point(clip, 960.0, 0.75, 0.25), Some(1));
            assert_eq!(clip.points.len(), 3);
            assert_eq!(add_point(clip, 960.0, 0.5, -0.5), Some(1));
            assert_eq!(clip.points.len(), 3);
            assert!((clip.points[1].value - 0.5).abs() < 1e-6);
            assert!(remove_point(clip, 1));
            assert_eq!(clip.points.len(), 2);
            assert_eq!(move_point(clip, 0, 48.0, 0.2), Some(0));
            assert_eq!(move_point(clip, 1, 48.0, 0.9), Some(0));
            assert_eq!(clip.points.len(), 1);
            assert!((clip.points[0].value - 0.9).abs() < 1e-6);
            set_clip_length(clip, 480);
            assert_eq!(clip.length, 480);
            assert!(clip.points.iter().all(|p| p.time <= 480.0));
            assert_eq!(remove_points_in_range(clip, 0.0, 48.0), 1);
            clear_points(clip);
            assert!(clip.points.is_empty());
        }

        let copy = duplicate_clip(&mut project, id).unwrap();
        assert_ne!(copy, id);
        assert_eq!(find_clip(&project, copy).unwrap().name, "Tempo Automation Copy");
        assert!(remove_clip(&mut project, id));
        assert!(find_clip(&project, id).is_none());
        assert_eq!(project.automation.len(), 1);
    }

    #[test]
    fn cleaning_sorts_deduplicates_and_clamps() {
        let mut clip = AutomationClipState {
            id: 1,
            name: "test".into(),
            color: [0; 3],
            target: AutoTarget::MasterVolume,
            points: Vec::new(),
            interp: Interp::Linear,
            enabled: true,
            length: 1_000,
        };
        let added = set_points(
            &mut clip,
            &[
                point(500.0, 0.2),
                point(0.0, 0.8),
                point(0.0, 0.4),
                point(9_999.0, 2.0),
                point(f64::NAN, -1.0),
            ],
        );
        assert_eq!(added, 3);
        assert_eq!(clip.points[0].time, 0.0);
        // The NaN timestamp clamps to zero and -1.0 clamps to 0.0; as the last
        // point at that timestamp it replaces the earlier duplicate.
        assert!((clip.points[0].value - 0.0).abs() < 1e-6);
        assert_eq!(clip.points[1].time, 500.0);
        assert_eq!(clip.points[2].time, 1_000.0);
        assert!((clip.points[2].value - 1.0).abs() < 1e-6);
    }

    #[test]
    fn schedule_filters_and_evaluates_lanes() {
        let mut project = StudioProject::default();
        let tempo = add_named_clip(
            &mut project,
            "tempo",
            AutoTarget::Tempo,
            960,
            0.0,
            [0; 3],
        )
        .unwrap();
        let master = add_named_clip(
            &mut project,
            "master",
            AutoTarget::MasterVolume,
            960,
            0.0,
            [0; 3],
        )
        .unwrap();
        let disabled = add_named_clip(
            &mut project,
            "disabled",
            AutoTarget::ChannelPan { channel_id: 1 },
            960,
            1.0,
            [0; 3],
        )
        .unwrap();
        {
            let clip = find_clip_mut(&mut project, tempo).unwrap();
            set_points(clip, &[point(960.0, 1.0), point(0.0, 0.0)]);
            set_clip_interp(clip, Interp::Linear);
            let clip = find_clip_mut(&mut project, master).unwrap();
            set_points(clip, &[point(0.0, 0.0), point(480.0, 1.0), point(960.0, 0.0)]);
            let clip = find_clip_mut(&mut project, disabled).unwrap();
            set_clip_enabled(clip, false);
        }

        let schedule = AutomationSchedule::from_project(&project);
        assert_eq!(schedule.len(), 2);
        assert!(schedule.has_tempo);
        assert!((schedule.lanes[0].value_at(480.0) - 0.5).abs() < 1e-5);
        assert!((schedule.lanes[1].value_at(480.0) - 1.0).abs() < 1e-5);
        assert!((schedule.lanes[1].value_at(720.0) - 0.5).abs() < 1e-5);
        assert_eq!(
            schedule.value_at(&AutoTarget::Tempo, 480.0),
            Some(0.5)
        );
        assert_eq!(
            schedule.value_at(&AutoTarget::ChannelPan { channel_id: 1 }, 0.0),
            None
        );
        let values = schedule.values_at(240.0);
        assert_eq!(values.len(), 2);
        assert!((values[0].1 - 0.25).abs() < 1e-5);
    }

    #[test]
    fn recorder_throttles_and_writes_clip() {
        let mut recorder = AutomationRecorder::with_settings(AutoTarget::MasterVolume, 10, 0.01);
        recorder.begin(0, 0.0);
        assert!(!recorder.capture(5, 0.5));
        assert!(recorder.capture(10, 0.5));
        assert!(!recorder.capture(20, 0.5));
        assert!(recorder.capture(30, 0.25));
        assert!(!recorder.capture(29, 0.1));
        let points = recorder.finish();
        assert_eq!(points.len(), 3);
        assert!(!recorder.active);

        let mut clip = AutomationClipState {
            id: 9,
            name: "recorded".into(),
            color: [0; 3],
            target: AutoTarget::MasterVolume,
            points: Vec::new(),
            interp: Interp::Linear,
            enabled: true,
            length: 960,
        };
        let mut second = AutomationRecorder::with_settings(AutoTarget::MasterVolume, 1, 0.0);
        second.begin(0, 0.0);
        assert!(second.capture(1, 0.5));
        assert_eq!(second.finish_into_clip(&mut clip), 2);
        assert_eq!(clip.points.len(), 2);
        assert!((clip.points[1].value - 0.5).abs() < 1e-6);
    }

    #[test]
    fn record_point_replaces_same_tick() {
        let mut clip = AutomationClipState {
            id: 1,
            name: "live".into(),
            color: [0; 3],
            target: AutoTarget::ChannelVolume { channel_id: 1 },
            points: Vec::new(),
            interp: Interp::Linear,
            enabled: true,
            length: 960,
        };
        assert!(record_point(&mut clip, 120, 0.4));
        assert!(record_point(&mut clip, 120, 0.7));
        assert_eq!(clip.points.len(), 1);
        assert!((clip.points[0].value - 0.7).abs() < 1e-6);
        assert!(record_point(&mut clip, 240, 0.2));
        assert_eq!(clip.points.len(), 2);
    }
}
