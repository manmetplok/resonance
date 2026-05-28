//! Library crate root for `resonance-app`. The binary entry point lives
//! in `main.rs` and is a thin wrapper around this library — splitting
//! the modules into a library lets the integration tests under
//! `resonance-app/tests/` exercise the real `view()` / `update()` paths
//! via `iced_test`.
//!
//! Visibility note: most modules and types were originally `pub(crate)`
//! while this crate was binary-only. They have been promoted to `pub`
//! as needed for the test fixtures (`demo::seed_demo_content`,
//! `Resonance`, `Message`, etc.) — everything else stays `pub(crate)`.

use resonance_audio::MidiDeviceInfo;
use resonance_audio::types::*;
use resonance_audio::AudioEngine;
use resonance_music_theory::TableRegistry;

pub mod chord_sheet_pdf;
pub mod compose;
pub mod demo;
pub mod piano_roll;
pub mod engine_events;
pub mod message;
pub mod midi_editor;
pub mod presets;
pub mod project;
pub mod recent;
pub mod state;
pub mod theme;
pub mod timeline;
pub mod timeline_draw;
pub mod timeline_input;
pub mod timeline_snap;
pub mod undo;
pub mod update;
pub mod util;
pub mod view;

pub use message::Message;
use state::*;
use undo::UndoHistory;

/// Application state.
pub struct Resonance {
    pub engine: AudioEngine,
    pub sample_rate: u32,
    pub(crate) input_devices: Vec<InputDeviceInfo>,
    pub(crate) default_input_device_name: Option<String>,
    /// Hardware MIDI input ports advertised by the OS. Refreshed
    /// periodically from `Tick` so hot-plugged devices appear.
    pub(crate) midi_input_devices: Vec<MidiDeviceInfo>,
    /// Hardware MIDI output ports.
    pub(crate) midi_output_devices: Vec<MidiDeviceInfo>,
    /// Wall-clock instant of the last MIDI device list refresh.
    pub(crate) midi_devices_last_refresh: std::time::Instant,
    /// Whether MIDI clock master output is enabled.
    pub(crate) midi_clock_send_enabled: bool,
    /// Hardware MIDI output port carrying the master clock.
    pub(crate) midi_clock_send_device: Option<String>,
    /// Whether MIDI clock slave (input) is enabled.
    pub(crate) midi_clock_recv_enabled: bool,
    /// Hardware MIDI input port carrying the master clock.
    pub(crate) midi_clock_recv_device: Option<String>,
    pub(crate) available_plugins: Vec<ScannedPlugin>,
    /// Cached pick-list option lists for the view layer. Rebuilt only
    /// when source data changes (devices, busses, plugin scan) so a
    /// continuous resize doesn't reallocate option vecs every frame.
    /// See `view::ui_caches` for the cache and rebuild API.
    pub(crate) view_caches: view::ui_caches::UiViewCaches,
    /// Lazy-memoised label strings for the transport bar's stat blocks
    /// (position, time, sig, key, loop). Re-formatted only when the
    /// underlying inputs change; refreshed once per paint from
    /// `view::transport::view_transport`. `RefCell` because `view()`
    /// takes `&self`. See `view::transport_labels`.
    pub(crate) transport_labels: std::cell::RefCell<view::transport_labels::TransportLabels>,
    pub(crate) error_message: Option<String>,
    pub(crate) master_volume: f32,
    pub(crate) master_level_l: f32,
    pub(crate) master_level_r: f32,
    /// FX plugins inserted on the master bus, rendered after every
    /// track and bus has been summed.
    pub(crate) master_plugins: Vec<PluginSlotState>,
    /// When true, the master FX chain is bypassed — the master fader
    /// and metering still run, but no master-bus plugins are processed.
    pub(crate) master_fx_bypassed: bool,
    pub(crate) view_mode: ViewMode,
    /// Audio clips on the timeline.
    pub(crate) clips: Vec<ClipState>,
    /// MIDI clips on the timeline.
    pub(crate) midi_clips: Vec<MidiClipState>,
    /// Compose tab state: section definitions, placements, chord progressions.
    pub(crate) compose: compose::ComposeState,
    /// Markov table registry for chord generators. Constructed once at
    /// startup with all built-in tables.
    pub(crate) table_registry: TableRegistry,

