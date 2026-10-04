//! Bus handlers: create/destroy, per-bus volume/pan/mute/name,
//! track→bus routing, aux sends. The bus insert chain is edited by the
//! owner-generic handlers in `chain.rs` (ARCH2-02).

use std::sync::Arc;

use crate::types::*;

use super::thread::{HandlerCtx, HandlerState};
use super::MAX_BUSSES;

/// Refuse an add whose id is already live in `busses`, rather than
/// silently replacing the bus it names — the bus twin of
/// `plugins::reject_if_plugin_id_in_use` (ARCH-04 D-3). Checked against
/// the published render graph: the engine thread is its only writer, so
/// nothing can add the id between this check and the add's publish.
fn reject_if_bus_id_in_use(ctx: &HandlerCtx, id: BusId) -> bool {
    if ctx.shared.graph.load().busses.contains_key(&id) {
        let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::internal(format!(
            "bus id {id} is already in use; refusing the add rather than replacing the live bus"
        ))));
        true
    } else {
        false
    }
}

pub(crate) fn handle_add_bus(ctx: &HandlerCtx, id: BusId, name: Option<String>) {
    if ctx.shared.graph.load().busses.len() >= MAX_BUSSES {
        let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::busy(format!(
            "Cannot add bus: maximum of {MAX_BUSSES} busses reached"
        ))));
        return;
    }
    if reject_if_bus_id_in_use(ctx, id) {
        return;
    }
    let name = name.unwrap_or_else(|| format!("Bus {id}"));
    let bus = Arc::new(Bus::new(id, name.clone()));
    ctx.shared.edit_busses(|busses| busses.insert(id, bus));
    let _ = ctx.event_tx.send(AudioEvent::BusAdded { bus_id: id, name });
}

pub(crate) fn handle_remove_bus(ctx: &HandlerCtx, bus_id: BusId) {
    // Re-route every track that fed this bus to master and unpublish the
    // bus in ONE graph (code review ARCH-02 B-3): a block sees the bus
    // with its feeders or neither, never a feeder pointing at a bus that
    // is gone. (The mixer's fall-back-to-master for a missing bus stays as
    // a defensive net; no published graph can reach it any more.) The
    // removed bus lives on in the replaced graph, which the retire sweep
    // drops once no reader pins it. Keep its plugin ids to tear down.
    let routed_here = |g: &crate::engine::RenderGraph| {
        g.bus(bus_id).is_some()
            || g.tracks.values().any(|t| t.output() == TrackOutput::Bus(bus_id))
    };
    let removed_plugins: Vec<PluginInstanceId> = if routed_here(&ctx.shared.graph.load()) {
        ctx.shared.edit_tracks_and_busses(|tracks, busses| {
            for track in tracks.values_mut() {
                if track.output() == TrackOutput::Bus(bus_id) {
                    Arc::make_mut(track).set_output(TrackOutput::Master);
                }
            }
            busses
                .shift_remove(&bus_id)
                .map(|bus| bus.plugin_ids.clone())
                .unwrap_or_default()
        })
    } else {
        Vec::new()
    };
    // Unpublish the bus's plugin instances; the retire sweep destroys
    // them off the audio path.
    super::plugins::remove_plugin_slots(ctx.shared, &removed_plugins);
    let _ = ctx.event_tx.send(AudioEvent::BusRemoved { bus_id });
}

pub(crate) fn handle_set_bus_volume(ctx: &HandlerCtx, bus_id: BusId, volume: f32) {
    if let Some(bus) = ctx.shared.graph.load().bus(bus_id) {
        bus.set_volume(volume);
    }
}

pub(crate) fn handle_set_bus_pan(ctx: &HandlerCtx, bus_id: BusId, pan: f32) {
    if let Some(bus) = ctx.shared.graph.load().bus(bus_id) {
        bus.set_pan(pan);
    }
}

