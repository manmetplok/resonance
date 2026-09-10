//! `track.remove_effect` / `track.move_effect` — editing the shape of a
//! track's insert chain, and the addressing both share.

use super::{ack, find_track, frozen_reject, instance_for, not_found_track, reject};
use crate::message::{Message, PluginMessage};
use crate::state::TrackState;
use crate::update::control::effect_addressing::{self, ChainWording};
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
    // `RemovePluginFromTrack` is a frozen-input edit (gates.rs); reject
    // rather than ack an edit the gate would swallow.
    if let Some(e) = frozen_reject(app, t.id) {
        return reject(request, e);
    }
    let entries = view_model::plugin_entries(app, &t);
    let (_entry, instance_id) = match effect_addressing::resolve_effect(
        &entries,
        params.slot,
        params.plugin_id.as_deref(),
        params.occurrence,
        "remove",
        &TrackWording { app, t: &t },
        |id, occurrence| instance_for(&t, id, occurrence),
    ) {
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
    // `MovePluginInTrack` is a frozen-input edit (gates.rs); reject
    // rather than ack an edit the gate would swallow.
    if let Some(e) = frozen_reject(app, t.id) {
        return reject(request, e);
    }
    let entries = view_model::plugin_entries(app, &t);
    let (entry, instance_id) = match effect_addressing::resolve_effect(
        &entries,
        params.slot,
        params.plugin_id.as_deref(),
        params.occurrence,
        "move",
        &TrackWording { app, t: &t },
        |id, occurrence| instance_for(&t, id, occurrence),
    ) {
        Ok(found) => found,
        Err(error) => return reject(request, error),
    };

    // The instrument-floor and end-clamp rules belong to the chain, not
    // to this RPC edge (`plugin_chain`); all this layer adds is wire
    // wording that names the offending slot.
    let to_slot = match crate::plugin_chain::resolve_effect_move(app, &t, entry.slot, params.to_slot)
    {
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
// track.replace_effect
// ---------------------------------------------------------------------------

/// `track.replace_effect` — put a different plugin in a chain slot,
/// keeping its position (ba doc #275 P5, todo #1309).
///
/// The addressing and the rejection wording are this layer's; everything
/// after the slot is resolved is shared with the bus and master surfaces
/// (`super::super::replace`).
pub(super) fn replace_effect(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: track::ReplaceEffectParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(t) = find_track(app, params.track_id.0).cloned() else {
        return not_found_track(request, params.track_id.0);
    };
    // `ReplacePlugin` is a frozen-input edit (gates.rs); reject rather
    // than ack an edit the gate would swallow.
    if let Some(e) = frozen_reject(app, t.id) {
        return reject(request, e);
    }
    let entries = view_model::plugin_entries(app, &t);
    let (entry, instance_id) = match effect_addressing::resolve_effect(
        &entries,
        params.slot,
        params.plugin_id.as_deref(),
        params.occurrence,
        "replace",
        &TrackWording { app, t: &t },
        |id, occurrence| instance_for(&t, id, occurrence),
    ) {
        Ok(found) => found,
        Err(error) => return reject(request, error),
    };
    crate::update::control::replace::replace_resolved_slot(
        app,
        request,
        instance_id,
        entry.slot,
        &params.new_plugin_id,
    )
}

// ---------------------------------------------------------------------------
// Track's wording for the shared chain-addressing resolver
// (`effect_addressing::resolve_effect`)
// ---------------------------------------------------------------------------

/// Track's [`ChainWording`]: the one surface with an instrument slot to
/// refuse, and the one whose not-found-by-id message predates
/// `effect_addressing` and still defers to
/// [`view_model::unknown_plugin_on_track`] rather than the shared
/// listing (ba doc #275; see the module doc on
/// [`effect_addressing`](crate::update::control::effect_addressing) for
/// why that isn't unified in this pass).
struct TrackWording<'a> {
    app: &'a Resonance,
    t: &'a TrackState,
}

impl ChainWording for TrackWording<'_> {
    fn no_address(&self, verb: &str, listing: &str) -> String {
        format!(
            "name the effect to {verb}: slot, or plugin_id (+ occurrence). Track {} carries \
             [{listing}]",
            self.t.id
        )
    }

    fn slot_not_found(&self, slot: u32, listing: &str) -> String {
        format!(
            "track {} has no plugin at slot {slot}; it carries [{listing}]",
            self.t.id
        )
    }

    fn id_not_found(&self, plugin_id: &str, occurrence: u32, _listing: &str) -> RpcError {
        view_model::unknown_plugin_on_track(self.app, self.t, plugin_id, occurrence)
    }

    /// **Replace is the exception**, and deliberately so: a synth that
    /// will not load is the worst case of a missing plugin, not an
    /// exempt one, and swapping it in place leaves the track with a
    /// sound source instead of none. Removing it would leave the track
    /// silent, and moving it would displace what every sub-track and the
    /// PDC table are anchored to — those two really are off limits.
    fn instrument_refusal(&self, entry: &track::PluginParamsEntry, verb: &str) -> Option<String> {
        let refusal = match verb {
            "remove" => "replace it with track.replace_effect instead of removing it",
            "move" => {
                "instruments are not chain-ordered inserts; it stays at the head of the chain"
            }
            _ => return None,
        };
        Some(format!(
            "slot {} on track {} is the track's INSTRUMENT ({}), not an effect; {refusal}",
            entry.slot, self.t.id, entry.plugin_id,
        ))
    }

    fn vanished(&self, plugin_id: &str, verb: &str) -> String {
        format!(
            "plugin {plugin_id:?} vanished from track {} between lookup and {verb}",
            self.t.id
        )
    }
}
