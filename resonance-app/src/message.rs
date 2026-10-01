/// Message types for the Resonance application.
///
/// Messages are grouped into per-concern sub-enums that mirror the
/// sub-state layout of [`crate::Resonance`]. Each sub-enum is handled by a
/// dedicated arm of the top-level match in `update.rs`. A sub-enum may be
/// declared beside its handler (`update/<domain>.rs`) and re-exported
/// here, so `crate::message::<Domain>Message` always resolves; the rest
/// move over one domain at a time (ARCH-01 A1-3 / ARCH-06).
use crate::compose::ComposeMessage;
use crate::control_socket::ControlMessage;
use crate::reference::ReferenceMessage;
use crate::state::{MixerInspectorGroup, ViewMode};
use resonance_audio::types::{BusId, PluginInstanceId, ScannedPlugin, SendSource, TrackId};

pub use crate::update::arrangement::ArrangementMessage;
pub use crate::update::automation::AutomationMessage;
pub use crate::update::browser::BrowserMessage;
pub use crate::update::bus::BusMessage;
pub use crate::update::chord_track::ChordTrackMessage;
pub use crate::update::clips::ClipMessage;
pub use crate::update::drag::{DragMessage, DropTarget};
pub use crate::update::export::ExportMessage;
pub use crate::update::external_instrument::ExternalInstrumentMessage;
pub use crate::update::freeze::FreezeMessage;
pub use crate::update::global_track::GlobalTrackMessage;
pub use crate::update::group::GroupMessage;
pub use crate::update::import::ImportMessage;
pub use crate::update::marker::MarkerMessage;
pub use crate::update::marker_ui::MarkerUiMessage;
pub use crate::update::master::MasterMessage;
pub use crate::update::midi_clip::MidiClipMessage;
pub use crate::update::midi_editor::MidiEditorMessage;
pub use crate::update::mixer::MixerMessage;
pub use crate::update::pool::PoolMessage;
pub use crate::update::project_io::ProjectIoMessage;
pub use crate::update::relink::{RelinkError, RelinkMessage};
pub use crate::update::takes::TakeMessage;
pub use crate::update::track::{BounceMessage, TrackMessage};
pub use crate::update::transport::TransportMessage;
pub use crate::update::viewport::ViewportMessage;
pub use crate::update::vocal_tuning::VocalTuningMessage;

#[derive(Debug, Clone)]
pub enum Message {
    Compose(ComposeMessage),
    GlobalTrack(GlobalTrackMessage),
    ChordTrack(ChordTrackMessage),
    Transport(TransportMessage),
    Marker(MarkerMessage),
    /// Structural bar shifts (`arrangement.*`, ba doc #275 P2): insert or
    /// remove bars, moving every clip, placement, marker and automation
    /// point after the cut in ONE undoable edit.
    Arrangement(ArrangementMessage),
    /// Transient marker interaction state (selection, context menu, inline
    /// rename) driven by the timeline ruler hit-testing (todo #369). These
    /// mutate only view state and never the persisted marker set, so they
    /// carry no undo weight — the actual edits they commit go out as
    /// [`MarkerMessage`] variants.
    MarkerUi(MarkerUiMessage),
    Track(TrackMessage),
    ExternalInstrument(ExternalInstrumentMessage),
    Bus(BusMessage),
    Mixer(MixerMessage),
    Freeze(FreezeMessage),
    Master(MasterMessage),
    Clip(ClipMessage),
    MidiClip(MidiClipMessage),
    MidiEditor(MidiEditorMessage),
    VocalTuning(VocalTuningMessage),
    Plugin(PluginMessage),
    Automation(AutomationMessage),
    Take(TakeMessage),
    Viewport(ViewportMessage),
    ProjectIo(ProjectIoMessage),
    Group(GroupMessage),
    Reference(ReferenceMessage),
    Export(ExportMessage),
    Import(ImportMessage),
    /// Audio media-pool import + placement orchestration (doc #175, ba
    /// todo #598): bring external audio into the project pool and,
    /// optionally, place it as a clip. See [`PoolMessage`].
    Pool(PoolMessage),
    /// Missing-file relink actions (doc #175, todo #600): locate a single
    /// missing pool asset, or batch-search a folder to resolve every
    /// missing asset by filename, then re-copy the audio into the project.
    /// Handled by `update::relink::handle`.
    Relink(RelinkMessage),
    Ui(UiMessage),
    /// Docked media-browser interaction: filesystem navigation, per-folder
    /// filter, favourite / recent toggles, Files/Pool tab, and the audition
    /// preview transport. All transient — none of it is undoable or
    /// persisted in the project (same rule as collapse state, doc #175).
    /// Handled by `update::browser::handle`.
    Browser(BrowserMessage),
    /// Drag-to-timeline placement gesture (doc #175, todo #605): dragging a
    /// media-browser row over the arrangement to preview and drop a clip.
    /// Transient preview — none of it is undoable or persisted; the drop
    /// fans out into a [`PoolMessage::ImportAndPlace`]. Handled by
    /// `update::drag::handle`.
    Drag(DragMessage),
    /// Control-endpoint traffic from the unix-socket threads (ba doc
    /// #265, todo #1147): client connect/disconnect events and parsed
    /// JSON-RPC requests, delivered in arrival order through the bridge
    /// subscription. Handled by `update::control::handle`, which replies
    /// over the request's reply channel; mutating methods are executed by
    /// synthesizing ordinary domain messages back through `update()`, so
    /// this envelope itself carries no undo weight.
    Control(ControlMessage),
    /// Timer tick driving VU meters and auto-follow. Kept at top level to
    /// avoid wrapping cost on the hot path.
    Tick,
    /// Walk one step back through the session-local undo history.
    Undo,
    /// Walk one step forward through the session-local redo history.
    Redo,
    /// The window manager requested that the window be closed.
    WindowCloseRequested(iced::window::Id),
}

