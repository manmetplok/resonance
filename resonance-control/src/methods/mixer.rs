//! `mixer.*` — per-track mix parameters. All return
//! [`crate::common::MutationAck`].

use crate::ids::TrackId;
use serde::{Deserialize, Serialize};

/// `mixer.set_volume` — set fader gain ([`SetVolumeParams`]).
pub const SET_VOLUME: &str = "mixer.set_volume";
/// `mixer.set_volume_db` — set the same fader in dB ([`SetVolumeDbParams`]).
pub const SET_VOLUME_DB: &str = "mixer.set_volume_db";
/// `mixer.set_pan` — set stereo pan ([`SetPanParams`]).
pub const SET_PAN: &str = "mixer.set_pan";
/// `mixer.set_mute` — mute/unmute ([`SetMuteParams`]).
pub const SET_MUTE: &str = "mixer.set_mute";
/// `mixer.set_solo` — solo/unsolo ([`SetSoloParams`]).
pub const SET_SOLO: &str = "mixer.set_solo";

/// All `mixer.*` method names.
pub const METHODS: &[&str] = &[SET_VOLUME, SET_VOLUME_DB, SET_PAN, SET_MUTE, SET_SOLO];

/// Lowest fader value the mixer allows, in dB. The app treats this as
/// silence (`db_to_gain` maps `<= -60 dB` to a gain of 0).
pub const VOLUME_DB_MIN: f32 = -60.0;
/// Highest fader value the mixer allows, in dB.
pub const VOLUME_DB_MAX: f32 = 6.0;

/// Params for `mixer.set_volume`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetVolumeParams {
    pub track_id: TrackId,
    /// Linear fader gain (1.0 = unity, 0.0 = silence).
    pub volume: f32,
}

/// Params for `mixer.set_volume_db` — the same fader as
/// [`SetVolumeParams`], in the unit balance work is actually done in.
///
/// Loudness differences are expressed in dB (and 1 LU == 1 dB), so a
/// track measuring 5.2 LU over its target is corrected by subtracting
/// 5.2 from its `volume_db`. Going through the linear form forces every
/// caller to reimplement `new = old * 10^(err/20)`; the app stores the
/// fader in dB anyway, so this form converts nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetVolumeDbParams {
    pub track_id: TrackId,
    /// Fader level in decibels: `0` is unity (no change), negative
    /// attenuates, positive boosts. Must lie within
    /// [`VOLUME_DB_MIN`]`..=`[`VOLUME_DB_MAX`] — the range the mixer
    /// fader itself spans — and `-60` is the app's silence floor rather
    /// than `-inf`.
    pub volume_db: f32,
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