    /// Tempo change events on the tempo track (sorted by bar number).
    pub(crate) tempo_events: Vec<state::TempoEvent>,
    /// Time signature change events on the signature track (sorted by bar).
    pub(crate) signature_events: Vec<state::SignatureEvent>,
    /// GUI-side tempo map — shared implementation with the audio engine.
    /// Rebuilt from `tempo_events` / `signature_events` whenever they change.
    pub(crate) tempo_map: TempoMap,

    // Sub-state groupings. See `state.rs` for definitions.
    pub(crate) transport: TransportState,
    pub(crate) viewport: ArrangeViewport,
    pub(crate) interaction: ClipInteractionState,
    pub(crate) io: ProjectIoState,
    pub(crate) mixer: MixerUiState,
    pub(crate) registry: TrackRegistry,
    /// Session-local undo/redo history. Cleared on project load.
    pub(crate) undo: UndoHistory,
    /// When set, the confirmation dialog for deleting a track with
    /// content is shown. Holds the track id that the user wants to remove.
    pub(crate) confirm_delete_track: Option<resonance_audio::types::TrackId>,
    /// When set, the "Bounce in place" dialog is shown for an external
    /// MIDI track. Holds the source track id plus the user's current
    /// device/port selection.
    pub(crate) bounce_dialog: Option<crate::view::bounce_dialog::BounceDialogState>,
    /// When set, a bounce-in-place run is in flight. Drives the modal
    /// progress overlay and gates transport / mutating UI so the user
    /// can't disturb the render mid-flight. Cleared by
    /// `TrackBounceCompleted`, `TrackBounceError`, or
    /// `TrackBounceCancelled`.
    pub(crate) bounce_in_progress: Option<crate::state::BounceProgressState>,
    /// True when the project has been modified since the last save.
    pub(crate) dirty: bool,
    /// When set, the "unsaved changes" quit-confirmation dialog is shown.
    /// Holds the window id so we can close it if the user confirms.
    pub(crate) confirm_quit: Option<iced::window::Id>,
    /// When set, the app should quit after the current save completes.
    /// Set by the "Save & Quit" flow in the unsaved-changes dialog.
    pub(crate) quit_after_save: Option<iced::window::Id>,
    /// Cache of the most recently observed CLAP state blob per plugin
    /// instance. Populated from `PluginStateSaved` / `AllPluginStatesSaved`
    /// engine events and read into undo snapshots so restores can replay
    /// plugin internal state via `LoadPluginState`. Stale between
    /// refreshes — parameter values in snapshots always come from live
    /// GUI state instead.
    pub(crate) plugin_state_cache:
        std::collections::HashMap<resonance_audio::types::PluginInstanceId, Vec<u8>>,

    /// Side-index mapping every live plugin instance to the slot that
    /// owns it (a track, a bus, or master). Kept in sync with
    /// `registry.tracks[*].plugins`, `registry.busses[*].plugins`, and
    /// `master_plugins` by `insert_plugin_index` / `remove_plugin_index`
    /// at each add/remove site, and wholesale via `rebuild_plugin_index`
    /// after seed / replay. Replaces the O(tracks × plugins) scan that
    /// `with_plugin_mut` did pre-index.
    pub(crate) plugin_index: std::collections::HashMap<
        resonance_audio::types::PluginInstanceId,
        crate::state::PluginLocator,
    >,

    // ---- Track presets ----
    /// Built-in default track presets (baked into the binary).
    pub(crate) default_presets: Vec<presets::TrackPreset>,
    /// User-saved track presets (loaded from disk on startup).
    pub(crate) user_presets: Vec<presets::TrackPreset>,
    /// When set, the next `TrackAdded` / `InstrumentTrackAdded` engine
    /// event will apply this preset to the newly created track.
    pub(crate) pending_track_preset: Option<presets::TrackPreset>,
    /// When set, the next `AllPluginStatesSaved` event will capture
    /// plugin states for this track and save it as a user preset.
    pub(crate) pending_preset_save: Option<resonance_audio::types::TrackId>,
    /// Plugin state blobs to apply as PluginAdded events arrive for a
    /// preset-created track. Tuple of (target track id, ordered list of
    /// state blobs matching the preset's plugin chain).
    pub(crate) pending_preset_plugin_states:
        Option<(resonance_audio::types::TrackId, Vec<Option<Vec<u8>>>)>,
}