/// The host's preset surfaces (plugin-preset-library.md §6.6, slice P6):
/// the plugin panel's bar, the browser it opens, and the media browser's
/// Presets tab. None records undo by itself; a load that sticks goes
/// through [`PluginMessage::LoadPluginPreset`], which does.
#[derive(Debug, Clone)]
pub enum PresetUiMessage {
    /// Load the next (`delta > 0`) or previous preset in bank order.
    Step {
        instance_id: PluginInstanceId,
        delta: i32,
    },
    /// Star or unstar the slot's loaded preset.
    ToggleFavorite(PluginInstanceId),
    OpenBrowser(PluginInstanceId),
    /// Close the browser, keeping the auditioned preset (one undo entry)
    /// or reverting to the sound it opened on.
    CloseBrowser {
        keep: bool,
    },
    BrowserSearch(String),
    /// ↑ / ↓ in the browser: audition the previous / next row.
    BrowserMove(i32),
    BrowserFavoritesOnly(bool),
    /// Audition the row: load it unrecorded, remembering the origin.
    BrowserAudition(usize),
    BrowserToggleRowFavorite(usize),
    MediaSearch(String),
    MediaFavoritesOnly(bool),
    MediaPlugin(crate::state::presets::PresetPluginChoice),
    MediaSelect(usize),
    /// Load the row onto the selected plugin slot (same plugin only).
    MediaLoad(usize),
    MediaToggleRowFavorite(usize),
    /// A press on a Presets-tab row: selects it and arms a drag.
    MediaPress(usize),
    /// The pointer moved (window coordinates) while a drag is armed.
    DragMoved(iced::Point),
    /// Released over a track header while a drag is armed: add the plugin
    /// there with the preset, if the pointer really moved.
    DropOnTrack(TrackId),
    /// Released anywhere while a drag is armed: disarm (after any drop).
    DragEnd,
    /// "with preset…" in an add picker: add the plugin, then load the
    /// preset onto it once it exists.
    AddWithPreset {
        owner: PresetAddOwner,
        pick: crate::state::presets::PresetAddPick,
    },
}

/// Which chain a "with preset…" add appends to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetAddOwner {
    Track(TrackId),
    Bus(resonance_audio::types::BusId),
    Master,
}

