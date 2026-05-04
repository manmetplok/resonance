use std::collections::HashMap;

use resonance_audio::types::{ClipId, TrackId};
use resonance_music_theory::{
    BassParams, Chord, GeneratedMaterial, GeneratorSpec, MelodyParams, MotifParams, PadParams,
    Scale,
};
use serde::{Deserialize, Serialize};

use crate::project::{ProjectSectionChord, ProjectSectionDefinition, ProjectSectionPlacement};

pub mod drumroll;
pub mod generate;
pub mod invariants;
pub mod messages;

pub use drumroll::DrumrollViewState;
pub use generate::{DeriveKind, GenerateParams};
pub use messages::ComposeMessage;

// ---------------------------------------------------------------------------
// Lane selection
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Per-lane generator configuration
// ---------------------------------------------------------------------------

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
    /// Section-shared motif knobs. Both melody-Motif and bass-Motif lanes
    /// in this section read from these so they share the underlying motif
    /// identity (intervals + rhythm + accents).
    pub motif: MotifParams,
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

#[derive(Debug)]
pub struct ComposeState {
    pub definitions: Vec<SectionDefinitionState>,
    pub placements: Vec<SectionPlacementState>,
    pub selected_placement_id: Option<u64>,
    pub scroll_y: f32,
    /// Monotonic id generator for definitions / placements / chords. A single
    /// counter is fine — ids only need to be unique within the project.
    pub next_id: u64,
    /// Last error produced by an invariant-violating operation, shown in the
    /// UI as a transient status. Cleared on next successful mutation.
    pub last_error: Option<String>,
    /// When `Some`, the inline "new section" form is visible and accepting
    /// input. `None` when the form is closed.
    pub new_section_form: Option<NewSectionForm>,
    /// When `Some`, the inline "edit section" form is visible. Only one of
    /// `new_section_form` and `edit_section_form` can be visible at a time.
    pub edit_section_form: Option<EditSectionForm>,
    /// Currently highlighted chord in the chord lane. The chord editor row
    /// only appears when this is set.
    pub selected_chord_id: Option<u64>,
    /// Which lane is focused in the Compose view. Determines what the
    /// right-hand inspector panel shows.
    pub selected_lane: SelectedLane,
    /// Transient UI state for the drumroll block (selected pad, euclidean
    /// form buffers, pad map). Not persisted.
    pub drumroll: DrumrollViewState,
    /// When `Some`, the indicated instrument track is shown in an
    /// expanded piano-roll view in the Compose tab. All other tracks
    /// collapse to minimal name-only strips while the editor is open.
    pub expanded_track_id: Option<resonance_audio::types::TrackId>,
    /// Vertical zoom (pixels per semitone row) for the expanded editor.
    pub expanded_zoom_y: f32,
    /// Horizontal scroll offset (pixels) for the expanded editor.
    pub expanded_scroll_x: f32,
    /// Vertical scroll offset (pixels) for the expanded editor.
    pub expanded_scroll_y: f32,
    /// Generated clips we created, keyed by (definition_id, placement_id,
    /// track_id). Runtime-only: rebuilt on project load by scanning clip
    /// names in `r.midi_clips`. The regeneration path uses this to delete
    /// old clips before issuing fresh ones.
    pub derived_clips: HashMap<(u64, u64, TrackId), ClipId>,
    /// Monotonic id used when allocating fresh `ClipId`s for derived
    /// clips. Kept in the high range so it never collides with engine-
    /// allocated ids coming from `CreateMidiClip`.
    pub next_derived_clip_id: u64,
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

/// Starting point for app-allocated derived clip ids. Chosen high enough
/// that it will never collide with engine-allocated clip ids (which count
/// up monotonically from 1 via `CreateMidiClip`), yet small enough to
/// leave headroom for a very long session. The engine bumps its own
/// allocator past any id seen via `LoadMidiClipDirect`, so using values
/// above this base is always safe.
pub const DERIVED_CLIP_ID_BASE: u64 = 1 << 40;

impl Default for ComposeState {
    fn default() -> Self {
        Self {
            definitions: Vec::new(),
            placements: Vec::new(),
            selected_placement_id: None,
            scroll_y: 0.0,
            next_id: 0,
            last_error: None,
            new_section_form: None,
            edit_section_form: None,
            selected_chord_id: None,
            selected_lane: SelectedLane::Chords,
            expanded_track_id: None,
            expanded_zoom_y: 12.0,
            expanded_scroll_x: 0.0,
            expanded_scroll_y: 0.0,
            drumroll: DrumrollViewState::default(),
            derived_clips: HashMap::new(),
            next_derived_clip_id: DERIVED_CLIP_ID_BASE,
        }
    }
}

impl ComposeState {
    /// Backwards-compatible accessor: returns the TrackId of the currently
    /// selected lane if it's an instrument or drum track.
    pub fn details_track_id(&self) -> Option<TrackId> {
        match &self.selected_lane {
            SelectedLane::Instrument(id) | SelectedLane::Drums(id) => Some(*id),
            SelectedLane::Chords => None,
        }
    }

