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
use crate::presets::TrackPreset;
use crate::project::LoadedProject;
use crate::reference::ReferenceMessage;
use crate::state::{
    ClipEdge, MixerInspectorGroup, ParsedImport, PlacementMode, PlacementStart, TempoAlignment,
    TempoChoice, ViewMode,
};
use resonance_audio::types::{
    AssetId, BusId, ClipId, FadeCurve, PluginInstanceId, SamplePos, ScannedPlugin, SendId,
    SendSource,
    TrackId, TrackOutput,
};
use resonance_audio::PoolImportOutcome;
use resonance_common::{TakeGroupId, TakeId, TimelineRange};

pub use crate::update::automation::AutomationMessage;
pub use crate::update::browser::BrowserMessage;
pub use crate::update::bus::BusMessage;
pub use crate::update::chord_track::ChordTrackMessage;
pub use crate::update::drag::{DragMessage, DropTarget};
pub use crate::update::export::ExportMessage;
pub use crate::update::external_instrument::ExternalInstrumentMessage;
pub use crate::update::freeze::FreezeMessage;
pub use crate::update::global_track::GlobalTrackMessage;
pub use crate::update::group::GroupMessage;
pub use crate::update::marker::MarkerMessage;
pub use crate::update::marker_ui::MarkerUiMessage;
pub use crate::update::master::MasterMessage;
pub use crate::update::midi_clip::MidiClipMessage;
pub use crate::update::midi_editor::MidiEditorMessage;
pub use crate::update::transport::TransportMessage;
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

/// Arrangement-marker actions, routed like [`TransportMessage`] and
/// handled by `update/marker.rs`. The mutating variants
/// (`AddAtPlayhead`, `Rename`, `Recolor`, `Delete`, `MoveStart`,
/// `SetRegionEnd`, `LoopToRegion`, `SeedFromSections`) record an undo
/// entry; the navigation variants (`JumpToNext`, `JumpToPrev`, `JumpTo`,
/// `PlayFromMarker`) only move the playhead / transport and are not
/// undoable, mirroring `SeekToSample` / `Play`.
/// Structural bar shifts. Both variants move everything after the cut
/// and are recorded as a single undo entry — the whole point of having
/// them at all is that a restructure is one transaction rather than a
/// few hundred per-object calls that can be interrupted half-done.
#[derive(Debug, Clone, Copy)]
pub enum ArrangementMessage {
    /// Open `count` bars at 1-based `at_bar`.
    InsertBars { at_bar: u32, count: u32 },
    /// Close `count` bars at 1-based `at_bar`, deleting what starts
    /// inside them.
    RemoveBars { at_bar: u32, count: u32 },
}

