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
