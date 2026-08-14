//! `track.remove_effect` / `track.move_effect` — editing the shape of a
//! track's insert chain, and the addressing both share.

use super::{ack, find_track, instance_for, not_found_track, reject};
use crate::message::{Message, PluginMessage};
use crate::state::TrackState;
use crate::update::control::{run_via_update, view_model};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::track::{self, RemoveEffectParams};
use resonance_control::{Request, Response, RpcError};

// ---------------------------------------------------------------------------
// track.remove_effect
// ---------------------------------------------------------------------------

/// `track.remove_effect` — take one effect off a track's insert chain
/// (ba doc #273, todo #1223).
///
/// Until this existed an effect could only ever be APPENDED, and
/// `track.add_effect` adds another instance on every call, so a wrong
/// add was unrecoverable over the control API. Dispatches the existing
/// `PluginMessage::RemovePluginFromTrack` through `run_via_update`, so
/// the removal is undoable like a manual one.
pub(super) fn remove_effect(
    app: &mut Resonance,
    request: &Request,
) -> (Response, Task<Message>) {
    let params: RemoveEffectParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(t) = find_track(app, params.track_id.0).cloned() else {
        return not_found_track(request, params.track_id.0);
    };
    let address = ChainAddress {
        slot: params.slot,
        plugin_id: params.plugin_id.as_deref(),
        occurrence: params.occurrence,
    };
    let (_entry, instance_id) =
        match resolve_chain_effect(app, &t, address, ChainVerb::Remove) {
            Ok(found) => found,
            Err(error) => return reject(request, error),
        };
    let task = run_via_update(
        app,
        Message::Plugin(PluginMessage::RemovePluginFromTrack(t.id, instance_id)),
    );
    (ack(app, request), task)
}

// ---------------------------------------------------------------------------
// track.move_effect
// ---------------------------------------------------------------------------

/// `track.move_effect` — reorder a track's insert chain (ba doc #273,
/// todo #1225).
///
/// `track.add_effect` only ever APPENDS, so before this the order of a
/// chain was whatever order it happened to be built in, and correcting
/// it meant tearing the chain down and rebuilding it — losing every
/// parameter set along the way. Dispatches the same
/// `AudioCommand::MovePlugin` the engine gained in todo #1224 through
/// `run_via_update`, so the reorder is undoable like a manual one.
///
/// The order is mirrored app-side immediately (and again, idempotently,
/// when `AudioEvent::PluginMoved` echoes) so a client can read back the
/// new slots in the same cycle — the same read-your-writes rule
/// `track.add_effect` follows since todo #1234.
pub(super) fn move_effect(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: track::MoveEffectParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(t) = find_track(app, params.track_id.0).cloned() else {
        return not_found_track(request, params.track_id.0);
    };
    let address = ChainAddress {
        slot: params.slot,
        plugin_id: params.plugin_id.as_deref(),
        occurrence: params.occurrence,
    };
    let (entry, instance_id) = match resolve_chain_effect(app, &t, address, ChainVerb::Move) {
        Ok(found) => found,
        Err(error) => return reject(request, error),
    };

    // The instrument-floor and end-clamp rules belong to the chain, not
    // to this RPC edge (`plugin_chain`); all this layer adds is wire
    // wording that names the offending slot.
    let to_slot = match crate::plugin_chain::resolve_effect_move(app, &t, params.to_slot) {
        Ok(slot) => slot,
        Err(floor) => {
            let entries = view_model::plugin_entries(app, &t);
            let instrument = entries
                .iter()
                .find(|e| e.kind == track::PluginKind::Instrument);
            return reject(
                request,
                RpcError::invalid_params(format!(
                    "slot {} on track {} is the track's INSTRUMENT ({}); effects sit after it, \
                     so to_slot must be at least {floor}",
                    instrument.map(|e| e.slot).unwrap_or(0),
                    t.id,
                    instrument.map(|e| e.plugin_id.as_str()).unwrap_or("?"),
                )),
            );
        }
    };
    if to_slot == entry.slot {
        // A no-op move records no undo entry and sends no command.
        return (ack(app, request), Task::none());
    }

    let task = run_via_update(
        app,
        Message::Plugin(PluginMessage::MovePluginInTrack {
            track_id: t.id,
            instance_id,
            to_index: to_slot as usize,
        }),
    );
    (ack(app, request), task)
}

