//! `automation.*` — parameter automation lanes (automation-control-api.md).
//!
//! A lane drives ONE target over time: a track / bus / master fader, a
//! track or bus pan or mute, or one parameter of one plugin. The model
//! keeps **one lane per target**, so a target is also the lane's address:
//! every method names the lane it reads or writes with the same
//! [`AutomationTargetSpec`] fields, flattened into its params.
//!
//! # Values
//!
//! A value is in the target's **real units** unless the call passes
//! `normalized: true`, which applies to the whole call:
//!
//! | target | real unit | accepted |
//! |---|---|---|
//! | `control: "volume"` | dB | `-60..=6`, or `"-inf"` (both are silence) |
//! | `control: "pan"` | `-1..=1` | number |
//! | `control: "mute"` | bool | `true` / `false` (also `0` / `1`) |
//! | `param` | the plugin's own `min..=max` | number, or a choice label |
//!
//! Plugin lanes are linear in the plugin's PLAIN units, so a sweep that
//! should sound even on a skewed parameter (a filter cutoff in Hz) needs
//! many points — `automation.shape` writes them.
//!
//! # Positions
//!
//! Point positions are [`PositionSpec`]s — 1-based, meter-aware
//! `{bar, beat}` or an absolute `{sample}` — and read back as
//! [`SongPosition`]s, so a reported position can be sent straight back.
//! A lane point is anchored at a SAMPLE: `transport.set_tempo` and
//! `arrangement.insert/remove_bars` re-anchor lanes musically, every
//! `global.*` tempo / meter event edit leaves them at their sample (so
//! their bar position moves), exactly like clips and markers.
//!
//! # Methods
//!
//! Every mutating method replies [`LaneEditResult`]: the revision plus
//! the lane exactly as it now reads back. The param/result types of all
//! seven methods are defined here; [`METHODS`] lists only the ones the
//! app implements today (see its doc).

use crate::common::{PositionSpec, SongPosition};
use crate::ids::TrackId;
use serde::{Deserialize, Serialize};

/// `automation.lanes` — read every lane, or a filtered subset
/// ([`LanesParams`] → [`LanesResult`]). Read-only.
pub const LANES: &str = "automation.lanes";
/// `automation.set_lane` — create a lane or replace ALL of its points
/// ([`SetLaneParams`] → [`LaneEditResult`]).
pub const SET_LANE: &str = "automation.set_lane";
/// `automation.add_points` — insert points, upserting an occupied frame
/// ([`AddPointsParams`] → [`LaneEditResult`] with `replaced`).
pub const ADD_POINTS: &str = "automation.add_points";
/// `automation.delete_points` — delete by index or by range
/// ([`DeletePointsParams`] → [`LaneEditResult`] with `deleted`).
pub const DELETE_POINTS: &str = "automation.delete_points";
/// `automation.set_enabled` — the lane's Read flag, declaratively
/// ([`SetEnabledParams`] → [`LaneEditResult`]).
pub const SET_ENABLED: &str = "automation.set_enabled";
/// `automation.remove_lane` — delete the whole lane
/// ([`RemoveLaneParams`] → [`LaneEditResult`] with `removed`).
pub const REMOVE_LANE: &str = "automation.remove_lane";
/// `automation.shape` — generate a ramp / curve / LFO over a range
/// ([`ShapeParams`] → [`LaneEditResult`] with `seed`).
pub const SHAPE: &str = "automation.shape";

/// The `automation.*` methods the app IMPLEMENTS — what `control.hello`
/// advertises and what gets an MCP tool.
///
/// Built slice by slice (automation-control-api.md §6): a method joins
/// this list in the same change that lands its app handler and its MCP
/// tool, never before — `combined_router_exposes_every_control_method`
/// holds the tool side to it. [`ADD_POINTS`], [`DELETE_POINTS`],
/// [`SET_ENABLED`] and [`REMOVE_LANE`] have wire types below but are NOT
/// listed yet.
pub const METHODS: &[&str] = &[LANES, SET_LANE, SHAPE];

/// Most points one call may write (`set_lane`, `add_points`, `shape`).
pub const MAX_POINTS_PER_CALL: usize = 2_048;
/// Most points one lane may hold.
pub const MAX_POINTS_PER_LANE: usize = 10_000;

// ---------------------------------------------------------------------------
// Target
// ---------------------------------------------------------------------------

