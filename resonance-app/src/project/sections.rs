use std::collections::HashMap;

use resonance_audio::types::TrackId;
use resonance_music_theory::{Chord, GeneratedMaterial, GeneratorSpec, MotifParams, MotifSource, Scale};
use serde::{Deserialize, Deserializer, Serialize};

use crate::compose::{GenerateParams, LaneGeneratorConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSectionDefinition {
    pub id: u64,
    pub name: String,
    pub color: [u8; 3],
    pub length_bars: u32,
    #[serde(default)]
    pub chords: Vec<ProjectSectionChord>,
    #[serde(default)]
    pub scale: Option<Scale>,
    /// Seed for the per-section progression walker. Older project files
    /// load with seed 0, which the walker still accepts.
    #[serde(default)]
    pub progression_seed: u64,
    /// Per-section knobs for the progression + derive generators.
    /// Retained for backwards-compatible loading of old projects.
    #[serde(default)]
    pub generate_params: GenerateParams,
    /// Optional chord generator specification.
    #[serde(default)]
    pub generator_spec: Option<GeneratorSpec>,
    /// Seed for the chord generator.
    #[serde(default)]
    pub generator_seed: u64,
    /// Last materialized output from the chord generator.
    #[serde(default)]
    pub generated_material: Option<GeneratedMaterial>,
    /// Per-track generator configuration for this section.
    #[serde(default)]
    pub lane_generators: HashMap<TrackId, LaneGeneratorConfig>,
    /// Beats per chord — layout parameter for chord generation.
    #[serde(default = "default_beats_per_chord")]
    pub beats_per_chord: u32,
    /// Build seventh chords during generation.
    #[serde(default)]
    pub seventh_chords: bool,
    /// Section-shared motif. Either generated procedurally from
    /// `MotifParams` or hand-drawn by the user. The JSON field is named
    /// `motif` for backwards compatibility — older project files stored a
    /// flat `MotifParams` here and still deserialize into
    /// `MotifSource::Generated(...)`.
    #[serde(
        default,
        rename = "motif",
        deserialize_with = "deserialize_motif_source_compat"
    )]
    pub motif_source: MotifSource,
    /// Which entry in the project's drum-pattern bank this section uses.
    /// `None` on legacy projects (loads as "use the project default").
    #[serde(default)]
    pub drum_pattern_id: Option<u64>,
}

/// Accept both the historical `motif: { seed, complexity, motif_len,
/// leap_chance }` JSON shape and the current `motif: { Generated: {...} }`
/// or `motif: { Manual: {...} }` enum shape, mapping the legacy form to
/// `MotifSource::Generated`.
fn deserialize_motif_source_compat<'de, D>(deserializer: D) -> Result<MotifSource, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Source(MotifSource),
        Legacy(MotifParams),
    }

    Ok(match Either::deserialize(deserializer)? {
        Either::Source(s) => s,
        Either::Legacy(p) => MotifSource::Generated(p),
    })
}

fn default_beats_per_chord() -> u32 {
    4
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSectionPlacement {
    pub id: u64,
    pub definition_id: u64,
    /// Zero-based bar index from project start.
    pub start_bar: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSectionChord {
    pub id: u64,
    /// Beats from section start.
    pub start_beat: u32,
    /// Length in beats; must be >= 1.
    pub duration_beats: u32,
    pub chord: Chord,
}
