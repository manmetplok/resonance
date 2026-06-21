//! Parameter-automation data model shared across the engine, app state and
//! project I/O (architecture doc #162, epic #14).
//!
//! One definition lives here so the realtime engine
//! (`resonance-audio`), the app (`resonance-app`) and project persistence all
//! agree on what a lane is, how a normalized lane value maps to the target's
//! real range, and how a lane is sampled at an arbitrary timeline frame.
//!
//! A `Breakpoint`'s `value` is always the **normalized** 0.0–1.0 lane value.
//! The real range it maps to depends on the target — decibels for gain,
//! `-1..=1` for pan, `0`/`1` for mute, the plugin's own `min..=max` for a CLAP
//! parameter. The mapping helpers ([`lane_value_to_real`] /
//! [`real_to_lane_value`]) are the single source of truth so UI labels and the
//! engine never disagree.

use serde::{Deserialize, Serialize};

/// Track identifier. Mirrors `resonance_audio::types::TrackId` (both are plain
/// `u64`); defined here because `resonance-common` sits below `resonance-audio`
/// in the dependency graph and cannot import it.
pub type TrackId = u64;
/// Bus identifier. Mirrors `resonance_audio::types::BusId`.
pub type BusId = u64;
/// Plugin-instance identifier. Mirrors `resonance_audio::types::PluginInstanceId`.
pub type PluginInstanceId = u64;
/// Identifier for an [`AutomationLane`], unique within a project.
pub type LaneId = u64;

/// Lowest dB value a gain lane maps to (normalized `0.0`). Matches the mixer
/// fader range in the app (`-60..=+6` dB).
pub const GAIN_MIN_DB: f32 = -60.0;
/// Highest dB value a gain lane maps to (normalized `1.0`).
pub const GAIN_MAX_DB: f32 = 6.0;

/// What an automation lane drives. Gain/pan/mute targets have fixed, known real
/// ranges; `PluginParam` carries only the addressing (instance + CLAP param id)
/// — its `min..=max` lives in the plugin's `ParamInfo` and is applied by the
/// engine, so the generic [`lane_value_to_plugin_param`] helper takes the range
/// explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutomationTarget {
    TrackGain(TrackId),
    TrackPan(TrackId),
    TrackMute(TrackId),
    BusGain(BusId),
    BusPan(BusId),
    BusMute(BusId),
    MasterGain,
    PluginParam {
        instance: PluginInstanceId,
        param_id: u32,
    },
}

/// How the value travels from one breakpoint to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum CurveKind {
    /// Straight-line interpolation to the next breakpoint.
    #[default]
    Linear,
    /// Hold this breakpoint's value until the next breakpoint (discrete / bool
    /// params like mute, or stepped plugin params).
    Stepped,
}

/// A single automation point: a normalized value at a timeline frame, plus the
/// curve used to reach the *next* point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Breakpoint {
    /// Timeline position in sample frames.
    pub time_frames: u64,
    /// Normalized lane value, `0.0..=1.0`.
    pub value: f32,
    /// Curve from this point to the next one.
    pub curve: CurveKind,
}

impl Breakpoint {
    /// Build a breakpoint, clamping `value` into `0.0..=1.0`.
    pub fn new(time_frames: u64, value: f32, curve: CurveKind) -> Self {
        Self {
            time_frames,
            value: value.clamp(0.0, 1.0),
            curve,
        }
    }
}

/// An automation lane: an ordered set of breakpoints driving one target.
///
/// `points` is kept sorted ascending by `time_frames`; use [`AutomationLane::new`]
/// or [`AutomationLane::insert_point`] to maintain the invariant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationLane {
    pub id: LaneId,
    pub target: AutomationTarget,
    /// When false the lane is "Read off": the static engine value is used and
    /// the points are kept for later.
    pub enabled: bool,
    /// Breakpoints, sorted ascending by `time_frames`.
    pub points: Vec<Breakpoint>,
}

impl AutomationLane {
    /// Create a lane, sorting `points` by time so the invariant holds even if
    /// the caller passed them out of order.
    pub fn new(id: LaneId, target: AutomationTarget, points: Vec<Breakpoint>) -> Self {
        let mut lane = Self {
            id,
            target,
            enabled: true,
            points,
        };
        lane.sort_points();
        lane
    }

    /// Re-establish the sorted-by-time invariant (stable, so equal-time points
    /// keep insertion order).
    pub fn sort_points(&mut self) {
        self.points.sort_by_key(|p| p.time_frames);
    }

    /// Insert a breakpoint at the correct position to keep `points` sorted.
    pub fn insert_point(&mut self, point: Breakpoint) {
        let idx = self
            .points
            .partition_point(|p| p.time_frames <= point.time_frames);
        self.points.insert(idx, point);
    }

    /// Sample the normalized lane value at `frame` (see [`sample_lane`]).
    pub fn sample(&self, frame: u64) -> f32 {
        sample_lane(&self.points, frame)
    }

