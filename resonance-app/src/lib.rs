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

use resonance_audio::types::*;
use resonance_audio::AudioEngine;
use resonance_music_theory::TableRegistry;

pub mod chord_box;
pub mod chord_sheet_pdf;
pub mod commands;
pub mod chord_track;
pub mod compose;
pub mod control_jobs;
pub mod control_socket;
pub mod demo;
pub mod engine_events;
pub mod focus;
pub mod message;
pub mod plugin_chain;
pub mod plugin_ui;
pub mod presets;
pub mod project;
pub mod recent;
pub mod reference;
pub mod settings;
pub mod state;
mod test_support;
#[doc(hidden)]
pub use test_support::TestChain;
pub use test_support::SendSlotAffordances;
pub mod theme;
pub mod undo;
pub mod update;
pub mod user_dirs;
pub mod util;
pub mod view;

pub use message::Message;
use state::*;
use undo::UndoHistory;

/// A track-preset save waiting for the engine to hand back its plugins'
/// state blobs (ba todo #1303).
///
/// See [`state::PresetState::pending_preset_save`] for why the save is
/// split in two.
#[derive(Debug, Clone)]
pub struct PendingPresetSave {
    /// The track being captured.
    pub track_id: resonance_audio::types::TrackId,
    /// What to call the preset. Already trimmed, and already checked
    /// against an existing preset of the same name.
    pub name: String,
}

/// Application state.
/// A plugin-preset capture waiting on the engine's state echo.
#[derive(Debug, Clone)]
pub(crate) struct PendingPluginPresetSave {
    pub(crate) instance_id: resonance_audio::types::PluginInstanceId,
    /// CLAP id, which is what names the preset directory.
    pub(crate) clap_id: String,
    /// Display name to write.
    pub(crate) name: String,
}

pub struct Resonance {
    pub engine: AudioEngine,
    pub sample_rate: u32,
    /// Hardware/external-device state (ARCH-06 second tier, A-12f): the
    /// audio input and MIDI device lists, the device-definition registry,
    /// per-track external-instrument config, and the MIDI-learn binding
    /// mirror. See `state::DeviceState`.
    pub(crate) devices: state::DeviceState,
    /// CLAP plugin scan result (ARCH-06 A6-2): what's available, what
    /// failed to load, and whether a rescan is in flight. See
    /// `state::PluginCatalog`.
    pub(crate) plugin_catalog: state::PluginCatalog,
    /// Whether the "this project uses plugins this machine hasn't got"
    /// warning is showing (ba doc #275 P5, todo #1309). Session state
    /// only — which slots are missing is derived from the chains by
    /// [`Resonance::missing_plugin_slots`].
    pub(crate) missing_plugins: crate::state::MissingPluginState,
    /// Transient view-layer/session UI state (ARCH-06 second tier, A-12e):
    /// current tab + the tab Performance mode was entered from, cached
    /// pick-list options, transport-bar labels, mixer-panel UI state, and
    /// timeline/clip interaction state, plus the last arrangement bar-shift
    /// report. Never persisted, never in the undo snapshot. See
    /// `state::UiTransientState`.
    pub(crate) ui: state::UiTransientState,
    /// The transient error/notification banner and its raise latches
    /// (ARCH-06 A6-2). See `state::Banners`.
    pub(crate) banners: state::Banners,
    /// The master bus: fader, meters, FX chain and FX-chain bypass
    /// (ARCH-06 A6-3). See `state::MasterState`.
    pub(crate) master: state::MasterState,
    /// Performance-mode footer selection: the active instrument/tuning and
    /// capo position that drive the live fingering diagrams. Mutated by the
    /// footer controls (`UiMessage::SetPerformanceTuning` /
    /// `SetPerformanceCapo`) and read by the diagram bands. See
    /// `state::PerformanceState`.
    pub(crate) performance: state::PerformanceState,
    /// Audio clips on the timeline.
    pub(crate) clips: Vec<ClipState>,
    /// MIDI clips on the timeline.
    pub(crate) midi_clips: Vec<MidiClipState>,
    /// Control-originated note edits mirrored into `midi_clips`
    /// optimistically and awaiting their engine echo (ba doc #265, Bug
    /// 2b: read-your-own-writes for `notes.*`). Keyed per clip; the
    /// matching `MidiNote*` echo drains one entry and no-ops. Empty for
    /// GUI edits.
    pub(crate) control_pending_note_echoes: state::PendingNoteEchoes,
    /// Compose tab state: section definitions, placements, chord progressions.
    pub(crate) compose: compose::ComposeState,
    /// Parameter-automation lanes, mirrored one-way from engine events,
    /// plus the transient live automated values. See `state::automation`.
    pub(crate) automation: state::AutomationState,

