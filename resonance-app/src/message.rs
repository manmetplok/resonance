/// Message types for the Resonance application.
///
/// Messages are grouped into per-concern sub-enums that mirror the
/// sub-state layout of [`crate::Resonance`]. Each sub-enum is handled by a
/// dedicated arm of the top-level match in `update.rs`.
use crate::compose::ComposeMessage;
use crate::presets::TrackPreset;
use crate::project::LoadedProject;
use crate::state::{
    ClipEdge, InstrumentIcon, InstrumentType, LoopDragTarget, SelectedGlobalEvent, ViewMode,
};
use resonance_audio::types::{
    BusId, ClipId, PluginInstanceId, ScannedPlugin, TrackId, TrackOutput,
};

#[derive(Debug, Clone)]
pub(crate) enum Message {
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
pub(crate) enum TransportMessage {
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
    CyclePrecountBars,
    CycleTimeSignature,
    ToggleLoop,
    StartLoopDrag(LoopDragTarget),
    UpdateLoopDrag(f32),
    EndLoopDrag,
}

#[derive(Debug, Clone)]
pub(crate) enum TrackMessage {
    AddTrack,
    AddInstrumentTrack,
    /// Direct removal — kept as a handler target for the engine command.
    /// All user-facing delete buttons go through `RequestRemoveTrack`
    /// which may show a confirmation dialog first.
    #[allow(dead_code)]
    RemoveTrack(TrackId),
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
    /// Change an instrument track's sub-type (synth vs drum).
    #[allow(dead_code)]
    SetInstrumentType(TrackId, InstrumentType),
    /// Change an instrument track's display icon.
    #[allow(dead_code)]
    SetInstrumentIcon(TrackId, InstrumentIcon),
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
}

#[derive(Debug, Clone)]
pub(crate) enum BusMessage {
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
pub(crate) enum MasterMessage {
    ToggleMasterFxBypass,
    AddPluginToMaster(ScannedPlugin),
    RemovePluginFromMaster(PluginInstanceId),
}

#[derive(Debug, Clone)]
pub(crate) enum ClipMessage {
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
}

#[derive(Debug, Clone)]
pub(crate) enum MidiClipMessage {
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
pub(crate) enum MidiEditorMessage {
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
    SelectNote {
        note_index: Option<usize>,
    },
    PreviewNote(TrackId, u8),
    StopPreview(TrackId, u8),
    ScrollX(f32),
    ScrollY(f32),
}

#[derive(Debug, Clone)]
pub(crate) enum PluginMessage {
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
pub(crate) enum ViewportMessage {
    ZoomIn,
    ZoomOut,
    ScrollX(f32),
    ScrollY(f32),
    ScrollToX(f32),
    ScrollToY(f32),
    ViewportWidth(f32),
    TimelineContentSize(f32, f32),
}

#[derive(Debug, Clone)]
pub(crate) enum ProjectIoMessage {
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
pub(crate) enum UiMessage {
    SwitchView(ViewMode),
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
pub(crate) enum GlobalTrackMessage {
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
