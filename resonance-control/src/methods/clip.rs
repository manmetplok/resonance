//! `clip.*` — audio clips on the timeline: placing a sample and editing
//! the placement.
//!
//! This is the audio counterpart to `notes.*`. A *clip* is one placement
//! of a pool asset ([`crate::methods::pool`]) on a track at a position,
//! with its own trim, fades and gain — all non-destructive, none of it
//! touching the imported file. `song.tracks` reports every clip on every
//! track (audio clips have `midi: false`), which is how a client finds
//! the `clip_id`s these methods take.
//!
//! Only audio clips are addressed here. A MIDI clip with the same id
//! space is edited through `notes.*`, and passing one is rejected rather
//! than silently misapplied.
//!
//! Positions follow the wire convention: give a [`PositionSpec`]
//! (musical `bar`/`beat`, or absolute `sample`) and get a resolved
//! [`SongPosition`] back. Lengths inside a clip — trims and fades — are
//! given as an [`AmountSpec`], because audio is not tempo-locked: beats
//! are convenient when the sample was cut to the grid, seconds/samples
//! when it wasn't.

use crate::common::{PositionSpec, SongPosition};
use crate::ids::{AssetId, ClipId, TrackId};
use serde::{Deserialize, Serialize};

/// `clip.place` — put a sample on a track ([`PlaceParams`] -> a job whose
/// result is [`PlaceResult`]).
pub const PLACE: &str = "clip.place";
/// `clip.move` — reposition a clip, optionally to another track
/// ([`MoveParams`] -> `MutationAck`).
pub const MOVE: &str = "clip.move";
/// `clip.trim` — change which part of the source a clip plays
/// ([`TrimParams`] -> [`TrimResult`]).
pub const TRIM: &str = "clip.trim";
/// `clip.delete` — remove a clip from the timeline ([`DeleteParams`] ->
/// `MutationAck`).
pub const DELETE: &str = "clip.delete";
/// `clip.set_gain` — per-clip gain in dB ([`SetGainParams`] ->
/// `MutationAck`).
pub const SET_GAIN: &str = "clip.set_gain";
/// `clip.set_fade` — fade-in/out lengths and curves ([`SetFadeParams`] ->
/// [`FadeResult`]).
pub const SET_FADE: &str = "clip.set_fade";

/// All `clip.*` method names.
pub const METHODS: &[&str] = &[PLACE, MOVE, TRIM, DELETE, SET_GAIN, SET_FADE];

/// Widest per-clip gain the app accepts, in decibels. Values outside
/// `-inf..=MAX_GAIN_DB` are clamped, not rejected.
pub const MAX_GAIN_DB: f32 = 24.0;

/// A length, given in whichever unit suits the material. Exactly one
/// field must be set; all of them unset, or more than one, is
/// `invalid_params`.
///
/// `beats` converts against the project tempo map at the clip's position,
/// so it tracks tempo changes the way the grid does. `seconds` and
/// `samples` are absolute — the right choice for a one-shot or a loop
/// that was not cut to this project's tempo.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AmountSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beats: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples: Option<u64>,
}

impl AmountSpec {
    pub fn beats(beats: f64) -> Self {
        Self {
            beats: Some(beats),
            ..Self::default()
        }
    }

    pub fn seconds(seconds: f64) -> Self {
        Self {
            seconds: Some(seconds),
            ..Self::default()
        }
    }

    pub fn samples(samples: u64) -> Self {
        Self {
            samples: Some(samples),
            ..Self::default()
        }
    }

    /// True when no unit was provided at all.
    pub fn is_empty(&self) -> bool {
        self.beats.is_none() && self.seconds.is_none() && self.samples.is_none()
    }

    /// How many units were provided — more than one is ambiguous and
    /// rejected by the server.
    pub fn given(&self) -> usize {
        self.beats.is_some() as usize
            + self.seconds.is_some() as usize
            + self.samples.is_some() as usize
    }
}

