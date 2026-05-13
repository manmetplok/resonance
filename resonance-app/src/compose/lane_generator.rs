//! Per-lane generator configuration. Each track in a section can carry
//! a generator (Bass / Melody / Pad / Drum) keyed by `TrackId`; absent
//! entries mean the lane is manual.

use std::collections::HashMap;

use resonance_music_theory::{BassParams, MelodyParams, PadParams, VocalParams};
use serde::{Deserialize, Serialize};

/// Persisted per-track generator configuration within a section. Absent
/// entry in the map = Manual (no generator for that lane).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaneGeneratorConfig {
    pub kind: LaneGeneratorKind,
    pub seed: u64,
}

/// What kind of generator drives a lane.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LaneGeneratorKind {
    Bass(BassParams),
    Melody(MelodyParams),
    Pad(PadParams),
    Drum(DrumLaneConfig),
    /// Vocal generator — lyrics + melody + voice/delivery. Persisted in
    /// the lane config so the right rail can be repainted from saved
    /// state. Generation itself is stubbed in
    /// `resonance_music_theory::derive_vocal` for now.
    Vocal(VocalParams),
}

/// Per-voice euclidean configuration for a drum lane.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DrumLaneConfig {
    /// Keyed by pad index.
    #[serde(deserialize_with = "deserialize_usize_keys", default)]
    pub voices: HashMap<usize, DrumVoiceMode>,
}

/// serde_json serializes `HashMap<usize, V>` with string keys but its
/// default `Deserialize` for `usize` rejects string keys on some platforms.
/// This helper parses them back.
fn deserialize_usize_keys<'de, V, D>(deserializer: D) -> Result<HashMap<usize, V>, D::Error>
where
    V: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    let string_map: HashMap<String, V> = HashMap::deserialize(deserializer)?;
    string_map
        .into_iter()
        .map(|(k, v)| {
            k.parse::<usize>()
                .map(|k| (k, v))
                .map_err(serde::de::Error::custom)
        })
        .collect()
}

/// Whether a single drum voice is manually edited, euclidean-generated,
/// or rhythmically locked to the section's shared motif.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode")]
pub enum DrumVoiceMode {
    Manual,
    Euclidean {
        steps: u32,
        hits: u32,
        rotation: i32,
    },
    /// Each onset of the section's shared motif fires this drum voice.
    /// Accented motif notes get a velocity boost. No tunable parameters
    /// — the motif identity is owned by `SectionDefinitionState::motif`,
    /// and edits there propagate via the regular `propagate_motif_change`
    /// path.
    Motif,
}

/// Tag-only enum used in the UI dropdown for selecting a generator kind
/// without carrying the full params.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneGeneratorKindTag {
    Manual,
    Bass,
    Melody,
    Pad,
    Vocal,
}
