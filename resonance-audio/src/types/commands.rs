//! GUI → engine command enum.
use std::path::PathBuf;

use super::{
    BusId, ClipId, MidiNote, PluginInstanceId, SamplePos, SignaturePoint, TempoPoint, TrackId,
    TrackOutput,
};

/// Commands sent from the GUI to the audio engine.
#[derive(Debug, Clone)]
pub enum AudioCommand {
    Play,
    /// Start recording on every armed track. When `precount_bars > 0`,
    /// the engine rewinds the playhead by that many bars, force-enables
    /// the metronome, and begins playback; the input stream opens and
    /// `recording` flips true once the playhead catches up to the
    /// original position.
    Record {
        precount_bars: u8,
    },
    Pause,
    Stop,
    SeekTo(SamplePos),
    ImportClip {
        track_id: TrackId,
        path: String,
        start_sample: SamplePos,
    },
    MoveClip {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        new_track_id: TrackId,
    },
    TrimClip {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        trim_start_frames: u64,
        trim_end_frames: u64,
    },
    DeleteClip {
        clip_id: ClipId,
    },
    SetTrackVolume {
        track_id: TrackId,
        volume: f32,
    },
    SetTrackPan {
        track_id: TrackId,
        pan: f32,
    },
    SetTrackMute {
        track_id: TrackId,
        muted: bool,
    },
    SetMasterVolume {
        volume: f32,
    },
    SetTrackSolo {
        track_id: TrackId,
        soloed: bool,
    },
    /// Add an audio track. When `id_hint`/`name` are provided (e.g. by
    /// project load), the engine honours them and bumps its allocator
    /// past the hinted id so subsequent fresh tracks don't collide.
    AddTrack {
        id_hint: Option<TrackId>,
        name: Option<String>,
    },
    RemoveTrack {
        track_id: TrackId,
    },
    /// Register an app-side sub-track with the audio engine so the mixer
    /// can drive it from its parent plugin's output port. The app
    /// allocates the id itself (high range, never colliding with
    /// engine ids counting up from 1) and passes it here. Called after
    /// `AudioEvent::PluginAdded` for every non-main output port on a
    /// multi-output plugin.
    CreateSubTrack {
        sub_id: TrackId,
        parent_track_id: TrackId,
        output_port_index: u32,
        name: String,
    },
    SetTrackRecordArm {
        track_id: TrackId,
        armed: bool,
    },
    SetTrackMonitor {
        track_id: TrackId,
        enabled: bool,
    },
    SetTrackMono {
        track_id: TrackId,
        mono: bool,
    },
    SetTrackInputDevice {
        track_id: TrackId,
        device_name: Option<String>,
    },
    /// Set the 0-indexed starting input channel for a track. Mono
    /// tracks capture just this channel; stereo tracks capture this
    /// channel as L and `port_index + 1` as R.
    SetTrackInputPort {
        track_id: TrackId,
        port_index: u16,
    },
    ListInputDevices,
    SetBpm {
        bpm: f32,
    },
    /// Send the full tempo event list so the engine can compute BPM
    /// at any playhead position internally, without per-tick updates.
    SetTempoEvents {
        tempo: Vec<TempoPoint>,
        signature: Vec<SignaturePoint>,
    },
    SetTimeSignature {
        numerator: u8,
        denominator: u8,
    },
    SetMetronomeEnabled {
        enabled: bool,
    },
    AddPlugin {
        track_id: TrackId,
        clap_file_path: String,
        clap_plugin_id: String,
        /// If provided, the engine uses this id instead of allocating a
        /// new one and bumps its allocator past it. Set by project load.
        id_hint: Option<PluginInstanceId>,
    },
    RemovePlugin {
        track_id: TrackId,
        instance_id: PluginInstanceId,
    },
    ScanPlugins,
    SetPluginParam {
        instance_id: PluginInstanceId,
        param_id: u32,
        value: f64,
    },
    /// Set loop (cycle) range. When enabled, playback wraps from loop_out back to
    /// loop_in, and any recording is trimmed to [loop_in, loop_out]. If
    /// loop_out <= loop_in, no clip is produced.
    SetLoopRange {
        enabled: bool,
        loop_in: SamplePos,
        loop_out: SamplePos,
    },
    SavePluginState {
        instance_id: PluginInstanceId,
    },
    LoadPluginState {
        instance_id: PluginInstanceId,
        data: Vec<u8>,
    },
    /// Open the plugin's editor window (requires CLAP_EXT_GUI).
    OpenPluginEditor {
        instance_id: PluginInstanceId,
    },
    /// Close the plugin's editor window.
    ClosePluginEditor {
        instance_id: PluginInstanceId,
    },
    /// Offline render of the project to a WAV file.
    BounceToWav {
        path: String,
    },
    /// Set the current project directory. Recorded and imported
    /// clips are written into `{project_dir}/audio/` as WAV files,
    /// and recording refuses to start if no project directory has
    /// been set. Sent by the app whenever a project is opened,
    /// created, or saved-as to a new location.
    SetProjectDir(PathBuf),
    /// Load an audio clip from a WAV file on disk (project load
    /// path). The engine memory-maps the file and references it
    /// via `ClipSource::Mapped`, so the PCM data never materialises
    /// as a contiguous in-RAM buffer.
    LoadClipFromWav {
        clip_id: ClipId,
        track_id: TrackId,
        start_sample: SamplePos,
        path: PathBuf,
        name: String,
        trim_start_frames: u64,
        trim_end_frames: u64,
    },
    /// Ensure every in-engine audio clip has a WAV file on disk at
    /// `{project_dir}/audio/clip_{id}.wav`. Recorded clips already
    /// do (they stream there during capture); in-RAM imported clips
    /// get transcoded. Emits `AudioEvent::ClipsSavedToProjectDir`
    /// when done so the save path can write project.json.
    SaveClipsToProjectDir,
    /// Batch save all plugin states for project save.
    SaveAllPluginStates,
    /// Remove all tracks, clips, and plugins (for project load).
    ClearAll,