/// A fade shape, lowercase on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum FadeShape {
    /// Straight line in gain.
    Linear,
    /// Constant-power ramp — the default, and what you want for a
    /// crossfade-style fade.
    #[default]
    EqualPower,
    /// Exponential, slow at the quiet end.
    Exp,
    /// Forward-compat catch-all for shapes introduced by newer peers.
    #[serde(other)]
    Unknown,
}

/// Params for `clip.place`.
///
/// Name the sample **either** by `asset_id` (already in the pool — see
/// `pool.list`) **or** by `path` (an absolute path the app can read);
/// supplying both, or neither, is `invalid_params`. A `path` that matches
/// an existing asset's `original_path` reuses that asset instead of
/// importing the file a second time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PlaceParams {
    /// The audio track to place on. An instrument/drums/vocal track is
    /// rejected — audio clips need an audio track (`track.add` with
    /// `kind: "audio"`).
    pub track_id: TrackId,
    /// Place this pool asset. Mutually exclusive with `path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<AssetId>,
    /// Import (if needed) and place this file. Mutually exclusive with
    /// `asset_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Where the clip starts. Defaults to bar 1.
    ///
    /// The clip is named after the source file's stem, as it is when
    /// dragged in from the browser; there is no rename on the wire.
    #[serde(default)]
    pub start: PositionSpec,
}

/// Job result of `clip.place` — everything about the placement, so no
/// follow-up `song.tracks` is needed to find the new clip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PlaceResult {
    pub clip_id: ClipId,
    pub track_id: TrackId,
    /// The pool asset the clip plays — freshly imported, or the one that
    /// was already there.
    pub asset_id: AssetId,
    pub start: SongPosition,
    pub length_beats: f64,
    pub length_samples: u64,
    pub name: String,
    pub revision: u64,
}

/// Params for `clip.move`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MoveParams {
    pub clip_id: ClipId,
    /// The clip's new start.
    pub start: PositionSpec,
    /// Move it to this track as well. Must be an audio track; omitted,
    /// the clip stays where it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
}

/// Params for `clip.trim` — which part of the source the clip plays.
///
/// `start_offset` hides that much of the source's head, `end_offset` that
/// much of its tail; together they must leave at least one frame audible.
/// Trimming the head does NOT move the clip: pass `start` too (or call
/// `clip.move`) if the audible part should stay put on the timeline.
/// Omitted fields keep their current value, so `{"clip_id": 7,
/// "end_offset": {"seconds": 2.0}}` shortens the tail and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TrimParams {
    pub clip_id: ClipId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_offset: Option<AmountSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_offset: Option<AmountSpec>,
    /// Reposition the clip in the same edit — the timeline start, not an
    /// offset into the source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<PositionSpec>,
}

/// Result of `clip.trim` — the geometry that actually landed after
/// clamping to the source's length.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TrimResult {
    pub clip_id: ClipId,
    pub start: SongPosition,
    /// Frames hidden at the head and tail of the source.
    pub start_offset_samples: u64,
    pub end_offset_samples: u64,
    /// What remains audible.
    pub length_samples: u64,
    pub length_beats: f64,
    pub revision: u64,
}

/// Params for `clip.delete`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DeleteParams {
    pub clip_id: ClipId,
}

/// Params for `clip.set_gain`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetGainParams {
    pub clip_id: ClipId,
    /// Per-clip gain in decibels; `0.0` is unity. Clamped to at most
    /// [`MAX_GAIN_DB`].
    pub gain_db: f32,
}

/// Params for `clip.set_fade`. Every field is optional and omitted ones
/// keep their current value; a call that sets nothing is
/// `invalid_params`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetFadeParams {
    pub clip_id: ClipId,
    /// Fade-in length. Clamped to the clip's audible length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in: Option<AmountSpec>,
    /// Fade-out length. Clamped to the clip's audible length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out: Option<AmountSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in_shape: Option<FadeShape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out_shape: Option<FadeShape>,
}

/// Result of `clip.set_fade` — the lengths after clamping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct FadeResult {
    pub clip_id: ClipId,
    pub fade_in_samples: u64,
    pub fade_out_samples: u64,
    pub fade_in_shape: FadeShape,
    pub fade_out_shape: FadeShape,
    pub revision: u64,
}