#[derive(Debug, Clone)]
pub enum PluginMessage {
    /// The host's preset surfaces (slice P6).
    PresetUi(PresetUiMessage),
    /// Bypass (or re-engage) ONE slot, wherever it sits — track, bus or
    /// master (ba doc #275 finding X3, todo #1305).
    ///
    /// Addressed by instance id alone, because that is already unique
    /// across all three chains; the surface it lives on is not part of
    /// the request and so cannot be got wrong. SETS rather than toggles,
    /// so a retried control-API call cannot flip a slot back on, and so
    /// project load can restore a saved state through the same path.
    SetPluginBypass {
        instance_id: PluginInstanceId,
        bypassed: bool,
    },
    AddPluginToTrack(TrackId, ScannedPlugin),
    /// Add a plugin and mirror a placeholder slot into `TrackState.plugins`
    /// immediately, so the caller can address the plugin without waiting
    /// for the engine's `PluginAdded` echo (ba doc #273, todo #1234).
    /// `engine_events::plugins::track_added` is idempotent, so the echo
    /// fills the placeholder's params in rather than pushing a duplicate.
    ///
    /// Since ARCH-04 D-1 the instance id is app-allocated
    /// (`Resonance::allocate_plugin_id`) for
    /// [`AddPluginToTrack`](Self::AddPluginToTrack) too — the engine has
    /// no allocator of its own left for plugins. What distinguishes this
    /// variant from that one is only the eager mirror: the GUI never
    /// sends `AddPluginToTrackWithId`, because it has no reply to answer
    /// synchronously and is content to wait for the echo like before.
    AddPluginToTrackWithId {
        track_id: TrackId,
        instance_id: PluginInstanceId,
        plugin: ScannedPlugin,
    },
    RemovePluginFromTrack(TrackId, PluginInstanceId),
    /// Reorder a track's insert chain: move `instance_id` to
    /// `to_index`, shifting everything between its old and new slot by
    /// one (ba doc #273, todo #1225). `to_index` is clamped to the last
    /// slot, so a value past the end means "the end".
    ///
    /// Sends `AudioCommand::MovePlugin` (todo #1224) *and* mirrors the
    /// new order into `TrackState.plugins` immediately, so a control
    /// client can read its own write back in the same cycle. The
    /// engine's `PluginMoved` echo replays the same move, which is a
    /// no-op once the order already matches.
    MovePluginInTrack {
        track_id: TrackId,
        instance_id: PluginInstanceId,
        to_index: usize,
    },
    /// Put `plugin` in the slot currently holding `instance_id`,
    /// **keeping the slot's position in its chain** (ba doc #275 P5,
    /// todo #1309).
    ///
    /// One message for all three chains, because plugin instance ids are
    /// unique across tracks, busses and master. What it does depends on
    /// what the slot already holds — a relocate of the same plugin keeps
    /// the instance and its preserved settings, a different plugin gets
    /// a fresh instance and the old one's settings are discarded with
    /// it. See [`crate::update::plugin_replace`].
    ///
    /// This exists because the alternative — remove then add — puts the
    /// replacement at the END of the chain, and chain order is audible.
    ReplacePlugin {
        instance_id: PluginInstanceId,
        plugin: ScannedPlugin,
    },
    SetPluginParam(PluginInstanceId, u32, f64),
    /// Recall a preset onto a plugin: every parameter it names, applied
    /// as **one** edit (ba todo #1333).
    ///
    /// Deliberately not `AudioCommand::LoadPluginState`, even though the
    /// plugin could parse the preset itself. The app's parameter mirror
    /// is filled once at instantiation and only updated for changes that
    /// went through `SetPluginParam` — a plugin-side load moves the sound
    /// and leaves `track.plugin_params` reporting the values it had
    /// before (the gap ba todo #1294 closes). Driving the same path
    /// every other parameter write takes keeps the mirror, the engine and
    /// the plugin's own window telling the same story.
    ///
    /// One message rather than a burst of `SetPluginParam` so a recall is
    /// one entry in the undo history: it is one gesture to the user, and
    /// stepping back through forty parameter changes to get before a
    /// preset load is not undo.
    LoadPluginPreset {
        instance_id: PluginInstanceId,
        /// `(param_id, value)` for every parameter the preset names, in
        /// the plugin's own declared order.
        values: Vec<(u32, f64)>,
        /// What to show as the loaded preset afterwards, for the reply
        /// and any future GUI readout.
        preset_name: String,
        /// The preset's state document with its identity, handed to the
        /// plugin after the params (`AudioCommand::LoadPluginPresetState`)
        /// so the rest of the sound — a model, an IR, user wavetables —
        /// comes along (plugin-preset-library.md §6.7). `None` recalls the
        /// params only.
        preset_state: Option<Vec<u8>>,
        /// The preset's id and source: the slot's loaded-preset identity
        /// until the plugin reports its own (slice P5).
        preset_id: String,
        preset_source: resonance_control::methods::plugin_preset::PluginPresetSource,
    },
    /// Recall a preset the plugin owns (one its preset-discovery factory
    /// listed): the plugin loads it through `clap.preset-load`, and the
    /// engine refreshes the mirror afterwards. One undo entry, like
    /// `LoadPluginPreset` (slice P8).
    LoadPluginPresetFromLocation {
        instance_id: PluginInstanceId,
        location: resonance_audio::types::PluginPresetLocation,
        load_key: Option<String>,
        preset_name: String,
        preset_id: String,
    },
    /// A ◀ / ▶ preset step: the `LoadPluginPreset` (or location load) in
    /// `load`, coalesced with the steps before it on the same plugin into
    /// one undo entry, and with the rest of the sound's state load
    /// debounced (an amp step reloads a model).
    PresetStep {
        instance_id: PluginInstanceId,
        load: Box<PluginMessage>,
    },
    /// Open the plugin's editor window (CLAP_EXT_GUI).
    OpenPluginEditor(PluginInstanceId),
    /// Close the plugin's editor window.
    ClosePluginEditor(PluginInstanceId),
    /// Route another track's or bus's audio into this plugin's external
    /// sidechain (key) input, or clear the route when `source` is `None`
    /// (control endpoint `track.set_sidechain` / `track.clear_sidechain`).
    ///
    /// Only the DETECTOR of the target plugin changes; the key never
    /// reaches the output. A plugin that declares no key port stores the
    /// route inertly rather than erroring, because the plugin at an
    /// instance id can be swapped underneath it.
    SetPluginSidechain {
        instance_id: PluginInstanceId,
        source: Option<SendSource>,
        enabled: bool,
    },
    /// Look for plugins installed since the app started (ba todo #1307,
    /// finding X10; control method `plugins.rescan`).
    ///
    /// The scan itself is the engine's, and it is additive: bundles
    /// already loaded stay loaded, so a rescan mid-session cannot
    /// disturb a plugin that is processing audio or holding an open
    /// editor. The refreshed catalog arrives as `PluginsScanned` and
    /// re-populates the add-plugin menus; bundles that refused to load
    /// arrive as `PluginScanFailed` and are shown rather than swallowed.
    ///
    /// Not undoable — the plugin catalog is a fact about the machine,
    /// not part of the project.
    RescanPlugins,
    /// "Open" for a plugin slot: always produces a window
    /// (mixer-cleanup.md §4). A plugin with its own GUI takes the
    /// `OpenPluginEditor` path; one without — or one that is missing on
    /// this machine — opens the host-drawn generic window (parameters,
    /// preset bar, or the missing-plugin recovery). Opening another
    /// plugin's generic window replaces the open one.
    OpenPluginWindow(PluginInstanceId),
    /// Close the generic window if it shows this plugin.
    ClosePluginWindow(PluginInstanceId),
    /// Title-bar drag of the generic window.
    PluginWindowDrag(PluginWindowDrag),
    /// Open the host-drawn generic parameter window for a slot even when
    /// the plugin has its own GUI — the way to its parameter list and
    /// preset bar for a plugin whose editor hides them. Focuses the slot.
    OpenGenericParams(PluginInstanceId),
    /// Make a slot the focused one (`MixerUiState::focused_slot`,
    /// mixer-cleanup.md §2.1) and select the channel it sits on, without
    /// opening anything. The preset commands then act on it.
    FocusSlot(PluginInstanceId),
}

