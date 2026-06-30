//! Engine → GUI event enum.
use resonance_common::{AudioFormat, AutomationLane, AutomationTarget, ExternalInstrument};

use crate::midi_hardware::MidiDeviceInfo;

use super::{
    AssetId, BusId, ClipId, FadeCurve, InputDeviceInfo, MidiNote, ParamInfo, PluginInstanceId,
    SamplePos, ScannedPlugin, TrackId,
};

/// Lifecycle stage of a single file in an `ImportAudioToPool` batch.
/// Drives the import/transcode progress modal: every file is reported
/// `Queued` up front, flips to `Working` while its worker decodes and
/// transcodes, and lands on `Done` once its `AssetImported` event has
/// been emitted. A file that fails reports no `Done` — it terminates
/// with [`AudioEvent::ImportFailed`] instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStage {
    Queued,
    Working,
    Done,
}

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
    /// Per-file progress for an `ImportAudioToPool` batch. See
    /// [`ImportStage`]. `asset_id` matches the eventual
    /// [`AudioEvent::AssetImported`] / [`AudioEvent::ImportFailed`] for
    /// the same file, so the progress modal can key its rows by id.
    ImportProgress {
        asset_id: AssetId,
        /// Original source path, as passed in the command.
        path: String,
        stage: ImportStage,
    },
    /// A source file was successfully imported into the project pool.
    /// The engine-format WAV now lives at `project_relative_path` (e.g.
    /// `"audio/asset_7.wav"`) inside the project directory, stereo f32 at
    /// the project sample rate. `channels` and `source_sample_rate`
    /// describe the *original* file (for display, e.g. "Mono · 44.1 kHz");
    /// `duration_frames` is the per-channel frame count of the imported
    /// (project-rate) WAV, i.e. what a placed clip would span.
    AssetImported {
        asset_id: AssetId,
        project_relative_path: String,
        original_path: String,
        format: AudioFormat,
        channels: u16,
        source_sample_rate: u32,
        duration_frames: u64,
        /// Downsampled waveform peaks: (min, max) per chunk of frames.
        peaks: Vec<(f32, f32)>,
    },
    /// A source file in an `ImportAudioToPool` batch failed to import
    /// (decode/transcode error, missing file, …). `reason` is
    /// user-facing. The batch continues with the remaining files.
    ImportFailed {
        asset_id: AssetId,
        path: String,
        reason: String,
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
    /// A clip's fade-in/out lengths and/or curves changed. Carries the
    /// engine-clamped values so the app mirror matches engine state.
    ClipFadeChanged {
        clip_id: ClipId,
        fade_in_frames: u64,
        fade_in_curve: FadeCurve,
        fade_out_frames: u64,
        fade_out_curve: FadeCurve,
    },
    /// A clip's per-clip gain changed. Carries the engine-clamped dB value.
    ClipGainChanged {
        clip_id: ClipId,
        gain_db: f32,
    },
    /// An automation lane was stored or replaced (or its read flag
    /// toggled). Carries the lane exactly as the engine holds it — points
    /// sorted, `enabled` reflecting the current read state — so the app
    /// mirror matches engine state.
    AutomationLaneChanged {
        lane: AutomationLane,
    },
    /// The automation lane for `target` was removed from engine state.
    AutomationLaneCleared {
        target: AutomationTarget,
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
    /// "Bounce in place" run was cancelled by the user via
    /// `AudioCommand::CancelBounce`. Distinct from `TrackBounceError`
    /// so the app can drop the modal without surfacing a noisy error
    /// banner. The engine guarantees the freshly-added target track is
    /// removed before this event fires, so the app only has to clear
    /// its in-progress state.
    TrackBounceCancelled {
        target_track_id: TrackId,
    },
    /// Periodic progress update for a "bounce in place" run, emitted
    /// from both the offline (`to_audio_clip`) and realtime
    /// (`poll_pending_bounce`) paths so the app can drive a progress
    /// bar. `fraction` is in `[0.0, 1.0]`.
    BounceProgress {
        fraction: f32,
    },
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

    // -- Vocal track events --
    VocalTrackAdded {
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
    // -- External-instrument tracks (doc #169, epic #39) --
    /// An external-instrument config was stored, replaced, or one of its
    /// fields changed (bank/program, latency offset). Carries the config
    /// exactly as the engine holds it so the app mirror matches engine state.
    ExternalInstrumentChanged {
        config: ExternalInstrument,
    },
    /// The external-instrument config for `track_id` was removed — the track
    /// is no longer an external instrument.
    ExternalInstrumentCleared {
        track_id: TrackId,
    },
    /// The external-instrument track's MIDI output device is offline: a patch
    /// send found no live connection, or a device re-check found it gone. The
    /// route is preserved (config untouched) so a replug reconnects. `device`
    /// is the configured MIDI output name, if any.
    ExternalInstrumentMidiOutOffline {
        track_id: TrackId,
        device: Option<String>,
    },
    /// The external-instrument track's audio-return input device is offline —
    /// a device re-check found it gone. The route is preserved so a replug
    /// reconnects. `device` is the configured return input name, if any.
    ExternalInstrumentReturnInputOffline {
        track_id: TrackId,
        device: Option<String>,
    },
    /// Result of `AudioCommand::DetectExternalInstrumentLatency`: the
    /// round-trip latency the engine measured for `track_id` by pinging the
    /// hardware (MIDI impulse out → audio return in). `latency_samples` is the
    /// measured round-trip at the engine sample rate; `latency_ms` is the same
    /// value in milliseconds for display. The engine has already applied this
    /// as the track's effective offset (the manual offset is the floor, so the
    /// applied value is `max(manual_offset, latency_samples)`) and the app
    /// mirror updates its displayed/applied offset to match.
    ExternalInstrumentLatencyMeasured {
        track_id: TrackId,
        latency_samples: i64,
        latency_ms: f32,
    },
    /// `AudioCommand::DetectExternalInstrumentLatency` could not measure a
    /// round-trip for `track_id` — the MIDI output was offline, the audio
    /// return delivered no/silent frames, or no impulse returned within the
    /// listen window. Nothing is changed (the existing offset stands); the app
    /// surfaces `reason` instead of leaving the user waiting on a hung ping.
    ExternalInstrumentLatencyDetectFailed {
        track_id: TrackId,
        reason: String,
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

    /// Snapshot of peak meters for VU display, sent in response to
    /// `AudioCommand::PollPeaks`. Replaces the older `read_and_clear_peaks`
    /// getter so the GUI never reaches into engine state directly.
    PeakSnapshot {
        track_peaks: Vec<(TrackId, f32, f32)>,
        bus_peaks: Vec<(BusId, f32, f32)>,
        master_peak_l: f32,
        master_peak_r: f32,
    },
}
