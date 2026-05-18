//! Tempo map and plugin/device info types.
//!
//! Split into submodules by responsibility:
//!
//! - [`map`]: the `TempoMap` struct, bar table construction, BPM lookup.
//! - [`conversion`]: pure beat ↔ sample ↔ tick conversion helpers.
//! - [`bars`]: bar / beat / subdivision math on `TempoMap`.
//! - [`format`]: `Display` impls and human-readable formatting helpers.
//!
//! The public API is re-exported here so callers can keep importing
//! from `crate::types::tempo::*` without caring about the split.

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

mod bars;
mod conversion;
mod format;
mod map;

pub use conversion::{
    arrival_bpm_at_bar, avg_bpm_for_bar, bpm_at_bar, sample_frac_to_tick_frac,
    tick_frac_to_sample_frac,
};
pub use map::{SignaturePoint, TempoMap, TempoPoint};