    pub fn fresh_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Allocate a fresh clip id to use with `LoadMidiClipDirect`. See
    /// [`DERIVED_CLIP_ID_BASE`] for why these live in the high range.
    pub fn fresh_derived_clip_id(&mut self) -> ClipId {
        let id = self.next_derived_clip_id;
        self.next_derived_clip_id += 1;
        id
    }

    pub fn find_definition(&self, id: u64) -> Option<&SectionDefinitionState> {
        self.definitions.iter().find(|d| d.id == id)
    }

    pub fn find_definition_mut(&mut self, id: u64) -> Option<&mut SectionDefinitionState> {
        self.definitions.iter_mut().find(|d| d.id == id)
    }

    pub fn find_placement(&self, id: u64) -> Option<&SectionPlacementState> {
        self.placements.iter().find(|p| p.id == id)
    }

    pub fn selected_placement(&self) -> Option<&SectionPlacementState> {
        self.selected_placement_id
            .and_then(|id| self.find_placement(id))
    }

    /// Serialize the current state to the persisted project representation.
    pub fn to_project_definitions(&self) -> Vec<ProjectSectionDefinition> {
        self.definitions
            .iter()
            .map(|d| ProjectSectionDefinition {
                id: d.id,
                name: d.name.clone(),
                color: d.color,
                length_bars: d.length_bars,
                chords: d
                    .chords
                    .iter()
                    .map(|c| ProjectSectionChord {
                        id: c.id,
                        start_beat: c.start_beat,
                        duration_beats: c.duration_beats,
                        chord: c.chord,
                    })
                    .collect(),
                scale: d.scale,
                progression_seed: d.progression_seed,
                generate_params: d.generate_params.clone(),
                generator_spec: d.generator_spec.clone(),
                generator_seed: d.generator_seed,
                generated_material: d.generated_material.clone(),
                lane_generators: d.lane_generators.clone(),
                beats_per_chord: d.beats_per_chord,
                seventh_chords: d.seventh_chords,
                motif: d.motif,
            })
            .collect()
    }

    pub fn to_project_placements(&self) -> Vec<ProjectSectionPlacement> {
        self.placements
            .iter()
            .map(|p| ProjectSectionPlacement {
                id: p.id,
                definition_id: p.definition_id,
                start_bar: p.start_bar,
            })
            .collect()
    }