/// A step of the generic plugin window's title-bar drag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PluginWindowDrag {
    /// The title bar was pressed.
    Begin,
    /// The pointer moved, in window coordinates.
    Moved(iced::Point),
    /// The button was released (or the pointer left the window).
    End,
}

impl PluginMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::{CoalesceKey, UndoAction};
        match self {
            Self::AddPluginToTrack(_, _)
            | Self::AddPluginToTrackWithId { .. }
            | Self::RemovePluginFromTrack(_, _)
            | Self::MovePluginInTrack { .. } => UndoAction::Record,
            // Replacing a slot's plugin is a chain edit like any other,
            // and one the user must be able to take back: a swap
            // DISCARDS the outgoing plugin's preserved state (ba todo
            // #1308), and the snapshot taken before this message is the
            // only place that state still exists afterwards. Never
            // coalesced — two replaces in a row are two decisions.
            Self::ReplacePlugin { .. } => UndoAction::Record,
            // Routing a key is a project edit like any other insert
            // change, so it takes an undo entry of its own.
            Self::SetPluginSidechain { .. } => UndoAction::Record,
            // Bypassing a slot is a project edit and it persists, so it
            // takes an entry. Deliberately NOT coalesced: a knob drag
            // emits one message per frame and wants collapsing, but two
            // bypass toggles are two decisions a user expects to undo
            // separately (ba todo #1305).
            Self::SetPluginBypass { .. } => UndoAction::Record,
            // A preset recall is one gesture, so it takes one entry —
            // and it must NOT coalesce with anything: coalescing a recall
            // into a neighbouring knob edit would make the two undo
            // together (ba todo #1333).
            Self::LoadPluginPreset { .. } => UndoAction::Record,
            Self::LoadPluginPresetFromLocation { .. } => UndoAction::Record,
            Self::PresetStep { instance_id, .. } => {
                UndoAction::RecordCoalesced(CoalesceKey::PluginPresetStep(*instance_id))
            }
            Self::SetPluginParam(instance_id, param_id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::PluginParam {
                    instance_id: *instance_id,
                    param_id: *param_id,
                })
            }
            // Browsing, auditioning and starring record nothing: a load
            // that sticks is re-dispatched as `LoadPluginPreset`, and a
            // "with preset…" add as the add it is.
            Self::PresetUi(_) => UndoAction::Skip,
            Self::OpenPluginEditor(_)
            // Opening, closing and moving a window are view state.
            | Self::OpenPluginWindow(_)
            | Self::ClosePluginWindow(_)
            | Self::PluginWindowDrag(_)
            | Self::OpenGenericParams(_)
            | Self::FocusSlot(_)
            // A rescan changes what the machine offers, not what the
            // project contains — there is nothing to undo (todo #1307).
            | Self::RescanPlugins
            | Self::ClosePluginEditor(_) => UndoAction::Skip,
        }
    }
}

