//! Aux-send + return-bus engine-event mirroring.
//!
//! Reconstructs the app's send graph purely from engine events — there
//! are no read-getters back into the engine. `AuxSendChanged` carries
//! the engine-resolved send (allocated id, clamped level), so the mirror
//! is always faithful. Bus return-role is mirrored onto the bus's own
//! `BusState::is_return`, so it is cleaned up automatically when the bus
//! is removed (the engine emits no per-send removals on bus removal).

use resonance_audio::types::*;

use crate::state::AuxSendRejection;
use crate::Resonance;

/// `BusRoleChanged` — toggle a bus's return-role flag. No-op if the bus
/// isn't mirrored yet (it always is: `BusAdded` precedes any role change).
pub(super) fn bus_role_changed(r: &mut Resonance, bus_id: BusId, is_return: bool) {
    r.registry.with_bus_mut(bus_id, |b| b.is_return = is_return);
}

/// `AuxSendChanged` — insert or update the resolved send. A successful
/// (re)route supersedes any rejection currently shown to the user.
pub(super) fn send_changed(
    r: &mut Resonance,
    send_id: SendId,
    source: SendSource,
    dest: BusId,
    level_db: f32,
    pre_fader: bool,
    enabled: bool,
) {
    r.aux.upsert(AuxSend {
        id: send_id,
        source,
        dest,
        level_db,
        pre_fader,
        enabled,
    });
    r.aux.last_rejection = None;
}

/// `AuxSendRemoved` — drop the send.
pub(super) fn send_removed(r: &mut Resonance, send_id: SendId) {
    r.aux.remove(send_id);
}

/// `AuxSendRejected` — record the reason for the UI to surface.
pub(super) fn send_rejected(
    r: &mut Resonance,
    source: SendSource,
    dest: BusId,
    reason: String,
) {
    r.aux.last_rejection = Some(AuxSendRejection {
        source,
        dest,
        reason,
    });
}