    /// Load state from a project file. Clears any existing state first.
    pub fn load_from_project(
        &mut self,
        definitions: &[ProjectSectionDefinition],
        placements: &[ProjectSectionPlacement],
    ) {
        self.definitions = definitions
            .iter()
            .map(|d| SectionDefinitionState {
                id: d.id,
                name: d.name.clone(),
                color: d.color,
                length_bars: d.length_bars,
                chords: d
                    .chords
                    .iter()
                    .map(|c| ChordState {
                        id: c.id,
                        start_beat: c.start_beat,
                        duration_beats: c.duration_beats,
                        chord: c.chord,
                    })
                    .collect(),
                scale: d.scale,
                progression_seed: d.progression_seed,
                generate_params: d.generate_params.clone(),
                generator_spec: d.generator_spec.clone(),
                generator_seed: d.generator_seed,
                generated_material: d.generated_material.clone(),
                lane_generators: d.lane_generators.clone(),
                beats_per_chord: d.beats_per_chord,
                seventh_chords: d.seventh_chords,
                motif: d.motif,
            })
            .collect();
        // Runtime-only state: start each load with an empty derived-clip
        // map. `update::project_io::replay_loaded_project` will rebuild
        // it by scanning clip names once the MIDI clips are in place.
        self.derived_clips.clear();
        self.next_derived_clip_id = DERIVED_CLIP_ID_BASE;
        self.placements = placements
            .iter()
            .map(|p| SectionPlacementState {
                id: p.id,
                definition_id: p.definition_id,
                start_bar: p.start_bar,
            })
            .collect();
        self.selected_placement_id = self.placements.first().map(|p| p.id);
        self.scroll_y = 0.0;
        self.last_error = None;
        self.expanded_track_id = None;
        self.selected_lane = SelectedLane::Chords;

        // Advance the id counter past anything we just loaded so fresh_id()
        // never collides with persisted ids.
        let max_id = self
            .definitions
            .iter()
            .map(|d| d.id)
            .chain(
                self.definitions
                    .iter()
                    .flat_map(|d| d.chords.iter().map(|c| c.id)),
            )
            .chain(self.placements.iter().map(|p| p.id))
            .max()
            .unwrap_or(0);
        self.next_id = self.next_id.max(max_id);
    }

    /// After loading a project, repopulate `derived_clips` by matching
    /// loaded MIDI clips to (placement, lane-generator) pairs by start
    /// sample + track id, and bump `next_derived_clip_id` past every
    /// clip id that already lives in the derived range.
    ///
    /// Without this, the first regenerate after load would allocate a
    /// fresh id starting at [`DERIVED_CLIP_ID_BASE`] — colliding with a
    /// derived clip already saved with that id. The engine would then
    /// hold two clips at the same id, and the second regenerate's
    /// `DeleteMidiClip` would wipe both (taking out an unrelated lane).
    pub fn rebuild_derived_clips(
        &mut self,
        midi_clips: &[crate::state::MidiClipState],
        samples_per_bar: u64,
    ) {
        self.derived_clips.clear();

        if samples_per_bar > 0 {
            for clip in midi_clips {
                if clip.start_sample % samples_per_bar != 0 {
                    continue;
                }
                let start_bar = (clip.start_sample / samples_per_bar) as u32;
                let entry = self.placements.iter().find_map(|p| {
                    if p.start_bar != start_bar {
                        return None;
                    }
                    let def = self.definitions.iter().find(|d| d.id == p.definition_id)?;
                    if !def.lane_generators.contains_key(&clip.track_id) {
                        return None;
                    }
                    Some((def.id, p.id))
                });
                if let Some((def_id, placement_id)) = entry {
                    self.derived_clips
                        .insert((def_id, placement_id, clip.track_id), clip.id);
                }
            }
        }

        let max_used = midi_clips
            .iter()
            .map(|c| c.id)
            .filter(|id| *id >= DERIVED_CLIP_ID_BASE)
            .max();
        if let Some(m) = max_used {
            self.next_derived_clip_id = self.next_derived_clip_id.max(m.saturating_add(1));
        }
    }

