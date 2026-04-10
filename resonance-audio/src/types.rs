/// Core types for the Resonance audio engine.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

pub type TrackId = u64;
pub type ClipId = u64;
pub type SamplePos = u64;
pub type PluginInstanceId = u64;
pub type BusId = u64;

/// Where a track's post-fader audio lands. Tracks either sum directly
/// into the master output (the default, matching pre-bus behaviour) or
/// route into a named bus for group processing before reaching master.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackOutput {
    Master,
    Bus(BusId),
}

/// Sentinel value used in `Track::output_bus_bits` to encode
/// `TrackOutput::Master` (so the enum can live in a single AtomicU64
/// for lock-free reads on the audio thread).
const TRACK_OUTPUT_MASTER: u64 = u64::MAX;

/// Ticks per quarter note for MIDI timing (standard PPQ).
pub const TICKS_PER_QUARTER_NOTE: u64 = 480;

/// Distinguishes audio recording/playback tracks from instrument (MIDI) tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackType {
    Audio,
    Instrument,
}

/// A single MIDI note in a clip.
#[derive(Debug, Clone)]
pub struct MidiNote {
    pub note: u8,
    pub velocity: f32,
    pub start_tick: u64,
    pub duration_ticks: u64,
}

/// A MIDI clip containing note data, placed on the timeline.
#[derive(Debug)]
pub struct MidiClip {
    pub id: ClipId,
    pub track_id: TrackId,
    /// Position on the timeline in samples (same units as AudioClip).
    pub start_sample: SamplePos,
    /// Logical length in ticks.
    pub duration_ticks: u64,
    /// Notes sorted by start_tick.
    pub notes: Vec<MidiNote>,
    pub name: String,
    pub trim_start_ticks: u64,
    pub trim_end_ticks: u64,
}

impl MidiClip {
    /// Visible duration in ticks after trim.
    pub fn visible_duration_ticks(&self) -> u64 {
        self.duration_ticks
            .saturating_sub(self.trim_start_ticks)
            .saturating_sub(self.trim_end_ticks)
    }

    /// Convert visible duration to samples using the tempo map.
    pub fn duration_samples(&self, samples_per_tick: f64) -> u64 {
        (self.visible_duration_ticks() as f64 * samples_per_tick) as u64
    }

    /// End position on timeline in samples.
    pub fn end_sample(&self, samples_per_tick: f64) -> SamplePos {
        self.start_sample + self.duration_samples(samples_per_tick)
    }
}

/// A note event to be sent to a plugin during audio processing.
#[derive(Debug, Clone)]
pub struct PendingNoteEvent {
    pub is_note_on: bool,
    pub note: u8,
    pub velocity: f32,
    pub sample_offset: u32,
}

