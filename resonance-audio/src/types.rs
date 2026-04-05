/// Core types for the Resonance audio engine.

pub type TrackId = u64;
pub type ClipId = u64;
pub type SamplePos = u64;

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
    SetTrackInputDevice {
        track_id: TrackId,
        device_name: Option<String>,
    },
    ListInputDevices,
}

/// Events sent from the audio engine back to the GUI.
#[derive(Debug, Clone)]
pub enum AudioEvent {
    PlayheadMoved(SamplePos),
    ClipImported {
        clip_id: ClipId,
        track_id: TrackId,
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
    RecordingStarted,
    RecordingFinished {
        clip_id: ClipId,
        track_id: TrackId,
        start_sample: SamplePos,
        duration_samples: u64,
        name: String,
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
    pub input_device_name: Option<String>,
}

impl Track {
    pub fn new(id: TrackId, name: String) -> Self {
        Self {
            id,
            volume: 1.0,
            muted: false,
            name,
            record_armed: false,
            input_device_name: None,
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
