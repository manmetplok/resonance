//! `harmony.*` — chords on a section definition's grid, plus
//! music-theory progression application.

use crate::common::{KeyScale, VocalLaneAffected, VocalMelodyChoice};
use crate::ids::{ChordId, SectionDefinitionId};
use serde::{Deserialize, Serialize};

/// `harmony.add_chord` — add a chord ([`AddChordParams`] -> [`AddChordResult`]).
pub const ADD_CHORD: &str = "harmony.add_chord";
/// `harmony.edit_chord` — change symbol/position/length of a chord
/// ([`EditChordParams`] -> [`ChordEditResult`]).
pub const EDIT_CHORD: &str = "harmony.edit_chord";
/// `harmony.delete_chord` — remove a chord
/// ([`DeleteChordParams`] -> [`ChordEditResult`]).
pub const DELETE_CHORD: &str = "harmony.delete_chord";
/// `harmony.apply_progression` — render a whole progression onto a
/// section's grid ([`ApplyProgressionParams`] -> [`ApplyProgressionResult`]).
pub const APPLY_PROGRESSION: &str = "harmony.apply_progression";

/// All `harmony.*` method names.
pub const METHODS: &[&str] = &[ADD_CHORD, EDIT_CHORD, DELETE_CHORD, APPLY_PROGRESSION];

/// Params for `harmony.add_chord`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AddChordParams {
    pub section_id: SectionDefinitionId,
    /// Section-relative beat the chord starts on (0-based).
    pub start_beat: f64,
    pub duration_beats: f64,
    /// Chord symbol, e.g. `"Am7"`.
    pub symbol: String,
    /// What to do with a vocal lane in this section whose melody the edit
    /// would change: `keep` (default — notes untouched, lane flagged
    /// `melody_predates_chords`), `refit` (pitches moved to fit the new
    /// chords, rhythm and lyrics kept) or `regenerate` (re-derived from
    /// the generator; hand edits lost). Omitted: the project's remembered
    /// choice, else `keep`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vocal_melody: Option<VocalMelodyChoice>,
}

/// Result of `harmony.add_chord`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddChordResult {
    pub chord_id: ChordId,
    pub revision: u64,
    /// The section's vocal lanes whose melody the edit would have
    /// changed, and what was done with each. Empty when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vocal_lanes_affected: Vec<VocalLaneAffected>,
}

/// Result of `harmony.edit_chord` / `harmony.delete_chord`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ChordEditResult {
    pub revision: u64,
    /// See [`AddChordResult::vocal_lanes_affected`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vocal_lanes_affected: Vec<VocalLaneAffected>,
}

/// Params for `harmony.edit_chord`; omitted fields stay unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct EditChordParams {
    pub section_id: SectionDefinitionId,
    pub chord_id: ChordId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_beat: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_beats: Option<f64>,
    /// What to do with a vocal lane in this section whose melody the edit
    /// would change: `keep` (default — notes untouched, lane flagged
    /// `melody_predates_chords`), `refit` (pitches moved to fit the new
    /// chords, rhythm and lyrics kept) or `regenerate` (re-derived from
    /// the generator; hand edits lost). Omitted: the project's remembered
    /// choice, else `keep`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vocal_melody: Option<VocalMelodyChoice>,
}

/// Params for `harmony.delete_chord`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DeleteChordParams {
    pub section_id: SectionDefinitionId,
    pub chord_id: ChordId,
    /// What to do with a vocal lane in this section whose melody the edit
    /// would change: `keep` (default — notes untouched, lane flagged
    /// `melody_predates_chords`), `refit` (pitches moved to fit the new
    /// chords, rhythm and lyrics kept) or `regenerate` (re-derived from
    /// the generator; hand edits lost). Omitted: the project's remembered
    /// choice, else `keep`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vocal_melody: Option<VocalMelodyChoice>,
}

/// Params for `harmony.apply_progression`.
///
/// Provide either explicit chord `symbols` (`["Am7","Dm7","G7","Cmaj7"]`)
/// or a music-theory request: `key` + `numerals` (`["i","iv","v"]`) or a
/// named `preset`. The progression replaces the section's existing
/// chords.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ApplyProgressionParams {
    pub section_id: SectionDefinitionId,
    /// Explicit chord symbols, laid out left to right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbols: Option<Vec<String>>,
    /// Key for numeral/preset requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<KeyScale>,
    /// Roman-numeral degrees, e.g. `["i","VI","III","VII"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numerals: Option<Vec<String>>,
    /// Named progression preset from `resonance-music-theory`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Beats per chord; defaults to one bar per chord.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beats_per_chord: Option<f64>,
    /// Add sevenths when rendering numerals/presets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sevenths: Option<bool>,
    /// What to do with a vocal lane in this section whose melody the edit
    /// would change: `keep` (default — notes untouched, lane flagged
    /// `melody_predates_chords`), `refit` (pitches moved to fit the new
    /// chords, rhythm and lyrics kept) or `regenerate` (re-derived from
    /// the generator; hand edits lost). Omitted: the project's remembered
    /// choice, else `keep`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vocal_melody: Option<VocalMelodyChoice>,
}

impl ApplyProgressionParams {
    /// Params selecting the given section with everything else omitted.
    pub fn for_section(section_id: SectionDefinitionId) -> Self {
        Self {
            section_id,
            symbols: None,
            key: None,
            numerals: None,
            preset: None,
            beats_per_chord: None,
            sevenths: None,
            vocal_melody: None,
        }
    }
}

/// Result of `harmony.apply_progression`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ApplyProgressionResult {
    /// Ids of the chords now on the grid, in order.
    pub chord_ids: Vec<ChordId>,
    pub revision: u64,
    /// See [`AddChordResult::vocal_lanes_affected`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vocal_lanes_affected: Vec<VocalLaneAffected>,
}
