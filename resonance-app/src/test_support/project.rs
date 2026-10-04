//! Project lifecycle, dispatch, undo and persistence test hooks:
//! engine-event/message routing, project-file build/replay, undo
//! snapshots and the io/dirty/error flags.

use crate::state;
use crate::Resonance;

impl Resonance {
    /// Test-only: borrow the MIDI Import modal state (or `None` when the
    /// modal is closed). Drives `tests/import_dialog.rs`, which asserts
    /// the open/close + review-stage plumbing without rendering the
    /// overlay.
    #[doc(hidden)]
    pub fn test_import_dialog(&self) -> Option<&state::ImportDialogState> {
        self.modals.import_dialog.as_ref()
    }

    /// Test-only: borrow the open Export-modal state (`None` when the
    /// modal is closed). Lets `tests/export_dialog_shell.rs` assert the
    /// open/close + mode-tab plumbing through the real `ExportMessage`
    /// reducer without poking at the `pub(crate)` field.
    #[doc(hidden)]
    pub fn test_export_dialog(&self) -> Option<&state::ExportDialogState> {
        self.modals.export_dialog.as_ref()
    }

    /// Test-only: feed an engine event through the real dispatch so
    /// integration tests exercise the same mirroring path the live app
    /// does. The returned follow-up `Task` is dropped — tests assert on
    /// the resulting state, not on emitted messages.
    #[doc(hidden)]
    pub fn test_apply_engine_event(&mut self, event: resonance_audio::types::AudioEvent) {
        let _ = crate::engine_events::handle_engine_event(self, event);
    }

    /// Test-only: the clip ids a manual save's WAV GC would keep right now
    /// (FU-V5a), with `engine_clips` standing in for the engine's report.
    #[doc(hidden)]
    pub fn test_clip_gc_keep(
        &self,
        engine_clips: &[resonance_audio::types::ClipId],
    ) -> Option<std::collections::BTreeSet<resonance_audio::types::ClipId>> {
        let file = crate::update::build_project_file(self);
        self.clip_gc_keep(&file, engine_clips.iter().copied())
    }

    /// Test-only: the save whose engine round-trip is collecting, as
    /// `(target dir, is_autosave)`, or `None` when none is.
    #[doc(hidden)]
    pub fn test_save_in_flight(&self) -> Option<(std::path::PathBuf, bool)> {
        self.io.save_state.as_ref().map(|s| (s.path.clone(), s.autosave))
    }

    /// Test-only: pretend the collecting save started `age` ago, for the
    /// save watchdog (code review STATE2-02).
    #[doc(hidden)]
    pub fn test_age_save_in_flight(&mut self, age: std::time::Duration) {
        if let Some(save) = self.io.save_state.as_mut() {
            save.started = std::time::Instant::now()
                .checked_sub(age)
                .expect("age fits the monotonic clock");
        }
    }

    /// Test-only: the persistent "Autosave failing" line, when shown
    /// (code review UX-13).
    #[doc(hidden)]
    pub fn test_autosave_failing(&self) -> Option<String> {
        self.banners.autosave_failing()
    }

