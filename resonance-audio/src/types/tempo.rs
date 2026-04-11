//! Tempo map and plugin/device info types.

/// Ticks per quarter note for MIDI timing (standard PPQ).
pub const TICKS_PER_QUARTER_NOTE: u64 = 480;

/// Describes an available audio input source (PipeWire/PulseAudio source).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceInfo {
    /// PipeWire source name (e.g. "alsa_input.usb-...").
    pub name: String,
    /// Human-readable description (e.g. "USB Microphone Analog Stereo").
    pub description: String,
    /// Number of input channels exposed by this device. 0 means the
    /// channel count couldn't be determined at enumeration time.
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
