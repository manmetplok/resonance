//! Insert-chain handlers shared by every [`ChainOwner`] — add, remove and
//! reorder a plugin instance, and the whole-chain FX bypass (code review
//! ARCH2-02).
//!
//! These used to be three near-identical copies: `plugins.rs` for a
//! track, `busses.rs` for a bus, `master.rs` for the master. The one
//! thing that genuinely differs is *where the chain lives*: a track's is
//! an `ArcSwap` on the shared `Track` (published with one `swap`, no
//! render-graph publish), while a bus's and the master's are plain `Vec`s
//! on copy-on-write graph nodes (`edit_bus` / `edit_master`). Everything
//! else — bundle lookup, instance creation, the duplicate-id refusal, the
//! load-failure report, slot publication, the clamped-index echo — is the
//! same, and lives here once. The per-owner chain edit is a `match` in
//! [`push`], [`retain`], [`move_within`] and [`handle_set_fx_bypass`].

use std::path::Path;
use std::sync::Arc;

use crate::types::*;

use super::plugins::{
    apply_bypass_request, ensure_bundle, reject_if_plugin_id_in_use, remove_plugin_slots,
    report_plugin_load_failure, resolve_plugin_id,
};
use super::thread::{HandlerCtx, HandlerState};

pub(crate) fn handle_add_plugin(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    owner: ChainOwner,
    clap_file_path: String,
    clap_plugin_id: String,
    id: PluginInstanceId,
) {
    if reject_if_plugin_id_in_use(ctx, id, &clap_plugin_id) {
        return;
    }

    let path = Path::new(&clap_file_path);

    let bundle_idx = match ensure_bundle(&mut state.bundles, path, &clap_plugin_id) {
        Ok(idx) => idx,
        Err(reason) => {
            report_plugin_load_failure(
                ctx,
                Some(id),
                &clap_plugin_id,
                &clap_file_path,
                reason.to_string(),
            );
            return;
        }
    };

    let actual_plugin_id =
        match resolve_plugin_id(&state.bundles[bundle_idx], clap_plugin_id.clone()) {
            Ok(resolved) => resolved,
            Err(reason) => {
                report_plugin_load_failure(
                    ctx,
                    Some(id),
                    &clap_plugin_id,
                    &clap_file_path,
                    reason.to_string(),
                );
                return;
            }
        };

    let plugin_name = state.bundles[bundle_idx]
        .descriptors()
        .iter()
        .find(|d| d.id == actual_plugin_id)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| actual_plugin_id.clone());

    match state.bundles[bundle_idx].create_instance(&actual_plugin_id, ctx.sample_rate) {
        Ok(instance) => {
            let instance_id = id;

            // Query params + has_gui + output port layout before moving
            // instance into shared map.
            let params = instance.query_params();
            let has_gui = instance.has_gui();
            let has_sidechain_input = instance.has_sidechain_input();
            let output_port_count = instance.output_port_count();
            let output_port_names = instance.output_port_names();

            // Publish the slot first, then name it on the chain: a block
            // between the two sees the slot unused, never a chain id with
            // no instance behind it.
            let slot = Arc::new(crate::clap_host::PluginSlot::new(instance));
            ctx.shared.edit_plugins(|plugins| plugins.insert(instance_id, slot));
            push(ctx, owner, instance_id);

            let _ = ctx.event_tx.send(AudioEvent::PluginAdded {
                owner,
                instance_id,
                plugin_name,
                clap_plugin_id: actual_plugin_id,
                clap_file_path,
                params,
                has_gui,
                has_sidechain_input,
                output_port_count,
                output_port_names,
            });
        }
        Err(e) => report_plugin_load_failure(
            ctx,
            Some(id),
            &actual_plugin_id,
            &clap_file_path,
            format!("Failed to create plugin instance: {}", e),
        ),
    }
}

/// Take `instance_id` off `owner`'s chain, unpublish the instance and
/// drop its sidechain (key) route. The slot is retired, not dropped: the
/// engine loop's sweep destroys it once no block pins it (B-4).
///
/// The route goes on every owner. Until ARCH2-02 only the track arm did
/// this and the app sent a separate `ClearSidechainRoute` for a bus or
/// master removal — the asymmetry this module exists to end. Without it
/// a stale route survives, and instance ids are recycled, so the next
/// plugin to land on the id would silently inherit someone else's key.
pub(crate) fn handle_remove_plugin(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    owner: ChainOwner,
    instance_id: PluginInstanceId,
) {
    retain(ctx, owner, instance_id);
    remove_plugin_slots(ctx.shared, &[instance_id]);
    let _ = ctx.event_tx.send(AudioEvent::PluginRemoved { owner, instance_id });
    super::sidechain::drop_plugin_route(ctx, state, instance_id);
}

/// Reorder `owner`'s chain. The plugin instances themselves are untouched,
/// only the order they are visited in; no lock is held across a
/// `process()` call and the audio thread never allocates. A plugin that is
/// not on the chain publishes nothing and reports `not_found`.
pub(crate) fn handle_move_plugin(
    ctx: &HandlerCtx,
    owner: ChainOwner,
    instance_id: PluginInstanceId,
    to_index: usize,
) {
    match move_within(ctx, owner, instance_id, to_index) {
        // Report the *clamped* index so the app mirrors what the engine
        // actually did rather than what was requested.
        Some(to_index) => {
            let _ = ctx.event_tx.send(AudioEvent::PluginMoved {
                owner,
                instance_id,
                to_index,
            });
        }
        None => {
            let _ = ctx.event_tx.send(AudioEvent::Error(EngineError::not_found(format!(
                "Cannot reorder plugin {instance_id} on {owner}: no such {}, or that \
                 plugin is not on its chain",
                match owner {
                    ChainOwner::Track(_) => "track",
                    ChainOwner::Bus(_) => "bus",
                    ChainOwner::Master => "chain",
                }
            ))));
        }
    }
}

