//! Reference-track (A/B) value types shared across the command/event
//! boundary for the reference-mix comparison feature.
//!
//! A *reference* is an external mastered track the user loads alongside
//! their mix to A/B against. The engine can hold several loaded
//! references, switch the monitored source between the project mix and
//! the active reference, loudness-match the reference to the mix, trim
//! its level, and carry per-reference comparison markers.
//!
//! These types travel inside [`crate::AudioCommand`] /
//! [`crate::AudioEvent`], which derive only `Debug`/`Clone`, so (like
//! the other command/event payloads) they intentionally do **not**
//! derive `serde` — persistence of references lives in the project
//! model, not in the engine wire types.

/// Identifier for a loaded reference track. Allocated by the engine on
/// [`crate::AudioCommand::LoadReferenceTrack`]; independent of
/// [`super::TrackId`] — a reference is never a project track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReferenceId(pub u32);

/// Which signal the A/B monitor is currently auditioning: the project
/// mix, or the active reference track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ABSource {
    /// The project's own mix bus (the default).
    #[default]
    Mix,
    /// The active reference track.
    Reference,
}

/// Progress stage of the offline analysis a reference goes through after
/// it is loaded, reported via
/// [`crate::AudioEvent::ReferenceAnalysisProgress`] so the UI can show a
/// determinate "analysing reference…" indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReferenceAnalysisStage {
    /// Decoding the source file to PCM.
    Decoding,
    /// Measuring integrated loudness (LUFS) for loudness matching.
    MeasuringLufs,
    /// Building the downsampled waveform overview.
    BuildingPeaks,
    /// Computing the loudness-match gain offset against the mix.
    ComputingOffset,
}

/// A user-placed comparison marker on a reference track — e.g. "drop",
/// "chorus" — at a sample position within the reference's own timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceMarker {
    /// Per-reference marker id, allocated by the engine.
    pub id: u32,
    /// Position within the reference track, in sample frames.
    pub position_samples: u64,
    /// User-facing label.
    pub label: String,
}
