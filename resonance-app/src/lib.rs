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

pub mod chord_box;
pub mod chord_sheet_pdf;
pub mod commands;
pub mod chord_track;
pub mod compose;
pub mod demo;
pub mod engine_events;
pub mod focus;
pub mod message;
pub mod presets;
pub mod project;
pub mod recent;
pub mod reference;
pub mod settings;
pub mod state;
mod test_support;
pub mod theme;
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
    /// underlying inputs change. Refreshed by `refresh_transport_labels`
    /// after every `update()` dispatch (plus at construction and after
    /// demo seeding) so `view()` only ever reads it — the view layer
    /// never mutates state. See `view::transport_labels`.
    pub(crate) transport_labels: view::transport_labels::TransportLabels,
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
    /// The view that was active when Performance mode was entered, so
    /// exiting (`F` toggle / `Esc` / the Exit button) returns the user to
    /// where they were rather than always to Arrange. `None` whenever the
    /// current `view_mode` is not `Performance`.
    pub(crate) pre_performance_view: Option<ViewMode>,
    /// Audio clips on the timeline.
    pub(crate) clips: Vec<ClipState>,
    /// MIDI clips on the timeline.
    pub(crate) midi_clips: Vec<MidiClipState>,
    /// App-side groove library: templates extracted from clips via the
    /// engine's `ExtractGrooveFromClip` command. Populated purely from
    /// `GrooveExtracted` engine events (ba todo #390) so the Compose /
    /// quantize UI can later offer them as "apply groove" presets.
    pub(crate) groove_library: Vec<resonance_audio::quantize::GrooveTemplate>,
    /// Compose tab state: section definitions, placements, chord progressions.
    pub(crate) compose: compose::ComposeState,

    /// Media pool: imported audio assets referenced by clips, plus the
    /// browser's favourite / recent folder lists (doc #175). Asset list
    /// and clip asset-refs persist in the project file; favourites and
    /// recent folders persist in user settings. See `state::pool`.
    pub(crate) pool: state::MediaPool,

    /// Reference-track (A/B) comparison state. See `crate::reference`.
    pub(crate) reference: reference::ReferenceState,
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
    /// Global chord track — song-wide harmonic backbone (chord regions +
    /// key context), timeline metadata owned by the app alongside the
    /// tempo/signature tracks. Pure metadata: nothing is sent to the
    /// realtime engine. See `chord_track`.
    pub(crate) chord_track: chord_track::ChordTrack,

    /// MIDI Learn / hardware control-surface mapping, mirrored from the
    /// engine's active binding set. A pure projection of `MidiBinding*` /
    /// `ControlSurface*` events — see `state::MidiMapState`.
    pub(crate) midi_map: MidiMapState,

    // Sub-state groupings. See `state.rs` for definitions.
    pub(crate) transport: TransportState,
    pub(crate) viewport: ArrangeViewport,
    pub(crate) markers: state::ArrangementMarkers,
    pub(crate) interaction: ClipInteractionState,
    pub(crate) io: ProjectIoState,
    pub(crate) mixer: MixerUiState,
    pub(crate) registry: TrackRegistry,
    /// GUI-side mirror of the engine's aux-send graph, reconstructed
    /// purely from `AuxSendChanged` / `AuxSendRemoved` / `AuxSendRejected`
    /// events. Bus return-role rides on `BusState::is_return`.
    pub(crate) aux: state::AuxSendState,
    /// Session-local undo/redo history. Cleared on project load.
    pub(crate) undo: UndoHistory,
    /// When set, the confirmation dialog for deleting a track with
    /// content is shown. Holds the track id that the user wants to remove.
    pub(crate) confirm_delete_track: Option<resonance_audio::types::TrackId>,
    /// When set, the "Bounce in place" dialog is shown for an external
    /// MIDI track. Holds the source track id plus the user's current
    /// device/port selection.
    pub(crate) bounce_dialog: Option<crate::state::BounceDialogState>,
    /// When set, the "Import MIDI" modal is shown. Holds the import
    /// flow's stage, the parsed per-track rows, and the user's tempo /
    /// placement choices. `None` when the modal is closed.
    pub(crate) import_dialog: Option<crate::state::ImportDialogState>,
    /// When set, a bounce-in-place run is in flight. Drives the modal
    /// progress overlay and gates transport / mutating UI so the user
    /// can't disturb the render mid-flight. Cleared by
    /// `TrackBounceCompleted`, `TrackBounceError`, or
    /// `TrackBounceCancelled`.
    pub(crate) bounce_in_progress: Option<crate::state::BounceProgressState>,
    /// When set, the Export modal is open. Holds the shared shell state
    /// (mode tab, source selection, range, format, destination) - see
    /// `state::ExportDialogState` and `view::export_dialog`.
    pub(crate) export_dialog: Option<crate::state::ExportDialogState>,
    /// App-side track-freeze orchestration: per-track freeze status plus
    /// the active "freeze selected / all" batch queue. Driven by the
    /// `FreezeMessage` handlers (ba todo #574) and the engine freeze-event
    /// mirror (ba todo #575). Cleared on project load.
    pub(crate) freeze: crate::state::FreezeState,
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

    /// Persistent application settings (autosave config, …), loaded from
    /// `config_dir()/resonance/settings.json` on startup. Read via
    /// [`Resonance::autosave_settings`]; re-persisted with
    /// `settings::persist` whenever the user changes them.
    pub(crate) settings: settings::AppSettings,

    /// Stable per-process identifier (pid + startup timestamp). Used to
    /// namespace the autosave scratch dir for a never-saved project so
    /// concurrent app instances never collide (epic #32 / doc #171).
    pub(crate) session_id: String,

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

