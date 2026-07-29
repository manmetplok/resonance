//! `generate.*` — run the app's generators into a section + track.

use crate::ids::{ClipId, SectionDefinitionId, TrackId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `generate.part` — generate a pad/bass/lead part from the section's
/// chords ([`PartParams`] -> [`GenerateResult`]).
pub const PART: &str = "generate.part";
/// `generate.drums` — generate a drum pattern for a section
/// ([`DrumsParams`] -> [`GenerateResult`]).
pub const DRUMS: &str = "generate.drums";

/// All `generate.*` method names.
pub const METHODS: &[&str] = &[PART, DRUMS];

/// Melodic generator roles, lowercase on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerateRole {
    Pad,
    Bass,
    Lead,
}

/// Params for `generate.part`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartParams {
    pub section_id: SectionDefinitionId,
    pub track_id: TrackId,
    pub role: GenerateRole,
    /// Number of chords to generate over; defaults to the section's grid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chord_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beats_per_chord: Option<f64>,
    /// Include sevenths in generated voicings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sevenths: Option<bool>,
    /// RNG seed for reproducible output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Per-role generator options, passed through to `GenerateParams`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Value>,
}

/// Params for `generate.drums`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrumsParams {
    pub section_id: SectionDefinitionId,
    pub track_id: TrackId,
    /// Named pattern/style; omitted picks the section's arrangement default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

/// Result of `generate.part` / `generate.drums`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerateResult {
    /// The clip the material landed in, when the generator produced one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_id: Option<ClipId>,
    pub revision: u64,
}