/// Crossfade `owner`'s whole chain out (or back in). Echoed even for an
/// unknown owner, as every owner's handler always did.
pub(crate) fn handle_set_fx_bypass(ctx: &HandlerCtx, owner: ChainOwner, bypassed: bool) {
    match owner {
        ChainOwner::Track(track_id) => {
            if let Some(track) = ctx.tracks().get(&track_id) {
                apply_bypass_request(ctx.shared, track.fx_bypass(), bypassed);
            }
        }
        ChainOwner::Bus(bus_id) => {
            if let Some(bus) = ctx.shared.graph.load().bus(bus_id) {
                apply_bypass_request(ctx.shared, bus.fx_bypass(), bypassed);
            }
        }
        ChainOwner::Master => {
            apply_bypass_request(ctx.shared, &ctx.shared.master_fx_bypass, bypassed);
        }
    }
    let _ = ctx
        .event_tx
        .send(AudioEvent::FxBypassChanged { owner, bypassed });
}

// ---------------------------------------------------------------------------
// The one place the three chain representations differ
// ---------------------------------------------------------------------------

/// Append `instance_id` to `owner`'s chain. An unknown owner is a no-op:
/// the slot stays published (and is reaped by the next `ClearAll` or
/// remove), exactly as before.
fn push(ctx: &HandlerCtx, owner: ChainOwner, instance_id: PluginInstanceId) {
    match owner {
        // `push_plugin` publishes the new chain via `ArcSwap::store`
        // (shared by every copy of the track), so no render-graph
        // publish — the audio thread is not blocked by the edit.
        ChainOwner::Track(track_id) => {
            if let Some(track) = ctx.tracks().get(&track_id) {
                ctx.shared.retired.retire(track.push_plugin(instance_id));
            }
        }
        ChainOwner::Bus(bus_id) => {
            ctx.shared
                .edit_bus(bus_id, |bus| bus.plugin_ids.push(instance_id));
        }
        ChainOwner::Master => {
            ctx.shared
                .edit_master(|master| master.plugin_ids.push(instance_id));
        }
    }
}

/// Drop `instance_id` from `owner`'s chain, publishing only when the
/// chain actually held it.
fn retain(ctx: &HandlerCtx, owner: ChainOwner, instance_id: PluginInstanceId) {
    match owner {
        // `retain_plugins` publishes a new chain via `ArcSwap::store`
        // (shared by every copy of the track), so reading the published
        // track map is enough — no render-graph publish, and the audio
        // thread is never blocked on the chain edit.
        ChainOwner::Track(track_id) => {
            if let Some(track) = ctx.tracks().get(&track_id) {
                ctx.shared
                    .retired
                    .retire(track.retain_plugins(|&id| id != instance_id));
            }
        }
        ChainOwner::Bus(bus_id) => {
            let on_chain = ctx
                .shared
                .graph
                .load()
                .bus(bus_id)
                .is_some_and(|bus| bus.plugin_ids.contains(&instance_id));
            if on_chain {
                ctx.shared
                    .edit_bus(bus_id, |bus| bus.plugin_ids.retain(|&id| id != instance_id));
            }
        }
        ChainOwner::Master => {
            if ctx.shared.graph.load().master.plugin_ids.contains(&instance_id) {
                ctx.shared
                    .edit_master(|master| master.plugin_ids.retain(|&id| id != instance_id));
            }
        }
    }
}

/// Move `instance_id` to `to_index` within `owner`'s chain. The slot it
/// landed on after clamping, or `None` when the owner is unknown or the
/// chain does not hold the instance — in which case nothing is published.
fn move_within(
    ctx: &HandlerCtx,
    owner: ChainOwner,
    instance_id: PluginInstanceId,
    to_index: usize,
) -> Option<usize> {
    match owner {
        // `move_plugin_into` builds the reordered Vec here on the engine
        // thread and publishes it with one `ArcSwap::store`, so reading
        // the published track map is enough.
        ChainOwner::Track(track_id) => ctx.tracks().get(&track_id).and_then(|track| {
            track.move_plugin_into(instance_id, to_index, |old| ctx.shared.retired.retire(old))
        }),
        // A bus chain is a plain `Vec` on the bus: copy-on-write the bus
        // in a new render graph (ba doc #273, todo #1237).
        ChainOwner::Bus(bus_id) => {
            let on_chain = ctx
                .shared
                .graph
                .load()
                .bus(bus_id)
                .is_some_and(|bus| bus.plugin_ids.contains(&instance_id));
            on_chain
                .then(|| {
                    ctx.shared
                        .edit_bus(bus_id, |bus| bus.move_plugin(instance_id, to_index))
                        .flatten()
                })
                .flatten()
        }
        ChainOwner::Master => {
            let on_chain = ctx.shared.graph.load().master.plugin_ids.contains(&instance_id);
            on_chain
                .then(|| {
                    ctx.shared
                        .edit_master(|master| master.move_plugin(instance_id, to_index))
                })
                .flatten()
        }
    }
}