/// Commands sent from the GUI to the audio engine.
#[derive(Debug, Clone)]
pub enum AudioCommand {
    Play,
    Record,
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
    AddTrack,
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
    /// Set punch in/out range. When enabled, recording is trimmed to [punch_in, punch_out].
    /// If punch_out <= punch_in, no clip is produced.
    SetPunchRange {
        enabled: bool,
        punch_in: SamplePos,
        punch_out: SamplePos,
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
    /// Add a track with a specific ID (for project load).
    AddTrackWithId {
        track_id: TrackId,
        name: String,
    },
    /// Add a plugin with a specific instance ID (for project load).
    AddPluginWithId {
        track_id: TrackId,
        instance_id: PluginInstanceId,
        clap_file_path: String,
        clap_plugin_id: String,
    },
    /// Load a pre-decoded audio clip directly into the engine (for project load).
    LoadClipDirect {
        clip_id: ClipId,
        track_id: TrackId,
        start_sample: SamplePos,
        data: Vec<f32>,
        name: String,
        trim_start_frames: u64,
        trim_end_frames: u64,
    },
    /// Request all clip audio data for project save.
    ExportAllClipData,
    /// Batch save all plugin states for project save.
    SaveAllPluginStates,
    /// Remove all tracks, clips, and plugins (for project load).
    ClearAll,

    // -- Instrument track commands --
    AddInstrumentTrack,
    AddInstrumentTrackWithId {
        track_id: TrackId,
        name: String,
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
    AddBus,
    /// Add a bus with a specific ID (for project load).
    AddBusWithId {
        bus_id: BusId,
        name: String,
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
    },
    /// Add a plugin to a bus with a specific instance ID (for project load).
    AddPluginToBusWithId {
        bus_id: BusId,
        instance_id: PluginInstanceId,
        clap_file_path: String,
        clap_plugin_id: String,
    },
    RemovePluginFromBus {
        bus_id: BusId,
        instance_id: PluginInstanceId,
    },
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
        /// `output_port_count`. Used to name auto-created sub-tracks after
        /// their source port.
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
    /// Exported clip audio data for project save.
    ClipDataExported {
        clip_id: ClipId,
        data: Vec<f32>,
    },
    /// All clip data has been exported.
    AllClipDataExported,
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
}

/// An audio clip stored in memory.
#[derive(Debug)]
pub struct AudioClip {
    pub id: ClipId,
    pub track_id: TrackId,
    /// Start position on the timeline in samples.
    pub start_sample: SamplePos,
    /// Decoded audio data: stereo interleaved f32 samples.
    pub data: Vec<f32>,
    /// Original file name.
    pub name: String,
    /// Non-destructive trim: frames to skip from the start of audio data.
    pub trim_start_frames: u64,
    /// Non-destructive trim: frames to skip from the end of audio data.
    pub trim_end_frames: u64,
}

/// Number of stereo frames per waveform peak bucket.
pub const WAVEFORM_PEAK_FRAMES: usize = 512;

/// Compute downsampled waveform peaks from stereo interleaved audio data.
/// Returns (min, max) pairs, one per chunk of `WAVEFORM_PEAK_FRAMES` frames.
/// Uses the mono mix (L+R)/2 for display.
pub fn compute_waveform_peaks(data: &[f32]) -> Vec<(f32, f32)> {
    let total_frames = data.len() / 2;
    let num_peaks = (total_frames + WAVEFORM_PEAK_FRAMES - 1) / WAVEFORM_PEAK_FRAMES;
    let mut peaks = Vec::with_capacity(num_peaks);
    for chunk_start in (0..total_frames).step_by(WAVEFORM_PEAK_FRAMES) {
        let chunk_end = (chunk_start + WAVEFORM_PEAK_FRAMES).min(total_frames);
        let mut min_val = f32::MAX;
        let mut max_val = f32::MIN;
        for f in chunk_start..chunk_end {
            let mono = (data[f * 2] + data[f * 2 + 1]) * 0.5;
            if mono < min_val {
                min_val = mono;
            }
            if mono > max_val {
                max_val = mono;
            }
        }
        peaks.push((min_val, max_val));
    }
    peaks
}

impl AudioClip {
    /// Total number of frames in the raw audio data.
    pub fn total_frames(&self) -> u64 {
        (self.data.len() / 2) as u64
    }

    /// Visible/audible duration in stereo sample frames (after trim).
    pub fn duration_frames(&self) -> u64 {
        self.total_frames()
            .saturating_sub(self.trim_start_frames)
            .saturating_sub(self.trim_end_frames)
    }

    /// End position on timeline in sample frames.
    pub fn end_sample(&self) -> SamplePos {
        self.start_sample + self.duration_frames()
    }
}

/// A track containing audio clips or MIDI clips.
///
/// Hot-path fields (volume, muted, monitor_enabled, record_armed) are atomic
/// so the audio callback can read them without taking a write lock.
#[derive(Debug)]
pub struct Track {
    pub id: TrackId,
    pub track_type: TrackType,
    volume_bits: AtomicU32,
    pan_bits: AtomicU32,
    muted: AtomicBool,
    soloed: AtomicBool,
    pub name: String,
    record_armed: AtomicBool,
    monitor_enabled: AtomicBool,
    /// If true, track captures a single input channel (duplicated to both L/R).
    /// If false, track captures a stereo pair.
    mono: AtomicBool,
    /// Post-fader peak level for left channel (for VU meters).
    peak_l_bits: AtomicU32,
    /// Post-fader peak level for right channel (for VU meters).
    peak_r_bits: AtomicU32,
    /// Output destination, encoded as `u64::MAX` for `Master` or a bus id.
    /// Stored as an atomic so the audio thread can read the routing
    /// without taking a write lock while the UI edits it.
    output_bus_bits: AtomicU64,
    pub input_device_name: Option<String>,
    /// 0-indexed starting input channel on the track's input device. For
    /// mono tracks this is the single channel captured and duplicated to
    /// L/R; for stereo tracks it's the L channel and `port_index + 1` is
    /// used as R. Defaults to 0 (first channel pair).
    input_port_bits: AtomicU32,
    /// Ordered list of plugin instance IDs forming the insert chain.
    /// For instrument tracks, the first plugin is the instrument; the rest are effects.
    pub plugin_ids: Vec<PluginInstanceId>,
    /// When set, this track is a sub-track fed by a non-main output port
    /// of `parent_track_id`'s instrument plugin. Sub-tracks never run
    /// their own plugin chain or receive MIDI events — the mixer drives
    /// them entirely from the parent plugin's `process_multi` output.
    /// The tuple is `(parent_track_id, output_port_index)` where index 0
    /// is reserved for the parent's own main output.
    pub sub_track_of: Option<(TrackId, u32)>,
}

impl Track {
    pub fn new(id: TrackId, name: String) -> Self {
        Self::with_type(id, name, TrackType::Audio)
    }

    pub fn with_type(id: TrackId, name: String, track_type: TrackType) -> Self {
        Self {
            id,
            track_type,
            volume_bits: AtomicU32::new(1.0f32.to_bits()),
            pan_bits: AtomicU32::new(0.0f32.to_bits()),
            muted: AtomicBool::new(false),
            soloed: AtomicBool::new(false),
            name,
            record_armed: AtomicBool::new(false),
            monitor_enabled: AtomicBool::new(false),
            mono: AtomicBool::new(true),
            peak_l_bits: AtomicU32::new(0),
            peak_r_bits: AtomicU32::new(0),
            output_bus_bits: AtomicU64::new(TRACK_OUTPUT_MASTER),
            input_device_name: None,
            input_port_bits: AtomicU32::new(0),
            plugin_ids: Vec::new(),
            sub_track_of: None,
        }
    }

    /// The track's 0-indexed starting input channel.
    pub fn input_port(&self) -> u16 {
        (self.input_port_bits.load(Ordering::Relaxed) & 0xFFFF) as u16
    }

    pub fn set_input_port(&self, port: u16) {
        self.input_port_bits
            .store(port as u32, Ordering::Relaxed);
    }

    /// Construct a sub-track feeding from `parent_track_id`'s output port
    /// index `output_port_index`. Starts muted-friendly (volume 1.0,
    /// pan 0.0) and routed to master; the app layer pushes user edits
    /// via the normal `SetTrackVolume` / `SetTrackOutput` / etc. commands.
    pub fn new_sub_track(
        id: TrackId,
        name: String,
        parent_track_id: TrackId,
        output_port_index: u32,
    ) -> Self {
        let mut t = Self::with_type(id, name, TrackType::Instrument);
        t.sub_track_of = Some((parent_track_id, output_port_index));
        t
    }

    pub fn output(&self) -> TrackOutput {
        match self.output_bus_bits.load(Ordering::Relaxed) {
            TRACK_OUTPUT_MASTER => TrackOutput::Master,
            bus_id => TrackOutput::Bus(bus_id),
        }
    }

    pub fn set_output(&self, output: TrackOutput) {
        let encoded = match output {
            TrackOutput::Master => TRACK_OUTPUT_MASTER,
            TrackOutput::Bus(id) => id,
        };
        self.output_bus_bits.store(encoded, Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }

    pub fn set_volume(&self, v: f32) {
        self.volume_bits.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn pan(&self) -> f32 {
        f32::from_bits(self.pan_bits.load(Ordering::Relaxed))
    }

    pub fn set_pan(&self, v: f32) {
        self.pan_bits.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn set_muted(&self, v: bool) {
        self.muted.store(v, Ordering::Relaxed);
    }

    pub fn soloed(&self) -> bool {
        self.soloed.load(Ordering::Relaxed)
    }

    pub fn set_soloed(&self, v: bool) {
        self.soloed.store(v, Ordering::Relaxed);
    }

    pub fn record_armed(&self) -> bool {
        self.record_armed.load(Ordering::Relaxed)
    }

    pub fn set_record_armed(&self, v: bool) {
        self.record_armed.store(v, Ordering::Relaxed);
    }

    pub fn monitor_enabled(&self) -> bool {
        self.monitor_enabled.load(Ordering::Relaxed)
    }

    pub fn set_monitor_enabled(&self, v: bool) {
        self.monitor_enabled.store(v, Ordering::Relaxed);
    }

    pub fn mono(&self) -> bool {
        self.mono.load(Ordering::Relaxed)
    }

    pub fn set_mono(&self, v: bool) {
        self.mono.store(v, Ordering::Relaxed);
    }

    /// Atomically update peak L to the max of the current and new value.
    pub fn update_peak_l(&self, v: f32) {
        self.peak_l_bits.fetch_max(v.to_bits(), Ordering::Relaxed);
    }

    /// Atomically update peak R to the max of the current and new value.
    pub fn update_peak_r(&self, v: f32) {
        self.peak_r_bits.fetch_max(v.to_bits(), Ordering::Relaxed);
    }

    /// Read and clear peak L, returning the peak since last call.
    pub fn swap_peak_l(&self) -> f32 {
        f32::from_bits(self.peak_l_bits.swap(0, Ordering::Relaxed))
    }

    /// Read and clear peak R, returning the peak since last call.
    pub fn swap_peak_r(&self) -> f32 {
        f32::from_bits(self.peak_r_bits.swap(0, Ordering::Relaxed))
    }
}

/// An audio bus: an intermediate summing point with its own plugin
/// chain, fader, pan, mute, and meters. Busses live between tracks and
/// master — tracks can route their post-fader audio to a bus, the bus
/// processes the sum through its plugin chain, then the bus sums into
/// master.
///
/// Hot-path fields mirror `Track` (atomic volume/pan/muted/peaks) so
/// the audio thread can read them without a write lock.
#[derive(Debug)]
pub struct Bus {
    pub id: BusId,
    volume_bits: AtomicU32,
    pan_bits: AtomicU32,
    muted: AtomicBool,
    pub name: String,
    peak_l_bits: AtomicU32,
    peak_r_bits: AtomicU32,
    /// Ordered list of plugin instance IDs forming the insert chain.
    pub plugin_ids: Vec<PluginInstanceId>,
}

impl Bus {
    pub fn new(id: BusId, name: String) -> Self {
        Self {
            id,
            volume_bits: AtomicU32::new(1.0f32.to_bits()),
            pan_bits: AtomicU32::new(0.0f32.to_bits()),
            muted: AtomicBool::new(false),
            name,
            peak_l_bits: AtomicU32::new(0),
            peak_r_bits: AtomicU32::new(0),
            plugin_ids: Vec::new(),
        }
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }

    pub fn set_volume(&self, v: f32) {
        self.volume_bits.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn pan(&self) -> f32 {
        f32::from_bits(self.pan_bits.load(Ordering::Relaxed))
    }

    pub fn set_pan(&self, v: f32) {
        self.pan_bits.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn set_muted(&self, v: bool) {
        self.muted.store(v, Ordering::Relaxed);
    }

    pub fn update_peak_l(&self, v: f32) {
        self.peak_l_bits.fetch_max(v.to_bits(), Ordering::Relaxed);
    }

    pub fn update_peak_r(&self, v: f32) {
        self.peak_r_bits.fetch_max(v.to_bits(), Ordering::Relaxed);
    }

    pub fn swap_peak_l(&self) -> f32 {
        f32::from_bits(self.peak_l_bits.swap(0, Ordering::Relaxed))
    }

    pub fn swap_peak_r(&self) -> f32 {
        f32::from_bits(self.peak_r_bits.swap(0, Ordering::Relaxed))
    }
}

/// Describes an available audio input source (PipeWire/PulseAudio source).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceInfo {
    /// PipeWire source name (e.g. "alsa_input.usb-...").
    pub name: String,
    /// Human-readable description (e.g. "USB Microphone Analog Stereo").
    pub description: String,
    /// Number of input channels exposed by this device. 0 means the
    /// channel count couldn't be determined at enumeration time; the UI
    /// should fall back to a sensible default (e.g. the count reported
    /// once the stream opens).
    pub channels: u16,
}

impl std::fmt::Display for InputDeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.description)
    }
}

/// Describes a plugin available in a .clap bundle (used during loading).
#[derive(Debug, Clone)]
pub struct PluginDescInfo {
    pub id: String,
    pub name: String,
    pub vendor: String,
    /// True if the plugin declared the `instrument` feature in its CLAP descriptor.
    pub is_instrument: bool,
}

/// A plugin parameter descriptor with current value.
#[derive(Debug, Clone)]
pub struct ParamInfo {
    pub id: u32,
    pub name: String,
    pub min_value: f64,
    pub max_value: f64,
    pub default_value: f64,
    pub current_value: f64,
}

/// A scanned plugin available for use, with its file path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedPlugin {
    pub clap_file_path: String,
    pub clap_plugin_id: String,
    pub name: String,
    pub vendor: String,
    /// True if the plugin declared the `instrument` feature in its CLAP descriptor.
    pub is_instrument: bool,
}

impl std::fmt::Display for ScannedPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.vendor.is_empty() {
            write!(f, "{}", self.name)
        } else {
            write!(f, "{} ({})", self.name, self.vendor)
        }
    }
}

