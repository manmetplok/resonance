/// Message types for the Resonance application.
///
/// Messages are grouped into per-concern sub-enums that mirror the
/// sub-state layout of [`crate::Resonance`]. Each sub-enum is handled by a
/// dedicated arm of the top-level match in `update.rs`.
use crate::compose::ComposeMessage;
use crate::presets::TrackPreset;
use crate::project::LoadedProject;
use crate::reference::ReferenceMessage;
use crate::state::{
    ClipEdge, ExportMode, LoopDragTarget, MixerInspectorGroup, ParsedImport, PlacementMode,
    PlacementStart, SelectedGlobalEvent, TempoAlignment, TempoChoice, ViewMode,
};
use resonance_audio::types::{
    BusId, ClipId, PluginInstanceId, ScannedPlugin, SendId, SendSource, TrackId, TrackOutput,
};

#[derive(Debug, Clone)]
pub enum Message {
    Compose(ComposeMessage),
    GlobalTrack(GlobalTrackMessage),
    Transport(TransportMessage),
    Marker(MarkerMessage),
    Track(TrackMessage),
    Bus(BusMessage),
    Mixer(MixerMessage),
    Master(MasterMessage),
    Clip(ClipMessage),
    MidiClip(MidiClipMessage),
    MidiEditor(MidiEditorMessage),
    VocalTuning(VocalTuningMessage),
    Plugin(PluginMessage),
    Viewport(ViewportMessage),
    ProjectIo(ProjectIoMessage),
    Reference(ReferenceMessage),
    Export(ExportMessage),
    Import(ImportMessage),
    Ui(UiMessage),
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

#[derive(Debug, Clone)]
pub enum TransportMessage {
    Play,
    Record,
    Pause,
    Stop,
    SkipBack,
    SkipForward,
    /// Move the playhead to the given sample position (ruler click, etc.).
    SeekToSample(u64),
    SetBpmText(String),
    CommitBpm,
    ToggleMetronome,
    CycleTimeSignature,
    ToggleLoop,
    StartLoopDrag(LoopDragTarget),
    UpdateLoopDrag(f32),
    EndLoopDrag,
}

/// Arrangement-marker actions, routed like [`TransportMessage`] and
/// handled by `update/marker.rs`. The mutating variants
/// (`AddAtPlayhead`, `Rename`, `Recolor`, `Delete`, `MoveStart`,
/// `SetRegionEnd`, `LoopToRegion`, `SeedFromSections`) record an undo
/// entry; the navigation variants (`JumpToNext`, `JumpToPrev`, `JumpTo`,
/// `PlayFromMarker`) only move the playhead / transport and are not
/// undoable, mirroring `SeekToSample` / `Play`.
#[derive(Debug, Clone)]
pub enum MarkerMessage {
    /// Drop a new point marker at the current playhead (snapped to the
    /// grid via `snap_sample_to_grid_tempo`).
    AddAtPlayhead,
    /// Replace all section-seeded markers with a fresh set derived from
    /// the current Compose section placements — one ranged marker per
    /// placement, named/coloured from its section definition. Markers the
    /// user placed by hand are left untouched.
    SeedFromSections,
    /// Rename the marker with the given id.
    Rename(u64, String),
    /// Recolor the marker with the given id.
    Recolor(u64, [u8; 3]),
    /// Delete the marker with the given id.
    Delete(u64),
    /// Move a marker's start to a new sample position (snapped to the
    /// grid). The collection re-sorts after the move.
    MoveStart(u64, u64),
    /// Set (or clear, with `None`) a marker's region end, turning a
    /// point marker into a ranged region and back.
    SetRegionEnd(u64, Option<u64>),
    /// Move the playhead to the next marker after the playhead.
    JumpToNext,
    /// Move the playhead to the previous marker before the playhead.
    JumpToPrev,
    /// Move the playhead to a specific marker.
    JumpTo(u64),
    /// Set the loop range to a marker's region and enable looping. A
    /// ranged marker loops over `[start, end]`; a point marker loops
    /// from its start to the next marker's start.
    LoopToRegion(u64),
    /// Seek to a marker and start playback.
    PlayFromMarker(u64),
}

#[derive(Debug, Clone)]
pub enum TrackMessage {
    AddTrack,
    AddInstrumentTrack,
    AddVocalTrack,
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
    AddTrackFromPreset(Box<TrackPreset>),
    /// Delete a user preset by name.
    DeleteUserPreset(String),
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

/// User actions in the Export modal (design doc #155). The scaffold wires
/// the shell lifecycle - open/close and the mode-tab switch - plus the
/// footer's primary action. The per-tab body controls (source checklist,
/// range/format, destination) emit their own messages added by the body
/// todos (#326/#327); `Confirm` kicks off the render in #330/#331.
#[derive(Debug, Clone)]
pub enum ExportMessage {
    /// Open the modal in its default (Audio-stems) state.
    Open,
    /// Dismiss the modal, discarding the transient selection.
    Close,
    /// Switch the active mode tab (Audio stems / MIDI).
    SetMode(ExportMode),
    /// Footer primary action - render the selected sources. Wired here so
    /// the shell is complete; the actual orchestration lands in #330/#331.
    Confirm,
}

#[derive(Debug, Clone)]
pub enum BusMessage {
    AddBus,
    RemoveBus(BusId),
    SetBusVolume(BusId, f32),
    SetBusPan(BusId, f32),
    ToggleBusMute(BusId),
    ToggleBusFxBypass(BusId),
    AddPluginToBus(BusId, ScannedPlugin),
    RemovePluginFromBus(BusId, PluginInstanceId),
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
pub enum MasterMessage {
    ToggleMasterFxBypass,
    AddPluginToMaster(ScannedPlugin),
    RemovePluginFromMaster(PluginInstanceId),
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
}

#[derive(Debug, Clone)]
pub enum MidiClipMessage {
    DeleteMidiClip(ClipId),
    StartMidiClipDrag {
        clip_id: ClipId,
        grab_offset_x: f32,
        start_x: f32,
        start_y: f32,
    },
    UpdateMidiClipDrag(f32, f32),
    EndMidiClipDrag,
    StartMidiClipTrim {
        clip_id: ClipId,
        edge: ClipEdge,
        anchor_x: f32,
    },
    UpdateMidiClipTrim(f32),
    EndMidiClipTrim,
}

#[derive(Debug, Clone)]
pub enum MidiEditorMessage {
    OpenMidiEditor(ClipId),
    /// Open the currently selected MIDI clip (if any) in the piano roll editor.
    OpenSelectedMidiClip,
    CloseMidiEditor,
    AddNote {
        clip_id: ClipId,
        note: u8,
        start_tick: u64,
        duration_ticks: u64,
        velocity: f32,
    },
    RemoveNote {
        clip_id: ClipId,
        note_index: usize,
    },
    /// Remove every currently-selected note from `clip_id` in one edit
    /// (the piano roll's Delete/Backspace on a multi-note selection).
    RemoveSelectedNotes {
        clip_id: ClipId,
    },
    MoveNote {
        clip_id: ClipId,
        note_index: usize,
        new_start_tick: u64,
        new_note: u8,
    },
    ResizeNote {
        clip_id: ClipId,
        note_index: usize,
        new_duration_ticks: u64,
    },
    /// Replace the selection with a single note, or clear it (`None`).
    /// Used by a plain click and by the vocal roll's single-select path.
    SelectNote {
        note_index: Option<usize>,
    },
    /// Toggle one note's membership in the selection (shift/ctrl-click).
    ToggleNoteSelection {
        note_index: usize,
    },
    /// Apply a rubber-band marquee result: the notes whose rectangles fall
    /// inside the drag rect. `additive` (shift held) unions with the
    /// current selection instead of replacing it.
    SelectNotesInRect {
        indices: Vec<usize>,
        additive: bool,
    },
    /// Select every note in the open clip (Ctrl/Cmd+A).
    SelectAllNotes,
    /// Drop the whole selection (click on empty space).
    ClearNoteSelection,
    PreviewNote(TrackId, u8),
    StopPreview(TrackId, u8),
    ScrollY(f32),
    /// Vocal-roll only: toggle the OpenUtau slur marker on the i-th
    /// note of `clip_id`. `+` continuation ↔ the auto-syllabified
    /// surface form. Lives on this enum so the vocal roll's key
    /// handlers can dispatch through the same router as the other
    /// note edits.
    ToggleSlur {
        clip_id: ClipId,
        note_index: usize,
    },
}

/// Vocal pitch-editor (graphical tuning) messages, doc #160. This todo
/// (#359) wires only the editor open/close lifecycle: opening on a vocal
/// clip requests pitch analysis (`AudioCommand::AnalyzeClipPitch`) and the
/// detected contour/notes arrive back via `AudioEvent::ClipPitchDetected`
/// to populate [`crate::state::ClipState::vocal_tuning`]. The per-note /
/// global edit variants and the editor view land in later todos.
#[derive(Debug, Clone)]
pub enum VocalTuningMessage {
    /// Open the pitch editor on the given audio clip and kick off pitch
    /// analysis. A no-op unless the clip exists on a vocal track.
    OpenPitchEditor(ClipId),
    /// Close the pitch editor, clearing the open-editor state.
    ClosePitchEditor,
}

#[derive(Debug, Clone)]
pub enum PluginMessage {
    AddPluginToTrack(TrackId, ScannedPlugin),
    RemovePluginFromTrack(TrackId, PluginInstanceId),
    TogglePluginPanel(PluginInstanceId),
    SetPluginParam(PluginInstanceId, u32, f64),
    /// Open the plugin's editor window (CLAP_EXT_GUI).
    OpenPluginEditor(PluginInstanceId),
    /// Close the plugin's editor window.
    ClosePluginEditor(PluginInstanceId),
}

#[derive(Debug, Clone)]
pub enum ViewportMessage {
    ZoomIn,
    ZoomOut,
    ScrollY(f32),
    /// Horizontal scroll-to, dispatched when the canvas-side trim/drag
    /// helpers auto-scroll the timeline as the cursor approaches an edge.
    ScrollToX(f32),
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
    ExportChordSheet,
    ChordSheetPathSelected(Option<String>, Vec<u8>),
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
    SelectTrack(Option<TrackId>),
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
    /// Toggle MIDI clock send (engine acts as clock master).
    ToggleMidiClockSend,
    /// Pick the hardware port for MIDI clock send. `None` clears.
    SetMidiClockSendDevice(Option<String>),
    /// Toggle MIDI clock receive (engine slaves to an external master).
    ToggleMidiClockRecv,
    /// Pick the hardware port for MIDI clock receive. `None` clears.
    SetMidiClockRecvDevice(Option<String>),
}

#[derive(Debug, Clone)]
pub enum GlobalTrackMessage {
    /// Add a tempo change event at the given bar with the given BPM.
    AddTempoEvent { bar: u32, bpm: f32 },
    /// Update an existing tempo event in-place (drag interaction).
    UpdateTempoEvent { index: usize, bar: u32, bpm: f32 },
    /// Start dragging a tempo event (undo begin + select).
    StartTempoDrag(usize),
    /// Finish dragging a tempo event (undo commit).
    EndTempoDrag,
    /// Add a time signature change event at the given bar.
    AddSignatureEvent {
        bar: u32,
        numerator: u8,
        denominator: u8,
    },
    /// Update an existing signature event's numerator or denominator.
    UpdateSignatureEvent {
        index: usize,
        numerator: u8,
        denominator: u8,
    },
    /// Select an event on a global track.
    SelectEvent(Option<SelectedGlobalEvent>),
    /// Delete the currently selected global track event.
    DeleteSelectedEvent,
}

/// User actions for the MIDI Import modal (see [`crate::state::ImportDialogState`]
/// and [`crate::view::import_dialog`]). Lifecycle: `Open` → file
/// chosen/parsed → review / tempo-conflict → `Confirm`, or `Cancel` to
/// dismiss. The interactions beyond open/close drive the dialog's review
/// state; the parse task and the actual import land in the follow-up
/// todos (doc #158), so their orchestration is not wired here yet.
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
    /// The user picked a file via the file dialog.
    FileChosen(std::path::PathBuf),
    /// A `.mid`/`.midi` file was dropped (onto the window or the modal).
    /// Opens the modal if it isn't already open, then kicks off the parse.
    FileDropped(std::path::PathBuf),
    /// Background parse finished — `Ok` carries the parsed summary + rows,
    /// `Err` a user-facing error string.
    ParseCompleted(Result<ParsedImport, String>),
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
    /// Confirm and start the import.
    Confirm,
}
