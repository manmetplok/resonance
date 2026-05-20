//! Global-track event types (tempo + time signature) and the selection
//! anchor used by the global-tracks shelf UI.

use resonance_audio::types::*;

/// A tempo change on the tempo track. Type alias for the engine's
/// `TempoPoint` — both sides share the same type and the same `TempoMap`
/// implementation, eliminating any risk of divergence.
pub type TempoEvent = TempoPoint;

/// A time signature change on the signature track. Type alias for the
/// engine's `SignaturePoint`.
pub type SignatureEvent = SignaturePoint;

/// Which global track lane an event belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlobalTrackKind {
    Tempo,
    Signature,
}

/// A selected event on a global track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SelectedGlobalEvent {
    pub kind: GlobalTrackKind,
    /// Index into the corresponding events vec.
    pub index: usize,
}