    /// Sample and map straight to the target's real range (gain dB, pan, mute).
    /// For `PluginParam` this returns the raw normalized value — the engine
    /// scales it with the plugin's `min..=max` via [`lane_value_to_plugin_param`].
    pub fn real_value_at(&self, frame: u64) -> f32 {
        lane_value_to_real(self.target, self.sample(frame))
    }
}

/// Sample a sorted breakpoint list at `frame`, returning the interpolated
/// normalized value.
///
/// - Empty list ⇒ `0.0`.
/// - Before the first / after the last point ⇒ that point's value (clamped).
/// - Between two points: `Linear` interpolates; `Stepped` holds the left value.
///
/// `points` must be sorted ascending by `time_frames` (the [`AutomationLane`]
/// constructors guarantee this). Allocation-free.
pub fn sample_lane(points: &[Breakpoint], frame: u64) -> f32 {
    match points {
        [] => 0.0,
        [single] => single.value,
        _ => {
            let first = &points[0];
            let last = &points[points.len() - 1];
            if frame <= first.time_frames {
                return first.value;
            }
            if frame >= last.time_frames {
                return last.value;
            }
            // `frame` is strictly inside the span; find the segment whose left
            // point is the last one at or before `frame`.
            let right = points.partition_point(|p| p.time_frames <= frame);
            let a = &points[right - 1];
            let b = &points[right];
            match a.curve {
                CurveKind::Stepped => a.value,
                CurveKind::Linear => {
                    let span = (b.time_frames - a.time_frames) as f32;
                    // span > 0: equal-time duplicates can't both land here
                    // because `a.time_frames <= frame < b.time_frames`.
                    let t = (frame - a.time_frames) as f32 / span;
                    a.value + (b.value - a.value) * t
                }
            }
        }
    }
}

/// Map a normalized lane value to the target's real value.
///
/// - gain targets ⇒ decibels in `GAIN_MIN_DB..=GAIN_MAX_DB`,
/// - pan targets ⇒ `-1.0..=1.0`,
/// - mute targets ⇒ `0.0` or `1.0` (threshold at `0.5`),
/// - `PluginParam` ⇒ the value unchanged (the engine applies the plugin's
///   own range via [`lane_value_to_plugin_param`]).
pub fn lane_value_to_real(target: AutomationTarget, value: f32) -> f32 {
    let v = value.clamp(0.0, 1.0);
    match target {
        AutomationTarget::TrackGain(_)
        | AutomationTarget::BusGain(_)
        | AutomationTarget::MasterGain => GAIN_MIN_DB + v * (GAIN_MAX_DB - GAIN_MIN_DB),
        AutomationTarget::TrackPan(_) | AutomationTarget::BusPan(_) => v * 2.0 - 1.0,
        AutomationTarget::TrackMute(_) | AutomationTarget::BusMute(_) => {
            if v >= 0.5 {
                1.0
            } else {
                0.0
            }
        }
        AutomationTarget::PluginParam { .. } => v,
    }
}

/// Inverse of [`lane_value_to_real`]: map a target's real value back to a
/// normalized `0.0..=1.0` lane value. Round-trips with `lane_value_to_real`
/// for continuous targets; mute snaps to `0.0`/`1.0`.
pub fn real_to_lane_value(target: AutomationTarget, real: f32) -> f32 {
    let v = match target {
        AutomationTarget::TrackGain(_)
        | AutomationTarget::BusGain(_)
        | AutomationTarget::MasterGain => {
            (real - GAIN_MIN_DB) / (GAIN_MAX_DB - GAIN_MIN_DB)
        }
        AutomationTarget::TrackPan(_) | AutomationTarget::BusPan(_) => (real + 1.0) / 2.0,
        AutomationTarget::TrackMute(_) | AutomationTarget::BusMute(_) => {
            if real >= 0.5 {
                1.0
            } else {
                0.0
            }
        }
        AutomationTarget::PluginParam { .. } => real,
    };
    v.clamp(0.0, 1.0)
}

/// Map a normalized lane value to a CLAP plugin parameter's real value given
/// its `min..=max` (from the plugin's `ParamInfo`). Linear in the parameter's
/// own range.
pub fn lane_value_to_plugin_param(value: f32, min: f64, max: f64) -> f64 {
    let v = value.clamp(0.0, 1.0) as f64;
    min + v * (max - min)
}

/// Inverse of [`lane_value_to_plugin_param`]: map a plugin parameter's real
/// value back to a normalized `0.0..=1.0` lane value. A degenerate
/// `min == max` range maps everything to `0.0`.
pub fn plugin_param_to_lane_value(real: f64, min: f64, max: f64) -> f32 {
    if max == min {
        return 0.0;
    }
    (((real - min) / (max - min)) as f32).clamp(0.0, 1.0)
}
