//! Test-support accessors for [`Resonance`].
//!
//! Public read-only accessors / mutators for integration tests. These
//! exist so `tests/*.rs` files (which are external compile units and
//! see only public API) can verify reducer-driven state changes
//! without poking at private fields. They're `#[doc(hidden)]` because
//! they aren't part of the library's user-facing surface —
//! application code inside the crate still goes through the
//! `pub(crate)` fields directly.
//!
//! Kept out of `lib.rs` purely for separation of concerns; this is a
//! plain `impl Resonance` continuation, not behind any feature gate
//! (integration tests can't see `#[cfg(test)]` items, and a required
//! cargo feature would complicate `cargo test`).

use crate::state;
use crate::Resonance;

impl Resonance {
    #[doc(hidden)]
    pub fn test_tempo_map(&self) -> &resonance_audio::types::TempoMap {
        &self.tempo_map
    }

    #[doc(hidden)]
    pub fn test_tempo_events(&self) -> &[state::TempoEvent] {
        &self.tempo_events
    }

    #[doc(hidden)]
    pub fn test_signature_events(&self) -> &[state::SignatureEvent] {
        &self.signature_events
    }

    #[doc(hidden)]
    pub fn test_transport_bpm(&self) -> f32 {
        self.transport.bpm
    }

    #[doc(hidden)]
    pub fn test_transport_time_sig(&self) -> (u8, u8) {
        (self.transport.time_sig_num, self.transport.time_sig_den)
    }

    #[doc(hidden)]
    pub fn test_selected_global_event(&self) -> Option<state::SelectedGlobalEvent> {
        self.interaction.selected_global_event
    }

    /// Test-only: borrow the open Export-modal state (`None` when the
    /// modal is closed). Lets `tests/export_dialog_shell.rs` assert the
    /// open/close + mode-tab plumbing through the real `ExportMessage`
    /// reducer without poking at the `pub(crate)` field.
    #[doc(hidden)]
    pub fn test_export_dialog(&self) -> Option<&state::ExportDialogState> {
        self.export_dialog.as_ref()
    }

    /// Test-only: borrow the track registry to walk `sorted_tracks()` /
    /// inspect sub-track links from an integration test (which doesn't
    /// see `pub(crate)` fields). Used by
    /// `tests/mixer_sub_track_grouping.rs` to assert the displayed
    /// strip order without parsing the rendered widget tree.
    #[doc(hidden)]
    pub fn test_registry(&self) -> &state::TrackRegistry {
        &self.registry
    }

    /// Test-only: feed an engine event through the real dispatch so
    /// integration tests exercise the same mirroring path the live app
    /// does. The returned follow-up `Task` is dropped — tests assert on
    /// the resulting state, not on emitted messages.
    #[doc(hidden)]
    pub fn test_apply_engine_event(&mut self, event: resonance_audio::types::AudioEvent) {
        let _ = crate::engine_events::handle_engine_event(self, event);
    }

    /// Test-only: read the mirrored aux-send graph. Driven from
    /// `tests/aux_send_mirror.rs` to assert events reconstruct state.
    #[doc(hidden)]
    pub fn test_aux_sends(&self) -> &[resonance_audio::types::AuxSend] {
        &self.aux.sends
    }

    /// Test-only: read the most recent aux-send rejection forwarded to
    /// the UI (`None` once a later send succeeds).
    #[doc(hidden)]
    pub fn test_aux_last_rejection(&self) -> Option<&state::AuxSendRejection> {
        self.aux.last_rejection.as_ref()
    }

    /// Test-only: read the mixer-side expanded-sub-track-parents set,
    /// also driven from `tests/mixer_sub_track_grouping.rs`.
    #[doc(hidden)]
    pub fn test_expanded_sub_track_parents(
        &self,
    ) -> &std::collections::HashSet<resonance_audio::types::TrackId> {
        &self.mixer.expanded_sub_track_parents
    }

    /// Test-only: forcibly clear an expanded-sub-track-parent flag so
    /// the test can flip between expanded / collapsed without dragging
    /// in the full `Message` plumbing.
    #[doc(hidden)]
    pub fn test_collapse_sub_track_parent(
        &mut self,
        parent_id: resonance_audio::types::TrackId,
    ) {
        self.mixer.expanded_sub_track_parents.remove(&parent_id);
    }