#[derive(Debug, Clone)]
pub enum TrackMessage {
    AddTrack,
    AddInstrumentTrack,
    /// Create an instrument track that starts already in external-instrument
    /// mode (doc #251 gap 1 affordance 2). Allocates the track id app-side so
    /// the external state lands atomically with the track in one undo step;
    /// the menu entry that dispatches it is a separate view todo.
    AddExternalInstrumentTrack,
    AddVocalTrack,
    /// Add a track with a caller-allocated id (control endpoint, doc
    /// #265, todo #1152). Unlike the GUI adds, the id is allocated
    /// app-side and passed to the engine as `id_hint` so the control
    /// reply can return the real `track_id` immediately; `drums` comes
    /// up as an instrument track whose instrument type is set to Drum
    /// when the engine echo mirrors it. Undoable as one step, like the
    /// other adds.
    AddControlTrack {
        id: TrackId,
        kind: crate::state::ControlTrackKind,
        name: Option<String>,
    },
    /// User clicked delete on a track — may require confirmation if it
    /// has content.
    RequestRemoveTrack(TrackId),
    /// User confirmed removal in the "track has content" dialog.
    ConfirmRemoveTrack,
    /// User cancelled the "track has content" dialog.
    CancelRemoveTrack,
    SetTrackVolume(TrackId, f32),
    SetTrackPan(TrackId, f32),
    SetMasterVolume(f32),
    ToggleMute(TrackId),
    ToggleSolo(TrackId),
    ToggleRecordArm(TrackId),
    ToggleMonitor(TrackId),
    ToggleTrackMono(TrackId),
    ToggleTrackFxBypass(TrackId),
    /// Rename a track (edited from the Compose instrument details panel).
    SetTrackName(TrackId, String),
    SetTrackInputDevice(TrackId, Option<String>),
    SetTrackInputPort(TrackId, u16),
    /// Pick the hardware MIDI input device for an instrument track.
    SetTrackMidiInputDevice(TrackId, Option<String>),
    /// Pick the hardware MIDI output device for an instrument track.
    SetTrackMidiOutputDevice(TrackId, Option<String>),
    /// Pick the input channel filter (`None` = omni / accept all).
    SetTrackMidiInputChannel(TrackId, Option<u8>),
    /// Pick the output channel (`None` = default to channel 1).
    SetTrackMidiOutputChannel(TrackId, Option<u8>),
    /// Toggle whether a parent track's sub-tracks are shown in the mixer.
    ToggleSubTracksVisible(TrackId),
    SetTrackOutput(TrackId, TrackOutput),
    /// Create a new track from a preset template.
    ///
    /// `id_hint` is the app-allocated track id, so a caller can address
    /// the new track without waiting for the engine's `*TrackAdded`
    /// echo; `None` lets the engine allocate, which is the GUI's path
    /// (the same split as `PluginMessage::AddPluginToTrackWithId`).
    /// `name` overrides the preset's own name for the track only — the
    /// preset keeps its name in the library (ba todo #1303).
    AddTrackFromPreset {
        preset: Box<TrackPreset>,
        id_hint: Option<TrackId>,
        name: Option<String>,
    },
    /// Delete a user preset by name.
    DeleteUserPreset(String),
    /// Open the "Save track as preset" name prompt, seeded with the
    /// track's own name (ba todo #1303, finding P1).
    OpenSavePresetPrompt(TrackId),
    /// Live edit of the name in that prompt.
    SetSavePresetName(String),
    /// Dismiss the prompt without saving.
    CloseSavePresetPrompt,
    /// Capture a track — its mixer settings, its instrument identity and
    /// its whole plugin chain including each plugin's opaque state — as
    /// a reusable user preset (ba todo #1303, finding P1; control method
    /// `track.save_preset`).
    ///
    /// The capture pipeline behind this has always worked; nothing ever
    /// started it, so the preset menu could only list presets a user had
    /// hand-written as JSON. Saving is a two-step: this arms
    /// `pending_preset_save` and asks the engine for the plugins' state
    /// blobs, and the `AllPluginStatesSaved` echo writes the file.
    ///
    /// `overwrite` is the destructive-operation flag: without it, a name
    /// that already exists is refused rather than replaced.
    SaveTrackAsPreset {
        track_id: TrackId,
        name: String,
        overwrite: bool,
    },
    /// "Bounce in place" — render this instrument track to a fresh
    /// audio track and mute the source. Routes to either the offline
    /// bounce (for tracks with an internal synth) or the bounce
    /// dialog (for external-MIDI tracks that need a real-time record
    /// from a chosen audio input).
    BounceInPlace(TrackId),
    /// Sub-flow for the realtime "Bounce in place" dialog (external
    /// MIDI tracks). Grouped under one variant so the top-level
    /// `TrackMessage` doesn't accumulate dialog plumbing.
    Bounce(BounceMessage),
}

/// User actions in the realtime bounce-in-place dialog (only shown for
/// external-MIDI instrument tracks). The dialog lifecycle: open →
/// `PickDevice` / `PickPort` → `Confirm` (kicks off the realtime bounce)
/// or `Cancel` (closes without side effects).
#[derive(Debug, Clone)]
pub enum BounceMessage {
    /// User picked an audio input device.
    PickDevice(Option<String>),
    /// User picked the starting input channel. In stereo mode the right
    /// channel is `port + 1`; in mono mode the same channel is captured
    /// to both L and R.
    PickPort(u16),
    /// Toggle stereo (`false`) vs mono (`true`) capture.
    SetMono(bool),
    /// User confirmed — kick off the realtime bounce.
    Confirm,
    /// User cancelled the dialog.
    Cancel,
    /// User clicked Cancel on the in-progress modal that's shown while
    /// a bounce is actually running. Distinct from `Cancel`, which only
    /// dismisses the pre-bounce input-picker dialog.
    CancelInProgress,
}