/// Startup tab requested via `--tab arrange|mixer|compose`. Read once at
/// `main` and threaded into `Resonance::new()` via this module-local
/// statics — keeps the iced application builder closure capture-free.
pub static STARTUP_TAB: std::sync::OnceLock<ViewMode> = std::sync::OnceLock::new();

/// Parse `--tab arrange|mixer|compose` (or `--tab=...`) from process args.
/// Returns `None` when the flag isn't present or the value is unknown.
pub fn parse_startup_tab() -> Option<ViewMode> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = if let Some(v) = arg.strip_prefix("--tab=") {
            v.to_string()
        } else if arg == "--tab" {
            args.next()?
        } else {
            continue;
        };
        return match value.to_ascii_lowercase().as_str() {
            "arrange" => Some(ViewMode::Arrange),
            "mixer" => Some(ViewMode::Mixer),
            "compose" => Some(ViewMode::Compose),
            other => {
                eprintln!("Unknown --tab value '{other}'. Expected arrange|mixer|compose.");
                None
            }
        };
    }
    None
}

impl Resonance {
    /// Read-only view onto the Compose tab's runtime state. Surfaced
    /// so integration tests (`tests/*.rs`) can interrogate section /
    /// pattern state without depending on the engine I/O paths.
    pub fn compose_state(&self) -> &compose::ComposeState {
        &self.compose
    }

    /// Read-only view onto the track registry. Surfaced for integration
    /// tests that need to look up the demo's drum track id without
    /// hard-coding it.
    pub fn track_registry(&self) -> &state::TrackRegistry {
        &self.registry
    }

    // Tempo / signature mutators live in `update/global_track.rs` —
    // they're the helpers that the `GlobalTrackMessage` handler uses to
    // keep the GUI-side `tempo_map` and the engine's tempo state in
    // sync. Plugin-index trio + `with_plugin_mut` live in
    // `state/plugin_index.rs`; `track_id_at_arrange_y` lives in
    // `state/arrange.rs`.

    // ---- Public read-only accessors for integration tests ----
    //
    // These exist so `tests/*.rs` files (which are external compile
    // units and see only public API) can verify reducer-driven state
    // changes without poking at private fields. They're `#[doc(hidden)]`
    // because they aren't part of the library's user-facing surface —
    // application code inside the crate still goes through the
    // `pub(crate)` fields directly.

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