// ---------------------------------------------------------------------------
// Shared chain addressing for remove_effect / move_effect
// ---------------------------------------------------------------------------

/// How a caller named one plugin on a track's chain: by `slot`, or by
/// `plugin_id` (+ `occurrence`). Exactly one form, never both.
#[derive(Clone, Copy)]
struct ChainAddress<'a> {
    slot: Option<u32>,
    plugin_id: Option<&'a str>,
    occurrence: Option<u32>,
}

/// What the caller is doing with the addressed plugin — only used to
/// word the rejections, so "name the effect to remove" doesn't appear on
/// a failed move.
#[derive(Clone, Copy)]
enum ChainVerb {
    Remove,
    Move,
}

impl ChainVerb {
    fn verb(self) -> &'static str {
        match self {
            ChainVerb::Remove => "remove",
            ChainVerb::Move => "move",
        }
    }

    /// Why the track's instrument is off limits for this verb.
    fn instrument_refusal(self) -> &'static str {
        match self {
            ChainVerb::Remove => {
                "replace it with track.add_instrument instead of removing it"
            }
            ChainVerb::Move => {
                "instruments are not chain-ordered inserts; it stays at the head of the chain"
            }
        }
    }
}

/// Resolve a [`ChainAddress`] to one EFFECT on the track, returning its
/// wire entry and the engine instance id behind it.
///
/// Shared by `track.remove_effect` and `track.move_effect` so the two
/// cannot drift: both accept the same two addressing forms, both refuse
/// an ambiguous or absent address, and both refuse the track's
/// instrument.
fn resolve_chain_effect(
    app: &Resonance,
    t: &TrackState,
    address: ChainAddress<'_>,
    verb: ChainVerb,
) -> Result<(track::PluginParamsEntry, resonance_audio::types::PluginInstanceId), RpcError> {
    let entries = view_model::plugin_entries(app, t);
    let entry = match (address.slot, address.plugin_id) {
        (Some(_), Some(_)) => {
            return Err(RpcError::invalid_params(
                "address the effect by slot OR by plugin_id (+ occurrence), not both",
            ))
        }
        (None, None) => {
            return Err(RpcError::invalid_params(format!(
                "name the effect to {}: slot, or plugin_id (+ occurrence). Track {} carries [{}]",
                verb.verb(),
                t.id,
                chain_description(&entries)
            )))
        }
        (Some(slot), None) => match entries.iter().find(|e| e.slot == slot) {
            Some(entry) => entry.clone(),
            None => {
                return Err(RpcError::not_found(format!(
                    "track {} has no plugin at slot {slot}; it carries [{}]",
                    t.id,
                    chain_description(&entries)
                )))
            }
        },
        (None, Some(plugin_id)) => {
            let occurrence = address.occurrence.unwrap_or(0);
            match entries
                .iter()
                .find(|e| e.plugin_id == plugin_id && e.occurrence == occurrence)
            {
                Some(entry) => entry.clone(),
                None => {
                    return Err(view_model::unknown_plugin_on_track(app, t, plugin_id, occurrence))
                }
            }
        }
    };

    if entry.kind == track::PluginKind::Instrument {
        return Err(RpcError::invalid_params(format!(
            "slot {} on track {} is the track's INSTRUMENT ({}), not an effect; {}",
            entry.slot,
            t.id,
            entry.plugin_id,
            verb.instrument_refusal()
        )));
    }

    let instance_id = instance_for(t, &entry.plugin_id, entry.occurrence).ok_or_else(|| {
        RpcError::not_found(format!(
            "plugin {:?} vanished from track {} between lookup and {}",
            entry.plugin_id,
            t.id,
            verb.verb()
        ))
    })?;
    Ok((entry, instance_id))
}

/// The chain as `slot:plugin_id` pairs, for error messages that let the
/// caller correct an address rather than guess again.
fn chain_description(entries: &[track::PluginParamsEntry]) -> String {
    entries
        .iter()
        .map(|e| format!("{}:{}", e.slot, e.plugin_id))
        .collect::<Vec<_>>()
        .join(", ")
}
