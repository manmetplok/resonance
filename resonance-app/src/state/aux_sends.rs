//! GUI-side aux-send state, mirrored from the engine.
//!
//! The send graph is reconstructed from `AuxSendChanged` /
//! `AuxSendRemoved` events — the app never reads the send list back from
//! the engine. Two non-send events also prune it, because the engine
//! does not: `TrackRemoved` and `BusRemoved` drop the edges that touched
//! the departed endpoint (see [`Self::drop_sends_touching_track`]). `AuxSendChanged` carries the engine-resolved send (the
//! allocated id plus any clamping of `level_db`), so the mirror always
//! matches engine state. A bus's return-role flag is mirrored separately
//! onto [`BusState::is_return`](super::BusState) from `BusRoleChanged`.
//!
//! The one writer that isn't an engine echo is the project-load replay
//! (ba todo #1269), which seeds the saved graph here at the same moment
//! it re-registers it with the engine — exactly as it seeds `TrackState`
//! alongside `AddTrack`. The engine's `AuxSendChanged` echoes land a
//! moment later and overwrite each seeded entry with the resolved one,
//! so the engine stays the authority on what is actually live.

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
    /// Id counter for sends the *app* creates up front, so a control
    /// method can return the `send_id` in its reply instead of waiting
    /// for the engine's `AuxSendChanged` echo (ba doc #273, todo #1229).
    /// Lives in a high range for the same reason
    /// `TrackRegistry::next_return_bus_id` does: the engine bumps its own
    /// allocator past any id it receives as a hint, so the two ranges
    /// never overlap. `0` means "not seeded yet".
    pub next_control_send_id: SendId,
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

    /// Drop every send that starts at or ends on a deleted endpoint,
    /// returning the ids that were dropped.
    ///
    /// A send is an edge, so it stops meaning anything the moment either
    /// end goes away. The caller MUST send `RemoveAuxSend` for each id
    /// returned: the engine does NOT prune `state.aux_sends` on
    /// `RemoveTrack` / `RemoveBus`, so pruning only the mirror would
    /// leave the engine holding an edge that no longer appears in any
    /// view — unremovable, because `find_send` resolves ids through the
    /// mirror, and still counted by the feedback-cycle check.
    #[must_use = "the engine keeps its copy until RemoveAuxSend is sent"]
    pub fn drop_sends_touching_track(&mut self, track_id: TrackId) -> Vec<SendId> {
        self.drain_sends(|s| matches!(s.source, SendSource::Track(id) if id == track_id))
    }

    /// As [`Self::drop_sends_touching_track`], for a removed bus — which
    /// can be either end of the edge.
    #[must_use = "the engine keeps its copy until RemoveAuxSend is sent"]
    pub fn drop_sends_touching_bus(&mut self, bus_id: BusId) -> Vec<SendId> {
        self.drain_sends(|s| {
            s.dest == bus_id || matches!(s.source, SendSource::Bus(id) if id == bus_id)
        })
    }

    fn drain_sends(&mut self, doomed: impl Fn(&AuxSend) -> bool) -> Vec<SendId> {
        let ids: Vec<SendId> = self
            .sends
            .iter()
            .filter(|s| doomed(s))
            .map(|s| s.id)
            .collect();
        self.sends.retain(|s| !doomed(s));
        ids
    }

    /// Allocate a fresh app-chosen send id, skipping past any id already
    /// mirrored from the engine. Handed to the engine as a `SetAuxSend`
    /// hint, so it must not clash with one the engine allocated itself.
    pub fn allocate_control_send_id(&mut self) -> SendId {
        if self.next_control_send_id == 0 {
            self.next_control_send_id = super::ids::CONTROL_SEND_ID_BASE;
        }
        let sends = &self.sends;
        super::ids::allocate_unused(&mut self.next_control_send_id, |id| {
            sends.iter().any(|s| s.id == id)
        })
    }
}