    /// Test-only: the persistent engine-health status line, when the
    /// engine is unhealthy (code review UX-04).
    #[doc(hidden)]
    pub fn test_engine_status(&self) -> Option<&'static str> {
        self.banners.engine_health.message()
    }

    /// Test-only: set the engine health the tick would poll, without
    /// reading the process-wide disconnect latch (for golden images).
    #[doc(hidden)]
    pub fn test_set_engine_health(&mut self, health: state::EngineHealth) {
        self.banners.engine_health = health;
    }

    /// Test-only: the open autosave-recovery prompt, if any (FU-M12a).
    #[doc(hidden)]
    pub fn test_recovery_prompt(&self) -> Option<&state::RecoveryPrompt> {
        self.io.recovery_prompt.as_ref()
    }

    /// Test-only: run the startup scan for a crashed untitled session
    /// that `Resonance::new` runs — in a test app, over the hermetic
    /// scratch root.
    #[doc(hidden)]
    pub fn test_offer_orphaned_session(&mut self) {
        crate::update::project_io::recovery::offer_orphaned_session(self);
    }

    /// Test-only: open the recovery prompt for `offer` directly (for the
    /// modal's golden images).
    #[doc(hidden)]
    pub fn test_set_recovery_prompt(&mut self, prompt: state::RecoveryPrompt) {
        self.io.recovery_prompt = Some(prompt);
    }

    /// Test-only: replace the in-memory autosave settings (nothing is
    /// persisted), e.g. to drive the autosave trigger with a 0 s interval.
    #[doc(hidden)]
    pub fn test_set_autosave_settings(&mut self, settings: crate::settings::AutosaveSettings) {
        self.settings.autosave = settings;
    }

    #[doc(hidden)]
    /// Test-only: point this app's `amp_models.*` handlers at `models` /
    /// `marks` (every test app already gets private temporary ones; this is
    /// for a test that seeds its own). No environment variable involved.
    pub fn test_set_amp_library_roots(&mut self, models: std::path::PathBuf, marks: std::path::PathBuf) {
        self.control.amp_library = crate::update::control::AmpLibraryCache::new(
            crate::update::control::AmpLibraryRoots {
                models: Some(models),
                marks: Some(marks),
            },
        );
    }

    #[doc(hidden)]
    /// Test-only: point this app's `drum_kits.*` handlers at `kits` /
    /// `marks`, as [`Self::test_set_amp_library_roots`] does for the amp.
    pub fn test_set_drum_kit_library_roots(&mut self, kits: std::path::PathBuf, marks: std::path::PathBuf) {
        self.control.drum_kit_library = crate::update::control::DrumKitLibraryCache::new(
            crate::update::control::DrumKitLibraryRoots {
                kits: Some(kits),
                marks: Some(marks),
            },
        );
    }

    /// Test-only: the `drum_kits.*` roots this app uses.
    #[doc(hidden)]
    pub fn test_drum_kit_library_roots(&self) -> crate::update::control::DrumKitLibraryRoots {
        self.control.drum_kit_library.roots.clone()
    }

    /// Test-only: the `amp_models.*` roots this app uses.
    #[doc(hidden)]
    pub fn test_amp_library_roots(&self) -> crate::update::control::AmpLibraryRoots {
        self.control.amp_library.roots.clone()
    }

    /// Test-only: run the tick's deferred-label expiry as of `now`.
    #[doc(hidden)]
    pub fn test_expire_pending_labels(&mut self, now: std::time::Instant) {
        crate::update::control::expire_pending_labels(self, now);
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
    ) -> crossbeam_channel::Receiver<resonance_audio::types::AudioCommand> {
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

    /// Test-only: overwrite the sample rate. Tempo-map projections used
    /// by the MIDI clip trim reducer depend on `sample_rate`; integration
    /// tests fix it to a known value so the projection math is
    /// deterministic.
    #[doc(hidden)]
    pub fn test_set_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
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

    /// Test-only: whether an offline bounce / render is in flight. Lets
    /// the control `render.mixdown` tests (doc #265, todo #1157) confirm
    /// the engine command fired (or, for a guard hit, that it did not).
    #[doc(hidden)]
    pub fn test_is_bouncing(&self) -> bool {
        self.io.bouncing
    }

    /// Test-only: anchor the project at `path` so `can_record_undo`
    /// (which needs a saved-path to replay snapshots against) is true.
    /// Lets reducer tests exercise undo/redo round-trips without going
    /// through a real save dialog.
    #[doc(hidden)]
    pub fn test_set_project_path(&mut self, path: std::path::PathBuf) {
        self.io.project_path = Some(path);
    }

    /// Test-only: point plugin presets at a private directory, so a test
    /// never reads or writes the developer's real
    /// `~/.local/share/resonance/plugin-presets` (ba todo #1333, and the
    /// hermeticity rule in ba doc #285).
    #[doc(hidden)]
    pub fn test_set_plugin_preset_root(&mut self, root: std::path::PathBuf) {
        self.presets.plugin_preset_root = Some(root);
        self.presets.library_cache = std::sync::OnceLock::new();
    }

    /// Test-only: send the parked state loads of preset step runs now,
    /// as the tick does once a run has been quiet for the debounce.
    #[doc(hidden)]
    pub fn test_flush_step_state(&mut self) {
        crate::update::plugin::flush_step_state(self, true);
    }

    /// Test-only: the preset state (the host preset surfaces' lists, the
    /// per-slot loaded-preset identity).
    #[doc(hidden)]
    pub fn test_presets(&self) -> &crate::state::PresetState {
        &self.presets
    }

    /// Test-only: mark the open project dirty (or clean) so the
    /// control-endpoint destructive-op guards (`needs_confirmation` on
    /// `project.new`/`project.open` with unsaved changes, doc #265) can
    /// be exercised without performing a real edit.
    #[doc(hidden)]
    pub fn test_set_dirty(&mut self, dirty: bool) {
        self.session.dirty = dirty;
    }

    /// Test-only: the clip id the app's one clip allocator
    /// (`EntityIds::clips`, D-7b) would hand out next, without taking it.
    #[doc(hidden)]
    pub fn test_next_clip_id(&self) -> resonance_audio::types::ClipId {
        self.media.ids.clips.peek()
    }

    /// Test-only: serialize the current GUI state to the on-disk
    /// [`crate::project::ProjectFile`] shape, so a persistence test can
    /// inspect (or round-trip) the reference A/B block without writing to
    /// disk.
    #[doc(hidden)]
    pub fn test_build_project_file(&self) -> crate::project::ProjectFile {
        crate::update::project_io::build_project_file(self)
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

    /// Test-only: like [`Self::test_handle_engine_event`], but hand back
    /// the `Task` the event produced (e.g. the post-load `scroll_to`).
    #[doc(hidden)]
    pub fn test_engine_event_task(
        &mut self,
        event: resonance_audio::types::AudioEvent,
    ) -> iced::Task<crate::message::Message> {
        crate::engine_events::handle_engine_event(self, event)
    }

    /// Test-only: capture an undo snapshot of the current declarative
    /// state, so a test can inspect what a restore would bring back or
    /// hand it to [`Self::test_begin_restore_from_snapshot`].
    #[doc(hidden)]
    pub fn test_snapshot_for_undo(&self) -> crate::undo::UndoSnapshot {
        self.snapshot_for_undo()
    }

    /// Test-only: whether two snapshots describe the same undoable state
    /// (project file and notes) — the fixed-point check the
    /// snapshot/restore invariant test runs after each restore path.
    #[doc(hidden)]
    pub fn test_snapshot_same_state(a: &crate::undo::UndoSnapshot, b: &crate::undo::UndoSnapshot) -> bool {
        a.same_state(b)
    }

    /// Test-only: the gesture-end check `commit_undo_gesture` runs —
    /// whether the live state differs from the snapshot a gesture opened
    /// with (code review STATE-07). Exposed for the snapshot cost probe.
    #[doc(hidden)]
    pub fn test_gesture_changed_since(&self, before: &crate::undo::UndoSnapshot) -> bool {
        self.gesture_changed_since(before)
    }

    /// Test-only: borrow the undo history, so the import-placement tests
    /// can assert a single pre-import entry was recorded (ba todo #598).
    #[doc(hidden)]
    pub fn test_undo_history(&self) -> &crate::undo::UndoHistory {
        &self.session.undo
    }

    /// Test-only: drop the undo history and the dirty flag, so a fixture
    /// that seeds state through recorded-take events (which are undoable
    /// edits, STATE-02) starts its test from a clean history.
    #[doc(hidden)]
    pub fn test_forget_history(&mut self) {
        self.session.undo.clear();
        self.session.dirty = false;
    }

    /// Test-only: restore a previously captured snapshot as an undo/redo
    /// does — in place, by diff against the live state (`reconcile_all`
    /// with `Origin::Undo`), synchronously and without a `ClearAll`.
    #[doc(hidden)]
    pub fn test_begin_restore_from_snapshot(&mut self, snapshot: crate::undo::UndoSnapshot) {
        self.begin_restore_from_snapshot(snapshot);
    }

    /// Test-only: route a message through the *full* `update()` entry,
    /// including the pre-dispatch gates and undo bookkeeping. Used by the
    /// frozen-input read-only gating tests (ba todo #576), which need the
    /// gate that `test_dispatch` deliberately skips.
    #[doc(hidden)]
    pub fn test_update(&mut self, message: crate::message::Message) {
        let _ = self.update(message);
    }

    /// Test-only: whether the project is marked dirty (unsaved changes). A
    /// blocked frozen-input edit must leave this untouched — proof it never
    /// mutated state or recorded undo.
    #[doc(hidden)]
    pub fn test_dirty(&self) -> bool {
        self.session.dirty
    }

    /// Test-only: whether the undo stack has a restorable snapshot. A
    /// blocked frozen-input edit must not grow it.
    #[doc(hidden)]
    pub fn test_can_undo(&self) -> bool {
        self.session.undo.can_undo()
    }

    /// Test-only: the `Reconcile` domains the last restore ran, in order,
    /// with its origin (ARCH-01 A-13).
    #[doc(hidden)]
    pub fn test_reconcile_trace(
        &self,
    ) -> &[(crate::update::project_io::reconcile::Origin, &'static str)] {
        &self.io.reconcile_trace
    }

    /// Test-only: no removal / move echo of a diff restore is still owed
    /// (ARCH-01 A-13h, `RestoreEchoes`).
    #[doc(hidden)]
    pub fn test_restore_echoes_settled(&self) -> bool {
        self.io.restore_echoes.is_empty()
    }

    /// Test-only: the chain the plugin side-index files `instance_id`
    /// under, if any.
    #[doc(hidden)]
    pub fn test_plugin_index(
        &self,
        instance_id: resonance_audio::types::PluginInstanceId,
    ) -> Option<crate::state::ChainOwner> {
        self.plugin_mirror.index.get(&instance_id).copied()
    }

    /// Test-only: the current project path. `None` for an untitled project
    /// (including one freshly instantiated from a template).
    #[doc(hidden)]
    pub fn test_project_path(&self) -> Option<&std::path::Path> {
        self.io.project_path.as_deref()
    }

    /// Test-only: the token of the most recently started disk open — the
    /// one whose `OpenLoadFinished` will be adopted (FU-A1a).
    #[doc(hidden)]
    pub fn test_pending_open_token(&self) -> u64 {
        self.io.open_token
    }

    /// Test-only: whether a project is active (startup modal dismissed).
    #[doc(hidden)]
    pub fn test_has_active_project(&self) -> bool {
        self.io.has_active_project
    }

    /// Test-only: borrow the persisted app settings so a test can assert
    /// the media-browser favourites / recent folders that
    /// [`Self::test_sync_media_browser_settings`] wrote.
    #[doc(hidden)]
    pub fn test_settings(&self) -> &crate::settings::AppSettings {
        &self.settings
    }

    /// Test-only: borrow the recent-projects list. Lets
    /// `tests/hermetic_construction.rs` prove the hermetic constructor read
    /// nothing out of the user's config (ba doc #285).
    #[doc(hidden)]
    pub fn test_recent_projects(&self) -> &[crate::recent::RecentEntry] {
        &self.io.recent_projects
    }

    /// Test-only: borrow the user's saved track presets. Same purpose as
    /// [`Self::test_recent_projects`] — the built-in presets live in
    /// `default_presets` and are unaffected.
    #[doc(hidden)]
    pub fn test_user_presets(&self) -> &[crate::presets::TrackPreset] {
        &self.presets.user_presets
    }

    /// Test-only: list the device definitions the registry resolved. Used to
    /// show that the *compiled-in* bundled definitions survive hermetic
    /// construction even though the user-authored directory is not read.
    #[doc(hidden)]
    pub fn test_device_definitions(&self) -> Vec<&resonance_common::DeviceDefinition> {
        self.devices.registry.list()
    }

    /// Test-only: read the current error message banner, if any.
    #[doc(hidden)]
    pub fn test_error_message(&self) -> Option<&str> {
        self.banners.error_message.as_deref()
    }

    /// Test-only: returns `true` when a user-facing error message has been
    /// set (i.e. `error_message` is `Some`). Used by import-gate tests that
    /// need to confirm the app showed an error without reading private fields.
    #[doc(hidden)]
    pub fn test_error_message_is_set(&self) -> bool {
        self.banners.error_message.is_some()
    }

    /// Test-only: replay a [`crate::project::ProjectFile`] into this app as
    /// if it had just been loaded from disk, rebuilding GUI state and
    /// re-issuing engine commands. `midi_notes` are taken as empty (tests
    /// that need notes can extend this); the project dir is a placeholder
    /// since lane/transport replay needs no on-disk files.
    #[doc(hidden)]
    pub fn test_replay_loaded_project(&mut self, file: crate::project::ProjectFile) {
        let loaded = crate::project::LoadedProject {
            file,
            project_dir: std::path::PathBuf::from("/tmp/resonance-test-project.rproj"),
            midi_notes: std::collections::HashMap::new(),
            plugin_states: std::collections::HashMap::new(),
        };
        crate::update::project_io::replay_loaded_project(self, Box::new(loaded));
    }

    /// Test-only: replay a whole [`crate::project::LoadedProject`] — the
    /// shape [`crate::project::load_project`] returns, opaque plugin-state
    /// blobs included. Unlike [`Self::test_replay_loaded_project`] this
    /// lets a test open a project that really came off disk, which is the
    /// only way to exercise what happens to a plugin's blob when the
    /// plugin itself can't be instantiated.
    #[doc(hidden)]
    pub fn test_replay_loaded_project_from(&mut self, loaded: crate::project::LoadedProject) {
        crate::update::project_io::replay_loaded_project(self, Box::new(loaded));
    }

    /// Test-only: the plugin-state blobs a save would write, given the
    /// blobs the engine reported for its live instances (empty for a
    /// project whose plugins all failed to instantiate). Exactly what the
    /// save collector and template capture use.
    #[doc(hidden)]
    pub fn test_plugin_states_for_save(
        &self,
        engine_states: Vec<(resonance_audio::types::PluginInstanceId, Vec<u8>)>,
    ) -> Vec<(resonance_audio::types::PluginInstanceId, Vec<u8>)> {
        crate::update::plugin_states_for_save(self, engine_states)
    }

    /// Test-only: capture the open project as a user template under
    /// `root`, instead of the user's real template library. Drives the
    /// same body the `SaveAsTemplate` message runs.
    #[doc(hidden)]
    pub fn test_save_as_template_in(
        &self,
        root: &std::path::Path,
        name: &str,
        description: &str,
    ) -> Result<std::path::PathBuf, String> {
        crate::update::project_io::save_current_as_template_in(
            self,
            root,
            name,
            description,
            crate::update::project_io::TemplateCaptureOptions {
                include_markers_and_tempo: true,
                include_master_chain: true,
            },
        )
    }

    /// Test-only: the untitled project's clip/undo anchor dir, if one was
    /// set up when it landed (code review UX-03).
    #[doc(hidden)]
    pub fn test_untitled_anchor(&self) -> Option<&std::path::Path> {
        self.io.untitled_anchor.as_deref()
    }

    /// Test-only: the brief undo / redo notice beside the project title
    /// (code review UX-12).
    #[doc(hidden)]
    pub fn test_history_notice(&self) -> Option<&str> {
        self.banners.history_notice.as_ref().map(|n| n.text.as_str())
    }

    /// Test-only: age the undo / redo notice as the tick would at `now`.
    #[doc(hidden)]
    pub fn test_expire_history_notice(&mut self, now: std::time::Instant) {
        self.banners.expire_history_notice(now);
    }

    /// Test-only: the switch parked behind the unsaved-changes dialog
    /// (code review UX-01).
    #[doc(hidden)]
    pub fn test_confirm_switch(&self) -> Option<&state::ProjectSwitch> {
        self.modals.confirm_switch.as_ref()
    }

    /// Test-only: the switch waiting on the dialog's "Save".
    #[doc(hidden)]
    pub fn test_switch_after_save(&self) -> Option<&state::ProjectSwitch> {
        self.modals.switch_after_save.as_ref()
    }

    /// Test-only: the user preset whose inline delete confirm is showing
    /// (code review UX-14).
    #[doc(hidden)]
    pub fn test_preset_delete_armed(&self) -> Option<&str> {
        self.ui.mixer.preset_delete_armed.as_deref()
    }

    /// Test-only: replace the user track presets the add-track menu lists
    /// (no disk read), for the UX-14 confirm snapshot.
    #[doc(hidden)]
    pub fn test_set_user_presets(&mut self, presets: Vec<crate::presets::TrackPreset>) {
        self.presets.user_presets = presets;
    }
}
