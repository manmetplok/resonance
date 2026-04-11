use resonance_music_theory::{Chord, Scale};
use serde::{Deserialize, Serialize};

use crate::compose::GenerateParams;

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
    #[serde(default)]
    pub generate_params: GenerateParams,
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
