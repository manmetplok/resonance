//! Shared primitives used across method params and song views.
//!
//! Conventions (doc #265): enums are lowercase strings on the wire;
//! results echo both musical and sample positions; every mutating
//! result carries `{revision}`.

use serde::{Deserialize, Serialize};

/// A resolved song position: both musical (1-based bar, 1-based beat
/// within the bar, fractional) and absolute sample time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SongPosition {
    pub bar: u32,
    pub beat: f64,
    pub sample: u64,
}

/// A client-supplied position: either musical (`bar` [+ `beat`]) or
/// `sample`. Servers resolve it and echo back a full [`SongPosition`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct PositionSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<u64>,
}

impl PositionSpec {
    /// A musical position (1-based bar, 1-based beat).
    pub fn musical(bar: u32, beat: f64) -> Self {
        Self {
            bar: Some(bar),
            beat: Some(beat),
            ..Self::default()
        }
    }

    /// An absolute sample position.
    pub fn sample(sample: u64) -> Self {
        Self {
            sample: Some(sample),
            ..Self::default()
        }
    }

    /// True when no coordinate was provided at all.
    pub fn is_empty(&self) -> bool {
        self.bar.is_none() && self.beat.is_none() && self.sample.is_none()
    }
}

/// A half-open beat range within a clip or section, used to window
/// note queries.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BeatRange {
    pub start_beat: f64,
    pub end_beat: f64,
}

/// A time signature, e.g. `{"numerator":4,"denominator":4}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeSignature {
    pub numerator: u8,
    pub denominator: u8,
}

/// A key/scale, e.g. `{"tonic":"A","scale":"minor"}`. `tonic` is a pitch
/// name (`"A"`, `"F#"`, ...); `scale` is a lowercase scale name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyScale {
    pub tonic: String,
    pub scale: String,
}

/// Track kinds, lowercase on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Instrument,
    Drums,
    Vocal,
    Audio,
    Bus,
    /// Forward-compat catch-all for kinds introduced by newer peers.
    #[serde(other)]
    Unknown,
}

/// Transport states, lowercase on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportState {
    Stopped,
    Playing,
    Paused,
    Recording,
}

/// Result of a simple mutation: the revision counter after the edit.
///
/// `revision` is bumped by the app once per committed undoable
/// transaction, so clients can detect concurrent GUI edits. Richer
/// mutation results embed a `revision` field of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationAck {
    pub revision: u64,
}
