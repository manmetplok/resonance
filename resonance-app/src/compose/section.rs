//! Section runtime state: the runtime mirror of `ProjectSectionDefinition`
//! plus its placements, chord events, and the inline new/edit-section forms.

use std::collections::HashMap;

use resonance_audio::types::TrackId;
use resonance_music_theory::{Chord, GeneratedMaterial, GeneratorSpec, MotifSource, Scale};

use super::generate::GenerateParams;
use super::lane_generator::LaneGeneratorConfig;

/// Which lane is currently focused in the Compose view. Determines what the
/// right-hand inspector panel shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectedLane {
    /// The chord strip at the top of the section.
    Chords,
    /// A synth (non-drum) instrument track.
    Instrument(TrackId),
    /// A drum track.
    Drums(TrackId),
}

/// Runtime mirror of `ProjectSectionDefinition`. Kept separate so future
/// runtime-only fields (e.g. editor UI state) can be added without touching
/// the persisted shape.
#[derive(Debug, Clone)]
pub struct SectionDefinitionState {
    pub id: u64,
    pub name: String,
    pub color: [u8; 3],
    pub length_bars: u32,
    pub chords: Vec<ChordState>,
    pub scale: Option<Scale>,
    /// Seed the progression walker uses for this section. Bumped by
    /// the "reroll" action so each click produces a new progression.
    pub progression_seed: u64,
    /// Persisted generator knobs — chord count, beats per chord,
    /// per-derive params (pad register, bass style, melody style).
    /// Retained for backwards-compatible loading of old projects;
    /// new code reads `generator_spec` + `lane_generators` instead.
    pub generate_params: GenerateParams,
    /// Optional chord generator specification (MarkovProgression).
    pub generator_spec: Option<GeneratorSpec>,
    /// Seed for the chord generator. Re-rolling increments this to
    /// produce a fresh progression from the same spec.
    pub generator_seed: u64,
    /// Last materialized output from the chord generator. Persisted so
    /// the section is fully reconstructable without re-running the
    /// generator. The `locked` flag on each chord carries through as
    /// both user intent and output.
    pub generated_material: Option<GeneratedMaterial>,
    /// Per-track generator configuration for this section. Keyed by
    /// TrackId. An absent entry means the lane is Manual (no generator).
    pub lane_generators: HashMap<TrackId, LaneGeneratorConfig>,
    /// Beats each chord occupies on the section grid. Kept at section
    /// level because it's a layout parameter, not a generator parameter.
    pub beats_per_chord: u32,
    /// Build diatonic seventh chords instead of triads during chord
    /// generation.
    pub seventh_chords: bool,
    /// Section-shared motif source. Either generated procedurally or
    /// hand-drawn by the user. Every motif-style lane in this section
    /// reads from this so they share the underlying motif identity
    /// (intervals + rhythm + accents).
    pub motif_source: MotifSource,
    /// Which drum pattern this section plays. `None` means "use the
    /// project default" — resolved via
    /// [`crate::compose::ComposeState::pattern_for_definition`].
    pub drum_pattern_id: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SectionPlacementState {
    pub id: u64,
    pub definition_id: u64,
    pub start_bar: u32,
}

#[derive(Debug, Clone)]
pub struct ChordState {
    pub id: u64,
    pub start_beat: u32,
    pub duration_beats: u32,
    pub chord: Chord,
}

#[derive(Debug, Clone)]
pub struct NewSectionForm {
    pub name: String,
    pub length_input: String,
    pub color: [u8; 3],
}

#[derive(Debug, Clone)]
pub struct EditSectionForm {
    pub definition_id: u64,
    pub name: String,
    pub length_input: String,
}
