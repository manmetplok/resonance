//! Stem-export protocol descriptors (ba todo #322 / #325).
//!
//! These are pure data — the GUI → engine command surface for "export
//! stems" — so they live in the protocol layer alongside [`AudioCommand`]
//! rather than in the engine. The engine's stem renderer
//! (`engine::bounce::stem`) hangs its rendering behaviour and the WAV
//! encoder helpers off these same types.
//!
//! [`AudioCommand`]: super::AudioCommand

use super::{BusId, TrackId};

/// Which slice of the mix a stem captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StemSource {
    /// A single track plus its sub-tracks (multi-output instruments fan
    /// out to sibling sub-tracks). Excludes master FX + master volume,
    /// like a bounce-in-place clip, so the stem re-imports at unity.
    Track(TrackId),
    /// A return / aux / group bus: every track routed to the bus —
    /// top-level tracks (which bring their sub-tracks with them, so an
    /// instrument is never split across stems) and **sub-tracks routed
    /// to the bus on their own**, which is how a multi-output
    /// instrument gets a group bus (ba todo #1239). The bus's own FX
    /// chain runs (it is fed only by the in-filter tracks), but master
    /// FX + master volume are excluded.
    Bus(BusId),
    /// The full mix — every track, with master FX, master volume and the
    /// final hard-clip applied, identical to the project bounce.
    Master,
}

/// PCM encoding for a written stem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StemBitDepth {
    /// 16-bit signed integer.
    Int16,
    /// 24-bit signed integer (packed 3 bytes/sample).
    Int24,
    /// 32-bit IEEE float (the engine's native format — lossless).
    Float32,
}

/// One output of a multi-stem export: the mix slice to render plus the
/// WAV file it is written to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StemTarget {
    /// Which track / bus / master mix this stem captures.
    pub source: StemSource,
    /// Absolute path for this stem's `.wav` file.
    pub path: String,
}
