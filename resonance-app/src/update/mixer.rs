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
//! Since ARCH-04 D-2 a create and an edit are two different engine
//! commands: [`AudioCommand::AddAuxSend`] under an app-allocated id
//! (`Resonance::allocate_send_id`), refused on a collision, and
//! [`AudioCommand::SetAuxSend`] — what the "set level / re-route / toggle
//! pre-post / toggle enable" edits all resolve to, via [`upsert_send`]: read
//! the send's current mirrored fields, apply the one change, and re-send the
//! whole send under its existing id, which the engine treats as an in-place
//! edit (see `types/commands.rs`).

use iced::Task;
use resonance_audio::types::{AudioCommand, AuxSend, BusId, SendId, SendSource};

use crate::message::Message;
use crate::Resonance;

/// Aux-send + return-bus actions raised from the Mixer inspector's
/// ROUTING group. Every variant maps to one engine command (or, for
/// [`CreateReturnFromSend`](MixerMessage::CreateReturnFromSend), a short
/// ordered sequence). The handlers never mutate the send graph directly:
/// the engine validates each command and echoes `AuxSendChanged` /
/// `AuxSendRemoved` / `BusRoleChanged`, which the engine-event mirror
/// (ba todo #478) folds into [`AuxSendState`](crate::state::AuxSendState).
/// That single-writer rule keeps the GUI from showing a route the engine
/// rejected as cyclic.
#[derive(Debug, Clone)]
pub enum MixerMessage {
    /// Create a new aux send from `source` into return bus `dest` with
    /// default routing (0 dB, post-fader, enabled). Since ARCH-04 D-2 the
    /// [`SendId`] is app-allocated here too (`Resonance::allocate_send_id`),
    /// same as [`AddSendWithId`](Self::AddSendWithId) — what distinguishes
    /// the two is only that this one waits for the engine's
    /// `AuxSendChanged` echo to mirror the send, rather than mirroring it
    /// eagerly.
    AddSend { source: SendSource, dest: BusId },
    /// Create an aux send whose id the *app* chose up front, with
    /// explicit level and tap point, so a caller can use the id without
    /// waiting for the engine's `AuxSendChanged` echo (ba doc #273).
    /// Same pattern as [`BusMessage::AddBusWithId`].
    AddSendWithId {
        id: SendId,
        source: SendSource,
        dest: BusId,
        level_db: f32,
        pre_fader: bool,
    },
    /// Remove the send with this id.
    RemoveSend(SendId),
    /// Set a send's level in dB (slider drag). Coalesces into a single
    /// undo entry per drag, like the volume/pan faders.
    SetSendLevel(SendId, f32),
    /// Re-route an existing send into a different return bus.
    SetSendDest(SendId, BusId),
    /// Flip a send between a pre- and post-fader source tap.
    ToggleSendPreFader(SendId),
    /// Enable / disable a send while keeping its routing and level.
    ToggleSendEnabled(SendId),
    /// Mark a bus as an aux *return* bus, or clear the flag.
    SetBusReturnRole(BusId, bool),
    /// Create a brand-new FX return bus and route `source` into it in one
    /// gesture: add a bus, flag it as a return, then upsert the send.
    CreateReturnFromSend { source: SendSource },
}

impl MixerMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::{CoalesceKey, UndoAction};
        match self {
            // Aux-send edits. A level drag coalesces into one entry per
            // gesture (like the volume/pan faders); every other send action is
            // a discrete, atomic edit. The send graph rides the `ProjectFile`
            // snapshot since ba todo #1269, so these entries restore
            // end-to-end; here we only classify the bookkeeping (dirty-mark +
            // redo-clear).
            Self::SetSendLevel(send_id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::SendLevel(*send_id))
            }
            Self::AddSend { .. }
            | Self::AddSendWithId { .. }
            | Self::RemoveSend(_)
            | Self::SetSendDest(_, _)
            | Self::ToggleSendPreFader(_)
            | Self::ToggleSendEnabled(_)
            | Self::SetBusReturnRole(_, _)
            | Self::CreateReturnFromSend { .. } => UndoAction::Record,
        }
    }
}

pub fn handle(r: &mut Resonance, m: MixerMessage) -> Task<Message> {
    match m {
        MixerMessage::AddSend { source, dest } => {
            // App-allocated (ARCH-04 D-2); still no eager mirror, same as
            // the plain plugin/bus GUI adds — the mixer waits for
            // `AuxSendChanged`. Default routing matches a typical
            // "post-fader reverb send at unity".
            let id = r.aux.allocate_send_id();
            let _ = r.engine.send(AudioCommand::AddAuxSend {
                id,
                source,
                dest,
                level_db: 0.0,
                pre_fader: false,
                enabled: true,
            });
        }
        MixerMessage::AddSendWithId {
            id,
            source,
            dest,
            level_db,
            pre_fader,
        } => {
            let _ = r.engine.send(AudioCommand::AddAuxSend {
                id,
                source,
                dest,
                level_db,
                pre_fader,
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
            // Allocate the new bus's id (and the new send's) up front so
            // we can name the return role and the send's destination
            // without waiting for the engine's `BusAdded` echo. The three
            // commands run in order: add the bus, flag it a return, then
            // route the send into it.
            let bus_id = r.registry.allocate_bus_id();
            let send_id = r.aux.allocate_send_id();
            let name = next_return_bus_name(r);
            let _ = r.engine.send(AudioCommand::AddBus {
                id: bus_id,
                name: Some(name),
            });
            let _ = r.engine.send(AudioCommand::SetBusRole {
                bus_id,
                is_return: true,
            });
            let _ = r.engine.send(AudioCommand::AddAuxSend {
                id: send_id,
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
        id: send.id,
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
