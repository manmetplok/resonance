//! Roadmap group (6): each plugin instance's state — its opaque blob, its
//! per-slot bypass and its parameter values (ARCH-01 A-13f).
//!
//! One domain, [`PluginState`], after every chain has been added (full
//! path) or matched (diff path) by the entity domains, and before
//! `Routing`. Three phases, in this order on every origin:
//!
//! 1. **Blobs** — `LoadPluginState` + the app-side `state_cache` copy.
//! 2. **Bypass** — `SetPluginBypass`. After the blob, so a state load can
//!    never land on top of a bypass the render path has already pushed
//!    into a plugin's own bypass parameter (`PluginSlot::sync_own_bypass`
//!    is edge-triggered and would not push it again).
//! 3. **Params** — parked for the `PluginAdded` echo after a `ClearAll`,
//!    driven directly on the diff path. After the blob, which they win
//!    over.

use std::collections::HashSet;
use std::sync::Arc;

use resonance_audio::types::{AudioCommand, PluginInstanceId};

use super::{Reconcile, ReconcileCtx};
use crate::project::{ProjectFile, ProjectPlugin};
use crate::state::PluginSlotState;
use crate::Resonance;

/// Plugin state blobs, per-slot bypass and parameter values.
///
/// * After a `ClearAll`: every saved blob goes out (the instances are
///   fresh) and is cached app-side; every bypassed slot gets
///   `SetPluginBypass` (the engine default is running — the placeholder
///   slots `Tracks` / `Busses` / `Master` created already show it
///   bypassed); every saved param override is parked in
///   `pending_plugin_param_overrides` for the instance's `PluginAdded`
///   echo, which runs after this restore returns and would otherwise
///   overwrite the values with the plugin's defaults.
/// * Diff: a blob goes out only when the live cache moved on since the
///   snapshot (FU-A2b); bypass is sent for the slots that changed (ba
///   todo #1305); params are driven to the snapshot's values (STATE-03,
///   FU-A2b).
///
/// A plugin that never comes back (missing `.clap`): the blob in
/// `state_cache` and the parked params are then the only surviving copy
/// of its settings, which every project-writing path reads back (ba doc
/// #275, P5).
pub(crate) struct PluginState;

impl Reconcile for PluginState {
    const NAME: &'static str = "plugin_state";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        let after_clear_all = old.is_none();

        // 1. Blobs.
        let mut pushed = HashSet::new();
        for pp in all_plugins(new) {
            let Some(blob) = ctx.plugin_states.get(&pp.instance_id) else {
                continue;
            };
            // Diff path: when the snapshot holds the very blob still
            // cached (`Arc::ptr_eq`), nothing but the params can have
            // drifted — the blob is refreshed at every point that changes
            // anything else — so pushing it would only reset the params
            // phase 3 has to repair (FU-A2b). After a `ClearAll` the
            // instance is fresh, so every blob goes.
            if !after_clear_all
                && r.plugin_mirror
                    .state_cache
                    .get(&pp.instance_id)
                    .is_some_and(|live| Arc::ptr_eq(live, blob))
            {
                continue;
            }
            let _ = r.engine.send(AudioCommand::LoadPluginState {
                instance_id: pp.instance_id,
                data: blob.to_vec(),
            });
            pushed.insert(pp.instance_id);
            // Keep the blob app-side, byte for byte, so the cache matches
            // the state the engine now holds. A live plugin overwrites this
            // entry with its own fresh blob on the `PluginStateSaved` echo
            // that follows `PluginAdded`; a missing one never does, and
            // this copy is what the next save — and undo — writes.
            r.plugin_mirror.state_cache.insert(pp.instance_id, Arc::clone(blob));
        }

        // 2. Bypass.
        if after_clear_all {
            // Through the very same command a user toggle sends, so there
            // is one path into the engine. Only when bypassed: the
            // engine's default is running, and a command per slot on every
            // load would be noise.
            for pp in all_plugins(new).filter(|pp| pp.bypassed) {
                let _ = r.engine.send(AudioCommand::SetPluginBypass {
                    instance_id: pp.instance_id,
                    bypassed: true,
                });
            }
        } else {
            for pt in &new.tracks {
                if let Some(t) = r.registry.tracks.iter_mut().find(|t| t.id == pt.id) {
                    apply_plugin_bypass(&r.engine, &mut t.plugins, &pt.plugins);
                }
            }
            for pb in &new.busses {
                if let Some(b) = r.registry.busses.iter_mut().find(|b| b.id == pb.id) {
                    apply_plugin_bypass(&r.engine, &mut b.plugins, &pb.plugins);
                }
            }
            apply_plugin_bypass(&r.engine, &mut r.master.plugins, &new.master_plugins);
        }

