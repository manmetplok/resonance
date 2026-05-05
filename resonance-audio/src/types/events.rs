//! Engine → GUI event enum.
use crate::midi_hardware::MidiDeviceInfo;

use super::{
    BusId, ClipId, InputDeviceInfo, MidiNote, ParamInfo, PluginInstanceId, SamplePos,
    ScannedPlugin, TrackId,
};

/// Inline clip payload for the offline "bounce in place" flow. The
/// realtime flow leaves this `None` because the clip arrives via the
/// regular `RecordingFinished` channel.
#[derive(Debug, Clone)]
pub struct BouncedClipData {
    pub clip_id: ClipId,
    pub start_sample: SamplePos,
    pub duration_samples: u64,
    pub name: String,
    /// Downsampled waveform peaks: (min, max) per chunk of frames.
    pub waveform_peaks: Vec<(f32, f32)>,
}

/// Events sent from the audio engine back to the GUI.
#[derive(Debug, Clone)]
pub enum AudioEvent {
    PlayheadMoved(SamplePos),
    SampleRateDetected {
        sample_rate: u32,
    },
    ClipImported {
        clip_id: ClipId,
        track_id: TrackId,
        start_sample: SamplePos,
        duration_samples: u64,
        name: String,
        /// Downsampled waveform peaks: (min, max) per chunk of frames.
        waveform_peaks: Vec<(f32, f32)>,
    },
    TrackAdded {
        track_id: TrackId,
    },
    TrackRemoved {
        track_id: TrackId,
    },
    ClipDeleted {
        clip_id: ClipId,
    },
    ClipMoved {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        new_track_id: TrackId,
    },
    ClipTrimmed {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        new_duration_samples: u64,
        trim_start_frames: u64,
        trim_end_frames: u64,
    },
    Stopped,
    Error(String),
    InputDevicesListed {
        devices: Vec<InputDeviceInfo>,
        default_name: Option<String>,
    },
    RecordingStarted {
        start_sample: SamplePos,
    },
    RecordingFinished {
        clip_id: ClipId,
        track_id: TrackId,
        start_sample: SamplePos,
        duration_samples: u64,
        name: String,
        /// Downsampled waveform peaks: (min, max) per chunk of frames.
        waveform_peaks: Vec<(f32, f32)>,
    },
    PluginAdded {
        track_id: TrackId,
        instance_id: PluginInstanceId,
        plugin_name: String,
        clap_plugin_id: String,
        clap_file_path: String,
        params: Vec<ParamInfo>,
        /// Whether the plugin exposes a CLAP GUI the host can open.
        has_gui: bool,
        /// Number of audio output ports declared by this plugin instance.
        /// `1` for every legacy single-output plugin. Multi-output plugins
        /// (e.g. `resonance-drums`) report a larger number and the app
        /// auto-creates one sub-track per non-main output port.
        output_port_count: usize,
        /// Human-readable name of each output port, same length as
        /// `output_port_count`.
        output_port_names: Vec<String>,
    },
    PluginRemoved {
        track_id: TrackId,
        instance_id: PluginInstanceId,
    },
    PluginsScanned {
        plugins: Vec<ScannedPlugin>,
    },
    PluginStateSaved {
        instance_id: PluginInstanceId,
        data: Vec<u8>,
    },
    BounceComplete {
        path: String,
    },
    BounceError(String),
    /// "Bounce in place" finished. Covers both the offline (internal
    /// synth) and realtime (external MIDI) flows; `clip` is `Some` when
    /// the engine rendered the clip inline (offline) and `None` when
    /// the realtime branch delivered it via the existing
    /// `RecordingFinished` event. The app mirrors the source mute
    /// locally and reorders the new track to sit beneath the source.
    TrackBounceCompleted {
        source_track_id: TrackId,
        target_track_id: TrackId,
        clip: Option<BouncedClipData>,
    },
    /// A "bounce in place" run failed. The string is user-facing.
    TrackBounceError(String),
    /// Response to `SaveClipsToProjectDir`: every in-engine audio
    /// clip has a `.wav` file on disk. The map is `clip_id` →
    /// project-relative path (e.g. `"audio/clip_42.wav"`), which
    /// the save path writes into `project.json`.
    ClipsSavedToProjectDir {
        clip_files: Vec<(ClipId, String)>,
    },
    /// All plugin states saved in batch.
    AllPluginStatesSaved {
        states: Vec<(PluginInstanceId, Vec<u8>)>,
    },
    /// Engine has been cleared of all state.
    AllCleared,