    /// Test-only: borrow the track registry to walk `sorted_tracks()` /
    /// inspect sub-track links from an integration test (which doesn't
    /// see `pub(crate)` fields). Used by
    /// `tests/mixer_sub_track_grouping.rs` to assert the displayed
    /// strip order without parsing the rendered widget tree.
    #[doc(hidden)]
    pub fn test_registry(&self) -> &state::TrackRegistry {
        &self.registry
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

    /// Test-only: flip the project-active flag so the message gate in
    /// `gates_message` lets reducer-driven `MidiClipMessage` /
    /// `ClipMessage` traffic through. Demo seeding does this in the
    /// real app; reducer tests that don't seed the demo flip it
    /// directly.
    #[doc(hidden)]
    pub fn test_set_active_project(&mut self, active: bool) {
        self.io.has_active_project = active;
    }

    pub(crate) fn sorted_tracks(&self) -> &[TrackState] {
        self.registry.sorted_tracks()
    }

    pub(crate) fn sorted_busses(&self) -> &[BusState] {
        self.registry.sorted_busses()
    }

    /// Run `f` on the track with the given id, returning whatever `f`
    /// returns. `None` if the track doesn't exist.
    pub(crate) fn with_track_mut<R>(
        &mut self,
        id: TrackId,
        f: impl FnOnce(&mut TrackState) -> R,
    ) -> Option<R> {
        let result = self.registry.with_track_mut(id, f);
        debug_assert!(result.is_some(), "with_track_mut: no track with id {id:?}");
        result
    }

    /// Run `f` on the bus with the given id, returning whatever `f`
    /// returns. `None` if the bus doesn't exist.
    pub(crate) fn with_bus_mut<R>(
        &mut self,
        id: BusId,
        f: impl FnOnce(&mut BusState) -> R,
    ) -> Option<R> {
        let result = self.registry.with_bus_mut(id, f);
        debug_assert!(result.is_some(), "with_bus_mut: no bus with id {id:?}");
        result
    }

    /// Construct the application state used by the binary's `iced::application`
    /// builder. Spins up the real audio engine, requests initial device /
    /// plugin lists, and seeds an empty project. On engine init failure
    /// shows a native error dialog and exits — there is no headless path.
    pub fn new() -> (Self, iced::Task<Message>) {
        let engine = match AudioEngine::new() {
            Ok(engine) => engine,
            Err(e) => {
                // Surface the failure as a native dialog so the user
                // sees the cause instead of a stderr backtrace, then
                // exit cleanly. The app cannot run without an audio
                // engine; we don't currently support running
                // headless / engine-offline.
                eprintln!("Audio engine init failed: {e}");
                rfd::MessageDialog::new()
                    .set_title("Resonance — Audio device not available")
                    .set_description(format!(
                        "The audio engine could not be started:\n\n{e}\n\n\
                         Check that an audio output device is connected and that \
                         no other application is holding it exclusively, then \
                         relaunch Resonance."
                    ))
                    .set_level(rfd::MessageLevel::Error)
                    .show();
                std::process::exit(1);
            }
        };

        // Request input device list and plugin scan on startup
        let _ = engine.send(AudioCommand::ListInputDevices);
        let _ = engine.send(AudioCommand::ListMidiInputDevices);
        let _ = engine.send(AudioCommand::ListMidiOutputDevices);
        let _ = engine.send(AudioCommand::ScanPlugins);

        let recent_projects = recent::load();

        let app = Self {
            engine,
            sample_rate: 44100, // overwritten by SampleRateDetected event
            input_devices: Vec::new(),
            default_input_device_name: None,
            midi_input_devices: Vec::new(),
            midi_output_devices: Vec::new(),
            midi_devices_last_refresh: std::time::Instant::now(),
            midi_clock_send_enabled: false,
            midi_clock_send_device: None,
            midi_clock_recv_enabled: false,
            midi_clock_recv_device: None,
            available_plugins: Vec::new(),
            view_caches: view::ui_caches::UiViewCaches::default(),
            transport_labels: std::cell::RefCell::new(
                view::transport_labels::TransportLabels::default(),
            ),
            error_message: None,
            master_volume: 0.0, // 0 dB = unity gain
            master_level_l: 0.0,
            master_level_r: 0.0,
            master_plugins: Vec::new(),
            master_fx_bypassed: false,
            view_mode: STARTUP_TAB.get().copied().unwrap_or(ViewMode::Arrange),
            clips: Vec::new(),
            midi_clips: Vec::new(),
            compose: compose::ComposeState::default(),
            table_registry: TableRegistry::with_builtins(),

            tempo_events: vec![state::TempoEvent { bar: 0, bpm: 120.0 }],
            signature_events: vec![state::SignatureEvent {
                bar: 0,
                numerator: 4,
                denominator: 4,
            }],
            tempo_map: TempoMap::default(),

            transport: TransportState::default(),
            viewport: ArrangeViewport::default(),
            interaction: ClipInteractionState::default(),
            io: ProjectIoState {
                recent_projects,
                ..ProjectIoState::default()
            },
            mixer: MixerUiState::default(),
            registry: TrackRegistry {
                next_sub_track_id: 1_000_000_000,
                ..TrackRegistry::default()
            },
            undo: UndoHistory::new(),
            plugin_state_cache: std::collections::HashMap::new(),
            plugin_index: std::collections::HashMap::new(),
            confirm_delete_track: None,
            bounce_dialog: None,
            bounce_in_progress: None,
            dirty: false,
            confirm_quit: None,
            quit_after_save: None,
            default_presets: presets::default_presets(),
            user_presets: presets::load_user_presets(),
            pending_track_preset: None,
            pending_preset_save: None,
            pending_preset_plugin_states: None,
        };

        (app, iced::Task::none())
    }
}

