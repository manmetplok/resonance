use resonance_music_theory::{Chord, Scale};

use crate::project::{ProjectSectionChord, ProjectSectionDefinition, ProjectSectionPlacement};

pub mod invariants;
pub mod messages;

pub use messages::ComposeMessage;

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

#[derive(Debug, Default)]
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
    /// When `Some`, the right side of that track's row in the Compose tab
    /// shows an instrument details panel (name / type / icon) instead of
    /// the note grid.
    pub details_track_id: Option<resonance_audio::types::TrackId>,
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

impl ComposeState {
    pub fn fresh_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
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
        self.selected_placement_id.and_then(|id| self.find_placement(id))
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
            })
            .collect();
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

        // Advance the id counter past anything we just loaded so fresh_id()
        // never collides with persisted ids.
        let max_id = self
            .definitions
            .iter()
            .map(|d| d.id)
            .chain(self.definitions.iter().flat_map(|d| d.chords.iter().map(|c| c.id)))
            .chain(self.placements.iter().map(|p| p.id))
            .max()
            .unwrap_or(0);
        self.next_id = self.next_id.max(max_id);
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
                    chord: Chord::new(PitchClass::D, ChordQuality::Min7)
                        .with_bass(PitchClass::F),
                },
            ],
            scale: Some(Scale::new(PitchClass::C, Mode::Minor)),
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
    }

    /// Old project files predating this feature won't have `chords` or
    /// `scale` keys on the definition. Both must default so the file still
    /// loads.
    #[test]
    fn loads_old_project_files_without_chords_or_scale() {
        let legacy_json =
            r#"[{"id":1,"name":"Intro","color":[0,0,0],"length_bars":4}]"#;
        let parsed: Vec<crate::project::ProjectSectionDefinition> =
            serde_json::from_str(legacy_json).expect("legacy deserialize");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "Intro");
        assert_eq!(parsed[0].length_bars, 4);
        assert!(parsed[0].chords.is_empty());
        assert_eq!(parsed[0].scale, None);
    }
}