    // -- Instrument track events --
    InstrumentTrackAdded {
        track_id: TrackId,
    },

    // -- MIDI clip events --
    MidiClipCreated {
        clip_id: ClipId,
        track_id: TrackId,
        start_sample: SamplePos,
        duration_ticks: u64,
        name: String,
        notes: Vec<MidiNote>,
        trim_start_ticks: u64,
        trim_end_ticks: u64,
    },
    MidiClipMoved {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        new_track_id: TrackId,
    },
    MidiClipTrimmed {
        clip_id: ClipId,
        new_start_sample: SamplePos,
        trim_start_ticks: u64,
        trim_end_ticks: u64,
    },
    MidiClipDeleted {
        clip_id: ClipId,
    },

    // -- MIDI note editing events --
    MidiNoteAdded {
        clip_id: ClipId,
        note: MidiNote,
    },
    MidiNoteRemoved {
        clip_id: ClipId,
        note_index: usize,
    },
    MidiNoteMoved {
        clip_id: ClipId,
        note_index: usize,
        new_start_tick: u64,
        new_note: u8,
    },
    MidiNoteResized {
        clip_id: ClipId,
        note_index: usize,
        new_duration_ticks: u64,
    },
    MidiNoteVelocitySet {
        clip_id: ClipId,
        note_index: usize,
        velocity: f32,
    },

    // -- Bus events --
    BusAdded {
        bus_id: BusId,
        name: String,
    },
    BusRemoved {
        bus_id: BusId,
    },
    BusPluginAdded {
        bus_id: BusId,
        instance_id: PluginInstanceId,
        plugin_name: String,
        clap_plugin_id: String,
        clap_file_path: String,
        params: Vec<ParamInfo>,
        has_gui: bool,
    },
    BusPluginRemoved {
        bus_id: BusId,
        instance_id: PluginInstanceId,
    },

    // -- Master FX events --
    MasterPluginAdded {
        instance_id: PluginInstanceId,
        plugin_name: String,
        clap_plugin_id: String,
        clap_file_path: String,
        params: Vec<ParamInfo>,
        has_gui: bool,
    },
    MasterPluginRemoved {
        instance_id: PluginInstanceId,
    },
    TrackFxBypassChanged {
        track_id: TrackId,
        bypassed: bool,
    },
    BusFxBypassChanged {
        bus_id: BusId,
        bypassed: bool,
    },
    MasterFxBypassChanged {
        bypassed: bool,
    },

    // -- Hardware MIDI I/O --
    /// Result of `AudioCommand::ListMidiInputDevices`.
    MidiInputDevicesListed {
        devices: Vec<MidiDeviceInfo>,
    },
    /// Result of `AudioCommand::ListMidiOutputDevices`.
    MidiOutputDevicesListed {
        devices: Vec<MidiDeviceInfo>,
    },
    /// Incoming MIDI Clock Start (0xFA) — external master started its
    /// transport and the engine is now playing in sync with it.
    MidiClockStarted,
    /// Incoming MIDI Clock Continue (0xFB) — external master resumed.
    MidiClockContinued,
    /// Incoming MIDI Clock Stop (0xFC) — external master stopped.
    MidiClockStopped,
    /// External master tempo derived from incoming clock pulses, in
    /// BPM. Sent at most a few times per second once the input clock
    /// stabilises so the GUI BPM display can mirror the master.
    MidiClockTempoDetected {
        bpm: f32,
    },
}