/// Which lane: exactly one OWNER (`track_id`, `bus_id` or `master:
/// true`) and exactly one of `control` (a mixer lane) or `param` (a
/// plugin lane).
///
/// `control` and `param` are separate fields on purpose: a plugin whose
/// parameter is called "Volume" would otherwise be ambiguous with the
/// fader.
///
/// Flattened into every method's params, so the wire form is
/// `{"track_id": 3, "param": "Filter Cutoff"}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AutomationTargetSpec {
    /// Owner: a track (ids from `song_summary`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
    /// Owner: a group / return bus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bus_id: Option<TrackId>,
    /// Owner: the master bus. Master has only a `volume` mixer lane.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub master: bool,
    /// A mixer lane: `volume`, `pan` or `mute` (master: `volume` only).
    /// `device` lanes (external-instrument parameters) are listed by
    /// `automation.lanes` but cannot be written here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<LaneControl>,
    /// A plugin lane: the parameter by name (case-insensitive), by its
    /// numeric CLAP id as a string, or by a first-party plugin's string
    /// key — exactly as `track_set_plugin_param` takes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    /// Plugin lanes only: the plugin's CLAP id. Omitted = the track's
    /// instrument (on a bus or the master: the first plugin in the chain).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Plugin lanes only: which of several same-id plugins (0-based;
    /// default 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

/// A mixer (non-plugin) lane kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum LaneControl {
    /// The fader, in dB.
    Volume,
    /// Stereo balance, `-1..=1`.
    Pan,
    /// Mute, a bool.
    Mute,
    /// A parameter of an external instrument's device definition. Read
    /// only here: listed by `automation.lanes`, refused as a write target.
    Device,
}

// ---------------------------------------------------------------------------
// Values and points
// ---------------------------------------------------------------------------

/// One lane value: a number, a bool (mute), or a string — `"-inf"` for
/// a silent volume, or a stepped plugin parameter's choice label.
///
/// Read back the same way: a volume at the floor reads `"-inf"`, a mute
/// lane reads `true` / `false`, everything else a number (a stepped
/// parameter's label is in the point's `text`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum AutomationValue {
    Number(f64),
    Bool(bool),
    Text(String),
}

/// How the value travels from a point to the NEXT one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum AutomationCurve {
    /// Straight line to the next point.
    Linear,
    /// Hold this value until the next point.
    Stepped,
}

/// An input point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PointSpec {
    /// `{bar, beat}` (1-based, meter-aware) or `{sample}`.
    pub position: PositionSpec,
    /// In the target's real units, or `0..=1` when the call is
    /// `normalized`.
    pub value: AutomationValue,
    /// Curve to the next point. Default: `stepped` for mute and stepped
    /// plugin parameters, `linear` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<AutomationCurve>,
}

/// A point as it reads back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PointView {
    /// 0-based index in the lane's time-sorted point list — the whole
    /// lane's, even when `automation.lanes` windowed it with `range`.
    pub index: u32,
    pub position: SongPosition,
    /// The real value (see [`AutomationValue`]); the normalized value
    /// when the lane's range is unknown (`status` other than `ok`, or a
    /// device lane).
    pub value: AutomationValue,
    /// The stored `0..=1` lane value.
    pub normalized: f64,
    /// The value for a reader: `"300 Hz"`, `"-inf dB"`, a choice label.
    pub text: String,
    pub curve: AutomationCurve,
}

/// A lane's health.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum LaneStatus {
    /// Drives its target.
    Ok,
    /// Its target is gone (a plugin instance or parameter that no longer
    /// exists; an older project file can carry these). Inert.
    Orphaned,
    /// The plugin is a missing-plugin placeholder; the lane is kept for
    /// when it loads again.
    PluginMissing,
    /// The plugin is still loading and has not reported its parameters
    /// yet. Retry shortly.
    PluginInitializing,
}

/// A lane's target as it reads back: the [`AutomationTargetSpec`] that
/// addresses it (pass it straight back to any `automation.*` method),
/// plus what the lookup resolved.
///
/// An `orphaned` plugin lane has no owner fields and no `plugin_id` —
/// the instance it named is gone — and reports `param` as the numeric
/// id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LaneTargetView {
    #[serde(flatten)]
    pub spec: AutomationTargetSpec,
    /// The plugin's display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_name: Option<String>,
    /// The parameter's numeric CLAP id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param_id: Option<u32>,
    /// The parameter's display name (`param` falls back to the id when
    /// the name would not resolve uniquely).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param_name: Option<String>,
}

/// One lane, in full.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LaneView {
    pub lane_id: u64,
    pub target: LaneTargetView,
    /// The Read flag: false means the lane is kept but not played.
    pub enabled: bool,
    pub status: LaneStatus,
    /// The real unit: `"dB"`, `"bool"`, the plugin's unit, or empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    /// The real value at normalized 0 (absent when unknown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// The real value at normalized 1 (absent when unknown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Points on the whole lane (`points` may be a `range` window).
    pub point_count: u32,
    pub points: Vec<PointView>,
}

/// One lane, compact: what `song.tracks` and `master.summary` list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LaneSummary {
    pub lane_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<LaneControl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// How many points the lane holds.
    pub points: u32,
    pub enabled: bool,
    pub status: LaneStatus,
}

