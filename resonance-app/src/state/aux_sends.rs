//! GUI-side aux-send state, mirrored from the engine.
//!
//! The send graph is reconstructed *purely* from `AuxSendChanged` /
//! `AuxSendRemoved` events — the app never reads the send list back from
//! the engine. `AuxSendChanged` carries the engine-resolved send (the
//! allocated id plus any clamping of `level_db`), so the mirror always
//! matches engine state. A bus's return-role flag is mirrored separately
//! onto [`BusState::is_return`](super::BusState) from `BusRoleChanged`.

use resonance_audio::types::*;

/// An aux send the engine refused to register, carried so the mixer view
/// can surface why (a self-route or a feedback cycle). Mirrored from
/// `AudioEvent::AuxSendRejected`.
#[derive(Debug, Clone, PartialEq)]
pub struct AuxSendRejection {
    pub source: SendSource,
    pub dest: BusId,
    pub reason: String,
}

/// GUI-side mirror of the engine's aux-send graph.
#[derive(Debug, Default)]
pub struct AuxSendState {
    /// Every live aux send. Insertion-ordered: a freshly created send is
    /// appended; an edited one keeps its slot (see [`Self::upsert`]).
    pub sends: Vec<AuxSend>,
    /// The most recent send the engine rejected, with a plain-language
    /// reason suitable for the UI. Cleared once a send is successfully
    /// created or updated (the user's retry superseded the error).
    pub last_rejection: Option<AuxSendRejection>,
}

impl AuxSendState {
    /// Insert or replace the send carrying `send.id`. Mirrors the
    /// engine's upsert: `AuxSendChanged` is emitted for both a newly
    /// created send and an in-place edit, always with the full resolved
    /// send, so a matching id replaces rather than duplicates.
    pub fn upsert(&mut self, send: AuxSend) {
        match self.sends.iter_mut().find(|s| s.id == send.id) {
            Some(existing) => *existing = send,
            None => self.sends.push(send),
        }
    }

    /// Drop the send with `send_id`, if present.
    pub fn remove(&mut self, send_id: SendId) {
        self.sends.retain(|s| s.id != send_id);
    }
}
