//! `mixer.*` — per-track mix parameters. All return
//! [`crate::common::MutationAck`].

use crate::ids::TrackId;
use serde::{Deserialize, Serialize};

/// `mixer.set_volume` — set fader gain ([`SetVolumeParams`]).
pub const SET_VOLUME: &str = "mixer.set_volume";
/// `mixer.set_pan` — set stereo pan ([`SetPanParams`]).
pub const SET_PAN: &str = "mixer.set_pan";
/// `mixer.set_mute` — mute/unmute ([`SetMuteParams`]).
pub const SET_MUTE: &str = "mixer.set_mute";
/// `mixer.set_solo` — solo/unsolo ([`SetSoloParams`]).
pub const SET_SOLO: &str = "mixer.set_solo";

/// All `mixer.*` method names.
pub const METHODS: &[&str] = &[SET_VOLUME, SET_PAN, SET_MUTE, SET_SOLO];

/// Params for `mixer.set_volume`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetVolumeParams {
    pub track_id: TrackId,
    /// Linear fader gain (1.0 = unity, 0.0 = silence).
    pub volume: f32,
}

/// Params for `mixer.set_pan`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetPanParams {
    pub track_id: TrackId,
    /// Stereo pan in `-1.0..=1.0` (0 = center).
    pub pan: f32,
}

/// Params for `mixer.set_mute`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetMuteParams {
    pub track_id: TrackId,
    pub muted: bool,
}

/// Params for `mixer.set_solo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetSoloParams {
    pub track_id: TrackId,
    pub soloed: bool,
}
