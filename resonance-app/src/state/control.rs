//! Control-endpoint state (ba doc #265, todo #1147).
//!
//! GUI-side bookkeeping for the unix-socket control endpoint: the
//! listener lifecycle handle plus per-connection handshake state. The
//! socket threads themselves live in [`crate::control_socket`]; the
//! request handlers in `update/control`.

use crate::control_jobs::JobBoard;
use crate::control_socket::{ConnId, ControlServer};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Track kind a `track.add` control request creates (todo #1152).
/// Drums are instrument tracks with the Drum instrument type; the wire
/// bus/unknown kinds are rejected before this is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlTrackKind {
    Instrument,
    Drums,
    Vocal,
    Audio,
}

/// Per-connection session state, created on `Connected` and dropped on
/// `Disconnected`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ControlSession {
    /// The protocol version the client declared in `control.hello`.
    /// `None` until the client has said hello (allowed but discouraged —
    /// doc #265 says hello *should* be first).
    pub protocol_version: Option<u32>,
    /// Set when the client declared an incompatible protocol version.
    /// Every subsequent request on this connection is rejected.
    pub incompatible: bool,
}

/// The control endpoint's app-side state.
///
/// Transient: never persisted, never in the undo snapshot. The connected
/// client count feeds the window chrome's remote-control indicator
/// (todo #1159).
#[derive(Default)]
pub struct ControlEndpointState {
    /// Listener lifecycle handle. `None` when the endpoint is disabled
    /// (`RESONANCE_NO_CONTROL=1`), failed to bind, or was never started
    /// (tests construct the app without a socket).
    pub server: Option<ControlServer>,
    /// Live client connections keyed by connection id.
    pub sessions: HashMap<ConnId, ControlSession>,
    /// Async job ledger (todo #1149), shared with the socket threads:
    /// the update loop starts and resolves jobs, the per-connection
    /// reader threads block on it to serve `job.wait`.
    pub jobs: Arc<JobBoard>,
    /// Tracks added via `track.add` (todo #1152) whose name / drum
    /// instrument type still need applying once the engine echo mirrors
    /// the track into the registry (the mirror ignores the engine's name
    /// and always builds a synth track). Keyed by the app-allocated id;
    /// drained by `apply_pending_control_track`.
    pub pending_tracks: HashMap<resonance_audio::types::TrackId, PendingControlTrack>,
}

/// Deferred post-mirror setup for a control-added track.
#[derive(Debug, Clone)]
pub struct PendingControlTrack {
    pub kind: ControlTrackKind,
    pub name: Option<String>,
}

impl crate::Resonance {
    /// If `track_id` was added via the control endpoint and is now
    /// mirrored in the registry, apply its deferred name and (for a
    /// `drums` add) Drum instrument type, then clear the pending entry.
    /// Called right after the add (in case the echo already landed) and
    /// from the `*TrackAdded` engine-event handlers.
    pub fn apply_pending_control_track(&mut self, track_id: resonance_audio::types::TrackId) {
        let Some(pending) = self.control.pending_tracks.get(&track_id).cloned() else {
            return;
        };
        if !self.registry.tracks.iter().any(|t| t.id == track_id) {
            return; // not mirrored yet; the echo handler will call again
        }
        self.registry.with_track_mut(track_id, |t| {
            if let Some(name) = pending.name {
                t.name = name;
            }
            if matches!(pending.kind, ControlTrackKind::Drums) {
                t.instrument_type = crate::state::InstrumentType::Drum;
                t.instrument_icon = crate::state::InstrumentIcon::Drum;
            }
        });
        self.control.pending_tracks.remove(&track_id);
    }
}
