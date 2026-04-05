/// Core types for the Resonance audio engine.

pub type TrackId = u64;
pub type ClipId = u64;
pub type SamplePos = u64;
pub type PluginInstanceId = u64;

/// Commands sent from the GUI to the audio engine.
#[derive(Debug, Clone)]
pub enum AudioCommand {
    Play,
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
    DeleteClip {
        clip_id: ClipId,
    },
    SetTrackVolume {
        track_id: TrackId,
        volume: f32,
    },
    SetTrackMute {
        track_id: TrackId,
        muted: bool,
    },
    AddTrack,
    RemoveTrack {
        track_id: TrackId,
    },
    SetTrackRecordArm {
        track_id: TrackId,
        armed: bool,
    },
    SetTrackMonitor {
        track_id: TrackId,
        enabled: bool,
    },
    SetTrackInputDevice {
        track_id: TrackId,
        device_name: Option<String>,
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
    },
    PluginAdded {
        track_id: TrackId,
        instance_id: PluginInstanceId,
        plugin_name: String,
        params: Vec<ParamInfo>,
    },
    PluginRemoved {
        track_id: TrackId,
        instance_id: PluginInstanceId,
    },
    PluginsScanned {
        plugins: Vec<ScannedPlugin>,
    },
}

/// An audio clip stored in memory.
#[derive(Debug, Clone)]
pub struct AudioClip {
    pub id: ClipId,
    pub track_id: TrackId,
    /// Start position on the timeline in samples.
    pub start_sample: SamplePos,
    /// Decoded audio data: stereo interleaved f32 samples.
    pub data: Vec<f32>,
    /// Original file name.
    pub name: String,
}

impl AudioClip {
    /// Duration in stereo sample frames.
    pub fn duration_frames(&self) -> u64 {
        (self.data.len() / 2) as u64
    }

    /// End position on timeline in sample frames.
    pub fn end_sample(&self) -> SamplePos {
        self.start_sample + self.duration_frames()
    }
}

/// A track containing audio clips.
#[derive(Debug, Clone)]
pub struct Track {
    pub id: TrackId,
    pub volume: f32,
    pub muted: bool,
    pub name: String,
    pub record_armed: bool,
    pub monitor_enabled: bool,
    pub input_device_name: Option<String>,
    /// Ordered list of plugin instance IDs forming the insert chain.
    pub plugin_ids: Vec<PluginInstanceId>,
}

impl Track {
    pub fn new(id: TrackId, name: String) -> Self {
        Self {
            id,
            volume: 1.0,
            muted: false,
            name,
            record_armed: false,
            monitor_enabled: false,
            input_device_name: None,
            plugin_ids: Vec::new(),
        }
    }
}

/// Describes an available audio input source (PipeWire/PulseAudio source).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceInfo {
    /// PipeWire source name (e.g. "alsa_input.usb-...").
    pub name: String,
    /// Human-readable description (e.g. "USB Microphone Analog Stereo").
    pub description: String,
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
}
