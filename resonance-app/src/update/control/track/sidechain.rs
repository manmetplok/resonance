//! `track.set_sidechain` / `track.clear_sidechain` — the key route into
//! a plugin's external sidechain input.

use super::{ack, find_track, instance_for, not_found_track, reject};
use crate::message::{Message, PluginMessage};
use crate::state::TrackState;
use crate::update::control::{run_via_update, song};
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
        if let Some(slot) = t.plugins.iter().find(|p| p.has_sidechain_input) {
            return Ok(slot.instance_id);
        }
    }

    let entries = song::plugin_entries(app, t);
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
            Some(id) => song::unknown_plugin_on_track(app, t, id, occurrence),
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
/// the output. Plugins that declare no key port (most of them) store the
/// route inertly — the mixer only connects a port the plugin actually
/// declared — so this is not rejected on plugin kind, which would break
/// the moment an instance's plugin were swapped.
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

    // Exactly one source.
    let source = match (params.source_track_id, params.source_bus_id) {
        (Some(track), None) => {
            if find_track(app, track.0).is_none() {
                return reject(
                    request,
                    RpcError::not_found(format!("no source track with id {track}")),
                );
            }
            resonance_audio::types::SendSource::Track(track.0)
        }
        (None, Some(bus)) => {
            if !app.registry.busses.iter().any(|b| b.id == bus.0) {
                return reject(
                    request,
                    RpcError::not_found(format!("no source bus with id {bus}")),
                );
            }
            resonance_audio::types::SendSource::Bus(bus.0)
        }
        (Some(_), Some(_)) => {
            return reject(
                request,
                RpcError::invalid_params(
                    "give exactly one of source_track_id or source_bus_id, not both",
                ),
            )
        }
        (None, None) => {
            return reject(
                request,
                RpcError::invalid_params(
                    "track.set_sidechain needs a source_track_id or a source_bus_id",
                ),
            )
        }
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

    // A plugin that declares no key port can never be handed one — the
    // mixer drops the key rather than connect a port the plugin never
    // declared. Storing the route anyway is the failure mode ba doc #275
    // reports as most expensive: the call succeeds, the audio is
    // unchanged, and a client with no ears has nothing to go on. This is
    // the one place that can say so, so it refuses here.
    if let Err(e) = require_key_port(&t, instance_id) {
        return reject(request, e);
    }

    let task = run_via_update(
        app,
        Message::Plugin(PluginMessage::SetPluginSidechain {
            instance_id,
            source: Some(source),
            enabled: params.enabled,
        }),
    );
    (ack(app, request), task)
}

/// Refuse a key route onto a plugin instance with no sidechain input,
/// naming the plugins on this track that do have one.
///
/// The flag comes from the engine's `PluginAdded` echo (the same
/// `has_sidechain_input` the mixer keys off), so this predicate cannot
/// drift from the one that decides delivery.
fn require_key_port(
    t: &TrackState,
    instance_id: resonance_audio::types::PluginInstanceId,
) -> Result<(), RpcError> {
    let Some(slot) = t.plugins.iter().find(|p| p.instance_id == instance_id) else {
        return Ok(());
    };
    if slot.has_sidechain_input {
        return Ok(());
    }
    let keyable: Vec<&str> = t
        .plugins
        .iter()
        .filter(|p| p.has_sidechain_input)
        .map(|p| p.clap_plugin_id.as_str())
        .collect();
    let hint = if keyable.is_empty() {
        "no plugin on this track declares one — add a plugin that does \
         (com.resonance.compressor, com.resonance.gate) and route the key into that"
            .to_string()
    } else {
        format!("plugins on this track that accept a key: [{}]", keyable.join(", "))
    };
    Err(RpcError::invalid_params(format!(
        "plugin {} on track {} declares no sidechain (key) input, so a key routed \
         into it would be silently ignored; {hint}",
        slot.clap_plugin_id, t.id
    )))
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
    let instance_id = match resolve_keyed_plugin(
        app,
        &t,
        params.plugin_id.as_deref(),
        params.occurrence,
    ) {
        Ok(id) => id,
        Err(e) => return reject(request, e),
    };
    let task = run_via_update(
        app,
        Message::Plugin(PluginMessage::SetPluginSidechain {
            instance_id,
            source: None,
            enabled: false,
        }),
    );
    (ack(app, request), task)
}