/// Aux-send + return-bus actions raised from the Mixer inspector's
/// ROUTING group. Every variant maps to one engine command (or, for
/// [`CreateReturnFromSend`](MixerMessage::CreateReturnFromSend), a short
/// ordered sequence). The handlers never mutate the send graph directly:
/// the engine validates each command and echoes `AuxSendChanged` /
/// `AuxSendRemoved` / `BusRoleChanged`, which the engine-event mirror
/// (ba todo #478) folds into [`AuxSendState`](crate::state::AuxSendState).
/// That single-writer rule keeps the GUI from showing a route the engine
/// rejected as cyclic.
#[derive(Debug, Clone)]
pub enum MixerMessage {
    /// Create a new aux send from `source` into return bus `dest` with
    /// default routing (0 dB, post-fader, enabled). The engine allocates
    /// the [`SendId`].
    AddSend { source: SendSource, dest: BusId },
    /// Create an aux send whose id the *app* chose up front, with
    /// explicit level and tap point, so a caller can use the id without
    /// waiting for the engine's `AuxSendChanged` echo (ba doc #273).
    /// Same hint pattern as [`BusMessage::AddBusWithId`].
    AddSendWithId {
        id: SendId,
        source: SendSource,
        dest: BusId,
        level_db: f32,
        pre_fader: bool,
    },
    /// Remove the send with this id.
    RemoveSend(SendId),
    /// Set a send's level in dB (slider drag). Coalesces into a single
    /// undo entry per drag, like the volume/pan faders.
    SetSendLevel(SendId, f32),
    /// Re-route an existing send into a different return bus.
    SetSendDest(SendId, BusId),
    /// Flip a send between a pre- and post-fader source tap.
    ToggleSendPreFader(SendId),
    /// Enable / disable a send while keeping its routing and level.
    ToggleSendEnabled(SendId),
    /// Mark a bus as an aux *return* bus, or clear the flag.
    SetBusReturnRole(BusId, bool),
    /// Create a brand-new FX return bus and route `source` into it in one
    /// gesture: add a bus, flag it as a return, then upsert the send.
    CreateReturnFromSend { source: SendSource },
}

#[derive(Debug, Clone)]
pub enum ClipMessage {
    DeleteClip(ClipId),
    StartClipDrag {
        clip_id: ClipId,
        grab_offset_x: f32,
        start_x: f32,
        start_y: f32,
    },
    UpdateClipDrag(f32, f32),
    EndClipDrag,
    StartClipTrim {
        clip_id: ClipId,
        edge: ClipEdge,
        anchor_x: f32,
    },
    UpdateClipTrim(f32),
    EndClipTrim,
    /// Begin dragging a fade handle. `edge` selects fade-in (`Left`) vs
    /// fade-out (`Right`); `anchor_x` is the pointer x at grab. Handled by
    /// the edit/drag update handlers (todo #317).
    StartClipFadeDrag {
        clip_id: ClipId,
        edge: ClipEdge,
        anchor_x: f32,
    },
    /// Update the active fade drag to pointer x.
    UpdateClipFadeDrag(f32),
    /// Commit the active fade drag.
    EndClipFadeDrag,
    /// Begin dragging the clip-gain bead. `anchor_y` is the pointer y at
    /// grab (gain is a vertical drag). Handled by todo #317.
    StartClipGainDrag {
        clip_id: ClipId,
        anchor_y: f32,
    },
    /// Update the active gain drag to pointer y.
    UpdateClipGainDrag(f32),
    /// Commit the active gain drag.
    EndClipGainDrag,
    // -- Inspector flyout edits (emitted by todo #319, handled by #317) --
    //
    // Discrete, atomic edits from the clip inspector flyout, complementing
    // the on-canvas direct manipulation above. Each one mutates the live
    // `ClipState` mirror and sends the matching engine command
    // (`SetClipFade` / `SetClipGain`); the undo system records one entry
    // per edit (see `undo::classify`). The flyout reads the current values
    // back from the same `ClipState` mirror, so on-canvas drags and the
    // numeric fields always agree.
    /// Set the fade-in length from the inspector's numeric field, in
    /// milliseconds. Converted to frames against the project sample rate
    /// and clamped to the clip's audible length.
    SetClipFadeInMs {
        clip_id: ClipId,
        ms: f32,
    },
    /// Set the fade-out length from the inspector's numeric field, in ms.
    SetClipFadeOutMs {
        clip_id: ClipId,
        ms: f32,
    },
    /// Set the clip gain from the inspector's numeric field, in decibels.
    SetClipGainDb {
        clip_id: ClipId,
        gain_db: f32,
    },
    /// Choose the fade-in curve from the inspector's curve picker.
    SetClipFadeInCurve {
        clip_id: ClipId,
        curve: FadeCurve,
    },
    /// Choose the fade-out curve from the inspector's curve picker.
    SetClipFadeOutCurve {
        clip_id: ClipId,
        curve: FadeCurve,
    },
    /// Reset the clip's fades and gain to their defaults (no fade, unity
    /// gain, default curves) — the inspector's "Reset to default" action.
    ResetClipFadeGain {
        clip_id: ClipId,
    },
    // -- Discrete placement edits (control endpoint `clip.*`, doc #265) --
    //
    // The on-canvas equivalents above are drag gestures: a Start/Update/End
    // triple whose geometry comes from pointer pixels. A remote client has
    // no pointer, so these two express the same two edits as one atomic,
    // already-resolved message — the same shape `MidiClipMessage::MoveClipTo`
    // takes for MIDI clips. Both mutate the live `ClipState` mirror and send
    // the matching engine command, and both are `UndoAction::Record`.
    /// Move an audio clip to an absolute timeline position, optionally onto
    /// another track. No snapping — the caller has already decided where it
    /// goes.
    MoveClipTo {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        /// The clip's track after the move; pass its current track to move
        /// it in time only.
        new_track_id: TrackId,
    },
    /// Set an audio clip's trim (and, with it, its timeline start) to
    /// absolute frame counts. The caller supplies values already clamped
    /// against the source length.
    TrimClipTo {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        trim_start_frames: u64,
        trim_end_frames: u64,
    },
    /// Cut a clip in two at an absolute timeline position (control
    /// endpoint `clip.split`, ba doc #275 P2). `new_clip_id` is allocated
    /// by the caller so the reply can name both halves without waiting
    /// for the engine echo, the same way `clip.place` does.
    SplitClipAt {
        clip_id: ClipId,
        new_clip_id: ClipId,
        at_sample: SamplePos,
    },
}