    /// MIDI quantize state: the project's user-extracted groove library
    /// and the last-used quantize / humanize settings (ba todo #395).
    /// Both halves persist in the project file and ride the undo snapshot.
    pub(crate) quantize: state::QuantizeState,

    /// Media-import/browse state (ARCH-06 second tier, A-12e): the media
    /// pool, in-flight import/transcode bookkeeping, the docked browser
    /// panel, drag-to-timeline placement, and the missing-file relink flow
    /// (doc #175). Only `media.pool` and `media.pool_import` persist /
    /// undo; the rest are transient session state. See `state::MediaState`.
    pub(crate) media: state::MediaState,

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

    // Sub-state groupings. See `state.rs` for definitions.
    pub(crate) transport: TransportState,
    pub(crate) viewport: ArrangeViewport,
    pub(crate) markers: state::ArrangementMarkers,
    /// Nesting depth of `update()`: handlers (control calls, the import
    /// dialog's Confirm) re-enter it. Only the outermost call decides
    /// whether a selection change grants the timeline the keyboard. A
    /// dispatch-loop re-entrancy guard, not view-presentation state, so it
    /// stays a top-level field rather than joining `state::UiTransientState`
    /// (A-12e survey).
    pub(crate) update_depth: u32,
    /// Settings of the MIDI editor's Quantize panel (todo #392). App-level
    /// so the chosen grid/strength/swing/mode persist across clip
    /// open/close; the Apply button reads this to build the bulk quantize.
    pub(crate) midi_quantize: state::MidiQuantizePanelState,
    pub(crate) io: ProjectIoState,
    pub(crate) registry: TrackRegistry,
    /// Track group (folder track) registry for group state management.
    pub(crate) track_groups: state::TrackGroupRegistry,
    /// GUI-side mirror of the engine's aux-send graph, reconstructed
    /// purely from `AuxSendChanged` / `AuxSendRemoved` / `AuxSendRejected`
    /// events. Bus return-role rides on `BusState::is_return`.
    pub(crate) aux: state::AuxSendState,
    /// GUI-side mirror of the engine's sidechain (key) routing table, one
    /// entry per keyed plugin instance (ba todo #1311). Seeded by the
    /// project-load replay and reconciled from `SidechainRouteChanged`.
    /// This is what makes a key route persistable: the save path
    /// serializes app state, and until this existed no app state held it.
    pub(crate) sidechain: state::SidechainState,
    /// GUI-side mirror of the engine's cycle-record take groups (epic #15,
    /// doc #165), reconstructed purely from `TakeCaptured` — one group per
    /// record run, one take per loop pass. This is what stops a captured
    /// pass from being dropped on the floor: the engine emits every take,
    /// and until this existed the dispatch discarded them.
    pub(crate) take_groups: state::TakeGroupState,
    /// App-side track-freeze orchestration: per-track freeze status plus
    /// the active "freeze selected / all" batch queue. Driven by the
    /// `FreezeMessage` handlers (ba todo #574) and the engine freeze-event
    /// mirror (ba todo #575). Cleared on project load.
    pub(crate) freeze: crate::state::FreezeState,
    /// Session-meta bookkeeping (ARCH-06 second tier, A-12g): the dirty
    /// flag, the monotonic edit counter, the per-process session id, and
    /// the undo/redo history — see `state::SessionMetaState` for why
    /// `control` (the survey's fifth candidate field) stays out. See
    /// `state::SessionMetaState`.
    pub(crate) session: state::SessionMetaState,
    /// Unix-socket control endpoint (doc #265, todo #1147): listener
    /// lifecycle handle plus per-connection handshake sessions. Transient
    /// — never persisted, never in the undo snapshot.
    pub(crate) control: crate::state::ControlEndpointState,
    /// Open-modal / confirmation-dialog flags and their transient input
    /// (ARCH-06 A6-2). See `state::ModalState`.
    pub(crate) modals: state::ModalState,
    /// The app's mirror of live plugin instances that isn't already
    /// owned per-track / per-bus (ARCH-06 A6-3): the cached CLAP state
    /// blob per instance, the instance-id-to-owner side-index, and the
    /// plugin-id allocator. See `state::PluginMirror`.
    pub(crate) plugin_mirror: state::PluginMirror,