    // -- Instrument track commands --
    /// Add an instrument track. See [`AudioCommand::AddTrack`] for how
    /// `id_hint`/`name` are honoured.
    AddInstrumentTrack {
        id_hint: Option<TrackId>,
        name: Option<String>,
    },

    // -- MIDI clip commands --
    CreateMidiClip {
        track_id: TrackId,
        start_sample: SamplePos,
        duration_ticks: u64,
        name: String,
    },
    LoadMidiClipDirect {
        clip_id: ClipId,
        track_id: TrackId,
        start_sample: SamplePos,
        duration_ticks: u64,
        notes: Vec<MidiNote>,
        name: String,
        trim_start_ticks: u64,
        trim_end_ticks: u64,
    },
    MoveMidiClip {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        new_track_id: TrackId,
    },
    TrimMidiClip {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        trim_start_ticks: u64,
        trim_end_ticks: u64,
    },
    DeleteMidiClip {
        clip_id: ClipId,
    },

    // -- MIDI note editing commands --
    AddMidiNote {
        clip_id: ClipId,
        note: MidiNote,
    },
    RemoveMidiNote {
        clip_id: ClipId,
        note_index: usize,
    },
    MoveMidiNote {
        clip_id: ClipId,
        note_index: usize,
        new_start_tick: u64,
        new_note: u8,
    },
    ResizeMidiNote {
        clip_id: ClipId,
        note_index: usize,
        new_duration_ticks: u64,
    },
    SetMidiNoteVelocity {
        clip_id: ClipId,
        note_index: usize,
        velocity: f32,
    },

    // -- Live MIDI input --
    SendNoteOn {
        track_id: TrackId,
        note: u8,
        velocity: f32,
    },
    SendNoteOff {
        track_id: TrackId,
        note: u8,
    },

    // -- Bus commands --
    /// Add a bus. See [`AudioCommand::AddTrack`] for how `id_hint`/`name`
    /// are honoured.
    AddBus {
        id_hint: Option<BusId>,
        name: Option<String>,
    },
    RemoveBus {
        bus_id: BusId,
    },
    SetBusVolume {
        bus_id: BusId,
        volume: f32,
    },
    SetBusPan {
        bus_id: BusId,
        pan: f32,
    },
    SetBusMute {
        bus_id: BusId,
        muted: bool,
    },
    SetBusName {
        bus_id: BusId,
        name: String,
    },
    SetTrackOutput {
        track_id: TrackId,
        output: TrackOutput,
    },
    AddPluginToBus {
        bus_id: BusId,
        clap_file_path: String,
        clap_plugin_id: String,
        /// If provided, the engine uses this id instead of allocating a
        /// new one and bumps its allocator past it. Set by project load.
        id_hint: Option<PluginInstanceId>,
    },
    RemovePluginFromBus {
        bus_id: BusId,
        instance_id: PluginInstanceId,
    },

    // -- Master FX chain + bypass --
    /// Add a plugin to the master bus insert chain. Master FX run after
    /// every track/bus has been summed, before the master volume pass.
    AddPluginToMaster {
        clap_file_path: String,
        clap_plugin_id: String,
        /// If provided, the engine uses this id instead of allocating a
        /// new one and bumps its allocator past it. Set by project load.
        id_hint: Option<PluginInstanceId>,
    },
    RemovePluginFromMaster {
        instance_id: PluginInstanceId,
    },
    /// Bypass every effect plugin on a track. Instrument plugins
    /// (slot 0 on instrument tracks) keep running.
    SetTrackFxBypass {
        track_id: TrackId,
        bypassed: bool,
    },
    SetBusFxBypass {
        bus_id: BusId,
        bypassed: bool,
    },
    SetMasterFxBypass {
        bypassed: bool,
    },
}