    /// Post-load migration: populate `lane_generators` from old `generate_params`
    /// + track roles when loading a project that predates the lane generator system.
    /// Call after tracks are loaded.
    pub fn migrate_old_generate_params(&mut self, tracks: &[crate::state::TrackState]) {
        use crate::state::TrackRole;

        // Migrate motif knobs from any melody lane that still carries them
        // (predates section-shared MotifParams). Take the first non-default
        // melody lane found.
        for def in &mut self.definitions {
            if def.motif != MotifParams::default() {
                continue;
            }
            let melody_default = MelodyParams::default();
            if let Some(legacy) = def.lane_generators.values().find_map(|cfg| {
                if let LaneGeneratorKind::Melody(m) = &cfg.kind {
                    let differs = m.complexity != melody_default.complexity
                        || m.motif_len != melody_default.motif_len
                        || m.leap_chance != melody_default.leap_chance;
                    if differs {
                        return Some((m.complexity, m.motif_len, m.leap_chance, cfg.seed));
                    }
                }
                None
            }) {
                def.motif.complexity = legacy.0;
                def.motif.motif_len = legacy.1;
                def.motif.leap_chance = legacy.2;
                if def.motif.seed == 0 {
                    def.motif.seed = legacy.3;
                }
            }
        }

        for def in &mut self.definitions {
            // Only migrate if lane_generators is empty (old project) and
            // generate_params has non-default values.
            if !def.lane_generators.is_empty() {
                continue;
            }

            // Migrate beats_per_chord / seventh_chords from old generate_params
            // if they're still at defaults (meaning the project file didn't have
            // the new fields).
            if def.beats_per_chord == 4 && def.generate_params.beats_per_chord != 4 {
                def.beats_per_chord = def.generate_params.beats_per_chord;
            }
            if !def.seventh_chords && def.generate_params.seventh_chords {
                def.seventh_chords = true;
            }

            // Create lane generator configs from tracks with roles.
            for t in tracks.iter().filter(|t| t.sub_track.is_none()) {
                match t.role {
                    Some(TrackRole::Bass) => {
                        def.lane_generators.insert(
                            t.id,
                            LaneGeneratorConfig {
                                kind: LaneGeneratorKind::Bass(def.generate_params.bass.clone()),
                                seed: def.progression_seed,
                            },
                        );
                    }
                    Some(TrackRole::Lead) => {
                        def.lane_generators.insert(
                            t.id,
                            LaneGeneratorConfig {
                                kind: LaneGeneratorKind::Melody(def.generate_params.melody.clone()),
                                seed: def.progression_seed,
                            },
                        );
                    }
                    Some(TrackRole::Pad) => {
                        def.lane_generators.insert(
                            t.id,
                            LaneGeneratorConfig {
                                kind: LaneGeneratorKind::Pad(def.generate_params.pad.clone()),
                                seed: def.progression_seed,
                            },
                        );
                    }
                    None => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use resonance_music_theory::{Chord, ChordQuality, Mode, PitchClass, Scale};

    fn state_with_chords_and_scale() -> ComposeState {
        let mut state = ComposeState::default();
        let def_id = state.fresh_id();
        let chord_a = state.fresh_id();
        let chord_b = state.fresh_id();
        state.definitions.push(SectionDefinitionState {
            id: def_id,
            name: "Verse".to_string(),
            color: [10, 20, 30],
            length_bars: 8,
            chords: vec![
                ChordState {
                    id: chord_a,
                    start_beat: 0,
                    duration_beats: 4,
                    chord: Chord::new(PitchClass::C, ChordQuality::Maj7),
                },
                ChordState {
                    id: chord_b,
                    start_beat: 4,
                    duration_beats: 2,
                    chord: Chord::new(PitchClass::D, ChordQuality::Min7).with_bass(PitchClass::F),
                },
            ],
            scale: Some(Scale::new(PitchClass::C, Mode::Minor)),
            progression_seed: 12345,
            generate_params: GenerateParams::default(),
            generator_spec: None,
            generator_seed: 0,
            generated_material: None,
            lane_generators: HashMap::new(),
            beats_per_chord: 4,
            seventh_chords: false,
            motif: MotifParams::default(),
        });
        let placement_id = state.fresh_id();
        state.placements.push(SectionPlacementState {
            id: placement_id,
            definition_id: def_id,
            start_bar: 0,
        });
        state.selected_placement_id = Some(placement_id);
        state
    }

    /// Round-tripping ComposeState → ProjectFile shapes → ComposeState must
    /// preserve every chord and the selected scale exactly. This is the
    /// exact path taken by Save -> Load in the real app.
    #[test]
    fn in_memory_roundtrip_preserves_chords_and_scale() {
        let src = state_with_chords_and_scale();
        let persisted_defs = src.to_project_definitions();
        let persisted_placements = src.to_project_placements();

        let mut dst = ComposeState::default();
        dst.load_from_project(&persisted_defs, &persisted_placements);

        assert_eq!(dst.definitions.len(), 1);
        let def = &dst.definitions[0];
        assert_eq!(def.name, "Verse");
        assert_eq!(def.length_bars, 8);
        assert_eq!(def.color, [10, 20, 30]);
        assert_eq!(def.scale, Some(Scale::new(PitchClass::C, Mode::Minor)));

        assert_eq!(def.chords.len(), 2);
        assert_eq!(def.chords[0].start_beat, 0);
        assert_eq!(def.chords[0].duration_beats, 4);
        assert_eq!(def.chords[0].chord.root, PitchClass::C);
        assert_eq!(def.chords[0].chord.quality, ChordQuality::Maj7);
        assert_eq!(def.chords[0].chord.bass, None);
        assert_eq!(def.chords[1].chord.root, PitchClass::D);
        assert_eq!(def.chords[1].chord.quality, ChordQuality::Min7);
        assert_eq!(def.chords[1].chord.bass, Some(PitchClass::F));

        assert_eq!(dst.placements.len(), 1);
        assert_eq!(dst.placements[0].definition_id, def.id);
    }

    /// The actual save path calls serde_json on `ProjectSectionDefinition`
    /// values. Make sure that round trip is lossless too.
    #[test]
    fn json_serde_roundtrip_preserves_chords_and_scale() {
        let src = state_with_chords_and_scale();
        let persisted = src.to_project_definitions();

        let json = serde_json::to_string(&persisted).expect("serialize");
        let parsed: Vec<crate::project::ProjectSectionDefinition> =
            serde_json::from_str(&json).expect("deserialize");

        let mut dst = ComposeState::default();
        dst.load_from_project(&parsed, &[]);

        let def = &dst.definitions[0];
        assert_eq!(def.name, "Verse");
        assert_eq!(def.scale, Some(Scale::new(PitchClass::C, Mode::Minor)));
        assert_eq!(def.chords.len(), 2);
        assert_eq!(def.chords[0].chord.quality, ChordQuality::Maj7);
        assert_eq!(def.chords[1].chord.bass, Some(PitchClass::F));
        // Generation fields round-trip unchanged.
        assert_eq!(def.progression_seed, 12345);
        assert_eq!(def.generate_params.chord_count, 4);
        assert_eq!(def.generate_params.beats_per_chord, 4);
    }

    /// Old project files predating this feature won't have `chords` or
    /// `scale` keys on the definition. Both must default so the file still
    /// loads.
    #[test]
    fn loads_old_project_files_without_chords_or_scale() {
        let legacy_json = r#"[{"id":1,"name":"Intro","color":[0,0,0],"length_bars":4}]"#;
        let parsed: Vec<crate::project::ProjectSectionDefinition> =
            serde_json::from_str(legacy_json).expect("legacy deserialize");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "Intro");
        assert_eq!(parsed[0].length_bars, 4);
        assert!(parsed[0].chords.is_empty());
        assert_eq!(parsed[0].scale, None);
    }

    /// Section motif knobs round-trip through the project I/O path.
    #[test]
    fn section_motif_round_trips_through_project_io() {
        let mut state = state_with_chords_and_scale();
        state.definitions[0].motif = MotifParams {
            seed: 0xDEAD_BEEF_1234_5678,
            complexity: 0.73,
            motif_len: 5,
            leap_chance: 0.42,
        };

        let project_defs = state.to_project_definitions();
        let project_placements = state.to_project_placements();

        let mut restored = ComposeState::default();
        restored.load_from_project(&project_defs, &project_placements);

        let restored_motif = restored.definitions[0].motif;
        assert_eq!(restored_motif.seed, 0xDEAD_BEEF_1234_5678);
        assert_eq!(restored_motif.complexity, 0.73);
        assert_eq!(restored_motif.motif_len, 5);
        assert_eq!(restored_motif.leap_chance, 0.42);
    }

    /// A project file without the `motif` key (predating this feature)
    /// must still deserialize, with motif defaulting via serde.
    #[test]
    fn legacy_project_without_motif_field_loads_with_defaults() {
        let json = r#"{
            "id": 1,
            "name": "Legacy",
            "color": [0, 0, 0],
            "length_bars": 8
        }"#;
        let parsed: crate::project::ProjectSectionDefinition =
            serde_json::from_str(json).expect("legacy section JSON should parse");
        assert_eq!(parsed.motif, MotifParams::default());
    }

    /// Older projects: a melody lane carries non-default complexity / motif_len /
    /// leap_chance. After migration these should land on `def.motif`.
    #[test]
    fn migration_lifts_legacy_melody_motif_knobs_onto_section() {
        use resonance_music_theory::{MelodyParams, MelodyStyle};

        let mut state = ComposeState::default();
        let def_id = state.fresh_id();
        state.definitions.push(SectionDefinitionState {
            id: def_id,
            name: "Verse".to_string(),
            color: [0, 0, 0],
            length_bars: 8,
            chords: Vec::new(),
            scale: None,
            progression_seed: 0,
            generate_params: GenerateParams::default(),
            generator_spec: None,
            generator_seed: 0,
            generated_material: None,
            lane_generators: HashMap::new(),
            beats_per_chord: 4,
            seventh_chords: false,
            motif: MotifParams::default(),
        });

        let custom_melody = MelodyParams {
            style: MelodyStyle::Motif,
            complexity: 0.85,
            motif_len: 6,
            leap_chance: 0.55,
            ..MelodyParams::default()
        };
        state.definitions[0].lane_generators.insert(
            100,
            LaneGeneratorConfig {
                kind: LaneGeneratorKind::Melody(custom_melody),
                seed: 0xCAFEBABE,
            },
        );

        state.migrate_old_generate_params(&[]);

        let migrated = &state.definitions[0];
        assert_eq!(migrated.motif.complexity, 0.85);
        assert_eq!(migrated.motif.motif_len, 6);
        assert_eq!(migrated.motif.leap_chance, 0.55);
        assert_eq!(migrated.motif.seed, 0xCAFEBABE);
    }

    /// If the section's motif was loaded with explicit values, the
    /// migration path must not overwrite them.
    #[test]
    fn migration_skips_when_motif_already_customized() {
        use resonance_music_theory::{MelodyParams, MelodyStyle};

        let mut state = ComposeState::default();
        let def_id = state.fresh_id();
        state.definitions.push(SectionDefinitionState {
            id: def_id,
            name: "Verse".to_string(),
            color: [0, 0, 0],
            length_bars: 8,
            chords: Vec::new(),
            scale: None,
            progression_seed: 0,
            generate_params: GenerateParams::default(),
            generator_spec: None,
            generator_seed: 0,
            generated_material: None,
            lane_generators: HashMap::new(),
            beats_per_chord: 4,
            seventh_chords: false,
            motif: MotifParams {
                seed: 7,
                complexity: 0.1,
                motif_len: 2,
                leap_chance: 0.05,
            },
        });

        let custom_melody = MelodyParams {
            style: MelodyStyle::Motif,
            complexity: 0.99,
            motif_len: 6,
            leap_chance: 0.99,
            ..MelodyParams::default()
        };
        state.definitions[0].lane_generators.insert(
            100,
            LaneGeneratorConfig {
                kind: LaneGeneratorKind::Melody(custom_melody),
                seed: 1234,
            },
        );

        state.migrate_old_generate_params(&[]);

        let after = &state.definitions[0];
        assert_eq!(after.motif.complexity, 0.1);
        assert_eq!(after.motif.motif_len, 2);
        assert_eq!(after.motif.leap_chance, 0.05);
        assert_eq!(after.motif.seed, 7);
    }
}
