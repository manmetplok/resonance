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
///
/// Everything after `current_value` is the parameter's *meaning* rather
/// than its number (ba todo #1290, finding X8). A bare `2.0` in `0..=4`
/// tells a reader nothing about a filter type, and `0.71` with no unit
/// is not a Q; the plugin knows both and had no way to say so. These
/// fields carry what the CLAP params extension already exposes — its own
/// formatting of the value, the group it belongs to, whether it steps —
/// up to the app and out over the control API.
#[derive(Debug, Clone, Default)]
pub struct ParamInfo {
    pub id: u32,
    pub name: String,
    pub min_value: f64,
    pub max_value: f64,
    pub default_value: f64,
    pub current_value: f64,
    /// The plugin's own rendering of `current_value` — `"40 %"`,
    /// `"-6.0 dB"`, `"Low-pass"` — from CLAP `value_to_text`. Empty when
    /// the plugin offers no conversion, in which case a reader formats
    /// the number itself.
    ///
    /// It is a snapshot: it describes `current_value` and nothing else,
    /// so a writer that moves the value must refresh it (the engine
    /// echoes a fresh one on
    /// [`crate::types::AudioEvent::PluginParamText`]).
    pub text: String,
    /// Unit suffix alone (`"dB"`, `"%"`, `"Hz"`), recovered by stripping
    /// the number off `text` — CLAP has no unit field, so the display is
    /// the only place one exists. Empty when the parameter is unitless
    /// or its display is not a number (a choice name has no unit).
    pub unit: String,
    /// CLAP `IS_STEPPED`: the parameter moves in whole numbers (a
    /// choice, a count, a switch), so a writer sends `3`, never `2.7`.
    pub stepped: bool,
    /// Choice labels for a stepped parameter that names its values,
    /// indexed from `min_value` (`choices[0]` is the label at the
    /// minimum). Empty when the parameter is continuous, when its steps
    /// are plain numbers, or when there are too many to enumerate.
    pub choices: Vec<String>,
    /// CLAP's `module`: the parameter's group as a `/`-separated path
    /// (`"Multiband/Low"`), or empty for an ungrouped parameter. Lets a
    /// reader of a ~60-parameter plugin see which stage a parameter
    /// belongs to instead of one flat list (ba todo #1289).
    pub module: String,
    /// CLAP `IS_HIDDEN`: the plugin asks that this parameter not be
    /// shown. It is still automatable and still saved, so it stays in
    /// this list — readers that draw a parameter list skip it.
    pub hidden: bool,
}

/// A scanned plugin available for use, with its file path.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScannedPlugin {
    pub clap_file_path: String,
    pub clap_plugin_id: String,
    pub name: String,
    pub vendor: String,
    /// True if the plugin declared the `instrument` feature in its CLAP descriptor.
    pub is_instrument: bool,
    /// Factory presets baked into the plugin binary, as
    /// `(name, state json)`, read at scan time from the first-party
    /// `resonance_factory_presets` symbol. Empty for a plugin that ships
    /// none and for every plugin that is not one of ours (ba todo #1333).
    pub factory_presets: Vec<(String, String)>,
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
