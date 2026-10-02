//! `transport.*` — playback and global musical parameters.
//!
//! `play`/`stop`/`pause`/`loop_toggle` take no params (`()`). All
//! transport methods return [`TransportResult`]; tempo/time-signature/key
//! edits go through the normal undoable update path.

use crate::common::{PositionSpec, SongPosition, TransportState};
use serde::{Deserialize, Serialize};

/// `transport.play` — start playback at the playhead. No params.
pub const PLAY: &str = "transport.play";
/// `transport.stop` — stop and return to the stop position. No params.
pub const STOP: &str = "transport.stop";
/// `transport.pause` — pause at the current position. No params.
pub const PAUSE: &str = "transport.pause";
/// `transport.seek` — move the playhead ([`SeekParams`]).
pub const SEEK: &str = "transport.seek";
/// `transport.loop_set` — set the loop region ([`LoopSetParams`]).
pub const LOOP_SET: &str = "transport.loop_set";
/// `transport.loop_toggle` — toggle looping. No params.
pub const LOOP_TOGGLE: &str = "transport.loop_toggle";
/// `transport.set_tempo` — set the song tempo ([`SetTempoParams`]).
///
/// The arrangement keeps its MUSICAL positions: every clip start,
/// automation breakpoint, marker, the loop range and the playhead are
/// re-anchored so a clip at bar 9 is still at bar 9 afterwards. Audio
/// clip *durations* are not rescaled — a recorded take is real time (use
/// `clip.set_stretch` for that) — so its start moves while its length
/// stays. Before this, lengths followed the tempo map and starts did not,
/// which slid a clip at bar 9 to bar 10.33 (ba doc #275 P1.4).
pub const SET_TEMPO: &str = "transport.set_tempo";
/// `transport.set_time_signature` — set the time signature
/// ([`SetTimeSignatureParams`]).
pub const SET_TIME_SIGNATURE: &str = "transport.set_time_signature";
/// `transport.set_key` — set the global key where the app exposes one
/// ([`SetKeyParams`]); otherwise key lives per-section (`section.set_scale`).
pub const SET_KEY: &str = "transport.set_key";

/// All `transport.*` method names.
pub const METHODS: &[&str] = &[
    PLAY,
    STOP,
    PAUSE,
    SEEK,
    LOOP_SET,
    LOOP_TOGGLE,
    SET_TEMPO,
    SET_TIME_SIGNATURE,
    SET_KEY,
];

/// Params for `transport.seek`: a musical (`bar` [+ `beat`]) or `sample`
/// position, flattened into the params object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SeekParams {
    #[serde(flatten)]
    pub position: PositionSpec,
}

/// Params for `transport.loop_set`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct LoopSetParams {
    pub start: PositionSpec,
    pub end: PositionSpec,
    /// Also enable/disable looping; omitted leaves the toggle unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// Params for `transport.set_tempo`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SetTempoParams {
    pub bpm: f64,
}

/// Params for `transport.set_time_signature`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SetTimeSignatureParams {
    pub numerator: u8,
    pub denominator: u8,
}

/// Params for `transport.set_key`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SetKeyParams {
    /// Pitch name, e.g. `"A"`, `"F#"`.
    pub tonic: String,
    /// Lowercase scale name, e.g. `"minor"`.
    pub scale: String,
}

/// Result of every `transport.*` method: the transport after the call.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TransportResult {
    pub state: TransportState,
    pub playhead: SongPosition,
    pub looping: bool,
    pub revision: u64,
}