#[derive(Debug, Clone)]
pub enum PluginMessage {
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
    /// Add a plugin whose instance id the *app* chose up front, and
    /// mirror a placeholder slot into `TrackState.plugins` immediately,
    /// so the caller can address the plugin without waiting for the
    /// engine's `PluginAdded` echo (ba doc #273, todo #1234). Same hint
    /// pattern as [`BusMessage::AddBusWithId`] and the project-load
    /// replay; the engine bumps its own allocator past a hinted id only
    /// when that hint is BELOW
    /// [`CONTROL_PLUGIN_ID_BASE`](crate::state::plugin_index::CONTROL_PLUGIN_ID_BASE)
    /// — hints from the control range deliberately leave the engine
    /// allocator untouched, which is what keeps the two ranges from
    /// colliding. `engine_events::plugins::track_added` is idempotent,
    /// so the echo fills the placeholder's params in rather than pushing
    /// a duplicate.
    ///
    /// The GUI never sends this — its adds stay engine-allocated via
    /// [`AddPluginToTrack`](Self::AddPluginToTrack).
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
    TogglePluginPanel(PluginInstanceId),
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
}

/// Comping edits to a cycle-record take lane (design doc #165, epic #15,
/// todo #411).
///
/// Every variant names the [`TakeGroup`](resonance_common::TakeGroup) it
/// edits by the engine's `group_id` and resolves to at most one
/// `SetTakeComp` plus one `SetActiveTake`, applied optimistically to
/// [`TakeGroupState`](crate::state::TakeGroupState) and confirmed by the
/// engine's `TakeCompChanged` / `ActiveTakeChanged` echoes. Each is one
/// atomic undo entry.
///
/// A message that would change nothing — an unknown group or take, a
/// selection that is already current, a promote that lands outside the
/// chosen take's recorded audio — is refused **before** dispatch by
/// `Resonance::take_edit_is_refused`, so it spends no undo entry and bumps
/// no revision (the same rule ba todo #1261 established for a refused
/// chain reorder).
///
/// Expanding / collapsing a lane is deliberately *not* here: it is
/// transient view state carried by `UiMessage::ToggleTakeLane` (todo
/// #413), never an undo entry and never persisted.
#[derive(Debug, Clone)]
pub enum TakeMessage {
    /// Solo one take of `group_id` across the whole slot, overriding the
    /// comp — or clear the override with `None` so the comp plays again.
    ///
    /// Refused when the group does not hold `take_id`: the engine drops
    /// such a command silently and emits **no** echo (ba doc #292), so a
    /// rejected selection would otherwise leave the mirror asserting a
    /// solo that never happened.
    SetActiveTake {
        group_id: TakeGroupId,
        take_id: Option<TakeId>,
    },
    /// Cut the comp in two at the transport playhead, creating a boundary
    /// to promote against. Refused unless the playhead lies strictly
    /// inside the group's slot and inside a segment (a cut on an existing
    /// boundary changes nothing).
    SplitCompAtPlayhead { group_id: TakeGroupId },
    /// Promote `take_id` across `range`, replacing whatever covered it.
    ///
    /// `range` is a *request*: it is clamped to the group's slot and to
    /// the region the take's own recording actually spans before anything
    /// is sent, and the message is refused if nothing survives. The engine
    /// does not sanitise segment ranges, and a segment over a region its
    /// take cannot fill renders as silence inside the composite.
    PromoteTakeSegment {
        group_id: TakeGroupId,
        take_id: TakeId,
        range: TimelineRange,
    },
    /// Remove `take_id` from the lane and re-cover the slot from the takes
    /// that remain.
    ///
    /// **A group's last take removes the lane** (ba todo #1401): an empty
    /// group keeps its slot forever, has nothing to comp or draw, and an
    /// empty comp is exactly the state in which the cover falls back to
    /// the most recent pass — the take just deleted. Refused only when the
    /// group or the take is not there.
    ///
    /// Sent to the engine as `RemoveTake` / `RemoveTakeGroup` rather than
    /// as a comp edit, because only the engine can park the take's
    /// recording out of the render — an un-parked one plays raw, at full
    /// gain, on the ordinary clip path.
    DeleteTake {
        group_id: TakeGroupId,
        take_id: TakeId,
    },
}