/// A span of the timeline, half-open: `start` inclusive, `end` exclusive.
/// An omitted `start` is the song start; an omitted `end` runs past the
/// last point.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PointRange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<PositionSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<PositionSpec>,
}

// ---------------------------------------------------------------------------
// Method params / results
// ---------------------------------------------------------------------------

/// Params for `automation.lanes`. Every field narrows the listing; with
/// none, every lane is listed, orphans included. An owner filter lists
/// the lanes that owner carries (a plugin lane belongs to the track or
/// bus hosting the instance).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LanesParams {
    #[serde(flatten)]
    pub target: AutomationTargetSpec,
    /// Only report points inside this window (each point keeps its whole-
    /// lane `index`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<PointRange>,
}

/// Result of `automation.lanes`: lanes in (owner, kind, parameter) order
/// — tracks in mixer order, then busses, then master, then orphans.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LanesResult {
    pub revision: u64,
    pub lanes: Vec<LaneView>,
}

/// Params for `automation.set_lane`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetLaneParams {
    #[serde(flatten)]
    pub target: AutomationTargetSpec,
    /// The lane's COMPLETE new point list (at least one; at most
    /// [`MAX_POINTS_PER_CALL`]). Any order; two points on the same frame
    /// are refused.
    pub points: Vec<PointSpec>,
    /// Every `value` is `0..=1` rather than real units.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub normalized: bool,
    /// The Read flag. Omitted keeps an existing lane's flag, and a new
    /// lane starts enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// Params for `automation.add_points`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddPointsParams {
    #[serde(flatten)]
    pub target: AutomationTargetSpec,
    /// Points to insert. A point on a frame the lane already holds
    /// REPLACES that point; two inputs on one frame are refused.
    pub points: Vec<PointSpec>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub normalized: bool,
}

/// Params for `automation.delete_points`: give exactly one of `indices`
/// or `range`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DeletePointsParams {
    #[serde(flatten)]
    pub target: AutomationTargetSpec,
    /// Point indices as `automation.lanes` reports them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indices: Option<Vec<u32>>,
    /// Every point in this half-open window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<PointRange>,
    /// Required when the deletion would empty — and so remove — the lane.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub confirm: bool,
}

/// Params for `automation.set_enabled`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetEnabledParams {
    #[serde(flatten)]
    pub target: AutomationTargetSpec,
    /// The Read flag to SET (not toggle).
    pub enabled: bool,
}

/// Params for `automation.remove_lane`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RemoveLaneParams {
    #[serde(flatten)]
    pub target: AutomationTargetSpec,
    /// Required when the lane holds points.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub confirm: bool,
}

/// The curve `automation.shape` generates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    /// Straight line `from` → `to` (two points).
    Ramp,
    /// Geometric `from` → `to`; both must be > 0. Not for volume / pan /
    /// mute (dB is already logarithmic; use `ramp`).
    Exp,
    /// `from` ↔ `to`, `cycles` times.
    Sine,
    /// `from` ↔ `to` with exact corners.
    Triangle,
    /// Alternating `from` / `to`, stepped.
    Square,
    /// A stair `from` → `to`, stepped; `resolution` = steps per bar.
    Steps,
    /// A bounded, seeded random walk starting at `from`.
    RandomWalk,
}

/// Params for `automation.shape`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ShapeParams {
    #[serde(flatten)]
    pub target: AutomationTargetSpec,
    /// Where the curve starts, holding `from`.
    pub start: PositionSpec,
    /// The generated curve INCLUDES a point at `end` holding `to`; points
    /// already in `[start, end]` are replaced, points outside are kept.
    /// Must lie after `start`.
    pub end: PositionSpec,
    pub shape: ShapeKind,
    /// The value at `start`, in real units (or `0..=1` when `normalized`).
    pub from: AutomationValue,
    /// The value at `end`, in real units (or `0..=1` when `normalized`).
    pub to: AutomationValue,
    /// Whole periods over the range (`sine`, `triangle`, `square`; at
    /// least 1, default 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycles: Option<u32>,
    /// Points per bar (per-shape default: `exp` / `sine` 16, `steps` 1,
    /// `random_walk` 4; ignored by `ramp`, `triangle`, `square`). At
    /// least 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<u32>,
    /// `random_walk` seed (default 0); the seed used is echoed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub normalized: bool,
}

/// Result of every mutating `automation.*` method.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LaneEditResult {
    pub revision: u64,
    /// The lane exactly as it now reads back; `null` once removed.
    pub lane: Option<LaneView>,
    /// True when the call removed the lane (`remove_lane`, or a
    /// `delete_points` that emptied it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removed: bool,
    /// `add_points`: how many existing points were replaced in place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced: Option<u32>,
    /// `delete_points`: how many points were deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<u32>,
    /// `shape` with `random_walk`: the seed used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}
