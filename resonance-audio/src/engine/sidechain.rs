//! Engine-thread handlers for external sidechain (key) routing.
//!
//! The authoritative table lives here, on the control thread, keyed by
//! plugin instance — one key source per instance, because a detector has
//! exactly one input. Every mutation republishes an immutable snapshot
//! into [`SharedState::sidechain_routes`](crate::engine::SharedState) via
//! `ArcSwap`, so the render path reads it wait-free and never locks.
//!
//! Whether a route actually does anything is decided at render time: the
//! mixer only connects a key port on a plugin that declares one. Storing
//! a route for a plugin without a key port is therefore harmless rather
//! than an error — which matters because the plugin at an instance id can
//! be swapped underneath a stored route.

use std::collections::HashMap;

use crate::types::{AudioEvent, PluginInstanceId, SendSource, SidechainRoute};

use super::thread::{HandlerCtx, HandlerState};

/// Engine-thread-local route table, one entry per plugin instance.
pub type SidechainRoutes = HashMap<PluginInstanceId, SidechainRoute>;

/// Republish the audio-thread-visible snapshot. Called after every
/// mutation so the render path never sees a half-updated table.
pub(crate) fn publish(ctx: &HandlerCtx, routes: &SidechainRoutes) {
    let snapshot: Vec<SidechainRoute> = routes.values().copied().collect();
    ctx.shared
        .sidechain_routes
        .store(std::sync::Arc::new(snapshot));
}

/// Store (or replace) `plugin`'s key route and echo the result.
pub(crate) fn handle_set(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    plugin: PluginInstanceId,
    source: SendSource,
    enabled: bool,
) {
    state.sidechain_routes.insert(
        plugin,
        SidechainRoute {
            plugin,
            source,
            enabled,
        },
    );
    publish(ctx, &state.sidechain_routes);
    let _ = ctx.event_tx.send(AudioEvent::SidechainRouteChanged {
        plugin,
        source: Some(source),
        enabled,
    });
}

/// Drop `plugin`'s key route. Emits the echo only when a route was
/// actually present, matching the "missing lookup ⇒ no event" convention
/// the other handlers follow.
pub(crate) fn handle_clear(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    plugin: PluginInstanceId,
) {
    if state.sidechain_routes.remove(&plugin).is_some() {
        publish(ctx, &state.sidechain_routes);
        let _ = ctx.event_tx.send(AudioEvent::SidechainRouteChanged {
            plugin,
            source: None,
            enabled: false,
        });
    }
}

/// Drop the route belonging to a plugin instance that has just been
/// removed. Without this a stale route survives, and instance ids are
/// recycled — so the next plugin to land on that id would silently
/// inherit someone else's key.
pub(crate) fn drop_plugin_route(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    plugin: PluginInstanceId,
) {
    if state.sidechain_routes.remove(&plugin).is_some() {
        publish(ctx, &state.sidechain_routes);
    }
}

/// Drop every route keyed from a track or bus that has just been deleted,
/// for the same reason: a route pointing at a recycled source id would
/// quietly start ducking from the wrong channel.
pub(crate) fn drop_source_routes(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    source: SendSource,
) {
    let before = state.sidechain_routes.len();
    state.sidechain_routes.retain(|_, r| r.source != source);
    if state.sidechain_routes.len() != before {
        publish(ctx, &state.sidechain_routes);
    }
}
