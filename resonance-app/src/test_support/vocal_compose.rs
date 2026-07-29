//! Compose/vocal-domain test hooks: section definitions and
//! placements, chord track, groove/quantize state, and the
//! reference-track (A/B) block.

use crate::state;
use crate::Resonance;

impl Resonance {
    /// Test-only: read the app-side groove library populated from
    /// `GrooveExtracted` engine events (ba todo #390).
    #[doc(hidden)]
    pub fn test_groove_library(&self) -> &[resonance_audio::quantize::GrooveTemplate] {
        &self.groove_library
    }

    /// Test-only: read the MIDI editor's Quantize panel settings (todo
    /// #392) so panel-control reducer tests can assert the setter handlers
    /// updated the bound state.
    #[doc(hidden)]
    pub fn test_quantize_panel(&self) -> &state::MidiQuantizePanelState {
        &self.midi_quantize
    }

    /// Test-only: the [`LaneGeneratorKindTag`] configured on a track
    /// within a section definition, or `None` when the lane is manual
    /// (no generator). Drives the `generate.part` control-endpoint tests
    /// (ba todo #1154).
    ///
    /// [`LaneGeneratorKindTag`]: crate::compose::LaneGeneratorKindTag
    #[doc(hidden)]
    pub fn test_lane_generator_tag(
        &self,
        definition_id: u64,
        track_id: resonance_audio::types::TrackId,
    ) -> Option<crate::compose::LaneGeneratorKindTag> {
        use crate::compose::{LaneGeneratorKind, LaneGeneratorKindTag};
        self.compose
            .find_definition(definition_id)
            .and_then(|d| d.lane_generators.get(&track_id))
            .map(|cfg| match &cfg.kind {
                LaneGeneratorKind::Bass(_) => LaneGeneratorKindTag::Bass,
                LaneGeneratorKind::Melody(_) => LaneGeneratorKindTag::Melody,
                LaneGeneratorKind::Pad(_) => LaneGeneratorKindTag::Pad,
                LaneGeneratorKind::Vocal(_) => LaneGeneratorKindTag::Vocal,
                // Drum lanes have no melodic tag; report Manual so the
                // control tests never see a drum lane where a melodic one
                // is expected.
                LaneGeneratorKind::Drum(_) => LaneGeneratorKindTag::Manual,
            })
    }

    /// Test-only: number of derived MIDI clips generated for a track
    /// (across all sections + placements). A `generate.part` call
    /// produces one per placement of the target section.
    #[doc(hidden)]
    pub fn test_derived_clip_count(&self, track_id: resonance_audio::types::TrackId) -> usize {
        self.compose
            .derived_clips
            .keys()
            .filter(|(_, _, t)| *t == track_id)
            .count()
    }

    /// Test-only: the primary drum pattern id assigned to a section's
    /// arrangement, or `None` when the arrangement is empty.
    #[doc(hidden)]
    pub fn test_section_primary_pattern(&self, definition_id: u64) -> Option<u64> {
        self.compose
            .find_definition(definition_id)
            .and_then(|d| d.primary_pattern_id())
    }

    /// Test-only: push a drum instrument track (the default
    /// [`test_add_track`](Self::test_add_track) makes a synth). Needed by
    /// the `generate.drums` control tests, which require an
    /// `InstrumentType::Drum` target.
    #[doc(hidden)]
    pub fn test_add_drum_track(&mut self, track_id: resonance_audio::types::TrackId) {
        let order = self.registry.tracks.len();
        let mut track = crate::state::TrackState::new_instrument(track_id, order);
        track.instrument_type = crate::state::InstrumentType::Drum;
        self.registry.tracks.push(track);
        self.registry.resort_tracks();
        self.compose.refresh_track_count(&self.registry.tracks);
    }