/// A button on the autosave-recovery prompt. `OpenLastSaved` is offered
/// for a saved project, `Discard` for a crashed untitled session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryChoice {
    RecoverAutosave,
    OpenLastSaved,
    Discard,
    Cancel,
}

#[derive(Debug, Clone)]
pub enum UiMessage {
    SwitchView(ViewMode),
    /// Toggle full-screen Performance mode on/off (the `F` keyboard
    /// shortcut). Entering remembers the previously active view so `F`
    /// or `Esc` returns to it; never auto-opens on record-arm and never
    /// disturbs transport state.
    TogglePerformanceMode,
    /// Leave Performance mode (the `Esc` keyboard shortcut), restoring the
    /// view that was active when Performance mode was entered. A no-op when
    /// not in Performance mode.
    ExitPerformanceMode,
    OpenSettings,
    CloseSettings,
    OpenAddTrackMenu,
    CloseAddTrackMenu,
    /// Show / hide the Reference & A/B right-rail in the Mix view.
    ToggleReferencePanel,
    DismissError,
    /// User clicked "New Project" in the startup modal.
    StartNewProject,
    /// Replace the open project with a fresh, untitled empty one (the New
    /// Project command) — the control API's `project.new` path. Refused
    /// while the project has unsaved changes, a load or save is running,
    /// or an offline render owns the engine.
    NewEmptyProject,
    /// Select (highlight) a track in the arrange view, or deselect all.
    /// Whether the click replaces or extends the multi-selection is read
    /// from the live modifier state ([`ModifiersChanged`]).
    SelectTrack(Option<TrackId>),
    /// Select (highlight) a BUS strip in the mixer, or deselect. Busses
    /// need their own message because bus ids and track ids are separate
    /// id spaces that overlap numerically — `SelectTrack(Some(1))` and
    /// "bus 1" are different things, and one field cannot hold both.
    /// Selecting a bus clears the track selection and vice versa: the
    /// inspector shows exactly one channel.
    SelectBus(Option<BusId>),
    /// Live keyboard modifier state changed. Tracked so a track-header
    /// click can tell a plain select from an additive (Cmd/Shift) one
    /// without the mouse event carrying modifiers (todo #684).
    ModifiersChanged(iced::keyboard::Modifiers),
    /// User confirmed "Save & Quit" in the unsaved-changes dialog.
    ConfirmSaveAndQuit,
    /// User confirmed "Discard & Quit" in the unsaved-changes dialog.
    ConfirmDiscardAndQuit,
    /// User cancelled the unsaved-changes quit dialog.
    CancelQuit,
    /// Toggle the global tracks area (tempo, time signature) in the arrange view.
    ToggleGlobalTracks,
    /// Fold / unfold one of the mixer-inspector groups (CHAIN / SENDS /
    /// ROUTING / AUTOMATION / TRACK). Runtime UI state only.
    ToggleMixerInspectorGroup(MixerInspectorGroup),
    /// Fold / unfold a track's take lane — the stack of cycle-recorded
    /// takes shown beneath it (epic #15, doc #165). Runtime UI state only:
    /// the takes persist, whether their folder is open does not. The comp
    /// ribbon on the track lane stays visible either way.
    ToggleTakeLane(TrackId),
    /// Switch arrange-view playhead follow on/off (persisted in settings,
    /// code review FU-V3b).
    ToggleFollowPlayhead,
    /// Switch periodic autosave on/off (persisted in settings, code review
    /// FU-M12a / ba todo #471).
    ToggleAutosave,
    /// Set the autosave interval in seconds (persisted in settings).
    SetAutosaveInterval(u32),
    /// Toggle MIDI clock send (engine acts as clock master).
    ToggleMidiClockSend,
    /// Pick the hardware port for MIDI clock send. `None` clears.
    SetMidiClockSendDevice(Option<String>),
    /// Toggle MIDI clock receive (engine slaves to an external master).
    ToggleMidiClockRecv,
    /// Pick the hardware port for MIDI clock receive. `None` clears.
    SetMidiClockRecvDevice(Option<String>),
    /// Select the Performance-mode instrument/tuning by its index into
    /// `resonance_music_theory::ALL_TUNINGS` (the footer's segmented pill
    /// selector). Out-of-range indices are ignored. The live fingering
    /// diagrams re-voice against the new tuning.
    SetPerformanceTuning(usize),
    /// Set the Performance-mode capo position in frets (the footer's `Capo`
    /// stepper). Clamped to `0..=state::performance::MAX_CAPO`; the diagrams
    /// re-voice with the capo applied.
    SetPerformanceCapo(u8),
    /// Show / hide the arrangement-markers overview popover anchored under
    /// the transport bar (the transport "flag" button). Runtime UI state
    /// only (todo #370).
    ToggleMarkersOverview,
    /// Close the markers overview popover — backdrop click, or after an
    /// overview entry jumps the playhead (todo #370).
    CloseMarkersOverview,
    /// A key press outside the command registry that is also an ordinary
    /// typing key (the held `B` reference audition). It does not act
    /// directly: this probes widget focus (see [`crate::focus`]) and
    /// resolves to [`ShortcutResolved`] (UPD-11). Registry shortcuts take
    /// the same gate through [`ShortcutProbed`].
    RequestShortcut(Box<Message>),
    /// Result of the focus probe started by [`RequestShortcut`]: the
    /// wrapped message is dispatched only when no text field held focus
    /// (`editing == false`).
    ShortcutResolved {
        message: Box<Message>,
        editing: bool,
    },
    /// Close the audio-import transcode-progress modal (doc #175, todo
    /// #606) once all files have reached a terminal state. Presentational
    /// only — the import already ran; this just hides the overlay and
    /// clears the transient progress tracker. Never undoable.
    DismissImportProgress,
    /// Close the missing-plugin load warning (ba doc #275 P5, todo
    /// #1309) and keep it closed for this project.
    ///
    /// It stays closed even as further failures arrive, because they do
    /// arrive one at a time: without that, dismissing the warning during
    /// a load that is still reporting would be undone by the next
    /// failure and the modal would look un-closable. Presentational
    /// only — the dead slots and their preserved settings are untouched,
    /// so this is never undoable.
    DismissMissingPlugins,
    /// Re-open the missing-plugin load warning after it was dismissed
    /// (Settings -> Plugins). Overrides the dismissal, because the user
    /// asked for it this time.
    ShowMissingPlugins,
    /// Open the right-click track context menu (design doc #181, ba todo
    /// #581), anchored at `x` / `y` in arrange-area space. Also selects the
    /// track so the "Freeze selected tracks" entry targets what was clicked.
    OpenTrackMenu {
        id: TrackId,
        x: f32,
        y: f32,
    },
    /// Close the track context menu (backdrop click, or after an entry
    /// dispatched its action).
    CloseTrackMenu,
    /// A global key press, forwarded by the keyboard subscription
    /// (command-palette.md §4.1). `captured` is set when a widget (a
    /// focused text field, a key-owning canvas) already consumed it. The
    /// reducer applies the overlay gate, looks the chord up in the active
    /// keymap, drops key repeat the command doesn't want, and runs it.
    ShortcutKey {
        chord: crate::commands::KeyChord,
        repeat: bool,
        captured: bool,
    },
    /// The typing gate's answer for a `NotWhileTyping` shortcut: the
    /// command runs only when no text field held focus (`editing ==
    /// false`).
    ShortcutProbed {
        command: crate::commands::CommandId,
        editing: bool,
    },
    /// Open the command palette in a mode (⌘K / ⇧⌘P); closes it when it
    /// is already open.
    OpenPalette(crate::palette::PaletteMode),
    /// Close the command palette, keeping its query for next time.
    ClosePalette,
    /// Command-palette interaction.
    Palette(crate::palette::PaletteMsg),
    /// Preferences › Keyboard: the settings tab, preset and rebinding.
    Keymap(crate::update::keymap::KeymapMsg),
    /// Select the MASTER strip, so the inspector describes the master
    /// (mixer-cleanup.md §3.3). Clears the track and bus selections: the
    /// inspector shows exactly one channel.
    SelectMaster,
    /// The app window was opened or resized (`window::Event::Opened` /
    /// `Resized`). Kept as `UiTransientState::window_size`, which the
    /// floating generic plugin window clamps its position to.
    WindowResized(iced::Size),
}

