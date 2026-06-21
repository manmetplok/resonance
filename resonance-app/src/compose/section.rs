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
    /// Ordered drum arrangement for this section: the sequence of pattern
    /// entries the drums play across the section's bars. An empty
    /// arrangement means "use the project default pattern for the whole
    /// section" — resolved via
    /// [`crate::compose::ComposeState::pattern_for_definition`]. The first
    /// entry's pattern is the section's "primary" choice (see
    /// [`SectionDefinitionState::primary_pattern_id`]); the full sequence
    /// is resolved into per-bar spans by
    /// [`crate::compose::ComposeState::resolve_arrangement_for`].
    pub arrangement: Vec<PatternEntry>,
}

/// How long a single [`PatternEntry`] lasts within a section's bar grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryLength {
    /// Repeat the entry's pattern `n` times back-to-back. The concrete
    /// bar span is `n * pattern.length_bars` — so a 2-bar pattern with
    /// `RepeatN(3)` occupies 6 bars. `RepeatN(0)` contributes nothing.
    RepeatN(u32),
    /// Occupy a fixed number of bars regardless of the pattern's own
    /// intrinsic bar length. The pattern loops/tiles to fill the span;
    /// `Bars(0)` contributes nothing.
    Bars(u32),
}

/// One entry in a section's ordered drum arrangement. Plays `pattern_id`
/// for `length` (see [`EntryLength`]), optionally swapping in `fill` on the
/// last bar of the entry's span — a one-bar fill capping a repeated loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternEntry {
    /// Pattern played for the bulk of this entry.
    pub pattern_id: u64,
    /// How long the entry lasts.
    pub length: EntryLength,
    /// Optional fill pattern that replaces the last bar of the entry's
    /// span. `None` means the entry plays `pattern_id` throughout.
    pub fill: Option<u64>,
}

impl PatternEntry {
    /// A plain entry that plays `pattern_id` once with no fill.
    pub fn once(pattern_id: u64) -> Self {
        Self {
            pattern_id,
            length: EntryLength::RepeatN(1),
            fill: None,
        }
    }
}

impl SectionDefinitionState {
    /// The section's "primary" drum pattern: the first arrangement
    /// entry's pattern, or `None` when the arrangement is empty (meaning
    /// "fall through to the project default"). Back-compat shim for
    /// callers that previously read the old `drum_pattern_id: Option<u64>`
    /// field — they resolve the first covered bar.
    pub fn primary_pattern_id(&self) -> Option<u64> {
        self.arrangement.first().map(|e| e.pattern_id)
    }

    /// Collapse the arrangement to a single-pattern entry, or clear it
    /// back to "use the default" when `pattern_id` is `None`. Back-compat
    /// shim for the old `drum_pattern_id = …` assignment; richer
    /// multi-entry arrangements are built directly via the `arrangement`
    /// field.
    pub fn set_primary_pattern(&mut self, pattern_id: Option<u64>) {
        self.arrangement = match pattern_id {
            Some(id) => vec![PatternEntry::once(id)],
            None => Vec::new(),
        };
    }

    /// Drop every arrangement entry (and fill) that references
    /// `pattern_id`. Used when a pattern is deleted from the bank so no
    /// entry points at a stale pattern. Entries whose *fill* matches lose
    /// just the fill; entries whose main pattern matches are removed.
    pub fn remove_pattern_references(&mut self, pattern_id: u64) {
        self.arrangement.retain(|e| e.pattern_id != pattern_id);
        for entry in &mut self.arrangement {
            if entry.fill == Some(pattern_id) {
                entry.fill = None;
            }
        }
    }
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
