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

    /// Test-only: borrow the MIDI Import modal state (or `None` when the
    /// modal is closed). Drives `tests/import_dialog.rs`, which asserts
    /// the open/close + review-stage plumbing without rendering the
    /// overlay.
    #[doc(hidden)]
    pub fn test_import_dialog(&self) -> Option<&state::ImportDialogState> {
        self.import_dialog.as_ref()
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

    /// Test-only: swap in a command-capturing engine and hand back the
    /// receiver its [`send`](resonance_audio::AudioEngine::send) calls
    /// queue onto. Lets `tests/aux_send_handlers.rs` assert the exact
    /// `AudioCommand`s an update handler emits, with no real audio device
    /// and no engine thread. The previously installed engine is dropped
    /// (its shutdown handshake runs on `Drop`).
    #[doc(hidden)]
    pub fn test_capture_engine(
        &mut self,
    ) -> resonance_audio::__test_support::Receiver<resonance_audio::types::AudioCommand> {
        let (engine, cmd_rx) = resonance_audio::AudioEngine::for_test_capture();
        self.engine = engine;
        cmd_rx
    }

    /// Test-only: route a message straight through the dispatcher,
    /// bypassing the startup/bounce gates and undo bookkeeping. Mirrors
    /// `test_apply_engine_event` for the user-input side, so a handler
    /// test doesn't need to flip `has_active_project` just to get a
    /// message delivered.
    #[doc(hidden)]
    pub fn test_dispatch(&mut self, message: crate::message::Message) {
        let _ = self.dispatch(message);
    }

    /// Test-only: seed the aux-send mirror directly so a handler test can
    /// exercise the "edit an existing send" upsert path without first
    /// driving the create round trip. Mirrors what an `AuxSendChanged`
    /// echo would produce.
    #[doc(hidden)]
    pub fn test_seed_aux_send(&mut self, send: resonance_audio::types::AuxSend) {
        self.aux.upsert(send);
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

    /// Test-only: capture an undo snapshot of the current declarative
    /// state. Used to prove the chord track survives the snapshot's
    /// `extras` round-trip without driving the engine.
    #[doc(hidden)]
    pub fn test_snapshot_for_undo(&self) -> crate::undo::UndoSnapshot {
        self.snapshot_for_undo()
    }

    /// Test-only: restore a previously captured snapshot, exercising the
    /// fast (`try_diff_replay`) restore path when the snapshot is
    /// structure-identical to the current state. Used to prove that an
    /// undo/redo of a scalar clip edit (e.g. fade/gain) is applied
    /// surgically without a full reload (todo #321, doc #156).
    #[doc(hidden)]
    pub fn test_begin_restore_from_snapshot(&mut self, snapshot: crate::undo::UndoSnapshot) {
        self.begin_restore_from_snapshot(snapshot);
    }

    /// Test-only: apply the runtime-only undo extras, exercising the
    /// slow-path restore of chord-track state without an engine replay.
    #[doc(hidden)]
    pub fn test_finalize_undo_restore(&mut self, extras: crate::undo::UndoExtras) {
        self.finalize_undo_restore(extras);
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

    /// Test-only: read a track's freeze status (defaults to idle).
    #[doc(hidden)]
    pub fn test_freeze_status(
        &self,
        track_id: resonance_audio::types::TrackId,
    ) -> crate::state::FreezeStatus {
        self.freeze.status(track_id)
    }

    /// Test-only: force a track's freeze status, mirroring what the engine
    /// freeze-event mirror (ba todo #575) would set on completion.
    #[doc(hidden)]
    pub fn test_set_freeze_status(
        &mut self,
        track_id: resonance_audio::types::TrackId,
        status: crate::state::FreezeStatus,
    ) {
        self.freeze.set(track_id, status);
    }

    /// Test-only: read the active freeze batch queue, if any.
    #[doc(hidden)]
    pub fn test_freeze_queue(&self) -> Option<&crate::state::FreezeQueue> {
        self.freeze.queue.as_ref()
    }

    /// Test-only: advance the freeze batch to the next track, as the
    /// engine completion mirror (ba todo #575) will once it lands. Returns
    /// `true` when a next freeze was started.
    #[doc(hidden)]
    pub fn test_advance_freeze_queue(&mut self) -> bool {
        crate::update::freeze::advance_freeze_queue(self)
    }

    /// Test-only: select a track so `FreezeSelectedTracks` has a target.
    #[doc(hidden)]
    pub fn test_select_track(&mut self, track_id: resonance_audio::types::TrackId) {
        self.interaction.selected_track = Some(track_id);
    }

    /// Test-only: append a track of the given type so freeze tests have a
    /// registry to operate on without an engine round-trip.
    #[doc(hidden)]
    pub fn test_add_track(
        &mut self,
        track_id: resonance_audio::types::TrackId,
        track_type: resonance_audio::types::TrackType,
    ) {
        use resonance_audio::types::TrackType;
        let order = self.registry.tracks.len();
        let track = match track_type {
            TrackType::Audio => crate::state::TrackState::new_audio(track_id, order),
            TrackType::Instrument => crate::state::TrackState::new_instrument(track_id, order),
            TrackType::Vocal => crate::state::TrackState::new_vocal(track_id, order),
        };
        self.registry.tracks.push(track);
        self.registry.resort_tracks();
    }

    /// Test-only: drive an undo-restore reconciliation directly with a
    /// target freeze map, exercising `apply_freeze_restore` without the
    /// full snapshot/replay pipeline.
    #[doc(hidden)]
    pub fn test_apply_freeze_restore(
        &mut self,
        target: std::collections::HashMap<
            resonance_audio::types::TrackId,
            crate::state::FreezeStatus,
        >,
    ) {
        self.apply_freeze_restore(target);
    }

    /// Test-only: drive the clip fade/gain undo re-apply directly with a
    /// target map, exercising `apply_clip_fade_gain_restore` (the shared
    /// re-sync used by both restore paths) without the full
    /// snapshot/replay pipeline. Mirrors `test_apply_freeze_restore`.
    #[doc(hidden)]
    pub fn test_apply_clip_fade_gain_restore(
        &mut self,
        map: &std::collections::HashMap<resonance_audio::types::ClipId, crate::undo::ClipFadeGain>,
    ) {
        self.apply_clip_fade_gain_restore(map);
    }

    /// Test-only: the current project path. `None` for an untitled project
    /// (including one freshly instantiated from a template).
    #[doc(hidden)]
    pub fn test_project_path(&self) -> Option<&std::path::Path> {
        self.io.project_path.as_deref()
    }

    /// Test-only: whether a project is active (startup modal dismissed).
    #[doc(hidden)]
    pub fn test_has_active_project(&self) -> bool {
        self.io.has_active_project
    }

    // ---- Media pool (doc #175) ---------------------------------------

    /// Test-only: borrow the media pool so persistence / mirror tests can
    /// assert the restored asset list, missing flags, usage counts, and
    /// favourite / recent folder lists.
    #[doc(hidden)]
    pub fn test_pool(&self) -> &crate::state::MediaPool {
        &self.pool
    }

    /// Test-only: add an imported asset to the pool (and refresh usage),
    /// standing in for the import-to-pool orchestration (ba todo #598)
    /// so a persistence test can seed a pool before serializing.
    #[doc(hidden)]
    pub fn test_add_pool_asset(&mut self, asset: crate::state::PoolAsset) {
        self.add_pool_asset(asset);
    }

    /// Test-only: remove an asset from the pool, returning it if present.
    #[doc(hidden)]
    pub fn test_remove_pool_asset(
        &mut self,
        id: resonance_audio::types::AssetId,
    ) -> Option<crate::state::PoolAsset> {
        self.remove_pool_asset(id)
    }

    /// Test-only: point a clip at a pool asset (or clear the link with
    /// `None`) and refresh usage, standing in for the placement /
    /// relink handlers (ba todos #598 / #600).
    #[doc(hidden)]
    pub fn test_relink_clip(
        &mut self,
        clip_id: resonance_audio::types::ClipId,
        asset_id: Option<resonance_audio::types::AssetId>,
    ) {
        self.relink_clip(clip_id, asset_id);
    }

    /// Test-only: replay just the media-pool block of a saved
    /// [`crate::project::ProjectFile`] into this app, resolving relative
    /// asset paths against `project_dir`. Exercises the same restore path
    /// a full project load runs (missing-file flagging, usage recompute)
    /// without constructing a whole `LoadedProject`.
    #[doc(hidden)]
    pub fn test_restore_pool(
        &mut self,
        file: &crate::project::ProjectFile,
        project_dir: &std::path::Path,
    ) {
        crate::update::project_io::restore_pool(self, file, project_dir);
    }

    /// Test-only: borrow the persisted app settings so a test can assert
    /// the media-browser favourites / recent folders that
    /// [`Self::test_sync_media_browser_settings`] wrote.
    #[doc(hidden)]
    pub fn test_settings(&self) -> &crate::settings::AppSettings {
        &self.settings
    }

    /// Test-only: mirror the pool's favourites / recent folders into app
    /// settings *without* writing to disk (doc #175). Pairs with
    /// [`Self::test_settings`] to assert the synced lists in a hermetic
    /// test — unlike [`Self::test_persist_media_browser_settings`], this
    /// never touches the real `config_dir()`.
    #[doc(hidden)]
    pub fn test_sync_media_browser_settings(&mut self) {
        self.sync_media_browser_settings();
    }

    /// Test-only: mirror the pool's favourites / recent folders into app
    /// settings and persist them to disk (doc #175), standing in for the
    /// browser favourite/recent handlers (ba todo #599). Writes to the
    /// real `config_dir()`, so prefer [`Self::test_sync_media_browser_settings`]
    /// in tests that only need to assert the in-memory document.
    #[doc(hidden)]
    pub fn test_persist_media_browser_settings(&mut self) {
        self.persist_media_browser_settings();
    }

    /// Test-only: pin a favourite folder on the pool, standing in for the
    /// browser's favourite toggle (ba todo #599).
    #[doc(hidden)]
    pub fn test_pool_add_favourite(&mut self, path: std::path::PathBuf) {
        self.pool.add_favourite(path);
    }

    /// Test-only: record a most-recently-visited folder on the pool,
    /// standing in for the browser's navigation handler (ba todo #599).
    #[doc(hidden)]
    pub fn test_pool_push_recent(&mut self, path: std::path::PathBuf) {
        self.pool.push_recent_folder(path);
    }
}