impl UiMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            // Tabs, dialogs, menus, selection, performance mode and app
            // settings: pure UI or user-settings state, never a project edit.
            Self::SwitchView(..)
            | Self::TogglePerformanceMode
            | Self::ExitPerformanceMode
            | Self::OpenSettings
            | Self::CloseSettings
            | Self::OpenAddTrackMenu
            | Self::CloseAddTrackMenu
            | Self::ToggleReferencePanel
            | Self::DismissError
            | Self::StartNewProject
            | Self::NewEmptyProject
            | Self::SelectTrack(..)
            | Self::SelectBus(..)
            | Self::WindowResized(..)
            | Self::ModifiersChanged(..)
            | Self::ConfirmSaveAndQuit
            | Self::ConfirmDiscardAndQuit
            | Self::CancelQuit
            | Self::ToggleGlobalTracks
            | Self::ToggleMixerInspectorGroup(..)
            | Self::ToggleTakeLane(..)
            | Self::ToggleFollowPlayhead
            | Self::ToggleAutosave
            | Self::SetAutosaveInterval(..)
            | Self::ToggleMidiClockSend
            | Self::SetMidiClockSendDevice(..)
            | Self::ToggleMidiClockRecv
            | Self::SetMidiClockRecvDevice(..)
            | Self::SetPerformanceTuning(..)
            | Self::SetPerformanceCapo(..)
            | Self::ToggleMarkersOverview
            | Self::CloseMarkersOverview
            | Self::RequestShortcut(..)
            | Self::ShortcutResolved { .. }
            | Self::DismissImportProgress
            | Self::DismissMissingPlugins
            | Self::ShowMissingPlugins
            | Self::OpenTrackMenu { .. }
            | Self::CloseTrackMenu
            // Keyboard envelopes: the message they resolve to re-enters
            // `update()` and is classified on its own, so one shortcut is
            // one undo entry (or none).
            | Self::ShortcutKey { .. }
            | Self::ShortcutProbed { .. }
            // The palette itself is pure UI; the command a row runs
            // re-enters `update()` and is classified on its own.
            | Self::OpenPalette(..)
            | Self::ClosePalette
            | Self::Palette(..)
            // Keymap edits are user settings, not project state.
            | Self::Keymap(..)
            | Self::SelectMaster => UndoAction::Skip,
        }
    }
}
