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
        self.import_dialog.as_ref()
    }

    /// Test-only: borrow the open Export-modal state (`None` when the
    /// modal is closed). Lets `tests/export_dialog_shell.rs` assert the
    /// open/close + mode-tab plumbing through the real `ExportMessage`
    /// reducer without poking at the `pub(crate)` field.
    #[doc(hidden)]
    pub fn test_export_dialog(&self) -> Option<&state::ExportDialogState> {
        self.export_dialog.as_ref()
    }

    /// Test-only: feed an engine event through the real dispatch so
    /// integration tests exercise the same mirroring path the live app
    /// does. The returned follow-up `Task` is dropped — tests assert on
    /// the resulting state, not on emitted messages.
    #[doc(hidden)]
    pub fn test_apply_engine_event(&mut self, event: resonance_audio::types::AudioEvent) {
        let _ = crate::engine_events::handle_engine_event(self, event);
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
        self.plugin_preset_root = Some(root);
    }

    /// Test-only: mark the open project dirty (or clean) so the
    /// control-endpoint destructive-op guards (`needs_confirmation` on
    /// `project.new`/`project.open` with unsaved changes, doc #265) can
    /// be exercised without performing a real edit.
    #[doc(hidden)]
    pub fn test_set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
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

    /// Test-only: capture an undo snapshot of the current declarative
    /// state, so a test can inspect what a restore would bring back or
    /// hand it to [`Self::test_begin_restore_from_snapshot`].
    #[doc(hidden)]
    pub fn test_snapshot_for_undo(&self) -> crate::undo::UndoSnapshot {
        self.snapshot_for_undo()
    }

    /// Test-only: whether two snapshots describe the same undoable state
    /// (project file, notes and extras) — the fixed-point check the
    /// snapshot/restore invariant test runs after each restore path.
    #[doc(hidden)]
    pub fn test_snapshot_same_state(a: &crate::undo::UndoSnapshot, b: &crate::undo::UndoSnapshot) -> bool {
        a.same_state(b)
    }

    /// Test-only: borrow the undo history, so the import-placement tests
    /// can assert a single pre-import entry was recorded (ba todo #598).
    #[doc(hidden)]
    pub fn test_undo_history(&self) -> &crate::undo::UndoHistory {
        &self.undo
    }

    /// Test-only: drop the undo history and the dirty flag, so a fixture
    /// that seeds state through recorded-take events (which are undoable
    /// edits, STATE-02) starts its test from a clean history.
    #[doc(hidden)]
    pub fn test_forget_history(&mut self) {
        self.undo.clear();
        self.dirty = false;
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
        self.dirty
    }

    /// Test-only: whether the undo stack has a restorable snapshot. A
    /// blocked frozen-input edit must not grow it.
    #[doc(hidden)]
    pub fn test_can_undo(&self) -> bool {
        self.undo.can_undo()
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
        &self.user_presets
    }

    /// Test-only: list the device definitions the registry resolved. Used to
    /// show that the *compiled-in* bundled definitions survive hermetic
    /// construction even though the user-authored directory is not read.
    #[doc(hidden)]
    pub fn test_device_definitions(&self) -> Vec<&resonance_common::DeviceDefinition> {
        self.device_registry.list()
    }

    /// Test-only: capture the current undo snapshot's runtime extras (the
    /// part of an undo entry that the `ProjectFile` shape doesn't carry,
    /// including external-instrument config). Lets a reducer test prove the
    /// external-instrument config is captured for undo without standing up
    /// the async engine replay loop.
    #[doc(hidden)]
    pub fn test_snapshot_undo_extras(&self) -> crate::undo::UndoExtras {
        self.snapshot_for_undo().extras
    }

    /// Test-only: read the current error message banner, if any.
    #[doc(hidden)]
    pub fn test_error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    /// Test-only: returns `true` when a user-facing error message has been
    /// set (i.e. `error_message` is `Some`). Used by import-gate tests that
    /// need to confirm the app showed an error without reading private fields.
    #[doc(hidden)]
    pub fn test_error_message_is_set(&self) -> bool {
        self.error_message.is_some()
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
}
