//! `section.*` — section definitions and arrangement placements.
//! Chords on a section's grid live under `harmony.*`.

use crate::common::KeyScale;
use crate::ids::{SectionDefinitionId, SectionPlacementId, TrackId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `section.create` — create a definition ([`CreateParams`] -> [`CreateResult`]).
///
/// Also **places** the new definition on the timeline, after the last
/// existing placement, unless [`CreateParams::place`] is `false`. That
/// implicit placement is not implied by the name and is the first thing
/// a client trips over: `section.create` then `section.place` fails
/// "Placement would overlap an existing section", because the section is
/// already there. Pass `place: false` to build a definition library and
/// position every section deliberately.
pub const CREATE: &str = "section.create";
/// `section.rename` — rename a definition ([`RenameParams`] -> `MutationAck`).
pub const RENAME: &str = "section.rename";
/// `section.resize` — change a definition's length
/// ([`ResizeParams`] -> `MutationAck`).
pub const RESIZE: &str = "section.resize";
/// `section.delete` — delete a definition and its placements;
/// destructive, requires `"confirm": true` ([`DeleteParams`] -> `MutationAck`).
pub const DELETE: &str = "section.delete";
/// `section.place` — place a definition in the arrangement
/// ([`PlaceParams`] -> [`PlaceResult`]).
pub const PLACE: &str = "section.place";
/// `section.remove_placement` — remove one placement (the definition
/// survives) ([`RemovePlacementParams`] -> `MutationAck`).
pub const REMOVE_PLACEMENT: &str = "section.remove_placement";
/// `section.set_scale` — set a definition's key/scale
/// ([`SetScaleParams`] -> `MutationAck`).
pub const SET_SCALE: &str = "section.set_scale";
/// `section.set_lane_generator` — configure (or clear) the generator on a
/// `(section definition, track)` lane
/// ([`SetLaneGeneratorParams`] -> [`SetLaneGeneratorResult`]).
pub const SET_LANE_GENERATOR: &str = "section.set_lane_generator";

/// All `section.*` method names.
pub const METHODS: &[&str] = &[
    CREATE,
    RENAME,
    RESIZE,
    DELETE,
    PLACE,
    REMOVE_PLACEMENT,
    SET_SCALE,
    SET_LANE_GENERATOR,
];

/// Params for `section.create`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CreateParams {
    pub name: String,
    /// Length in bars, 1..=100000 ([`crate::MAX_BARS`]).
    pub length_bars: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<KeyScale>,
    /// Also place the definition on the timeline, after the last
    /// existing placement. Defaults to `true` (the historical
    /// behaviour). Pass `false` to create the definition only, then
    /// position it with `section.place` — otherwise that `section.place`
    /// is rejected as overlapping the implicit placement.
    #[serde(default = "default_place")]
    pub place: bool,
}

fn default_place() -> bool {
    true
}

/// Result of `section.create`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CreateResult {
    pub section_id: SectionDefinitionId,
    pub revision: u64,
}

/// Params for `section.rename`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RenameParams {
    pub section_id: SectionDefinitionId,
    pub name: String,
}

/// Params for `section.resize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ResizeParams {
    pub section_id: SectionDefinitionId,
    /// New length in bars, 1..=100000 ([`crate::MAX_BARS`]).
    pub length_bars: u32,
}

/// Params for `section.delete`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DeleteParams {
    pub section_id: SectionDefinitionId,
    /// Required (`true`); the error otherwise summarizes what would be lost.
    #[serde(default)]
    pub confirm: bool,
}

/// Params for `section.place`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PlaceParams {
    pub definition_id: SectionDefinitionId,
    /// 1-based bar to place the section at, at most 100000
    /// ([`crate::MAX_BARS`]).
    pub start_bar: u32,
}

/// Result of `section.place`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PlaceResult {
    pub placement_id: SectionPlacementId,
    pub revision: u64,
}

/// Params for `section.remove_placement`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RemovePlacementParams {
    pub placement_id: SectionPlacementId,
}

/// Params for `section.set_scale`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetScaleParams {
    pub section_id: SectionDefinitionId,
    pub scale: KeyScale,
}

/// Lane generator kinds, lowercase on the wire. Mirrors the app's
/// `LaneGeneratorKindTag`; `manual` clears any generator (the lane's
/// notes become hand-authored). Drum lanes are configured via
/// `generate.drums` and are intentionally not a variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum LaneKind {
    Manual,
    Bass,
    Melody,
    Pad,
    Vocal,
}

/// Params for `section.set_lane_generator`: configure (or clear) the
/// generator on the lane for a `(section definition, track)` pair.
/// Idempotent.
///
/// # Vocals span sections, not the whole song
///
/// Lyrics and the SVS voice live per **(section definition, track)**
/// vocal lane — a vocal generator installed here only covers this one
/// section on this one track. `vocal.set_lyrics` resolves a track's
/// **first** vocal lane in placement order, so a song that sings in four
/// sections needs **four vocal tracks**: configure one `vocal` lane per
/// `(section, track)`, then target each with `vocal.set_lyrics` /
/// `vocal.render`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetLaneGeneratorParams {
    pub section_id: SectionDefinitionId,
    pub track_id: TrackId,
    /// The generator to install; `manual` removes any existing generator.
    pub kind: LaneKind,
    /// RNG seed for reproducible output; omitted derives a deterministic
    /// per-kind seed from the section id (matching the GUI default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Per-kind generator params, deserialized into the app's
    /// `BassParams` / `MelodyParams` / `PadParams` / `VocalParams`.
    /// Absent uses generator defaults; ignored for `manual`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Value>,
}

/// Result of `section.set_lane_generator`: the revision counter after the
/// edit (a [`crate::common::MutationAck`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetLaneGeneratorResult {
    pub revision: u64,
}