    /// Test-only: read the GUI-side MIDI clip list. Used by reducer
    /// tests under `tests/` that need to inspect post-drag/trim clip
    /// geometry without poking at the engine round-trip.
    #[doc(hidden)]
    pub fn test_midi_clips(&self) -> &[state::MidiClipState] {
        &self.midi_clips
    }

    /// Test-only: push a MIDI clip directly into GUI state, bypassing
    /// the engine notification round-trip. Returns the clip's id so
    /// the test can dispatch trim/drag messages against it.
    #[doc(hidden)]
    pub fn test_push_midi_clip(&mut self, clip: state::MidiClipState) {
        self.midi_clips.push(clip);
    }

    /// Test-only: overwrite the sample rate. Tempo-map projections used
    /// by the MIDI clip trim reducer depend on `sample_rate`; integration
    /// tests fix it to a known value so the projection math is
    /// deterministic.
    #[doc(hidden)]
    pub fn test_set_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
    }

    /// Test-only: rebuild the GUI-side tempo map from the current
    /// `tempo_events` / `signature_events`. Mirrors what the global-
    /// track reducers call after a tempo edit; surfaced so tests can
    /// seed a custom tempo map without going through the message path.
    #[doc(hidden)]
    pub fn test_rebuild_tempo_map(&mut self) {
        self.rebuild_tempo_map();
    }

    /// Test-only: push a tempo event so the rebuilt tempo map has the
    /// requested ramp/step. Caller must follow with
    /// `test_rebuild_tempo_map` (and usually `test_set_sample_rate`).
    #[doc(hidden)]
    pub fn test_push_tempo_event(&mut self, event: state::TempoEvent) {
        self.tempo_events.push(event);
    }

    /// Test-only: overwrite the arrange-view zoom (pixels per second).
    /// MIDI clip trim translates a pointer-pixel delta into samples via
    /// `delta_px / zoom`, so the reducer test fixes a known zoom value
    /// to make the delta arithmetic deterministic.
    #[doc(hidden)]
    pub fn test_set_arrange_zoom(&mut self, zoom: f32) {
        self.viewport.zoom = zoom;
    }

    /// Test-only: push a track straight into the registry, bypassing the
    /// engine round-trip, and refresh the compose track-count cache the
    /// engine handlers would normally keep fresh. Used by the vocal
    /// placeholder snapshot tests to add a `TrackType::Vocal` track that
    /// has no lane-generator config.
    #[doc(hidden)]
    pub fn test_push_track(&mut self, track: state::TrackState) {
        self.registry.next_track_order = self.registry.next_track_order.max(track.order + 1);
        self.registry.tracks.push(track);
        self.compose.refresh_track_count(&self.registry.tracks);
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

    /// Test-only: flip the project-active flag so the message gate in
    /// `gates_message` lets reducer-driven `MidiClipMessage` /
    /// `ClipMessage` traffic through. Demo seeding does this in the
    /// real app; reducer tests that don't seed the demo flip it
    /// directly.
    #[doc(hidden)]
    pub fn test_set_active_project(&mut self, active: bool) {
        self.io.has_active_project = active;
    }

    /// Test-only: the currently active top-level [`ViewMode`].
    #[doc(hidden)]
    pub fn test_view_mode(&self) -> state::ViewMode {
        self.view_mode
    }

    /// Test-only: directly set the active view (bypassing the reducer)
    /// to establish a starting tab for Performance-mode toggle tests.
    #[doc(hidden)]
    pub fn test_set_view_mode(&mut self, mode: state::ViewMode) {
        self.view_mode = mode;
    }

    /// Test-only: whether the transport reports as playing. Used to
    /// assert that entering/leaving Performance mode never starts or
    /// stops playback.
    #[doc(hidden)]
    pub fn test_transport_playing(&self) -> bool {
        self.transport.playing
    }

    /// Test-only: force the transport's playing flag so a test can prove
    /// a view switch preserves it (no engine round-trip involved).
    #[doc(hidden)]
    pub fn test_set_transport_playing(&mut self, playing: bool) {
        self.transport.playing = playing;
    }

    /// Test-only: arm/disarm the first track's record flag so a test can
    /// assert that record-arm never auto-opens Performance mode.
    #[doc(hidden)]
    pub fn test_arm_first_track(&mut self, armed: bool) {
        if let Some(track) = self.registry.tracks.first_mut() {
            track.record_armed = armed;
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

    /// Test-only: serialize the current GUI state to the on-disk
    /// [`crate::project::ProjectFile`] shape, so a persistence test can
    /// inspect (or round-trip) the reference A/B block without writing to
    /// disk.
    #[doc(hidden)]
    pub fn test_build_project_file(&self) -> crate::project::ProjectFile {
        crate::update::project_io::build_project_file(self)
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

    /// Test-only: anchor a project path so `can_record_undo` is satisfied
    /// (it requires `has_active_project` *and* a `project_path`). Pair
    /// with [`Self::test_set_active_project`] to make undo/redo recordable
    /// in a reducer test without a real save.
    #[doc(hidden)]
    pub fn test_set_project_path(&mut self, path: std::path::PathBuf) {
        self.io.project_path = Some(path);
    }

    /// Test-only: fold an engine event into app state, exercising the same
    /// `engine_events` dispatch the live event pump uses. Lets tests verify
    /// that the engine's authoritative echoes update the GUI mirror.
    #[doc(hidden)]
    pub fn test_handle_engine_event(
        &mut self,
        event: resonance_audio::types::AudioEvent,
    ) {
        let _ = crate::engine_events::handle_engine_event(self, event);
    }

    /// Test-only: read the GUI-side audio clip list. Used by the
    /// engine-event mirroring tests to assert that fade/gain events
    /// land on the matching `ClipState`.
    #[doc(hidden)]
    pub fn test_clips(&self) -> &[state::ClipState] {
        &self.clips
    }

    /// Test-only: push an audio clip straight into GUI state, bypassing
    /// the engine `ClipImported` round-trip, so a test can then drive
    /// fade/gain events against a known clip id.
    #[doc(hidden)]
    pub fn test_push_clip(&mut self, clip: state::ClipState) {
        self.clips.push(clip);
    }

    /// Test-only: force the transport's recording flag so a test can render
    /// the Performance status bar in its recording state.
    #[doc(hidden)]
    pub fn test_set_transport_recording(&mut self, recording: bool) {
        self.transport.recording = recording;
    }

    /// Test-only: read the GUI-side MIDI control-surface mapping, so the
    /// engine-event mirroring tests can assert bindings / learn state.
    #[doc(hidden)]
    pub fn test_midi_map(&self) -> &state::MidiMapState {
        &self.midi_map
    }

    /// Test-only: arm MIDI Learn for `target` (the UI-side step that
    /// normally precedes a `MidiLearnCaptured` event), so a test can then
    /// verify the capture handler clears learn mode.
    #[doc(hidden)]
    pub fn test_arm_midi_learn(&mut self, target: resonance_common::MidiTarget) {
        self.midi_map.learn_target = Some(target);
    }

    /// Test-only: borrow the arrangement-marker collection so the marker
    /// reducer tests can assert post-dispatch state (count, order,
    /// names, colours, region bounds) without poking private fields.
    #[doc(hidden)]
    pub fn test_markers(&self) -> &state::ArrangementMarkers {
        &self.markers
    }

    /// Test-only: insert a marker straight into state, bypassing the
    /// `AddAtPlayhead` snap path, so a test can seed markers at exact
    /// sample positions before exercising rename / move / jump / loop
    /// reducers. Returns the marker's id.
    #[doc(hidden)]
    pub fn test_add_marker(&mut self, marker: state::ArrangementMarker) -> u64 {
        self.markers.add(marker)
    }

    /// Test-only: the current transport playhead sample. Marker
    /// navigation reducers move this in lockstep with the `SeekTo`
    /// command sent to the engine.
    #[doc(hidden)]
    pub fn test_playhead(&self) -> u64 {
        self.transport.playhead
    }

    /// Test-only: the transport loop range / enabled flags
    /// `(loop_in, loop_out, loop_enabled)`. `LoopToRegion` sets these in
    /// lockstep with the `SetLoopRange` command sent to the engine.
    #[doc(hidden)]
    pub fn test_loop_state(&self) -> (u64, u64, bool) {
        (
            self.transport.loop_in,
            self.transport.loop_out,
            self.transport.loop_enabled,
        )
    }

    /// Test-only: which audio clip's vocal pitch editor is open, if any
    /// (doc #160). Set by the `VocalTuningMessage::OpenPitchEditor`
    /// reducer when opened on a vocal clip.
    #[doc(hidden)]
    pub fn test_editing_pitch_clip(&self) -> Option<resonance_audio::types::ClipId> {
        self.interaction.editing_pitch_clip
    }
}
