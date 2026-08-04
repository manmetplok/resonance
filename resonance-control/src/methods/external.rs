//! `external.*` — external-instrument tracks: outboard synths driven over
//! MIDI whose audio comes back in on an input.
//!
//! An external-instrument track is a normal instrument track whose
//! "instrument" is hardware. It pairs two routes that have to be set up
//! separately:
//!
//! - **MIDI out** ([`SET_MIDI_OUT`]) — the port + channel the track's
//!   notes are played to.
//! - **Audio return** ([`SET_RETURN`]) — the input device + channel the
//!   synth's audio arrives on.
//!
//! Until both are set the track makes no sound at all: it isn't a
//! plugin, so nothing is rendered locally. [`STATUS`] reports which half
//! is missing, and [`DEVICES`] lists the names both setters accept.
//!
//! Simple mutations return [`crate::common::MutationAck`]; [`DEVICES`]
//! and [`STATUS`] are read-only.

use crate::ids::TrackId;
use serde::{Deserialize, Deserializer, Serialize};

/// Deserializer for the `Option<Option<T>>` "tri-state" setter fields.
///
/// Serde's default maps JSON `null` to the OUTER `None`, which collapses
/// "leave this alone" and "clear this" into the same value — so a
/// `{"bank": null}` meant to clear the bank would read as "field
/// omitted" and silently do nothing. Because serde only calls this when
/// the key is present, wrapping the inner parse in `Some` keeps the two
/// apart: absent -> `None`, `null` -> `Some(None)`, value ->
/// `Some(Some(v))`.
fn present_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// `external.enable` — put an existing track into external-instrument
/// mode ([`TrackParams`] -> `MutationAck`).
pub const ENABLE: &str = "external.enable";
/// `external.disable` — take a track out of external-instrument mode
/// ([`TrackParams`] -> `MutationAck`).
pub const DISABLE: &str = "external.disable";
/// `external.devices` — the MIDI outputs and audio inputs available on
/// this machine, read-only (no params -> [`DevicesView`]).
pub const DEVICES: &str = "external.devices";
/// `external.status` — full external-instrument config + live device
/// status per track, read-only ([`StatusParams`] -> [`StatusView`]).
pub const STATUS: &str = "external.status";
/// `external.set_midi_out` — pick the hardware MIDI output device and
/// channel ([`SetMidiOutParams`] -> `MutationAck`).
pub const SET_MIDI_OUT: &str = "external.set_midi_out";
/// `external.set_return` — pick the audio-return input device and port
/// ([`SetReturnParams`] -> `MutationAck`).
pub const SET_RETURN: &str = "external.set_return";
/// `external.set_patch` — select bank/program on the synth
/// ([`SetPatchParams`] -> `MutationAck`).
pub const SET_PATCH: &str = "external.set_patch";
/// `external.set_latency` — manual round-trip latency offset
/// ([`SetLatencyParams`] -> `MutationAck`).
pub const SET_LATENCY: &str = "external.set_latency";
/// `external.detect_latency` — measure the round-trip latency by firing
/// a MIDI impulse and timing the return ([`TrackParams`] ->
/// `MutationAck`). The measurement lands asynchronously; read it back
/// with [`STATUS`].
pub const DETECT_LATENCY: &str = "external.detect_latency";
/// `external.set_monitor` — hear (or stop hearing) the audio return
/// ([`SetMonitorParams`] -> `MutationAck`).
pub const SET_MONITOR: &str = "external.set_monitor";
/// `external.set_record_arm` — arm the track so a record pass captures
/// the return ([`SetRecordArmParams`] -> `MutationAck`).
pub const SET_RECORD_ARM: &str = "external.set_record_arm";
/// `external.set_playback_source` — play the hardware live, or play the
/// recorded takes ([`SetPlaybackSourceParams`] -> `MutationAck`).
pub const SET_PLAYBACK_SOURCE: &str = "external.set_playback_source";
/// `external.bounce` — capture the hardware to a new audio track in real
/// time ([`BounceParams`] -> `MutationAck`).
pub const BOUNCE: &str = "external.bounce";

/// All `external.*` method names.
pub const METHODS: &[&str] = &[
    ENABLE,
    DISABLE,
    DEVICES,
    STATUS,
    SET_MIDI_OUT,
    SET_RETURN,
    SET_PATCH,
    SET_LATENCY,
    DETECT_LATENCY,
    SET_MONITOR,
    SET_RECORD_ARM,
    SET_PLAYBACK_SOURCE,
    BOUNCE,
];

/// Params for the methods that only name a track (`external.enable`,
/// `external.disable`, `external.detect_latency`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TrackParams {
    pub track_id: TrackId,
}

/// Params for `external.status`; omit `track_id` for every external
/// track.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct StatusParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
}

/// What an external-instrument track plays back, lowercase on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PlaybackSource {
    /// Re-drive the hardware from the track's timeline MIDI on every
    /// pass. The synth has to be powered and connected to hear anything,
    /// and an offline render captures nothing.
    Live,
    /// Play the recorded takes over the spans they cover, falling back
    /// to live in the gaps. This is what makes an external part render
    /// into a mixdown.
    Recorded,
    /// Forward-compat catch-all for sources introduced by newer peers.
    #[serde(other)]
    Unknown,
}

