//! Aux-send + return-bus update handlers (ba todo #477).
//!
//! Each [`MixerMessage`] emits the matching [`AudioCommand`] and returns;
//! the send graph itself is never mutated here. The engine validates the
//! command (cyclic-route check, level clamp) and echoes `AuxSendChanged` /
//! `AuxSendRemoved` / `BusRoleChanged`, which the engine-event mirror folds
//! into [`AuxSendState`](crate::state::AuxSendState). Keeping the engine the
//! single writer means a route the engine rejects never shows up as live in
//! the GUI — the rejection surfaces through `AuxSendRejected` instead.
//!
//! The "set level / re-route / toggle pre-post / toggle enable" edits all
//! resolve to a single `SetAuxSend` *upsert*: we read the send's current
//! mirrored fields, apply the one change, and re-send the whole send under
//! its existing id. The engine treats a `SetAuxSend` carrying a known id as
//! an in-place edit (see `types/commands.rs`).

use iced::Task;
use resonance_audio::types::{AudioCommand, AuxSend, SendId};

use crate::message::{Message, MixerMessage};
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: MixerMessage) -> Task<Message> {
    match m {
        MixerMessage::AddSend { source, dest } => {
            // Fresh send: let the engine allocate the id. Default routing
            // matches a typical "post-fader reverb send at unity".
            let _ = r.engine.send(AudioCommand::SetAuxSend {
                id_hint: None,
                source,
                dest,
                level_db: 0.0,
                pre_fader: false,
                enabled: true,
            });
        }
        MixerMessage::RemoveSend(send_id) => {
            let _ = r.engine.send(AudioCommand::RemoveAuxSend { send_id });
        }
        MixerMessage::SetSendLevel(send_id, level_db) => {
            upsert_send(r, send_id, |s| s.level_db = level_db);
        }
        MixerMessage::SetSendDest(send_id, dest) => {
            upsert_send(r, send_id, |s| s.dest = dest);
        }
        MixerMessage::ToggleSendPreFader(send_id) => {
            upsert_send(r, send_id, |s| s.pre_fader = !s.pre_fader);
        }
        MixerMessage::ToggleSendEnabled(send_id) => {
            upsert_send(r, send_id, |s| s.enabled = !s.enabled);
        }
        MixerMessage::SetBusReturnRole(bus_id, is_return) => {
            let _ = r.engine.send(AudioCommand::SetBusRole { bus_id, is_return });
        }
        MixerMessage::CreateReturnFromSend { source } => {
            // Allocate the new bus's id up front so we can name its return
            // role and the send's destination without waiting for the
            // engine's `BusAdded` echo. The three commands run in order:
            // add the bus, flag it a return, then route the send into it.
            let bus_id = r.registry.allocate_return_bus_id();
            let name = next_return_bus_name(r);
            let _ = r.engine.send(AudioCommand::AddBus {
                id_hint: Some(bus_id),
                name: Some(name),
            });
            let _ = r.engine.send(AudioCommand::SetBusRole {
                bus_id,
                is_return: true,
            });
            let _ = r.engine.send(AudioCommand::SetAuxSend {
                id_hint: None,
                source,
                dest: bus_id,
                level_db: 0.0,
                pre_fader: false,
                enabled: true,
            });
        }
    }
    Task::none()
}

/// Re-send an existing send as a `SetAuxSend` upsert after applying `edit`
/// to a copy of its current mirrored fields. No-op when the id is unknown
/// — the send's `AuxSendChanged` echo hasn't landed yet, or it was already
/// removed — since there is nothing to base the edit on.
fn upsert_send(r: &mut Resonance, send_id: SendId, edit: impl FnOnce(&mut AuxSend)) {
    let Some(mut send) = r.aux.sends.iter().find(|s| s.id == send_id).copied() else {
        return;
    };
    edit(&mut send);
    let _ = r.engine.send(AudioCommand::SetAuxSend {
        id_hint: Some(send.id),
        source: send.source,
        dest: send.dest,
        level_db: send.level_db,
        pre_fader: send.pre_fader,
        enabled: send.enabled,
    });
}

/// A display name for a freshly created FX return bus, numbered past the
/// return busses already in the registry (`FX Return 1`, `FX Return 2`, …).
/// The engine falls back to `Bus {id}` if this were ever empty, but it
/// never is.
fn next_return_bus_name(r: &Resonance) -> String {
    let n = r.registry.busses.iter().filter(|b| b.is_return).count() + 1;
    format!("FX Return {n}")
}