        // 3. Params.
        if after_clear_all {
            // Park the saved parameter overrides until `PluginAdded`
            // reports this instance's param list. Applying them here would
            // be undone: that event carries the values the plugin
            // instantiated with (its defaults) and overwrites `slot.params`
            // wholesale. See `Resonance::pending_plugin_param_overrides`.
            for pp in all_plugins(new).filter(|pp| !pp.params.is_empty()) {
                r.presets
                    .pending_plugin_param_overrides
                    .insert(pp.instance_id, pp.params.clone());
            }
        } else {
            // The blob just pushed is only as fresh as its last refresh
            // (plugin add, editor close, save) — never a param edit — so
            // the snapshot's own values are re-applied after it, to the
            // mirror and the engine, exactly as a load does. Without this
            // an undone knob kept its value on screen and in the next save
            // while the engine sat on the stale blob (STATE-03). A plugin
            // whose blob was not pushed only needs the params that differ
            // from the live mirror (FU-A2b).
            apply_all_plugin_params(r, new, &pushed);
        }
    }
}

/// Every plugin `file` carries: track chains, bus chains, the master
/// chain, each in chain order.
fn all_plugins(file: &ProjectFile) -> impl Iterator<Item = &ProjectPlugin> {
    file.tracks
        .iter()
        .flat_map(|t| t.plugins.iter())
        .chain(file.busses.iter().flat_map(|b| b.plugins.iter()))
        .chain(file.master_plugins.iter())
}

/// Apply the saved per-slot bypass to one live chain, telling the engine
/// about every slot that actually moved (ba todo #1305).
///
/// This is what makes bypass UNDOABLE rather than merely persisted. Undo
/// restores through the diff replay, not through a reload, and the diff
/// used to copy only `plugin_name` per slot — so an undo of a bypass
/// recorded its entry, replayed, and changed nothing. `plugin_set_matches`
/// compares slot IDENTITY only, deliberately: a bypass-only change is not
/// a structural change and must not force the whole project to reload.
/// That means the difference has to be applied here, or nowhere.
///
/// Sends only on a real change. The engine crossfades a bypass, and
/// re-asserting the state a slot is already in would start a fade for a
/// value that is not moving.
fn apply_plugin_bypass(
    engine: &resonance_audio::AudioEngine,
    slots: &mut [PluginSlotState],
    saved: &[ProjectPlugin],
) {
    for (slot, pp) in slots.iter_mut().zip(saved.iter()) {
        if slot.bypassed == pp.bypassed {
            continue;
        }
        slot.bypassed = pp.bypassed;
        let _ = engine.send(AudioCommand::SetPluginBypass {
            instance_id: slot.instance_id,
            bypassed: pp.bypassed,
        });
    }
}

fn apply_all_plugin_params(r: &mut Resonance, b: &ProjectFile, pushed: &HashSet<PluginInstanceId>) {
    for track in r.registry.tracks.iter_mut() {
        if let Some(pt) = b.tracks.iter().find(|t| t.id == track.id) {
            apply_plugin_params(&r.engine, &mut track.plugins, &pt.plugins, pushed);
        }
    }
    for bus in r.registry.busses.iter_mut() {
        if let Some(pb) = b.busses.iter().find(|x| x.id == bus.id) {
            apply_plugin_params(&r.engine, &mut bus.plugins, &pb.plugins, pushed);
        }
    }
    apply_plugin_params(&r.engine, &mut r.master.plugins, &b.master_plugins, pushed);
}

/// Drive every live slot's params to the snapshot's values: the saved
/// override where there is one, the plugin's default otherwise (only
/// non-defaults are saved). For a slot whose blob was just `pushed`, a
/// param is re-sent when it is non-default on either side — that covers
/// every changed value, and every value the stale blob may have reset.
/// For any other slot the engine still holds the live values the mirror
/// shows, so only the params that differ from the mirror are sent
/// (FU-A2b). A slot with no live params (a missing `.clap`) has nothing
/// to drive.
fn apply_plugin_params(
    engine: &resonance_audio::AudioEngine,
    slots: &mut [PluginSlotState],
    saved: &[ProjectPlugin],
    pushed: &HashSet<PluginInstanceId>,
) {
    for slot in slots.iter_mut() {
        let Some(pp) = saved.iter().find(|p| p.instance_id == slot.instance_id) else {
            continue;
        };
        let blob_pushed = pushed.contains(&slot.instance_id);
        for param in slot.params.iter_mut() {
            let target = pp
                .params
                .iter()
                .find(|p| p.id == param.id)
                .map_or(param.default_value, |p| p.value);
            let unchanged = if blob_pushed {
                param.current_value == param.default_value && target == param.default_value
            } else {
                param.current_value == target
            };
            if unchanged {
                continue;
            }
            param.current_value = target;
            let _ = engine.send(AudioCommand::SetPluginParam {
                instance_id: slot.instance_id,
                param_id: param.id,
                value: target,
            });
        }
    }
}