pub(crate) fn handle_set_bus_mute(ctx: &HandlerCtx, bus_id: BusId, muted: bool) {
    if let Some(bus) = ctx.shared.graph.load().bus(bus_id) {
        bus.set_muted(muted);
    }
}

pub(crate) fn handle_set_bus_name(ctx: &HandlerCtx, bus_id: BusId, name: String) {
    ctx.shared.edit_bus(bus_id, |bus| bus.name = name);
}

pub(crate) fn handle_set_track_output(ctx: &HandlerCtx, track_id: TrackId, output: TrackOutput) {
    // Structural (ARCH-02 B-3): a copy-on-write edit of the track,
    // published in a new render graph.
    ctx.shared.edit_track(track_id, |track| track.set_output(output));
}

/// Lower / upper bound on aux-send level in dB. Mirrors the spirit of
/// `handle_set_clip_gain`'s clamp: keep stored values finite and sane so
/// a stray `NaN`/`inf` from the GUI can never poison engine state.
const AUX_SEND_MIN_DB: f32 = -120.0;
const AUX_SEND_MAX_DB: f32 = 24.0;

/// Republish the control-thread aux-send table into the lock-free
/// snapshot the audio callback and bounce renderer read each block (see
/// [`SharedState::aux_sends`](crate::engine::SharedState)). Called after
/// every mutation of `state.aux_sends` so the render path never sees a
/// stale or partially-updated table.
pub(crate) fn publish_aux_sends(ctx: &HandlerCtx, state: &HandlerState) {
    let snapshot: Vec<AuxSend> = state.aux_sends.values().copied().collect();
    super::retire::publish(
        &ctx.shared.aux_sends,
        std::sync::Arc::new(snapshot),
        &ctx.shared.retired,
    );
}

pub(crate) fn handle_set_bus_role(ctx: &HandlerCtx, bus_id: BusId, is_return: bool) {
    // Silently no-op on an unknown bus, matching the other bus setters.
    if let Some(bus) = ctx.shared.graph.load().bus(bus_id) {
        bus.set_is_return(is_return);
    } else {
        return;
    }
    let _ = ctx
        .event_tx
        .send(AudioEvent::BusRoleChanged { bus_id, is_return });
}

/// Validate a prospective aux-send route: the destination and source must
/// exist, and registering it must not close a feedback loop. Shared by
/// [`handle_add_aux_send`] and [`handle_set_aux_send`] (ARCH-04 D-2) — the
/// only difference between the two is `updating`, which excludes a send's
/// own current edge from the cycle check: `None` for a brand-new send,
/// `Some(id)` for an edit of an existing one. Reports
/// `AudioEvent::AuxSendRejected` and returns `false` on any failure;
/// `true` means the route is safe to register.
fn validate_aux_send_route(
    ctx: &HandlerCtx,
    state: &HandlerState,
    updating: Option<SendId>,
    source: SendSource,
    dest: BusId,
) -> bool {
    // Reject up front with a plain-language reason; never store an
    // invalid send. The app surfaces `reason` to the user.
    let reject = |reason: String| {
        let _ = ctx.event_tx.send(AudioEvent::AuxSendRejected {
            source,
            dest,
            reason,
        });
    };

    // Destination must be a real bus.
    if !ctx.shared.graph.load().busses.contains_key(&dest) {
        reject(format!("Aux send destination bus {dest} does not exist"));
        return false;
    }
    // Source must exist (a track or a bus, depending on the variant).
    match source {
        SendSource::Track(tid) => {
            if !ctx.tracks().contains_key(&tid) {
                reject(format!("Aux send source track {tid} does not exist"));
                return false;
            }
        }
        SendSource::Bus(bid) => {
            if !ctx.shared.graph.load().busses.contains_key(&bid) {
                reject(format!("Aux send source bus {bid} does not exist"));
                return false;
            }
        }
    }

    if aux_send_would_cycle(state.aux_sends.values(), source, dest, updating) {
        reject(match source {
            SendSource::Bus(b) if b == dest => {
                format!("A bus cannot send to itself (bus {dest})")
            }
            SendSource::Bus(b) => format!(
                "Aux send from bus {b} to bus {dest} would create a feedback loop"
            ),
            // Unreachable: track sources never cycle.
            SendSource::Track(t) => format!("Aux send from track {t} is invalid"),
        });
        return false;
    }
    true
}