#[derive(Debug, Clone)]
pub enum ViewportMessage {
    ZoomIn,
    ZoomOut,
    ScrollY(f32),
    /// The arrange view's outer horizontal `Scrollable` moved or resized
    /// (its `on_scroll`): live x offset, visible width and content width,
    /// all in px. Feeds playhead follow (review FU-D1).
    ArrangeScrolled {
        offset_x: f32,
        visible_width: f32,
        content_width: f32,
    },
    ScrollToY(f32),
    ViewportWidth(f32),
    /// Total available height the timeline canvas + track-header column
    /// see for content. Reported by `TimelineCanvas::report_viewport`
    /// whenever `bounds.height` moves more than 1 px. The track-header
    /// column uses this to drop tracks below the viewport during manual
    /// virtualization (see `view/track_header.rs`).
    ViewportHeight(f32),
    TimelineContentSize(f32, f32),
}

#[derive(Debug, Clone)]
pub enum ProjectIoMessage {
    BounceToWav,
    BouncePathSelected(Option<String>),
    SaveProject,
    SaveProjectAs,
    /// Begin a periodic autosave snapshot. Routed through the same async
    /// engine save state machine as [`Self::SaveProject`], but writes the
    /// metadata to `project.autosave.json`, leaves the project dirty, and
    /// targets a per-session scratch dir when the project was never saved.
    /// Fired by the change-gated autosave timer (todo #465).
    Autosave,
    /// Capture the open project as a reusable user template (todo #666).
    /// `name`/`description` label it in the picker; the two booleans are
    /// the capture toggles (carry the tempo map / the master FX chain).
    SaveAsTemplate {
        name: String,
        description: String,
        include_markers_and_tempo: bool,
        include_master_chain: bool,
    },
    OpenProject,
    /// User clicked a recent entry in the startup modal.
    OpenRecent(std::path::PathBuf),
    SavePathSelected(Option<String>),
    OpenPathSelected(Option<String>),
    /// Async save completion. The `bool` is `true` when the completed
    /// save was an autosave (routes to `last_autosave_at`, keeps `dirty`
    /// set, skips the recents list) rather than a manual save.
    ProjectSaved(Result<(), String>, bool),
    ProjectLoaded(Result<Box<LoadedProject>, String>),
    /// A *user* template finished loading from disk (todo #665). Carries the
    /// same `LoadedProject` payload as [`Self::ProjectLoaded`], but the
    /// instantiate handler replays it as a fresh, untitled project (path left
    /// `None`) so the template source on disk is never overwritten.
    TemplateLoaded(Result<Box<LoadedProject>, String>),
    ExportChordSheet,
    ChordSheetPathSelected(Option<String>, Vec<u8>),
    /// The user's answer to the autosave-recovery prompt (code review
    /// FU-M12a).
    RecoveryChoice(RecoveryChoice),
    /// Open `path` without the recovery prompt; `recover` loads its
    /// autosave (when one is recoverable) instead of `project.json`. The
    /// control `project.open` path: a client can't answer a modal.
    OpenResolved {
        path: std::path::PathBuf,
        recover: bool,
    },
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
    /// The raw `F` key press. Unlike [`TogglePerformanceMode`] this does not
    /// toggle directly: it first probes the live widget tree for keyboard
    /// focus (see [`crate::focus`]) and only toggles when no text field is
    /// being edited, so typing `F` into a track name / BPM / lyrics field
    /// never flips Performance mode. Resolves to [`PerformanceToggleResolved`].
    RequestPerformanceToggle,
    /// Result of the focus probe started by [`RequestPerformanceToggle`].
    /// `editing` is `true` when a text field held focus at the moment `F` was
    /// pressed; the toggle is suppressed in that case.
    PerformanceToggleResolved {
        editing: bool,
    },
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
    /// Fold / unfold one of the mixer-inspector groups (SIGNAL /
    /// ROUTING / CHAIN). Runtime UI state only.
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
    /// Raw next/prev-marker key press (`.` / `,`). Like
    /// [`RequestPerformanceToggle`], this does not navigate directly: it
    /// first probes the live widget tree for keyboard focus (see
    /// [`crate::focus`]) so typing `.`/`,` into a track name / lyrics /
    /// section field never jumps the playhead. Resolves to
    /// [`MarkerNavResolved`]. `forward` picks next (`true`) vs prev.
    RequestMarkerNav {
        forward: bool,
    },
    /// Result of the focus probe started by [`RequestMarkerNav`]. When no
    /// text field held focus (`editing == false`) the corresponding
    /// [`crate::message::MarkerMessage::JumpToNext`] /
    /// [`crate::message::MarkerMessage::JumpToPrev`] is dispatched.
    MarkerNavResolved {
        forward: bool,
        editing: bool,
    },
    /// A global keyboard shortcut that is also an ordinary typing key
    /// (Enter, `B`, Cmd-Z / Cmd-Y). Like [`RequestPerformanceToggle`] it
    /// does not act directly: the keyboard subscription sees presses a
    /// focused text field already consumed, so this probes widget focus
    /// (see [`crate::focus`]) and resolves to [`ShortcutResolved`]
    /// (UPD-11).
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
}