/// Startup tab requested via `--tab arrange|mixer|compose|performance`. Read
/// once at `main` and threaded into `Resonance::new()` via this module-local
/// statics — keeps the iced application builder closure capture-free.
pub static STARTUP_TAB: std::sync::OnceLock<ViewMode> = std::sync::OnceLock::new();

/// Parse `--tab arrange|mixer|compose|performance` (or `--tab=...`) from args.
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
            "performance" => Some(ViewMode::Performance),
            other => {
                eprintln!(
                    "Unknown --tab value '{other}'. Expected arrange|mixer|compose|performance."
                );
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

    /// Read-only view onto the persisted autosave settings. The autosave
    /// timer (epic #32) and the settings UI read these; surfaced so the
    /// view layer and tests can interrogate the live config.
    pub fn autosave_settings(&self) -> &settings::AutosaveSettings {
        &self.settings.autosave
    }

    /// Stable per-process session id, used to namespace the autosave
    /// scratch dir for a never-saved project.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    // ---- Save/autosave status surface ----
    // Read-only views onto the save lifecycle, surfaced so the window
    // chrome (todo #470) and the autosave integration tests can observe
    // routing without reaching into `pub(crate)` state.

    /// Whether the project has unsaved changes since the last *manual*
    /// save. Autosaves deliberately leave this set.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Whether a manual save or autosave is currently writing to disk.
    pub fn is_saving(&self) -> bool {
        self.io.saving
    }

    /// Wall-clock time of the last successful manual save, if any.
    pub fn last_saved_at(&self) -> Option<std::time::SystemTime> {
        self.io.last_saved_at
    }

    /// Wall-clock time of the last successful autosave snapshot, if any.
    pub fn last_autosave_at(&self) -> Option<std::time::SystemTime> {
        self.io.last_autosave_at
    }

    /// Number of entries currently in the recent-projects list.
    pub fn recent_project_count(&self) -> usize {
        self.io.recent_projects.len()
    }

    // Tempo / signature mutators live in `update/global_track.rs` —
    // they're the helpers that the `GlobalTrackMessage` handler uses to
    // keep the GUI-side `tempo_map` and the engine's tempo state in
    // sync. Plugin-index trio + `with_plugin_mut` live in
    // `state/plugin_index.rs`; `track_id_at_arrange_y` lives in
    // `state/arrange.rs`.

    // Public `#[doc(hidden)]` `test_*` accessors / mutators for the
    // integration tests under `tests/` live in `test_support.rs`.

    pub(crate) fn sorted_tracks(&self) -> &[TrackState] {
        self.registry.sorted_tracks()
    }

    pub(crate) fn sorted_busses(&self) -> &[BusState] {
        self.registry.sorted_busses()
    }

    /// Run `f` on the track with the given id, returning whatever `f`
    /// returns. `None` if the track doesn't exist.
    ///
    /// A miss here is reachable in normal operation, not an invariant
    /// violation: the ids ride inside queued `Message`s, and tracks are
    /// removed asynchronously when the engine's `TrackRemoved` event
    /// lands — so a message emitted just before removal (a slider drag,
    /// an async task completing) can drain afterwards carrying a dead
    /// id. Such stragglers must no-op; we log them so a *systematic*
    /// wrong-id bug still surfaces. (This used to be a `debug_assert!`,
    /// which turned that benign race into a dev-build panic.)
    pub(crate) fn with_track_mut<R>(
        &mut self,
        id: TrackId,
        f: impl FnOnce(&mut TrackState) -> R,
    ) -> Option<R> {
        let result = self.registry.with_track_mut(id, f);
        if result.is_none() {
            eprintln!("with_track_mut: no track with id {id:?} (stale message after removal?)");
        }
        result
    }

    /// Run `f` on the bus with the given id, returning whatever `f`
    /// returns. `None` if the bus doesn't exist. Misses are a benign
    /// race, same as [`Self::with_track_mut`].
    pub(crate) fn with_bus_mut<R>(
        &mut self,
        id: BusId,
        f: impl FnOnce(&mut BusState) -> R,
    ) -> Option<R> {
        let result = self.registry.with_bus_mut(id, f);
        if result.is_none() {
            eprintln!("with_bus_mut: no bus with id {id:?} (stale message after removal?)");
        }
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
        let settings = settings::load();
        // Per-process session id: pid + startup nanos. Cheap, dependency
        // free, and unique enough to namespace the autosave scratch dir.
        let session_id = {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!("{}-{}", std::process::id(), nanos)
        };

        let mut app = Self {
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
            transport_labels: view::transport_labels::TransportLabels::default(),
            error_message: None,
            master_volume: 0.0, // 0 dB = unity gain
            master_level_l: 0.0,
            master_level_r: 0.0,
            master_plugins: Vec::new(),
            master_fx_bypassed: false,
            view_mode: STARTUP_TAB.get().copied().unwrap_or(ViewMode::Arrange),
            pre_performance_view: None,
            clips: Vec::new(),
            midi_clips: Vec::new(),
            groove_library: Vec::new(),
            compose: compose::ComposeState::default(),
            pool: state::MediaPool::with_user_folders(
                settings.media.favourites.clone(),
                settings.media.recent_folders.clone(),
            ),
            reference: reference::ReferenceState::default(),
            table_registry: TableRegistry::with_builtins(),

            tempo_events: vec![state::TempoEvent { bar: 0, bpm: 120.0 }],
            signature_events: vec![state::SignatureEvent {
                bar: 0,
                numerator: 4,
                denominator: 4,
            }],
            tempo_map: TempoMap::default(),
            chord_track: chord_track::ChordTrack::new(),

            midi_map: MidiMapState::default(),

            transport: TransportState::default(),
            viewport: ArrangeViewport::default(),
            markers: state::ArrangementMarkers::default(),
            interaction: ClipInteractionState::default(),
            io: ProjectIoState {
                recent_projects,
                ..ProjectIoState::default()
            },
            mixer: MixerUiState::default(),
            registry: TrackRegistry {
                next_sub_track_id: 1_000_000_000,
                next_return_bus_id: 2_000_000_000,
                ..TrackRegistry::default()
            },
            aux: state::AuxSendState::default(),
            undo: UndoHistory::new(),
            plugin_state_cache: std::collections::HashMap::new(),
            plugin_index: std::collections::HashMap::new(),
            confirm_delete_track: None,
            bounce_dialog: None,
            import_dialog: None,
            bounce_in_progress: None,
            export_dialog: None,
            freeze: crate::state::FreezeState::default(),
            dirty: false,
            confirm_quit: None,
            quit_after_save: None,
            settings,
            session_id,
            default_presets: presets::default_presets(),
            user_presets: presets::load_user_presets(),
            pending_track_preset: None,
            pending_preset_save: None,
            pending_preset_plugin_states: None,
        };

        // Derive the transport label strings once so the very first
        // frame (rendered before the first `Tick`) shows real values.
        app.refresh_transport_labels();

        (app, iced::Task::none())
    }

    /// Re-derive the transport stat-block label strings from current
    /// state. Called after every `update()` dispatch, at construction,
    /// and at the end of the demo seed functions — never from `view()`,
    /// which only reads the cache. Cheap when nothing changed (five
    /// small key comparisons inside `TransportLabels::refresh`).
    ///
    /// The `mem::take` round-trip exists because `refresh` needs
    /// `&Resonance` while the labels live on `Resonance` itself; taking
    /// the (small, all-owned) struct out for the duration sidesteps the
    /// double borrow without a `RefCell`.
    pub(crate) fn refresh_transport_labels(&mut self) {
        let mut labels = std::mem::take(&mut self.transport_labels);
        labels.refresh(self);
        self.transport_labels = labels;
    }
}

