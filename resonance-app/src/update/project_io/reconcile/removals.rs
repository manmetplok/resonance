//! Roadmap group (6), step 3: what a diff restore removes (ARCH-01 A-13h).
//!
//! `Stage::Removals` runs after `Timeline` and before `Entities`, and only
//! does anything on the diff path (after a `ClearAll` there is nothing
//! left to remove). Two domains, edges before the entities they connect:
//!
//! 1. [`RoutingRemovals`] — every aux send and sidechain key route `old`
//!    has and `new` does not keep. The removal half of `routing::Sends` /
//!    `routing::SidechainRoutes`, moved ahead of the entity removals so no
//!    edge ever names a removed bus or plugin, in the engine or in the
//!    mirror. The engine would tolerate the other order (`RemoveBus`
//!    leaves the bus's sends dangling until the app removes them, which is
//!    what the live delete does on the `BusRemoved` echo), but the mirror
//!    would name a removed endpoint in between, and the removals-first
//!    rule of A-13e (a replacement edge is cycle-checked against a graph
//!    without the edge it replaces) holds for the whole restore this way.
//! 2. [`EntityRemovals`] — every plugin instance `new` does not keep
//!    (`entities::kept_plugins`), then every bus `new` lacks.
//!
//! Removing before adding also lets an instance whose chain or identity
//! changed be removed and re-added under the same id: the engine refuses
//! an add whose id is still live.
//!
//! Each removal is mirrored at once, and its echo is recorded in
//! `io.restore_echoes` so the echo handlers leave the mirror alone when it
//! lands — by then a later restore may have put the same id back.

use std::collections::HashSet;

use resonance_audio::types::{AudioCommand, BusId, PluginInstanceId, SendSource};

use super::entities::{kept_plugins, plugin_owners};
use super::{Reconcile, ReconcileCtx};
use crate::project::ProjectFile;
use crate::state::PluginLocator;
use crate::Resonance;

/// The key routes of `old` a diff restore keeps: the target plugin is kept
/// (same instance, same chain) and `new` still keys it. Every other route
/// of `old` is cleared by [`RoutingRemovals`]; `routing::SidechainRoutes`
/// compares only these against `new`, so a route onto a re-added instance
/// is set again.
pub(super) fn kept_route_plugins(old: &ProjectFile, new: &ProjectFile) -> HashSet<PluginInstanceId> {
    let kept = kept_plugins(Some(old), new);
    let keyed: HashSet<PluginInstanceId> =
        new.sidechain_routes.iter().map(|route| route.plugin_instance_id).collect();
    old.sidechain_routes
        .iter()
        .map(|route| route.plugin_instance_id)
        .filter(|id| kept.contains(id) && keyed.contains(id))
        .collect()
}

/// The routing edges a diff restore removes, before any entity goes.
///
/// * After a `ClearAll`: nothing (`routing::Sends` / `SidechainRoutes`
///   empty their mirrors themselves).
/// * Diff: `RemoveAuxSend` for every send `old` has and `new` lacks;
///   `ClearSidechainRoute` for every route of `old` that is not kept
///   ([`kept_route_plugins`]). Each mirrored at once. Echoes
///   (`AuxSendRemoved`, `SidechainRouteChanged`) find the mirror already
///   agreeing.
pub(crate) struct RoutingRemovals;

impl Reconcile for RoutingRemovals {
    const NAME: &'static str = "routing_removals";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, _ctx: &ReconcileCtx<'_>) {
        let Some(old) = old else {
            return;
        };
        let target_sends: HashSet<u64> = new.sends.iter().map(|s| s.id).collect();
        for sa in &old.sends {
            if !target_sends.contains(&sa.id) {
                let _ = r.engine.send(AudioCommand::RemoveAuxSend { send_id: sa.id });
                r.aux.remove(sa.id);
            }
        }
        let kept_routes = kept_route_plugins(old, new);
        for ra in &old.sidechain_routes {
            if !kept_routes.contains(&ra.plugin_instance_id) {
                let _ = r.engine.send(AudioCommand::ClearSidechainRoute {
                    plugin: ra.plugin_instance_id,
                });
                r.sidechain.clear_plugin(ra.plugin_instance_id);
            }
        }
    }
}

