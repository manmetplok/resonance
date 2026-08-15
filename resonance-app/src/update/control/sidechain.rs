//! The rules every `*.set_sidechain` / `*.clear_sidechain` handler
//! shares, in one place (ba doc #275 P4, todo #1311).
//!
//! Key routing reaches the control API from three namespaces now —
//! `track.*`, `bus.*` and `master.*` — because a keyed compressor lives
//! on a bus at least as often as on a track, and the engine's route
//! table never cared where the plugin sat: it is keyed by plugin
//! instance, and the mixer connects a key port wherever it finds one.
//! Only the control layer was track-shaped.
//!
//! Three namespaces means three chances for the rules to drift, and two
//! of the rules are the ones ba doc #275 P0 was filed about — the ones
//! that stop a route being accepted and then silently ignored. So they
//! live here, once:
//!
//! - **exactly one source**, validated against the live registry
//!   ([`resolve_key_source`]);
//! - **the target must declare a key port**, or the call is refused with
//!   the chain's keyable plugins named ([`require_key_port`]);
//! - **an unqualified call targets the first keyable plugin**, not slot 0
//!   ([`first_keyable`]).
//!
//! What is NOT here is how each namespace finds its chain — a track's is
//! `TrackState::plugins`, a bus's is `BusState::plugins`, the master's is
//! `Resonance::master_plugins` — and the track path additionally keeps
//! its instrument-aware fallback for error wording. Those stay with
//! their namespace.

use crate::message::{Message, PluginMessage};
use crate::state::PluginSlotState;
use crate::Resonance;
use resonance_audio::types::{PluginInstanceId, SendSource};
use resonance_control::ids::TrackId;
use resonance_control::RpcError;

/// Resolve the `source_track_id` / `source_bus_id` pair into the one
/// [`SendSource`] a route may name, checking that it exists.
///
/// Exactly one must be given. Both, or neither, is `invalid_params`
/// rather than a guess: track ids and bus ids are independent namespaces
/// that both start at 1, so picking one for the caller would key the
/// detector off a *different channel that probably exists*, which is far
/// harder to notice than an error.
pub(super) fn resolve_key_source(
    app: &Resonance,
    method: &str,
    source_track_id: Option<TrackId>,
    source_bus_id: Option<TrackId>,
) -> Result<SendSource, RpcError> {
    match (source_track_id, source_bus_id) {
        (Some(track), None) => {
            if !app.registry.tracks.iter().any(|t| t.id == track.0) {
                return Err(RpcError::not_found(format!(
                    "no source track with id {track}"
                )));
            }
            Ok(SendSource::Track(track.0))
        }
        (None, Some(bus)) => {
            if !app.registry.busses.iter().any(|b| b.id == bus.0) {
                return Err(RpcError::not_found(format!("no source bus with id {bus}")));
            }
            Ok(SendSource::Bus(bus.0))
        }
        (Some(_), Some(_)) => Err(RpcError::invalid_params(
            "give exactly one of source_track_id or source_bus_id, not both",
        )),
        (None, None) => Err(RpcError::invalid_params(format!(
            "{method} needs a source_track_id or a source_bus_id"
        ))),
    }
}

/// The first plugin on `chain` that declares a sidechain (key) input.
///
/// This is what an omitted `plugin_id` resolves to. Addressing slot 0
/// instead — which is what the sidechain methods used to inherit from
/// `set_plugin_param` — sent every unqualified call at a track's
/// instrument, the one plugin in the chain guaranteed to have no key
/// port (ba doc #275 P0).
pub(super) fn first_keyable(chain: &[PluginSlotState]) -> Option<PluginInstanceId> {
    chain
        .iter()
        .find(|p| p.has_sidechain_input)
        .map(|p| p.instance_id)
}

/// Refuse a key route onto a plugin instance with no sidechain input,
/// naming the plugins on `host`'s chain that do have one.
///
/// The flag comes from the engine's `PluginAdded` / `BusPluginAdded` /
/// `MasterPluginAdded` echo — the same `has_sidechain_input` the mixer
/// keys off — so this predicate cannot drift from the one that decides
/// delivery.
///
/// Storing the route anyway is the failure mode ba doc #275 reports as
/// most expensive: the call succeeds, the audio is unchanged, and a
/// client with no ears has nothing to go on. This is the one place that
/// can say so.
///
/// An instance not on `chain` passes: it is not this check's job to
/// decide whether the target exists, and the caller has already resolved
/// it.
pub(super) fn require_key_port(
    chain: &[PluginSlotState],
    instance_id: PluginInstanceId,
    host: &str,
) -> Result<(), RpcError> {
    let Some(slot) = chain.iter().find(|p| p.instance_id == instance_id) else {
        return Ok(());
    };
    if slot.has_sidechain_input {
        return Ok(());
    }
    let keyable: Vec<&str> = chain
        .iter()
        .filter(|p| p.has_sidechain_input)
        .map(|p| p.clap_plugin_id.as_str())
        .collect();
    let hint = if keyable.is_empty() {
        format!(
            "no plugin on {host} declares one — add a plugin that does \
             (com.resonance.compressor, com.resonance.gate) and route the key into that"
        )
    } else {
        format!(
            "plugins on {host} that accept a key: [{}]",
            keyable.join(", ")
        )
    };
    Err(RpcError::invalid_params(format!(
        "plugin {} on {host} declares no sidechain (key) input, so a key routed into it \
         would be silently ignored; {hint}",
        slot.clap_plugin_id
    )))
}

/// The message that applies (or clears) a route. Routed through the full
/// `update()` path by each handler, so a remote key edit is undoable
/// exactly like a GUI one.
pub(super) fn route_message(
    instance_id: PluginInstanceId,
    source: Option<SendSource>,
    enabled: bool,
) -> Message {
    Message::Plugin(PluginMessage::SetPluginSidechain {
        instance_id,
        source,
        enabled,
    })
}

/// Resolve "which plugin on this chain" for a bus or master key route.
///
/// Simpler than the track resolver, which has an instrument slot to
/// reason about: a bus/master chain is effects only, so an omitted
/// `plugin_id` means "the keyable one" and nothing else could be meant.
pub(super) fn resolve_chain_target(
    chain: &[PluginSlotState],
    plugin_id: Option<&str>,
    occurrence: Option<u32>,
    host: &str,
) -> Result<PluginInstanceId, RpcError> {
    let Some(wanted) = plugin_id else {
        return first_keyable(chain).ok_or_else(|| {
            RpcError::invalid_params(format!(
                "no plugin on {host} declares a sidechain (key) input; it carries [{}]",
                chain_description(chain)
            ))
        });
    };
    let occurrence = occurrence.unwrap_or(0);
    chain
        .iter()
        .filter(|p| p.clap_plugin_id == wanted)
        .nth(occurrence as usize)
        .map(|p| p.instance_id)
        .ok_or_else(|| {
            RpcError::not_found(format!(
                "{host} has no plugin {wanted:?} at occurrence {occurrence}; it carries [{}]",
                chain_description(chain)
            ))
        })
}

/// A chain's CLAP ids, for the "it carries [...]" tail of an error.
fn chain_description(chain: &[PluginSlotState]) -> String {
    chain
        .iter()
        .map(|p| p.clap_plugin_id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
