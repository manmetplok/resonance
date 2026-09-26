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

    /// Test-only: the seed on a lane's generator config, so
    /// `section.set_lane_generator`'s explicit-seed path is observable
    /// (ba todo #1168).
    #[doc(hidden)]
    pub fn test_lane_generator_seed(
        &self,
        definition_id: u64,
        track_id: resonance_audio::types::TrackId,
    ) -> Option<u64> {
        self.compose
            .find_definition(definition_id)
            .and_then(|d| d.lane_generators.get(&track_id))
            .map(|cfg| cfg.seed)
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

    /// Test-only: rename the project's first drum pattern, returning its
    /// id. Lets a test stand a user-authored pattern next to a built-in
    /// of the same name and assert which one wins.
    #[doc(hidden)]
    pub fn test_rename_first_drum_pattern(&mut self, name: &str) -> u64 {
        let pattern = self
            .compose
            .drum_patterns
            .first_mut()
            .expect("the project has at least one drum pattern");
        pattern.name = name.to_owned();
        pattern.id
    }

    /// Test-only: the primary drum pattern id assigned to a section's
    /// arrangement, or `None` when the arrangement is empty.
    #[doc(hidden)]
    pub fn test_section_primary_pattern(&self, definition_id: u64) -> Option<u64> {
        self.compose
            .find_definition(definition_id)
            .and_then(|d| d.primary_pattern_id())
    }

    /// Test-only: a section definition's chords as `(start_beat,
    /// duration_beats)`, in grid order.
    #[doc(hidden)]
    pub fn test_section_chord_spans(&self, definition_id: u64) -> Vec<(u32, u32)> {
        let mut spans: Vec<(u32, u32)> = self
            .compose
            .find_definition(definition_id)
            .map(|d| d.chords.iter().map(|c| (c.start_beat, c.duration_beats)).collect())
            .unwrap_or_default();
        spans.sort_unstable();
        spans
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

    /// Test-only: the in-flight SVS render epoch of a vocal lane, bumped
    /// once per queued render. The render itself runs off-thread, so this
    /// counter is what a test asserts on to prove a lane was actually
    /// dispatched for re-render (ba doc #271 V2).
    #[doc(hidden)]
    pub fn test_vocal_render_epoch(
        &self,
        definition_id: u64,
        track_id: resonance_audio::types::TrackId,
    ) -> Option<u64> {
        self.compose
            .vocal_audio
            .render_epoch
            .get(&(definition_id, track_id))
            .copied()
    }

    /// Test-only: `(rendered_bpm, other_bpm)` when a section's one vocal
    /// render doesn't fit every placed bar's tempo (FU-M11b).
    #[doc(hidden)]
    pub fn test_vocal_tempo_mismatch(&self, definition_id: u64) -> Option<(f32, f32)> {
        crate::update::compose::vocal_tempo_mismatch(self, definition_id)
            .map(|m| (m.rendered_bpm, m.other_bpm))
    }

    /// Test-only: pin a vocal lane's in-flight render epoch, standing in
    /// for the bump `enqueue_vocal_render` performs when a render is
    /// queued. Lets a test stage the supersede race — a `VocalAudioReady`
    /// / `VocalAudioFailed` carrying any *other* epoch is stale and must
    /// resolve no control job.
    #[doc(hidden)]
    pub fn test_set_vocal_render_epoch(
        &mut self,
        definition_id: u64,
        track_id: resonance_audio::types::TrackId,
        epoch: u64,
    ) {
        self.compose
            .vocal_audio
            .render_epoch
            .insert((definition_id, track_id), epoch);
        self.compose
            .vocal_audio
            .in_flight_render
            .insert((definition_id, track_id), epoch);
    }

    /// Test-only: whether a render is still recorded as in flight for the
    /// lane (code review FU-M11a).
    #[doc(hidden)]
    pub fn test_vocal_render_in_flight(
        &self,
        definition_id: u64,
        track_id: resonance_audio::types::TrackId,
    ) -> bool {
        self.compose
            .vocal_audio
            .in_flight_render
            .contains_key(&(definition_id, track_id))
    }

    /// Test-only: install a rendered-vocal-audio clip entry for a lane,
    /// standing in for audio a previous session rendered. A re-render
    /// tears these down, so their disappearance is the observable "this
    /// lane's old audio was replaced".
    #[doc(hidden)]
    pub fn test_install_vocal_audio_clip(
        &mut self,
        definition_id: u64,
        placement_id: u64,
        track_id: resonance_audio::types::TrackId,
        clip_id: resonance_audio::types::ClipId,
        path: std::path::PathBuf,
    ) {
        self.compose
            .vocal_audio
            .clips
            .insert((definition_id, placement_id, track_id), (clip_id, path));
    }

    /// Test-only: the installed vocal-audio clip ids on a track, as
    /// `(definition_id, clip_id)` pairs.
    #[doc(hidden)]
    pub fn test_vocal_audio_clips(
        &self,
        track_id: resonance_audio::types::TrackId,
    ) -> Vec<(u64, resonance_audio::types::ClipId)> {
        let mut out: Vec<(u64, resonance_audio::types::ClipId)> = self
            .compose
            .vocal_audio
            .clips
            .iter()
            .filter(|((_, _, t), _)| *t == track_id)
            .map(|((def, _, _), (clip, _))| (*def, *clip))
            .collect();
        out.sort_unstable();
        out
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

    /// Test-only: list a reference load under a fresh id, as a user load
    /// does before the engine echoes anything (analysing, name from the
    /// file stem), so an engine-event-folding test can exercise the echo
    /// path without an active project or an engine. Returns the id the
    /// load was sent under.
    #[doc(hidden)]
    pub fn test_reference_push_pending(
        &mut self,
        path: &str,
    ) -> resonance_audio::types::ReferenceId {
        let id = self.reference.alloc_engine_id();
        self.reference
            .entries
            .push(crate::reference::ReferenceEntry::analyzing(
                id,
                crate::update::reference::reference_name(std::path::Path::new(path)),
                path.to_string(),
                resonance_audio::types::ReferenceAnalysisStage::Decoding,
            ));
        id
    }

    /// Test-only: replay just the reference A/B block of a saved
    /// [`crate::project::ProjectFile`] into this app, exercising the same
    /// restore path a full project load runs. Lets a persistence test
    /// verify reference round-trip / missing-file handling without
    /// constructing a whole `LoadedProject`.
    #[doc(hidden)]
    pub fn test_restore_references(&mut self, file: &crate::project::ProjectFile) {
        crate::update::project_io::restore_references(
            self,
            file,
            crate::update::project_io::ReferenceMonitorSource::File,
        );
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

    /// Test-only: overwrite a MIDI clip's per-note lyric side-table, as
    /// the vocal lane's slur/override edits do, so a persistence test can
    /// seed lyrics without generating a vocal line.
    #[doc(hidden)]
    pub fn test_set_clip_lyrics(&mut self, clip_id: resonance_audio::types::ClipId, lyrics: Vec<String>) {
        self.compose.vocal_audio.clip_lyrics.insert(clip_id, lyrics);
    }

    /// Test-only: drop a MIDI clip's lyric side-table entry altogether.
    #[doc(hidden)]
    pub fn test_clear_clip_lyrics(&mut self, clip_id: resonance_audio::types::ClipId) {
        self.compose.vocal_audio.clip_lyrics.remove(&clip_id);
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

    /// Test-only: snapshot every arrangement placement as
    /// `(placement_id, definition_id, start_bar)`. Lets the compose
    /// bar↔sample tests (ba todo #1163) pin exactly which bar a derived
    /// clip should land on.
    #[doc(hidden)]
    pub fn test_placements(&self) -> Vec<(u64, u64, u32)> {
        self.compose
            .placements
            .iter()
            .map(|p| (p.id, p.definition_id, p.start_bar))
            .collect()
    }

    /// Test-only: drop every arrangement placement so a test can install
    /// a single placement at a known bar (ba todo #1163).
    #[doc(hidden)]
    pub fn test_clear_placements(&mut self) {
        self.compose.placements.clear();
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
