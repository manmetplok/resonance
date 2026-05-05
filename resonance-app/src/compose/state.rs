//! `ComposeState` — the top-level runtime state for the Compose tab.
//! Owns section definitions, placements, the chord lane, the drumroll
//! editor state, and the table of derived MIDI clips.

use std::collections::HashMap;

use resonance_audio::types::{ClipId, TrackId};
use resonance_music_theory::{MelodyParams, MotifParams, MotifSource};

use crate::project::{ProjectSectionChord, ProjectSectionDefinition, ProjectSectionPlacement};

use super::drumroll::DrumrollViewState;
use super::lane_generator::{LaneGeneratorConfig, LaneGeneratorKind};
use super::section::{
    ChordState, EditSectionForm, NewSectionForm, SectionDefinitionState, SectionPlacementState,
    SelectedLane,
};

/// Starting point for app-allocated derived clip ids. Chosen high enough
/// that it will never collide with engine-allocated clip ids (which count
/// up monotonically from 1 via `CreateMidiClip`), yet small enough to
/// leave headroom for a very long session. The engine bumps its own
/// allocator past any id seen via `LoadMidiClipDirect`, so using values
/// above this base is always safe.
pub const DERIVED_CLIP_ID_BASE: u64 = 1 << 40;

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
                motif_source: d.motif_source.clone(),
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
                motif_source: d.motif_source.clone(),
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
        // melody lane found. Only applies in Generated mode — Manual motifs
        // never came from a melody lane.
        for def in &mut self.definitions {
            let MotifSource::Generated(motif) = &mut def.motif_source else {
                continue;
            };
            if *motif != MotifParams::default() {
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
                motif.complexity = legacy.0;
                motif.motif_len = legacy.1;
                motif.leap_chance = legacy.2;
                if motif.seed == 0 {
                    motif.seed = legacy.3;
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
