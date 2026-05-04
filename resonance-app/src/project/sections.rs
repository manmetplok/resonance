use std::collections::HashMap;

use resonance_audio::types::TrackId;
use resonance_music_theory::{Chord, GeneratedMaterial, GeneratorSpec, MotifParams, Scale};
use serde::{Deserialize, Serialize};

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
    /// Section-shared motif knobs. Older project files load with the
    /// default seed/complexity/etc.
    #[serde(default)]
    pub motif: MotifParams,
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
