//! `track.set_sidechain` / `track.clear_sidechain` — the key route into
//! a plugin's external sidechain input, for a plugin on a **track**.
//!
//! The bus and master arms live in `super::super::bus` / `master`; the
//! rules all three share (one source, key port required, unqualified
//! calls target the first keyable plugin) live in
//! [`super::super::sidechain`] so they cannot drift apart.

use super::{ack, find_track, frozen_reject, instance_for, not_found_track, reject};
use crate::message::Message;
use crate::state::TrackState;
use crate::update::control::sidechain::{
    first_keyable, require_key_port, resolve_key_source, route_message,
};
use crate::update::control::{run_via_update, view_model};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::track;
use resonance_control::{Request, Response, RpcError};

/// Resolve the plugin a sidechain request addresses, the same way
/// `track.set_plugin_param` does: an explicit `plugin_id` (+ optional
/// `occurrence`), else the track's instrument.
fn resolve_keyed_plugin(
    app: &Resonance,
    t: &TrackState,
    plugin_id: Option<&str>,
    occurrence: Option<u32>,
) -> Result<resonance_audio::types::PluginInstanceId, RpcError> {
    // No `plugin_id`: key the plugin that can actually take a key. The
    // instrument-first default this used to share with
    // `track.set_plugin_param` sent every unqualified call at slot 0 —
    // a synth, which has no key port — so the route was stored on the
    // one plugin in the chain guaranteed to ignore it (ba doc #275 P0).
    if plugin_id.is_none() {
        if let Some(instance_id) = first_keyable(&t.plugins) {
            return Ok(instance_id);
        }
    }

    let entries = view_model::plugin_entries(app, t);
    let occurrence = occurrence.unwrap_or(0);
    let entry = match plugin_id {
        Some(id) => entries
            .iter()
            .find(|e| e.plugin_id == id && e.occurrence == occurrence),
        None => entries
            .iter()
            .find(|e| e.kind == track::PluginKind::Instrument),
    };
    match entry {
        Some(entry) => instance_for(t, &entry.plugin_id, entry.occurrence).ok_or_else(|| {
            RpcError::not_found(format!(
                "plugin {:?} vanished from track {} between lookup and routing",
                entry.plugin_id, t.id
            ))
        }),
        None => Err(match plugin_id {
            Some(id) => view_model::unknown_plugin_on_track(app, t, id, occurrence),
            None => RpcError::invalid_params(format!(
                "track {} has no instrument; name a plugin_id (it carries: [{}])",
                t.id,
                entries
                    .iter()
                    .map(|e| e.plugin_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }),
    }
}

/// `track.set_sidechain` — feed another track's or bus's audio into a
/// plugin's external key input.
///
/// The key replaces the plugin's DETECTOR source only; it never reaches
/// the output.
pub(super) fn set_sidechain(
    app: &mut Resonance,
    request: &Request,
) -> (Response, Task<Message>) {
    let params: track::SetSidechainParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(t) = find_track(app, params.track_id.0).cloned() else {
        return not_found_track(request, params.track_id.0);
    };
    // Re-keying a detector is a frozen-input edit (`SetPluginSidechain`,
    // gates.rs); reject rather than ack an edit the gate would swallow.
    if let Some(e) = frozen_reject(app, t.id) {
        return reject(request, e);
    }

    let source = match resolve_key_source(
        app,
        track::SET_SIDECHAIN,
        params.source_track_id,
        params.source_bus_id,
    ) {
        Ok(source) => source,
        Err(e) => return reject(request, e),
    };

    let instance_id = match resolve_keyed_plugin(
        app,
        &t,
        params.plugin_id.as_deref(),
        params.occurrence,
    ) {
        Ok(id) => id,
        Err(e) => return reject(request, e),
    };

    if let Err(e) = require_key_port(&t.plugins, instance_id, &format!("track {}", t.id)) {
        return reject(request, e);
    }

    let task = run_via_update(
        app,
        route_message(instance_id, Some(source), params.enabled),
    );
    (ack(app, request), task)
}

/// `track.clear_sidechain` — drop a plugin's key route so it falls back
/// to keying off its own input.
pub(super) fn clear_sidechain(
    app: &mut Resonance,
    request: &Request,
) -> (Response, Task<Message>) {
    let params: track::ClearSidechainParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(t) = find_track(app, params.track_id.0).cloned() else {
        return not_found_track(request, params.track_id.0);
    };
    // Same frozen-input rule as `set_sidechain`: clearing a key route
    // dispatches `SetPluginSidechain` too.
    if let Some(e) = frozen_reject(app, t.id) {
        return reject(request, e);
    }
    let instance_id = match resolve_keyed_plugin(
        app,
        &t,
        params.plugin_id.as_deref(),
        params.occurrence,
    ) {
        Ok(id) => id,
        Err(e) => return reject(request, e),
    };
    let task = run_via_update(app, route_message(instance_id, None, false));
    (ack(app, request), task)
}