/// User actions for the MIDI Import modal (see [`crate::state::ImportDialogState`]
/// and [`crate::view::import_dialog`]). Lifecycle: `Open` → file
/// chosen/parsed → review / tempo-conflict → `Confirm`, or `Cancel` to
/// dismiss. Everything before `Confirm` is transient dialog state.
#[derive(Debug, Clone)]
pub enum ImportMessage {
    /// Open the modal at the Drop stage.
    Open,
    /// Dismiss the modal without importing.
    Cancel,
    /// A recognized MIDI file is being dragged over the window. Opens the
    /// modal at the Drop stage so the drop target is visible; a no-op when
    /// a dialog is already open. Emitted by the window file-drop
    /// subscription in `update.rs`.
    HoverFile,
    /// The dragged file(s) left the window without being dropped. Dismisses
    /// a dialog that was opened purely by the hover (and is still empty), so
    /// a stray drag-over doesn't leave the modal stuck open.
    HoverLeft,
    /// Open the OS file chooser for a `.mid`/`.midi` file; a pick comes
    /// back as [`Self::FileChosen`], a cancel as nothing.
    Choose,
    /// The user picked a file via the file dialog.
    FileChosen(std::path::PathBuf),
    /// A `.mid`/`.midi` file was dropped (onto the window or the modal).
    /// Opens the modal if it isn't already open, then kicks off the parse.
    FileDropped(std::path::PathBuf),
    /// Background parse finished — `Ok` carries the parsed summary + rows,
    /// `Err` a user-facing error string.
    ParseCompleted(Result<ParsedImport, String>),
    /// The parse task spawned for `path` finished. Applied as
    /// [`Self::ParseCompleted`] only while the dialog is still parsing
    /// that same file — a slower parse of a file the user has since
    /// replaced is dropped.
    Parsed {
        path: std::path::PathBuf,
        result: Result<ParsedImport, String>,
    },
    /// Toggle whether the row at this index is included in the import.
    ToggleTrack(usize),
    /// Select (`true`) or deselect (`false`) every row at once.
    SetAllTracks(bool),
    /// Rename the destination track for the row at this index.
    RenameTrack(usize, String),
    /// Choose how to reconcile the file vs project tempo.
    SetTempoChoice(TempoChoice),
    /// Set the timeline anchor for imported clips.
    SetPlacementStart(PlacementStart),
    /// Switch between new-tracks and merge-into-selected placement.
    SetPlacementMode(PlacementMode),
    /// Set the merge target track for `MergeIntoSelected`.
    SetMergeTarget(Option<TrackId>),
    /// Choose bar- vs time-aligned tempo-conflict resolution.
    SetConflictAlignment(TempoAlignment),
    /// Pick one TempoConflict option: the tempo choice together with its
    /// alignment (the alignment only matters when keeping the project's).
    ChooseTempo(TempoChoice, TempoAlignment),
    /// Accept the TempoConflict stage's choice and move on to Review.
    ResolveTempo,
    /// Import the selected tracks — the one message of the flow that
    /// edits the project, recorded as a single undo entry.
    Confirm,
}

