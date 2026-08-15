//! `track.*` and `mixer.*` control methods (ba doc #265, todo #1152).
//!
//! Every mutation synthesizes the existing `TrackMessage` /
//! `PluginMessage` and routes it through the FULL `update()` path via
//! [`super::run_via_update`], so remote edits are undoable exactly like
//! the GUI (Cmd-Z reverts them). Params are validated before dispatch so
//! rejections are precise (`invalid_params` / `not_found` /
//! `needs_confirmation`). Mutating replies carry the revision counter;
//! `track.add`/`track.rename`/plugin ops also echo the updated
//! [`TrackSummary`](resonance_control::methods::song::TrackSummary) via
//! [`AddResult`](resonance_control::methods::track::AddResult) /
//! [`super::song`] helpers.
//!
//! Track creation is asynchronous engine-side (the registry mirrors the
//! track on the `TrackAdded` echo), so `track.add` allocates the id
//! app-side and passes it as `id_hint` — the same pattern the
//! external-instrument add uses — so the reply can return the real
//! `track_id` immediately.
//!
//! # Layout (ba todo #1253)
//!
//! This file holds only the dispatch table and the handful of helpers
//! every handler family needs ([`find_track`], [`instance_for`], plus
//! the layer-wide [`reply`](super::reply) vocabulary re-exported for the
//! submodules); the families themselves live one per submodule, each
//! with its own error vocabulary:
//!
//! | module | methods |
//! |---|---|
//! | [`lifecycle`] | `track.add` / `rename` / `delete` / `add_instrument` / `add_effect` |
//! | [`output`] | `track.set_output` |
//! | [`sends`] | `track.add_send` / `set_send` / `remove_send` |
//! | [`chain`] | `track.remove_effect` / `move_effect` (+ chain addressing) |
//! | [`params`] | `track.set_plugin_param` (+ the bound tolerance) |
//! | [`mixer`] | `mixer.set_volume` / `set_volume_db` / `set_pan` / `set_mute` / `set_solo` |
//! | [`sidechain`] | `track.set_sidechain` / `clear_sidechain` |

use crate::message::Message;
use crate::state::TrackState;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::mixer as mixer_methods;
use resonance_control::methods::track as track_methods;
use resonance_control::{Request, Response};

/// The reply vocabulary (todo #1258): every family answers with these,
/// and they are re-exported here so the submodules keep saying
/// `super::ack` / `super::reject` / `super::not_found_track`.
pub(super) use super::reply::{ack, not_found_track, reject};

mod chain;
mod lifecycle;
mod mixer;
mod output;
mod params;
mod sends;
mod sidechain;

/// Shared with `bus.*` / `master.*`, which address plugin parameters the
/// same way: same f32-bound tolerance, same choice-label resolution.
pub(crate) use params::resolve_param_value;

/// Handle a `track.*` / `mixer.*` request, or `None` when `method`
/// belongs to another namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let out = match request.method.as_str() {
        track_methods::ADD => lifecycle::add(app, request),
        track_methods::RENAME => lifecycle::rename(app, request),
        track_methods::DELETE => lifecycle::delete(app, request),
        track_methods::ADD_INSTRUMENT => lifecycle::add_instrument(app, request),
        track_methods::ADD_EFFECT => lifecycle::add_effect(app, request),
        track_methods::REMOVE_EFFECT => chain::remove_effect(app, request),
        track_methods::MOVE_EFFECT => chain::move_effect(app, request),
        track_methods::SET_OUTPUT => output::set_output(app, request),
        track_methods::ADD_SEND => sends::add_send(app, request),
        track_methods::SET_SEND => sends::set_send(app, request),
        track_methods::REMOVE_SEND => sends::remove_send(app, request),
        track_methods::SET_PLUGIN_PARAM => params::set_plugin_param(app, request),
        track_methods::SET_SIDECHAIN => sidechain::set_sidechain(app, request),
        track_methods::CLEAR_SIDECHAIN => sidechain::clear_sidechain(app, request),
        mixer_methods::SET_VOLUME => mixer::set_volume(app, request),
        mixer_methods::SET_VOLUME_DB => mixer::set_volume_db(app, request),
        mixer_methods::SET_PAN => mixer::set_pan(app, request),
        mixer_methods::SET_MUTE => mixer::set_mute(app, request),
        mixer_methods::SET_SOLO => mixer::set_solo(app, request),
        _ => return None,
    };
    Some(out)
}

/// Look up a live track by wire id, or `None`.
fn find_track(app: &Resonance, id: u64) -> Option<&TrackState> {
    app.registry.tracks.iter().find(|t| t.id == id)
}

/// The engine instance id of the `occurrence`-th plugin with this CLAP
/// id on the track.
///
/// The instance id is the engine's handle and is not on the wire, so
/// every family that addresses a plugin by CLAP id ([`chain`],
/// [`params`], [`sidechain`]) recovers it from the same chain position
/// the wire entry came from — through this one function, so the three
/// cannot drift.
fn instance_for(
    t: &TrackState,
    plugin_id: &str,
    occurrence: u32,
) -> Option<resonance_audio::types::PluginInstanceId> {
    t.plugins
        .iter()
        .filter(|p| p.clap_plugin_id == plugin_id)
        .nth(occurrence as usize)
        .map(|p| p.instance_id)
}