/// Result of `external.devices`: what this machine currently offers.
/// Names are exactly the strings [`SET_MIDI_OUT`] / [`SET_RETURN`] take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DevicesView {
    /// Hardware MIDI output ports notes can be sent to.
    pub midi_outputs: Vec<String>,
    /// Audio input devices the synth's output can come back on.
    pub audio_inputs: Vec<AudioInputView>,
}

/// One audio input device and how many channels it exposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AudioInputView {
    pub name: String,
    /// Channel count; valid `port` values for `external.set_return` are
    /// `0..channels` (mono) or `0..channels-1` (stereo pair).
    pub channels: u16,
    /// True for the system's default capture device.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub default: bool,
}

/// Result of `external.status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct StatusView {
    pub tracks: Vec<ExternalTrackView>,
    pub revision: u64,
}

/// One external-instrument track's full configuration and live status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ExternalTrackView {
    pub track_id: TrackId,
    pub name: String,
    /// Derived lifecycle state: `"unconfigured"` (no MIDI out yet),
    /// `"configuring"` (no audio return, or not monitoring),
    /// `"live"` (fully wired and audible) or `"offline"` (a configured
    /// device disappeared — the route is kept, a replug restores it).
    pub status: String,
    /// Hardware MIDI output port the notes go to; `null` means the track
    /// drives nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi_out_device: Option<String>,
    /// 1-based MIDI channel (the app's own numbering), defaulting to 1.
    #[serde(default)]
    pub midi_out_channel: u8,
    /// Audio input device the synth returns on; `null` means no return
    /// is wired, so nothing can be heard or recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_device: Option<String>,
    /// 0-based first input channel of the return.
    #[serde(default)]
    pub return_port: u16,
    /// Selected bank as a combined 14-bit value (`MSB << 7 | LSB`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bank: Option<u16>,
    /// Selected program (`0..=127`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<u8>,
    /// Manual latency offset in samples aligning the return with the
    /// timeline. Positive delays the return.
    #[serde(default)]
    pub latency_offset_samples: i64,
    /// True while an `external.detect_latency` measurement is running.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub latency_detect_in_progress: bool,
    /// Why the last latency measurement failed, if it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_detect_error: Option<String>,
    /// True when the return is being monitored — i.e. audible.
    #[serde(default)]
    pub monitor_enabled: bool,
    /// True when a record pass would capture the return to the timeline.
    #[serde(default)]
    pub record_armed: bool,
    pub playback_source: PlaybackSource,
    /// Number of recorded takes (audio clips) on the track. With
    /// `playback_source: "recorded"` these are what plays back and what
    /// renders into a mixdown.
    #[serde(default)]
    pub take_count: usize,
    /// True when the configured MIDI output has gone offline.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub midi_out_offline: bool,
    /// True when the configured audio-return input has gone offline.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub return_input_offline: bool,
}

/// Params for `external.set_midi_out`. Both fields are optional; an
/// omitted field is left as it is, an explicit `null` device disconnects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetMidiOutParams {
    pub track_id: TrackId,
    /// A name from `external.devices`' `midi_outputs`. Explicit `null`
    /// disconnects the track from its synth.
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub device: Option<Option<String>>,
    /// 1-based MIDI channel, `1..=16`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<u8>,
}

/// Params for `external.set_return`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetReturnParams {
    pub track_id: TrackId,
    /// A name from `external.devices`' `audio_inputs`. Explicit `null`
    /// clears the return.
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub device: Option<Option<String>>,
    /// 0-based first input channel on that device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

/// Params for `external.set_patch`. An omitted field is left alone; an
/// explicit `null` clears that half of the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetPatchParams {
    pub track_id: TrackId,
    /// Combined 14-bit bank (`MSB << 7 | LSB`), `0..=16383`.
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub bank: Option<Option<u16>>,
    /// Program number, `0..=127`.
    #[serde(
        default,
        deserialize_with = "present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub program: Option<Option<u8>>,
}

/// Params for `external.set_latency`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetLatencyParams {
    pub track_id: TrackId,
    /// Samples to delay the return by so it lines up with the timeline.
    /// Negative pulls it earlier.
    pub offset_samples: i64,
}

/// Params for `external.set_monitor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetMonitorParams {
    pub track_id: TrackId,
    pub enabled: bool,
}

/// Params for `external.set_record_arm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetRecordArmParams {
    pub track_id: TrackId,
    pub armed: bool,
}

/// Params for `external.set_playback_source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetPlaybackSourceParams {
    pub track_id: TrackId,
    pub source: PlaybackSource,
}

/// Params for `external.bounce`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct BounceParams {
    pub track_id: TrackId,
    /// Audio input to capture from. Defaults to the track's configured
    /// audio return, which is almost always what you want.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// 0-based first input channel. Defaults to the track's return port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Capture one channel duplicated to both sides instead of a stereo
    /// pair. Defaults to the track's own mono setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mono: Option<bool>,
}