/// The plugin instances and busses a diff restore removes.
///
/// * After a `ClearAll`: nothing.
/// * Diff, in `old`'s chain order: every instance `new` does not keep
///   goes (`RemovePlugin` / `RemovePluginFromBus` /
///   `RemovePluginFromMaster`) — except one on a bus that goes too, which
///   `RemoveBus` drops with the bus (the engine sends no per-plugin echo
///   for those). Then every bus `new` lacks (`RemoveBus`). Tracks are
///   still gated by `structurally_compatible` (A-13i).
///
/// The mirror is pruned per entity, as the echo handlers
/// (`engine_events::plugins::*_removed`, `engine_events::tracks::bus_removed`)
/// would have: the slot, its cached blob, its parked params, its
/// side-index entry, the mixer's plugin / bus selection, a key route onto
/// it or keyed off the bus. An open editor goes with its slot
/// (`PluginSlotState::editor_open`) and its instance. The missing-plugin
/// warning is derived from the chains, so a removed missing slot leaves it
/// on its own. The output-destination picker and the side-index are
/// rebuilt by `EntityOrder` at the end of `Entities`.
pub(crate) struct EntityRemovals;

impl Reconcile for EntityRemovals {
    const NAME: &'static str = "entity_removals";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, _ctx: &ReconcileCtx<'_>) {
        let Some(old) = old else {
            return;
        };
        let kept = kept_plugins(Some(old), new);
        let target_busses: HashSet<BusId> = new.busses.iter().map(|b| b.id).collect();
        let removed_busses: Vec<BusId> = old
            .busses
            .iter()
            .map(|b| b.id)
            .filter(|id| !target_busses.contains(id))
            .collect();

        // `old`'s chain order, so the command sequence is deterministic.
        let owners = plugin_owners(old);
        let chain_order = old
            .tracks
            .iter()
            .flat_map(|t| t.plugins.iter())
            .chain(old.busses.iter().flat_map(|b| b.plugins.iter()))
            .chain(old.master_plugins.iter())
            .map(|pp| pp.instance_id);
        let mut removed_plugins: Vec<(PluginInstanceId, PluginLocator)> = Vec::new();
        for id in chain_order {
            if kept.contains(&id) {
                continue;
            }
            if let Some(&(owner, _)) = owners.get(&id) {
                removed_plugins.push((id, owner));
            }
        }

        for (instance_id, owner) in removed_plugins {
            let command = match owner {
                PluginLocator::Bus(bus_id) if removed_busses.contains(&bus_id) => None,
                PluginLocator::Track(track_id) => Some(AudioCommand::RemovePlugin {
                    track_id,
                    instance_id,
                }),
                PluginLocator::Bus(bus_id) => Some(AudioCommand::RemovePluginFromBus {
                    bus_id,
                    instance_id,
                }),
                PluginLocator::Master => Some(AudioCommand::RemovePluginFromMaster { instance_id }),
            };
            if let Some(command) = command {
                let _ = r.engine.send(command);
                r.io.restore_echoes.expect_plugin_removed(instance_id);
            }
            prune_plugin(r, owner, instance_id);
        }

        for bus_id in removed_busses {
            let _ = r.engine.send(AudioCommand::RemoveBus { bus_id });
            r.io.restore_echoes.expect_bus_removed(bus_id);
            prune_bus(r, bus_id);
        }
    }
}

/// What `engine_events::plugins::{track,bus,master}_removed` does to the
/// mirror for one instance.
fn prune_plugin(r: &mut Resonance, owner: PluginLocator, instance_id: PluginInstanceId) {
    let chain = match owner {
        PluginLocator::Track(id) => r
            .registry
            .tracks
            .iter_mut()
            .find(|t| t.id == id)
            .map(|t| &mut t.plugins),
        PluginLocator::Bus(id) => r
            .registry
            .busses
            .iter_mut()
            .find(|b| b.id == id)
            .map(|b| &mut b.plugins),
        PluginLocator::Master => Some(&mut r.master.plugins),
    };
    if let Some(chain) = chain {
        chain.retain(|p| p.instance_id != instance_id);
    }
    if r.ui.mixer.selected_plugin == Some(instance_id) {
        r.ui.mixer.selected_plugin = None;
    }
    r.plugin_mirror.state_cache.remove(&instance_id);
    // A re-add under this id (redo) parks its own copy; a stale one must
    // not be applied to it, nor written by the next save.
    r.presets.pending_plugin_param_overrides.remove(&instance_id);
    r.remove_plugin_index(instance_id);
    // `RoutingRemovals` cleared a route onto it in the engine already.
    r.sidechain.clear_plugin(instance_id);
}

/// What `engine_events::tracks::bus_removed` does to the mirror for one
/// bus, after its chain was pruned by [`prune_plugin`]. Its sends and the
/// key routes keyed off it are `RoutingRemovals`'; the tracks routed to it
/// take `new`'s output in `Tracks` (and `TrackOutputs` re-asserts it to
/// the engine, which moved them to the master on `RemoveBus`).
fn prune_bus(r: &mut Resonance, bus_id: BusId) {
    r.registry.busses.retain(|b| b.id != bus_id);
    if r.ui.mixer.selected_bus == Some(bus_id) {
        r.ui.mixer.selected_bus = None;
    }
    r.sidechain.drop_routes_from_source(SendSource::Bus(bus_id));
}