/// Tempo and time signature state.
#[derive(Debug, Clone)]
pub struct TempoMap {
    pub bpm: f32,
    pub numerator: u8,
    pub denominator: u8,
    pub metronome_enabled: bool,
}

impl Default for TempoMap {
    fn default() -> Self {
        Self {
            bpm: 120.0,
            numerator: 4,
            denominator: 4,
            metronome_enabled: false,
        }
    }
}

impl TempoMap {
    /// Samples per beat at the given sample rate.
    pub fn samples_per_beat(&self, sample_rate: u32) -> f64 {
        sample_rate as f64 * 60.0 / self.bpm as f64
    }

    /// Samples per bar at the given sample rate.
    pub fn samples_per_bar(&self, sample_rate: u32) -> f64 {
        self.samples_per_beat(sample_rate) * self.numerator as f64
    }

    /// Convert a sample position to (bar, beat, fractional_beat).
    /// Bar and beat are 1-based.
    pub fn position_to_bars(&self, sample_pos: u64, sample_rate: u32) -> (u32, u8, f64) {
        let spb = self.samples_per_beat(sample_rate);
        let total_beats = sample_pos as f64 / spb;
        let bar = (total_beats / self.numerator as f64).floor() as u32 + 1;
        let beat_in_bar = (total_beats % self.numerator as f64).floor() as u8 + 1;
        let frac = total_beats.fract();
        (bar, beat_in_bar, frac)
    }