    /// Persistent application settings (autosave config, …), loaded from
    /// `config_dir()/resonance/settings.json` on startup. Read via
    /// [`Resonance::autosave_settings`]; re-persisted with
    /// `settings::persist` whenever the user changes them.
    pub(crate) settings: settings::AppSettings,

    /// Track- and plugin-preset save/apply state (ARCH-06 A6-2). See
    /// `state::PresetState`.
    pub(crate) presets: state::PresetState,
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
                tracing::warn!(
                    "Unknown --tab value '{other}'. Expected arrange|mixer|compose|performance."
                );
                None
            }
        };
    }
    None
}

/// How much of the host machine a [`Resonance`] may read while assembling
/// itself (ba doc #285).
///
/// This is the *only* difference between [`Resonance::new`] and
/// [`Resonance::new_for_test`]; both build the same struct the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Host {
    /// The real app: read the user's recents, settings, saved track presets
    /// and user-authored device definitions from disk.
    Machine,
    /// Tests: touch nothing outside the process. Every one of those starts
    /// empty, so a test's result cannot depend on what is in `~/.config` —
    /// and cannot be changed by a test that writes there.
    None,
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
        &self.session.session_id
    }

    // ---- Save/autosave status surface ----
    // Read-only views onto the save lifecycle, surfaced so the window
    // chrome (todo #470) and the autosave integration tests can observe
    // routing without reaching into `pub(crate)` state.

    /// Whether the project has unsaved changes since the last *manual*
    /// save. Autosaves deliberately leave this set.
    pub fn is_dirty(&self) -> bool {
        self.session.dirty
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
            tracing::warn!("with_track_mut: no track {id:?} (stale message after removal?)");
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
            tracing::warn!("with_bus_mut: no bus with id {id:?} (stale message after removal?)");
        }
        result
    }

    /// Construct the application state used by the binary's `iced::application`
    /// builder. Spins up the real audio engine, requests initial device /
    /// plugin lists, and seeds an empty project. On engine init failure
    /// shows a native error dialog and exits — there is no headless path.
    ///
    /// Tests should call [`Resonance::new_for_test`] instead: this opens a real
    /// output stream, probes the machine's audio devices and loads whatever
    /// plugins are installed on it (ba doc #285).
    pub fn new() -> (Self, iced::Task<Message>) {
        // Engine startup options come from the user's settings, which the
        // app otherwise loads in `assemble` — after the engine exists.
        let options = resonance_audio::EngineOptions {
            render_threads: settings::load().audio.render_threads,
        };
        let engine = match AudioEngine::with_options(options) {
            Ok(engine) => engine,
            Err(e) => {
                // Surface the failure as a native dialog so the user
                // sees the cause instead of a stderr backtrace, then
                // exit cleanly. The app cannot run without an audio
                // engine; we don't currently support running
                // headless / engine-offline.
                tracing::error!("Audio engine init failed: {e}");
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

        let mut app = Self::assemble(engine, Host::Machine);
        // The session's default "Track 1" (FU-D4a): app-allocated and
        // sent synchronously, before iced's event loop can deliver any
        // other message, so nothing can ever race this for id 1. See
        // `send_startup_default_track`'s doc comment for why the engine
        // no longer creates this track itself.
        app.send_startup_default_track();
        // The engine's first clip-id grant (ARCH-04 D-7d): live MIDI on an
        // armed track and a record start need ids before any project load
        // (whose replay ends with a fresh grant of its own).
        app.send_clip_id_grant();
        // An untitled session that crashed leaves its autosave in the
        // scratch root: offer it over the startup screen (FU-M12a).
        crate::update::project_io::recovery::offer_orphaned_session(&mut app);
        (app, iced::Task::none())
    }

    /// Construct the application state with nothing of the host machine
    /// attached: a command-capturing engine (no output stream, no engine
    /// thread, no device enumeration, no `pw-metadata`/`pactl` subprocesses),
    /// no plugin scan, and no reads of the user's config — recents, settings
    /// and user presets all start empty.
    ///
    /// This is what integration tests want. Calling [`Resonance::new`] from a
    /// test makes the result depend on the machine it runs on: which audio
    /// device is free, which `.clap` files are installed, what is in
    /// `~/.config`. That is not hypothetical — a third-party plugin's broken
    /// teardown was aborting test processes at random until 60d13fb3, purely
    /// because tests loaded it (ba doc #285).
    ///
    /// Commands the app sends are accepted and thrown away. They have to be
    /// *accepted*: `AudioEngine::send` fails once its receiver is gone, and the
    /// app has real error paths behind that failure (`meter.measure` reports a
    /// dead engine rather than going pending, for one), so a hermetic engine
    /// that refused commands would quietly put tests on branches the live app
    /// never takes. A drain thread holds the receiving end and exits by itself
    /// when the engine is dropped.
    ///
    /// A test that wants to *assert* on the commands should use
    /// [`Resonance::new_for_test_with_capture`], or install a fresh channel
    /// mid-test with [`Resonance::test_capture_engine`].
    #[doc(hidden)]
    pub fn new_for_test() -> (Self, iced::Task<Message>) {
        let (app, task, cmd_rx) = Self::new_for_test_with_capture();
        std::thread::spawn(move || while cmd_rx.recv().is_ok() {});
        (app, task)
    }

    /// [`Resonance::new_for_test`] starting on `tab`.
    ///
    /// The startup tab is a per-app argument here rather than the process-wide
    /// [`STARTUP_TAB`] the binary's `--tab` flag writes. `STARTUP_TAB` is a
    /// `OnceLock`, so the first writer in a process wins and every later
    /// `set()` silently does nothing — which is invisible while each test file
    /// is its own process, and becomes a silent wrong-tab bug the moment two
    /// test files that want different tabs share one binary (ba doc #285 §4).
    #[doc(hidden)]
    pub fn new_for_test_on(tab: ViewMode) -> (Self, iced::Task<Message>) {
        let (mut app, task) = Self::new_for_test();
        app.ui.view_mode = tab;
        (app, task)
    }

    /// [`Resonance::new_for_test`], handing back the receiver its engine's
    /// commands queue onto so a test can assert on them from construction
    /// onwards — including asserting that construction emitted nothing.
    #[doc(hidden)]
    pub fn new_for_test_with_capture() -> (
        Self,
        iced::Task<Message>,
        crossbeam_channel::Receiver<AudioCommand>,
    ) {
        let (engine, cmd_rx) = AudioEngine::for_test_capture();
        (Self::assemble(engine, Host::None), iced::Task::none(), cmd_rx)
    }

    /// An app built around [`AudioEngine::for_test_disconnected`]: every
    /// `send` on it hits the disconnect branch immediately, as if the
    /// engine thread had already exited. For tests pinning what the app
    /// does once the engine is gone — e.g. the tick handler's "stopped
    /// responding" banner — without needing to actually kill a real
    /// engine thread mid-test.
    #[doc(hidden)]
    pub fn new_for_test_disconnected() -> (Self, iced::Task<Message>) {
        let engine = AudioEngine::for_test_disconnected();
        (Self::assemble(engine, Host::None), iced::Task::none())
    }

    /// Build the application state around an already-constructed `engine`.
    ///
    /// Split out of [`Resonance::new`] so [`Resonance::new_for_test`] can share
    /// every bit of assembly while reading none of the machine state. `host`
    /// decides only what gets read from outside the process; the resulting
    /// struct is put together identically either way.
    fn assemble(engine: AudioEngine, host: Host) -> Self {
        if matches!(host, Host::None) {
            // Not reading the user's state is half of it: a test app also
            // must not *write* it (recents on save/load, presets, settings)
            // nor let a later read see the real files (STATE-14 / FU-D5).
            user_dirs::enter_hermetic();
        }
        let (recent_projects, settings) = match host {
            Host::Machine => (recent::load(), settings::load()),
            Host::None => (Vec::new(), settings::AppSettings::default()),
        };
        // Per-process session id: pid + startup nanos. Cheap, dependency
        // free, and unique enough to namespace the autosave scratch dir.
        let session_id = {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!("{}-{}", std::process::id(), nanos)
        };

        // Device-definition registry (epic #40): bundled presets first, then
        // any user-authored definitions (last-wins by id). Built once here so
        // the External-Instrument inspector's device picker and the
        // `SetTrackDeviceParams` resolution read a stable list.
        // `scan_bundled` reads definitions embedded at compile time, so it is
        // hermetic and always runs; only the user-authored directory is a read
        // of machine state.
        let mut device_registry = resonance_common::DeviceDefinitionRegistry::default();
        device_registry.scan_bundled();
        if matches!(host, Host::Machine) {
            if let Some(dir) = resonance_common::user_definitions_dir() {
                device_registry.scan_dir(&dir);
            }
        }
        // Seed the cached device-preset pick-list options from the registry so
        // the inspector clones a refcounted slice instead of rebuilding the
        // option Vec every frame (view-performance rules).
        let mut view_caches = view::ui_caches::UiViewCaches::default();
        view_caches.rebuild_device_choices(&device_registry.list());

        let mut app = Self {
            engine,
            sample_rate: 44100, // overwritten by SampleRateDetected event
            devices: state::DeviceState {
                input: state::InputDevices::default(),
                midi: state::MidiDevices::default(),
                registry: device_registry,
                external_instruments: std::collections::HashMap::new(),
                midi_map: MidiMapState::default(),
            },
            plugin_catalog: state::PluginCatalog::default(),
            missing_plugins: crate::state::MissingPluginState::default(),
            ui: state::UiTransientState {
                view_caches,
                transport_labels: view::transport_labels::TransportLabels::default(),
                // `STARTUP_TAB` carries the binary's `--tab` flag, so it is a
                // read of *this process's* invocation and belongs to
                // `Host::Machine`. A hermetic app always starts on the
                // default tab and takes its startup tab as an argument
                // instead ([`Resonance::new_for_test_on`]) — otherwise one
                // test file's `set()` would decide the tab for every other
                // test sharing the binary.
                view_mode: match host {
                    Host::Machine => STARTUP_TAB.get().copied().unwrap_or(ViewMode::Arrange),
                    Host::None => ViewMode::Arrange,
                },
                pre_performance_view: None,
                last_arrangement_shift: None,
                interaction: ClipInteractionState::default(),
                mixer: MixerUiState::default(),
                keymap: crate::commands::BindingMap::resonance_default(),
                typing_probe: Default::default(),
            },
            banners: state::Banners::default(),
            master: state::MasterState {
                volume: 0.0, // 0 dB = unity gain
                level_l: 0.0,
                level_r: 0.0,
                plugins: Vec::new(),
                fx_bypassed: false,
            },
            performance: state::PerformanceState::default(),
            clips: Vec::new(),
            midi_clips: Vec::new(),
            control_pending_note_echoes: state::PendingNoteEchoes::default(),
            compose: compose::ComposeState::default(),
            automation: state::AutomationState::default(),
            quantize: state::QuantizeState::default(),
            media: state::MediaState {
                pool: state::MediaPool::with_user_folders(
                    settings.media.favourites.clone(),
                    settings.media.recent_folders.clone(),
                ),
                pool_import: state::PendingImports::default(),
                import_progress: state::ImportProgressTracker::default(),
                import_progress_modal_open: false,
                browser: state::BrowserState::default(),
                drag_placement: None,
                relink: state::RelinkState::default(),
                ids: state::ids::EntityIds::default(),
            },
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

            transport: TransportState::default(),
            viewport: ArrangeViewport::default(),
            markers: state::ArrangementMarkers::default(),
            update_depth: 0,
            midi_quantize: state::MidiQuantizePanelState::default(),
            io: ProjectIoState {
                recent_projects,
                ..ProjectIoState::default()
            },
            registry: TrackRegistry {
                // ARCH-04 D-4: no base to seed from any more — the app is
                // the only track-id allocator, so it starts at 1 like the
                // engine's own counter used to.
                next_track_id: 1,
                next_bus_id: state::ids::BUS_ID_BASE,
                ..TrackRegistry::default()
            },
            track_groups: state::TrackGroupRegistry::new(),
            aux: state::AuxSendState {
                next_send_id: 1,
                ..state::AuxSendState::default()
            },
            sidechain: state::SidechainState::default(),
            take_groups: state::TakeGroupState::default(),
            plugin_mirror: state::PluginMirror {
                next_id: 1,
                ..Default::default()
            },
            freeze: crate::state::FreezeState::default(),
            session: state::SessionMetaState {
                dirty: false,
                revision: 0,
                session_id,
                undo: UndoHistory::new(),
            },
            control: crate::state::ControlEndpointState::default(),
            modals: state::ModalState::default(),
            settings,
            presets: state::PresetState {
                default_presets: presets::default_presets(),
                user_presets: match host {
                    Host::Machine => presets::load_user_presets(),
                    Host::None => Vec::new(),
                },
                ..state::PresetState::default()
            },
        };

        // Derive the transport label strings once so the very first
        // frame (rendered before the first `Tick`) shows real values.
        app.refresh_transport_labels();

        // Build the tempo map's bar table up front. `TempoMap::default()`
        // has an empty table and `table_sample_rate: 0`, under which
        // `bar_to_sample` maps every bar to 0 — and anything gating on a
        // bar span (the Compose track canvas's `section_end <=
        // section_start` guard) silently draws nothing. Project load
        // replays rebuild it, but a File → New project never replays, so
        // without this the Compose synth lanes stay blank until the
        // project is reopened. Rebuilt again on `SampleRateDetected` once
        // the real device rate is known.
        app.rebuild_tempo_map();

        app
    }

    /// [`Resonance::new`] plus the unix-socket control endpoint (ba doc
    /// #265, todo #1147). This is what the binary uses; tests call
    /// `new()` directly so they never bind the per-user socket path.
    pub fn new_with_control() -> (Self, iced::Task<Message>) {
        let (mut app, task) = Self::new();
        app.start_control_server();
        (app, task)
    }

    /// Bind the control socket and start its listener thread, wiring the
    /// bridge channel that `subscription()` streams into `update()` as
    /// `Message::Control`. No-op when `RESONANCE_NO_CONTROL=1`; a bind
    /// failure (e.g. another live instance owns the socket) logs and
    /// leaves the endpoint off — the app itself keeps running.
    pub fn start_control_server(&mut self) {
        if control_socket::control_disabled() {
            return;
        }
        let (tx, rx) = iced::futures::channel::mpsc::unbounded();
        let jobs = std::sync::Arc::clone(&self.control.jobs);
        match control_socket::spawn(control_socket::socket_path(), tx, jobs) {
            Ok(server) => {
                control_socket::install_bridge(rx);
                self.control.server = Some(server);
            }
            Err(e) => tracing::warn!("control endpoint disabled: {e}"),
        }
    }

    /// Number of connected control clients. Feeds the window chrome's
    /// remote-control indicator (todo #1159) and the integration tests.
    pub fn control_client_count(&self) -> usize {
        self.control.sessions.len()
    }

    /// The monotonic edit counter carried by every mutating
    /// control-protocol reply (doc #265): bumped once per committed
    /// undoable transaction, including undo/redo restores.
    pub fn revision(&self) -> u64 {
        self.session.revision
    }

    /// Bump the monotonic edit counter (doc #265, todo #1147). The single
    /// choke point for every `revision` increment — routing all six
    /// call sites (`undo/mod.rs`, `undo/snapshot.rs`) through this method
    /// instead of touching the field directly caps the blast radius of a
    /// later `SessionMetaState` fold (A-12 survey) to this one function
    /// (ARCH-06, A-12g).
    pub(crate) fn bump_revision(&mut self) {
        self.session.revision = self.session.revision.wrapping_add(1);
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
        let mut labels = std::mem::take(&mut self.ui.transport_labels);
        labels.refresh(self);
        self.ui.transport_labels = labels;
    }
}