    /// Test-only: install a Vocal lane generator (default params, so
    /// TIGER voicebank) on a track within a section definition, seeding
    /// its draft. Drives the `vocal.*` control-endpoint tests (ba todo
    /// #1156).
    #[doc(hidden)]
    pub fn test_install_vocal_lane(
        &mut self,
        definition_id: u64,
        track_id: resonance_audio::types::TrackId,
    ) {
        use crate::compose::{LaneGeneratorConfig, LaneGeneratorKind};
        if let Some(def) = self.compose.find_definition_mut(definition_id) {
            def.lane_generators.insert(
                track_id,
                LaneGeneratorConfig {
                    kind: LaneGeneratorKind::Vocal(resonance_music_theory::VocalParams::default()),
                    seed: 1,
                },
            );
        }
    }

    /// Test-only: the lyric lines of a track's vocal lane in a section
    /// (draft text, in order). Empty when the lane isn't a vocal
    /// generator.
    #[doc(hidden)]
    pub fn test_vocal_lines(
        &self,
        definition_id: u64,
        track_id: resonance_audio::types::TrackId,
    ) -> Vec<String> {
        use crate::compose::LaneGeneratorKind;
        self.compose
            .find_definition(definition_id)
            .and_then(|d| d.lane_generators.get(&track_id))
            .and_then(|c| match &c.kind {
                LaneGeneratorKind::Vocal(p) => {
                    Some(p.draft.iter().map(|l| l.text.clone()).collect())
                }
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Test-only: the voicebank set on a track's vocal lane in a section.
    #[doc(hidden)]
    pub fn test_vocal_voicebank(
        &self,
        definition_id: u64,
        track_id: resonance_audio::types::TrackId,
    ) -> Option<resonance_music_theory::VocalVoicebank> {
        use crate::compose::LaneGeneratorKind;
        self.compose
            .find_definition(definition_id)
            .and_then(|d| d.lane_generators.get(&track_id))
            .and_then(|c| match &c.kind {
                LaneGeneratorKind::Vocal(p) => Some(p.voicebank),
                _ => None,
            })
    }

    /// Test-only: the project pronunciation dictionary as
    /// `(word, phonemes)` pairs.
    #[doc(hidden)]
    pub fn test_pronunciation_dictionary(&self) -> Vec<(String, Vec<String>)> {
        self.compose
            .pronunciation
            .project_dictionary
            .iter()
            .map(|e| {
                (
                    e.word.clone(),
                    e.phonemes.iter().map(|p| (*p).to_owned()).collect(),
                )
            })
            .collect()
    }

    /// Test-only: remove a track's lane-generator config from every
    /// compose section definition, turning a configured compose lane
    /// back into an unconfigured one (placeholder row in the vocal
    /// stack).
    #[doc(hidden)]
    pub fn test_remove_lane_generator(&mut self, track_id: resonance_audio::types::TrackId) {
        for def in &mut self.compose.definitions {
            def.lane_generators.remove(&track_id);
        }
    }

    /// Test-only: append an entry to the project pronunciation
    /// dictionary (doc #265 introspection tests; the dictionary-manager
    /// modal is the user-facing mutation path).
    #[doc(hidden)]
    pub fn test_push_dictionary_entry(&mut self, word: &str, phonemes: Vec<&'static str>) {
        self.compose.pronunciation.project_dictionary.push(
            crate::compose::vocal_svs::DictionaryEntry {
                word: word.to_owned(),
                phonemes,
                scope: crate::compose::vocal_svs::DictionaryScope::Project,
            },
        );
    }

    /// Test-only: read-only view of the reference-track (A/B) state.
    #[doc(hidden)]
    pub fn test_reference(&self) -> &crate::reference::ReferenceState {
        &self.reference
    }

    /// Test-only: enqueue a pending reference-load path, mimicking what a
    /// dispatched `LoadReferenceTrack` does, so an engine-event-folding
    /// test can exercise the id↔path correlation without an active project.
    #[doc(hidden)]
    pub fn test_reference_push_pending(&mut self, path: &str) {
        self.reference.pending_loads.push_back(path.to_string());
    }

    /// Test-only: replay just the reference A/B block of a saved
    /// [`crate::project::ProjectFile`] into this app, exercising the same
    /// restore path a full project load runs. Lets a persistence test
    /// verify reference round-trip / missing-file handling without
    /// constructing a whole `LoadedProject`.
    #[doc(hidden)]
    pub fn test_restore_references(&mut self, file: &crate::project::ProjectFile) {
        crate::update::project_io::restore_references(self, file);
    }

    /// Test-only: replay the compose tab (section definitions + placements,
    /// then the drum-pattern bank) of a saved
    /// [`crate::project::ProjectFile`] into this app — the same two steps
    /// `replay_loaded_project` runs for the Compose tab. Lets a persistence
    /// test exercise drum-arrangement migration end-to-end (including the
    /// legacy `drum_groups` → pattern promotion in `restore_drum_patterns`)
    /// without standing up a whole `LoadedProject`.
    #[doc(hidden)]
    pub fn test_replay_compose(&mut self, file: &crate::project::ProjectFile) {
        self.compose
            .load_from_project(&file.section_definitions, &file.section_placements);
        crate::update::project_io::restore_drum_patterns(&mut self.compose, file, false);
    }

    /// Test-only: read the project's quantize state (groove library +
    /// last-used quantize/humanize settings, ba todo #395).
    #[doc(hidden)]
    pub fn test_quantize(&self) -> &crate::state::QuantizeState {
        &self.quantize
    }

    /// Test-only: mutable access to the quantize state, so a persistence
    /// test can seed a groove library / settings before serializing.
    #[doc(hidden)]
    pub fn test_quantize_mut(&mut self) -> &mut crate::state::QuantizeState {
        &mut self.quantize
    }

    /// Test-only: replay just the quantize block of a saved
    /// [`crate::project::ProjectFile`] into this app, exercising the same
    /// restore path a full project load runs (ba todo #395).
    #[doc(hidden)]
    pub fn test_restore_quantize(&mut self, file: &crate::project::ProjectFile) {
        crate::update::project_io::restore_quantize(self, file);
    }

    /// Test-only: borrow the global chord track.
    #[doc(hidden)]
    pub fn test_chord_track(&self) -> &crate::chord_track::ChordTrack {
        &self.chord_track
    }

    /// Test-only: mutably borrow the global chord track so a test can
    /// stage regions/key changes directly (no `ChordTrackMessage`
    /// handlers exist yet — those land in a later todo).
    #[doc(hidden)]
    pub fn test_chord_track_mut(&mut self) -> &mut crate::chord_track::ChordTrack {
        &mut self.chord_track
    }

    /// Test-only: stage a compose section definition directly, bypassing
    /// the inline new-section form. Used by chord-track regeneration
    /// tests to set up a known progression to override.
    #[doc(hidden)]
    pub fn test_push_section_definition(
        &mut self,
        def: crate::compose::SectionDefinitionState,
    ) {
        self.compose.definitions.push(def);
    }

    /// Test-only: place a staged section definition at `start_bar`,
    /// returning the fresh placement id.
    #[doc(hidden)]
    pub fn test_place_section(&mut self, definition_id: u64, start_bar: u32) -> u64 {
        let id = self.compose.fresh_id();
        self.compose
            .placements
            .push(crate::compose::SectionPlacementState {
                id,
                definition_id,
                start_bar,
            });
        id
    }

    /// Test-only: run the chord-track harmony overlay for a section
    /// definition exactly as lane regeneration does, returning the
    /// effective `(chords, scale)` the generators would consume. Lets
    /// tests prove pinned chord-track regions and key context flow into
    /// regeneration without driving the audio engine.
    #[doc(hidden)]
    pub fn test_section_harmony(
        &self,
        definition_id: u64,
    ) -> (
        Vec<crate::compose::ChordState>,
        Option<resonance_music_theory::Scale>,
    ) {
        let Some(def) = self.compose.find_definition(definition_id) else {
            return (Vec::new(), None);
        };
        let mut def = def.clone();
        crate::update::compose::regenerate::apply_chord_track_harmony(self, definition_id, &mut def);
        (def.chords, def.scale)
    }
}