    /// Format a sample position as "bar.beat".
    pub fn format_position(&self, sample_pos: u64, sample_rate: u32) -> String {
        let (bar, beat, _) = self.position_to_bars(sample_pos, sample_rate);
        format!("{}.{}", bar, beat)
    }

    /// Format a sample position as "mm:ss.mmm" wall-clock time.
    pub fn format_time(&self, sample_pos: u64, sample_rate: u32) -> String {
        let total_secs = sample_pos as f64 / sample_rate as f64;
        let minutes = (total_secs / 60.0).floor() as u32;
        let seconds = total_secs - (minutes as f64 * 60.0);
        format!("{:02}:{:06.3}", minutes, seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_output_defaults_to_master() {
        let track = Track::new(1, "T1".to_string());
        assert_eq!(track.output(), TrackOutput::Master);
    }

    #[test]
    fn track_output_roundtrip_master() {
        let track = Track::new(1, "T1".to_string());
        track.set_output(TrackOutput::Master);
        assert_eq!(track.output(), TrackOutput::Master);
    }

    #[test]
    fn track_output_roundtrip_bus() {
        let track = Track::new(1, "T1".to_string());
        track.set_output(TrackOutput::Bus(42));
        assert_eq!(track.output(), TrackOutput::Bus(42));
    }

    #[test]
    fn track_output_roundtrip_various_bus_ids() {
        let track = Track::new(1, "T1".to_string());
        for id in [1u64, 7, 100, 1_000_000, u64::MAX - 1] {
            track.set_output(TrackOutput::Bus(id));
            assert_eq!(track.output(), TrackOutput::Bus(id));
        }
    }

    #[test]
    fn track_output_master_sentinel_is_u64_max() {
        // The sentinel chosen for Master is u64::MAX. Bus id u64::MAX is
        // reserved and intentionally indistinguishable from Master; the
        // engine's next_bus_id starts at 1 and grows, so this is safe in
        // practice but worth pinning in a test.
        let track = Track::new(1, "T1".to_string());
        track.set_output(TrackOutput::Master);
        assert_eq!(track.output(), TrackOutput::Master);
        // Flip from Master to a bus and back.
        track.set_output(TrackOutput::Bus(5));
        assert_eq!(track.output(), TrackOutput::Bus(5));
        track.set_output(TrackOutput::Master);
        assert_eq!(track.output(), TrackOutput::Master);
    }

    #[test]
    fn bus_atomic_accessors_roundtrip() {
        let bus = Bus::new(1, "Bus 1".to_string());

        // Defaults.
        assert_eq!(bus.volume(), 1.0);
        assert_eq!(bus.pan(), 0.0);
        assert!(!bus.muted());

        bus.set_volume(0.5);
        assert_eq!(bus.volume(), 0.5);

        bus.set_pan(-0.75);
        assert_eq!(bus.pan(), -0.75);

        bus.set_muted(true);
        assert!(bus.muted());
    }

    #[test]
    fn bus_peak_update_and_swap() {
        let bus = Bus::new(1, "Bus 1".to_string());

        bus.update_peak_l(0.3);
        bus.update_peak_l(0.5);
        bus.update_peak_l(0.2); // Should not decrease the stored max.
        bus.update_peak_r(0.8);

        // swap returns the stored max and resets to 0.
        assert_eq!(bus.swap_peak_l(), 0.5);
        assert_eq!(bus.swap_peak_r(), 0.8);
        assert_eq!(bus.swap_peak_l(), 0.0);
        assert_eq!(bus.swap_peak_r(), 0.0);
    }
}
