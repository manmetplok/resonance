//! `section.*` — section definitions and arrangement placements.
//! Chords on a section's grid live under `harmony.*`.

use crate::common::KeyScale;
use crate::ids::{SectionDefinitionId, SectionPlacementId};
use serde::{Deserialize, Serialize};

/// `section.create` — create a definition ([`CreateParams`] -> [`CreateResult`]).
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

/// All `section.*` method names.
pub const METHODS: &[&str] = &[
    CREATE,
    RENAME,
    RESIZE,
    DELETE,
    PLACE,
    REMOVE_PLACEMENT,
    SET_SCALE,
];

/// Params for `section.create`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateParams {
    pub name: String,
    pub length_bars: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<KeyScale>,
}

/// Result of `section.create`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateResult {
    pub section_id: SectionDefinitionId,
    pub revision: u64,
}

/// Params for `section.rename`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameParams {
    pub section_id: SectionDefinitionId,
    pub name: String,
}

/// Params for `section.resize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResizeParams {
    pub section_id: SectionDefinitionId,
    pub length_bars: u32,
}

/// Params for `section.delete`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteParams {
    pub section_id: SectionDefinitionId,
    /// Required (`true`); the error otherwise summarizes what would be lost.
    #[serde(default)]
    pub confirm: bool,
}

/// Params for `section.place`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceParams {
    pub definition_id: SectionDefinitionId,
    /// 1-based bar to place the section at.
    pub start_bar: u32,
}

/// Result of `section.place`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceResult {
    pub placement_id: SectionPlacementId,
    pub revision: u64,
}

/// Params for `section.remove_placement`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovePlacementParams {
    pub placement_id: SectionPlacementId,
}

/// Params for `section.set_scale`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetScaleParams {
    pub section_id: SectionDefinitionId,
    pub scale: KeyScale,
}