/// Audio import + placement orchestration (doc #175, ba todo #598).
/// Drives the end-to-end flow: a multi-file selection (from the "Import
/// audio…" dialog or a drag-and-drop) is imported into the project pool
/// via `AudioCommand::ImportAudioToPool`, and — for a drop — each file is
/// placed as an audio clip once its `AssetImported` event lands. Routed
/// through `update::pool::handle`.
///
/// `ImportFilesToPool` and `ImportAndPlace` are classified
/// `UndoAction::Record` (see `undo::classify`) so the whole import +
/// placement is a single undoable action: the undo snapshot is taken up
/// front, before the import is issued, so one undo reverts the pool
/// asset(s), any placed clip(s), and a spawned track. `PickFiles` and
/// `WindowAudioDrop` are entry-point messengers — `PickFiles` opens the
/// OS dialog (no state change until `ImportFilesToPool` fires back) and
/// `WindowAudioDrop` resolves to `ImportAndPlace` inside the handler —
/// so both are classified `UndoAction::Skip`.
#[derive(Debug, Clone)]
pub enum PoolMessage {
    /// Open the OS multi-file audio picker (the "Import audio…" chrome
    /// button, ba todo #608). The picked paths come back as a
    /// `ImportFilesToPool` message via `Task::perform`; cancelling the
    /// dialog yields an empty path list that is silently dropped.
    /// Classified `UndoAction::Skip` — no state changes at dispatch time.
    PickFiles,
    /// An audio file was dropped onto the arrangement window from the OS
    /// (ba todo #608). The handler resolves the drop to a new audio track
    /// at the current playhead position and calls through to the shared
    /// `import()` helper. One message fires per dropped file (iced emits
    /// one `FileDropped` event per path). Classified `UndoAction::Skip`
    /// (the resulting `ImportAndPlace` that the handler re-dispatches
    /// records the actual undo entry).
    WindowAudioDrop(std::path::PathBuf),
    /// Import one or more files into the pool **without** placing a clip
    /// (the "Import audio…" dialog / pool-only path).
    ImportFilesToPool(Vec<std::path::PathBuf>),
    /// Import one or more files and place them as clips at `target` (a
    /// drop on an existing lane, or on the new-audio-track zone).
    ImportAndPlace {
        paths: Vec<std::path::PathBuf>,
        target: DropTarget,
    },
    /// Import one or more files and place them at an EXACT position on an
    /// existing track — no grid snap (control endpoint `clip.place`, doc
    /// #265).
    ///
    /// [`ImportAndPlace`](Self::ImportAndPlace) snaps the drop position to
    /// the timeline grid at the current zoom, which is right for a pointer
    /// and wrong for an API: a client that asked for a sample position
    /// would get a different one depending on how far the user happened to
    /// be zoomed in. This variant places where it was told.
    ImportAndPlaceExact {
        paths: Vec<std::path::PathBuf>,
        track_id: TrackId,
        start_sample: SamplePos,
    },
    /// Place an asset that is ALREADY in the pool as a clip, with no
    /// import step (control endpoint `clip.place`, doc #265).
    ///
    /// The GUI has no equivalent — dragging a pool row always goes through
    /// `ImportAndPlace`, which short-circuits to the same placement once
    /// it sees the file is known. A remote client placing the same
    /// one-shot forty times should not re-decode it forty times, so this
    /// skips straight to the placement. `clip_id` is allocated by the
    /// caller (the derived-clip range, as `MidiClipMessage::CreateEmptyClip`
    /// does) so the control reply can name the clip immediately.
    /// Classified `UndoAction::Record`.
    PlacePooledAsset {
        clip_id: ClipId,
        asset_id: AssetId,
        track_id: TrackId,
        start_sample: SamplePos,
    },
}

