/// Message types for the Resonance application.
///
/// Messages are grouped into per-concern sub-enums that mirror the
/// sub-state layout of [`crate::Resonance`]. Each sub-enum is handled by a
/// dedicated arm of the top-level match in `update.rs`.
use crate::compose::ComposeMessage;
use crate::project::LoadedProject;
use crate::state::{ClipEdge, InstrumentIcon, InstrumentType, LoopDragTarget, ViewMode};
use resonance_audio::types::{
    BusId, ClipId, PluginInstanceId, ScannedPlugin, TrackId, TrackOutput,
};

#[derive(Debug, Clone)]
pub(crate) enum Message {
    Compose(ComposeMessage),
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
    RemoveTrack(TrackId),
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
    SetInstrumentType(TrackId, InstrumentType),
    /// Change an instrument track's display icon.
    SetInstrumentIcon(TrackId, InstrumentIcon),
    SetTrackInputDevice(TrackId, Option<String>),
    SetTrackInputPort(TrackId, u16),
    /// Toggle whether a parent track's sub-tracks are shown in the mixer.
    ToggleSubTracksVisible(TrackId),
    SetTrackOutput(TrackId, TrackOutput),
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
    SelectClip(Option<ClipId>),
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
}