/// Clamp a send level to the sane range, treating a non-finite value (a
/// stray `NaN`/`inf` from the GUI) as unity rather than storing it.
fn clamp_send_level(level_db: f32) -> f32 {
    if level_db.is_finite() {
        level_db.clamp(AUX_SEND_MIN_DB, AUX_SEND_MAX_DB)
    } else {
        0.0
    }
}

/// Refuse an `AddAuxSend` whose id is already live, rather than silently
/// turning what the caller meant as a create into an edit of the send that
/// id already names — the send twin of `plugins::reject_if_plugin_id_in_use`
/// (ARCH-04 D-2).
fn reject_if_send_id_in_use(ctx: &HandlerCtx, state: &HandlerState, id: SendId) -> bool {
    if state.aux_sends.contains_key(&id) {
        let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::internal(format!(
            "aux send id {id} is already in use; refusing the add rather than replacing the \
             live send"
        ))));
        true
    } else {
        false
    }
}

/// Create a new aux send under an app-allocated `id` (ARCH-04 D-2). Refuses
/// a colliding id instead of silently editing the send it already names —
/// see [`reject_if_send_id_in_use`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_add_aux_send(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    id: SendId,
    source: SendSource,
    dest: BusId,
    level_db: f32,
    pre_fader: bool,
    enabled: bool,
) {
    if reject_if_send_id_in_use(ctx, state, id) {
        return;
    }
    if !validate_aux_send_route(ctx, state, None, source, dest) {
        return;
    }
    let level_db = clamp_send_level(level_db);
    state.aux_sends.insert(
        id,
        AuxSend {
            id,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        },
    );
    // Make the new send visible to the audio + bounce render paths before
    // confirming the change to the app.
    publish_aux_sends(ctx, state);
    let _ = ctx.event_tx.send(AudioEvent::AuxSendChanged {
        send_id: id,
        source,
        dest,
        level_db,
        pre_fader,
        enabled,
    });
}

/// Edit an existing aux send in place — re-route / level / pre-post /
/// enable, all covered by resending the send's full state under its own
/// `id` (ARCH-04 D-2). A quiet no-op if `id` does not name a live send (an
/// edit racing its own removal): unlike a duplicate id on
/// [`handle_add_aux_send`], there is no caller invariant to complain about
/// here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_set_aux_send(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    id: SendId,
    source: SendSource,
    dest: BusId,
    level_db: f32,
    pre_fader: bool,
    enabled: bool,
) {
    if !state.aux_sends.contains_key(&id) {
        return;
    }
    if !validate_aux_send_route(ctx, state, Some(id), source, dest) {
        return;
    }
    let level_db = clamp_send_level(level_db);
    state.aux_sends.insert(
        id,
        AuxSend {
            id,
            source,
            dest,
            level_db,
            pre_fader,
            enabled,
        },
    );
    // Make the updated send visible to the audio + bounce render paths
    // before confirming the change to the app.
    publish_aux_sends(ctx, state);
    let _ = ctx.event_tx.send(AudioEvent::AuxSendChanged {
        send_id: id,
        source,
        dest,
        level_db,
        pre_fader,
        enabled,
    });
}

pub(crate) fn handle_remove_aux_send(ctx: &HandlerCtx, state: &mut HandlerState, send_id: SendId) {
    if state.aux_sends.shift_remove(&send_id).is_some() {
        publish_aux_sends(ctx, state);
        let _ = ctx.event_tx.send(AudioEvent::AuxSendRemoved { send_id });
    }
}