/// Missing-file relink actions (doc #175, todo #600). When a project is
/// loaded whose pool references a WAV that is no longer on disk, that
/// asset is flagged [`missing`](crate::state::pool::PoolAsset::missing) —
/// its clips are kept offline so nothing is lost — and these messages
/// drive resolving the file again:
///
/// * per-file [`Locate`](RelinkMessage::Locate) opens an OS file picker so
///   the user points one missing asset at a replacement file, and
/// * one-shot [`SearchFolder`](RelinkMessage::SearchFolder) picks a folder
///   and resolves *every* missing asset whose original filename is found
///   inside it (recursively).
///
/// Either way the resolved source is copied/transcoded back into the
/// project's `audio/` folder under the asset's stable
/// `asset_{id}.wav` name (reusing the import-to-pool path), the missing
/// flag is cleared, and the asset's clips are reloaded so playback
/// resumes. The metadata change rides the normal project snapshot, so the
/// relink is undoable. Routed through `update::relink::handle`.
#[derive(Debug, Clone)]
pub enum RelinkMessage {
    /// Open the OS file picker to locate a replacement file for one
    /// missing asset (the per-file `Locate…` action).
    Locate(AssetId),
    /// File-picker result for a single-asset [`Locate`](Self::Locate):
    /// `Some` with the chosen path, or `None` if the user cancelled.
    Located(AssetId, Option<std::path::PathBuf>),
    /// Open the OS folder picker for the one-shot `Search a folder…`
    /// batch relink.
    SearchFolder,
    /// Folder-picker result for [`SearchFolder`](Self::SearchFolder):
    /// `Some` folder resolves every missing asset whose original filename
    /// exists inside it; `None` if the user cancelled.
    FolderChosen(Option<std::path::PathBuf>),
    /// The background walk of a [`FolderChosen`](Self::FolderChosen)
    /// folder finished: the scan's token and the lowercased filename →
    /// path matches, or `None` if it was cancelled. Starts the imports.
    ScanFinished(
        u64,
        Option<std::collections::HashMap<String, std::path::PathBuf>>,
    ),
    /// Cancel the folder search in flight.
    CancelScan,
    /// A background relink import finished. `Ok` carries the transcoded
    /// asset's fresh metadata (it now lives in the project folder); `Err`
    /// carries the asset/file/reason of a failed import.
    Imported(Result<PoolImportOutcome, RelinkError>),
    /// Open the missing-files relink modal (todo #607). Fired on load when
    /// a project references missing assets, and by the Pool tab's inline
    /// `relink` chip. Snapshots the currently-missing assets into
    /// [`RelinkState::modal_targets`](crate::state::RelinkState::modal_targets).
    /// Presentational only — never undoable.
    ShowModal,
    /// Dismiss the relink modal (its "Leave offline" / close action). The
    /// tracked clips stay offline until relinked later. Presentational
    /// only — never undoable.
    DismissModal,
}

/// A failed relink import: which asset was being relinked, the source
/// file that was tried, and a user-facing reason.
#[derive(Debug, Clone)]
pub struct RelinkError {
    pub asset_id: AssetId,
    pub path: String,
    pub reason: String,
}
