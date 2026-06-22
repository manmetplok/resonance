/// Message types for the Resonance application.
///
/// Messages are grouped into per-concern sub-enums that mirror the
/// sub-state layout of [`crate::Resonance`]. Each sub-enum is handled by a
/// dedicated arm of the top-level match in `update.rs`.
use crate::compose::ComposeMessage;
use crate::presets::TrackPreset;
use crate::project::LoadedProject;
use crate::state::{ClipEdge, LoopDragTarget, MixerInspectorGroup, SelectedGlobalEvent, ViewMode};
use resonance_audio::types::{
    BusId, ClipId, PluginInstanceId, ScannedPlugin, TrackId, TrackOutput,
};

#[derive(Debug, Clone)]
pub enum Message {
    Compose(ComposeMessage),
    GlobalTrack(GlobalTrackMessage),
    Transport(TransportMessage),
    Track(TrackMessage),
    Bus(BusMessage),
    Master(MasterMessage),
    Clip(ClipMessage),
    MidiClip(MidiClipMessage),
    MidiEditor(MidiEditorMessage),
    Plugin(PluginMessage),
    Viewport(ViewportMessage),
    ProjectIo(ProjectIoMessage),
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
    OpenProject,
    /// User clicked a recent entry in the startup modal.
    OpenRecent(std::path::PathBuf),
    SavePathSelected(Option<String>),
    OpenPathSelected(Option<String>),
    ProjectSaved(Result<(), String>),
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
