use std::collections::HashMap;

use resonance_audio::types::TrackId;
use resonance_music_theory::{Chord, GeneratedMaterial, GeneratorSpec, MotifParams, MotifSource, Scale};
use serde::{Deserialize, Deserializer, Serialize};

use crate::compose::{GenerateParams, LaneGeneratorConfig};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// LEGACY field: written by project files predating the drum
    /// arrangement (epic #38). New saves leave this `None` and persist the
    /// full ordered arrangement in [`arrangement`](Self::arrangement)
    /// instead. On load, a legacy `Some(id)` (with an empty `arrangement`)
    /// migrates to a single-entry arrangement tiling the whole section —
    /// see `ComposeState::load_from_project`.
    #[serde(default)]
    pub drum_pattern_id: Option<u64>,
    /// Ordered drum arrangement for the section: the sequence of pattern
    /// entries the drums play across its bars (epic #38). Empty on legacy
    /// projects — the loader then falls back to
    /// [`drum_pattern_id`](Self::drum_pattern_id) (a single entry) or, if
    /// that is also absent, an empty arrangement meaning "use the project
    /// default pattern". Defaulted so older project files load unchanged.
    #[serde(default)]
    pub arrangement: Vec<ProjectPatternEntry>,
}

/// Persisted form of [`crate::compose::PatternEntry`] — one entry in a
/// section's ordered drum arrangement. A plain serde mirror so the
/// runtime type stays free of serialization concerns (matching the
/// `ProjectSectionChord` ↔ `ChordState` split).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPatternEntry {
    /// Pattern played for the bulk of this entry.
    pub pattern_id: u64,
    /// How long the entry lasts (repeat count or fixed bar span).
    pub length: ProjectEntryLength,
    /// Optional fill pattern swapped in on the entry's last bar. Omitted
    /// from JSON when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<u64>,
}

/// Persisted form of [`crate::compose::EntryLength`]. Serializes
/// externally tagged — `{"RepeatN": 3}` / `{"Bars": 4}` — a stable,
/// self-describing shape that round-trips both length modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectEntryLength {
    /// Repeat the entry's pattern `n` times; concrete span is
    /// `n * pattern.length_bars`.
    RepeatN(u32),
    /// Occupy a fixed number of bars regardless of the pattern's
    /// intrinsic bar length.
    Bars(u32),
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

// ---- Arrangement <-> persisted form -----------------------------------
//
// `From` conversions in both directions so `to_project_definitions` /
// `load_from_project` can map an arrangement to/from disk without leaking
// serde onto the runtime `PatternEntry` / `EntryLength` types.

impl From<crate::compose::EntryLength> for ProjectEntryLength {
    fn from(length: crate::compose::EntryLength) -> Self {
        match length {
            crate::compose::EntryLength::RepeatN(n) => ProjectEntryLength::RepeatN(n),
            crate::compose::EntryLength::Bars(b) => ProjectEntryLength::Bars(b),
        }
    }
}

impl From<ProjectEntryLength> for crate::compose::EntryLength {
    fn from(length: ProjectEntryLength) -> Self {
        match length {
            ProjectEntryLength::RepeatN(n) => crate::compose::EntryLength::RepeatN(n),
            ProjectEntryLength::Bars(b) => crate::compose::EntryLength::Bars(b),
        }
    }
}

impl From<&crate::compose::PatternEntry> for ProjectPatternEntry {
    fn from(entry: &crate::compose::PatternEntry) -> Self {
        ProjectPatternEntry {
            pattern_id: entry.pattern_id,
            length: entry.length.into(),
            fill: entry.fill,
        }
    }
}

impl From<&ProjectPatternEntry> for crate::compose::PatternEntry {
    fn from(entry: &ProjectPatternEntry) -> Self {
        crate::compose::PatternEntry {
            pattern_id: entry.pattern_id,
            length: entry.length.into(),
            fill: entry.fill,
        }
    }
}

/// One entry of the compose section→clip map: the clip the compose model
/// generated for `track_id`'s lane of `placement_id` (a placement of
/// `definition_id`). See [`crate::project::ProjectFile::derived_clips`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDerivedClip {
    pub definition_id: u64,
    pub placement_id: u64,
    pub track_id: TrackId,
    pub clip_id: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSectionPlacement {
    pub id: u64,
    pub definition_id: u64,
    /// Zero-based bar index from project start.
    pub start_bar: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSectionChord {
    pub id: u64,
    /// Beats from section start.
    pub start_beat: u32,
    /// Length in beats; must be >= 1.
    pub duration_beats: u32,
    pub chord: Chord,
}
