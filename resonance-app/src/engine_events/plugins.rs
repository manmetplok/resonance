//! App-side handlers for plugin lifecycle events from the engine —
//! covers track plugins, bus plugins, master plugins, and the
//! sub-track auto-creation policy that PluginAdded triggers.

use resonance_audio::types::*;

use crate::state::*;
use crate::Resonance;

#[allow(clippy::too_many_arguments)]
pub(super) fn track_added(
    r: &mut Resonance,
    track_id: TrackId,
    instance_id: PluginInstanceId,
    plugin_name: String,
    clap_plugin_id: String,
    clap_file_path: String,
    params: Vec<ParamInfo>,
    has_gui: bool,
    has_sidechain_input: bool,
    output_port_count: usize,
    output_port_names: Vec<String>,
) {
    // Idempotent: if the plugin slot already exists (created by project load),
    // just update its params and has_gui. Otherwise push a new slot.
    let mut inserted = false;
    if let Some(track) = r.registry.tracks.iter_mut().find(|t| t.id == track_id) {
        if let Some(slot) = track
            .plugins
            .iter_mut()
            .find(|p| p.instance_id == instance_id)
        {
            slot.params = params;
            slot.has_gui = has_gui;
            slot.has_sidechain_input = has_sidechain_input;
        } else {
            track.plugins.push(
                PluginSlotState::new(
                    instance_id,
                    plugin_name,
                    clap_plugin_id,
                    clap_file_path,
                    params,
                    has_gui,
                )
                .with_sidechain_input(has_sidechain_input),
            );
            inserted = true;
        }
    }
    if inserted {
        r.insert_plugin_index(instance_id, PluginLocator::Track(track_id));
    }

    // If this plugin was added as part of a preset, load the saved
    // plugin state blob (if any). Pop the first entry from the pending
    // list to stay in order.
    if let Some((pending_track, ref mut states)) = r.pending_preset_plugin_states {
        if pending_track == track_id {
            if let Some(Some(data)) = if states.is_empty() {
                None
            } else {
                Some(states.remove(0))
            } {
                let _ = r.engine
                    .send(AudioCommand::LoadPluginState { instance_id, data });
            }
        }
    }
    // Clean up once all preset plugin states have been consumed.
    if r.pending_preset_plugin_states
        .as_ref()
        .map(|(_, s)| s.is_empty())
        .unwrap_or(false)
    {
        r.pending_preset_plugin_states = None;
    }

    apply_pending_param_overrides(r, instance_id);

    // Seed the undo plugin-state cache with the plugin's initial CLAP
    // state. Snapshots taken before the user interacts with the plugin
    // will have the default blob to restore to, avoiding "undo resets
    // the plugin to uninitialised garbage" UX.
    let _ = r.engine
        .send(AudioCommand::SavePluginState { instance_id });

    ensure_subtracks(r, track_id, output_port_count, &output_port_names);
}

/// Re-apply the parameter values a project load parked for this plugin
/// instance (see [`Resonance::pending_plugin_param_overrides`]).
///
/// Called from every `PluginAdded` handler — track, bus, master —
/// *after* the handler has written the event's `params` into the slot,
/// because that write is exactly what would otherwise clobber the
/// restored values with the plugin's instantiation-time defaults. This
/// was the visible half of "plugin parameters are not persisted": a
/// param set to 777 came back as its 8000 default after save + reopen,
/// while mixer volume (a plain scalar mirrored straight from the file)
/// survived.
///
/// Both halves are updated: the app-side mirror that `song.tracks` /
/// `track.plugin_params` / the mixer panel / automation read, and the
/// engine, via the same `SetPluginParam` command a live edit uses — so
/// the restored value reaches the DSP on exactly the same terms as one
/// the user just typed. Ordering is safe: the load already queued
/// `LoadPluginState` before this event could be produced, so the
/// per-param sends land after the state blob and win over it.
///
/// Ids not present on the instantiated plugin are skipped: a plugin that
/// dropped or renumbered a parameter between versions must not have a
/// stale id pushed at it.
fn apply_pending_param_overrides(r: &mut Resonance, instance_id: PluginInstanceId) {
    let Some(overrides) = r.pending_plugin_param_overrides.remove(&instance_id) else {
        return;
    };
    let applied = r
        .with_plugin_mut(instance_id, |slot| {
            let mut applied = Vec::new();
            for (param_id, value) in &overrides {
                if let Some(param) = slot.params.iter_mut().find(|p| p.id == *param_id) {
                    param.current_value = *value;
                    applied.push((*param_id, *value));
                }
            }
            applied
        })
        .unwrap_or_default();
    for (param_id, value) in applied {
        let _ = r.engine.send(AudioCommand::SetPluginParam {
            instance_id,
            param_id,
            value,
        });
    }
}

/// Ensure each output port of a multi-output plugin is represented as a
/// sub-track. Sub-tracks are a UI-only concept: regular tracks with
/// `sub_track` set, that the mixer reads during mixdown to route output
/// ports.
///
/// **Why this is its own function:** sub-track creation is a *policy*, not
/// part of event handling. It is called from `track_added` after PluginAdded,
/// but the trigger and the action are conceptually separate. Pulling it out
/// makes the event handler readable and means the policy can be re-run
/// (e.g. after a project load that lost sub-tracks) without re-dispatching
/// a synthetic event.
fn ensure_subtracks(
    r: &mut Resonance,
    parent_track_id: TrackId,
    output_port_count: usize,
    output_port_names: &[String],
) {
    if output_port_count <= 1 {
        return;
    }
    let Some(parent_name) = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == parent_track_id)
        .map(|t| t.name.clone())
    else {
        debug_assert!(
            false,
            "sub-track creation: parent track {parent_track_id:?} not found"
        );
        return;
    };
    for port_idx in 1..output_port_count {
        let already = r.registry.tracks.iter().any(|t| {
            t.sub_track
                .map(|l| {
                    l.parent_track_id == parent_track_id
                        && l.output_port_index == port_idx as u32
                })
                .unwrap_or(false)
        });
        if already {
            continue;
        }
        let port_label = output_port_names
            .get(port_idx)
            .cloned()
            .unwrap_or_else(|| format!("Port {}", port_idx));
        let sub_id = r.registry.allocate_sub_track_id();
        let order = r.registry.next_track_order;
        r.registry.next_track_order += 1;
        let sub_name = format!("{} \u{2192} {}", parent_name, port_label);
        // Register the sub-track with the engine so its fader / pan /
        // mute / bus routing atomics live alongside the parent track and
        // the mixer's existing SetTrackVolume / SetTrackOutput / ...
        // commands work unchanged.
        let _ = r.engine.send(AudioCommand::CreateSubTrack {
            sub_id,
            parent_track_id,
            output_port_index: port_idx as u32,
            name: sub_name.clone(),
        });
        r.registry.tracks.push(TrackState::new_sub_track(
            sub_id,
            order,
            sub_name,
            parent_track_id,
            port_idx as u32,
        ));
    }
}

pub(super) fn track_removed(
    r: &mut Resonance,
    track_id: TrackId,
    instance_id: PluginInstanceId,
) {
    if r.mixer.selected_plugin == Some(instance_id) {
        r.mixer.selected_plugin = None;
    }
    if let Some(track) = r.registry.tracks.iter_mut().find(|t| t.id == track_id) {
        track.plugins.retain(|p| p.instance_id != instance_id);
    }
    r.plugin_state_cache.remove(&instance_id);
    r.remove_plugin_index(instance_id);
}

/// Mirror an engine-side chain reorder (`AudioEvent::MovePlugin` ->
/// `AudioEvent::PluginMoved`, ba todo #1224) onto the app's
/// `TrackState.plugins`.
///
/// The engine is the source of truth for chain order and reports the slot
/// the plugin actually landed on *after* its own clamping, so this replays
/// that index rather than re-deriving it. Mirroring matters beyond the
/// display: this `Vec`'s order is what project serialization writes, so a
/// reorder only survives save/load if it lands here too. `plugin_index` is
/// keyed by track, not by slot, so it needs no update.
pub(super) fn track_moved(
    r: &mut Resonance,
    track_id: TrackId,
    instance_id: PluginInstanceId,
    to_index: usize,
) {
    mirror_track_plugin_move(r, track_id, instance_id, to_index);
}

/// The mirror itself, shared with the control API's `track.move_effect`
/// (ba doc #273, todo #1225), which applies the order immediately so a
/// client can read back what it just set instead of waiting a cycle for
/// `PluginMoved`. Applying it twice is harmless: the second call finds
/// the plugin already at `to_index` and returns.
pub(crate) fn mirror_track_plugin_move(
    r: &mut Resonance,
    track_id: TrackId,
    instance_id: PluginInstanceId,
    to_index: usize,
) {
    let Some(track) = r.registry.tracks.iter_mut().find(|t| t.id == track_id) else {
        return;
    };
    let Some(from) = track
        .plugins
        .iter()
        .position(|p| p.instance_id == instance_id)
    else {
        return;
    };
    // `from` was found, so the chain is non-empty and this cannot wrap.
    let to = to_index.min(track.plugins.len() - 1);
    if from == to {
        return;
    }
    let slot = track.plugins.remove(from);
    track.plugins.insert(to, slot);
}

pub(super) fn scanned(r: &mut Resonance, plugins: Vec<ScannedPlugin>) {
    r.available_plugins = plugins;
    r.view_caches.rebuild_plugins(&r.available_plugins);
}

pub(super) fn state_saved(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    data: Vec<u8>,
) {
    // Also feeds the undo system's plugin-state cache so snapshots can
    // replay internal CLAP state on restore. The project-save path
    // drains the cache via `SaveAllPluginStates` separately.
    r.plugin_state_cache.insert(instance_id, data);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn bus_added(
    r: &mut Resonance,
    bus_id: BusId,
    instance_id: PluginInstanceId,
    plugin_name: String,
    clap_plugin_id: String,
    clap_file_path: String,
    params: Vec<ParamInfo>,
    has_gui: bool,
    has_sidechain_input: bool,
) {
    let mut inserted = false;
    if let Some(bus) = r.registry.busses.iter_mut().find(|b| b.id == bus_id) {
        if let Some(slot) = bus
            .plugins
            .iter_mut()
            .find(|p| p.instance_id == instance_id)
        {
            slot.params = params;
            slot.has_gui = has_gui;
            slot.has_sidechain_input = has_sidechain_input;
        } else {
            bus.plugins.push(
                PluginSlotState::new(
                    instance_id,
                    plugin_name,
                    clap_plugin_id,
                    clap_file_path,
                    params,
                    has_gui,
                )
                .with_sidechain_input(has_sidechain_input),
            );
            inserted = true;
        }
    }
    if inserted {
        r.insert_plugin_index(instance_id, PluginLocator::Bus(bus_id));
    }
    apply_pending_param_overrides(r, instance_id);
    let _ = r.engine
        .send(AudioCommand::SavePluginState { instance_id });
}

/// Mirror an engine-side bus chain reorder
/// (`AudioCommand::MovePluginInBus` -> `AudioEvent::BusPluginMoved`, ba
/// doc #273, todo #1237) onto `BusState.plugins` — the bus twin of
/// [`track_moved`].
pub(super) fn bus_moved(
    r: &mut Resonance,
    bus_id: BusId,
    instance_id: PluginInstanceId,
    to_index: usize,
) {
    mirror_bus_plugin_move(r, bus_id, instance_id, to_index);
}

/// The mirror itself, shared with the control API's `bus.move_effect`,
/// which applies the order immediately so a client can read back what it
/// just set. Applying it twice is harmless: the second call finds the
/// plugin already at `to_index` and returns.
pub(crate) fn mirror_bus_plugin_move(
    r: &mut Resonance,
    bus_id: BusId,
    instance_id: PluginInstanceId,
    to_index: usize,
) {
    let Some(bus) = r.registry.busses.iter_mut().find(|b| b.id == bus_id) else {
        return;
    };
    let Some(from) = bus.plugins.iter().position(|p| p.instance_id == instance_id) else {
        return;
    };
    // `from` was found, so the chain is non-empty and this cannot wrap.
    let to = to_index.min(bus.plugins.len() - 1);
    if from == to {
        return;
    }
    let slot = bus.plugins.remove(from);
    bus.plugins.insert(to, slot);
}

pub(super) fn bus_removed(
    r: &mut Resonance,
    bus_id: BusId,
    instance_id: PluginInstanceId,
) {
    if let Some(bus) = r.registry.busses.iter_mut().find(|b| b.id == bus_id) {
        bus.plugins.retain(|p| p.instance_id != instance_id);
    }
    if r.mixer.selected_plugin == Some(instance_id) {
        r.mixer.selected_plugin = None;
    }
    r.plugin_state_cache.remove(&instance_id);
    r.remove_plugin_index(instance_id);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn master_added(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    plugin_name: String,
    clap_plugin_id: String,
    clap_file_path: String,
    params: Vec<ParamInfo>,
    has_gui: bool,
    has_sidechain_input: bool,
) {
    if let Some(slot) = r
        .master_plugins
        .iter_mut()
        .find(|p| p.instance_id == instance_id)
    {
        slot.params = params;
        slot.has_gui = has_gui;
        slot.has_sidechain_input = has_sidechain_input;
    } else {
        r.master_plugins.push(
            PluginSlotState::new(
                instance_id,
                plugin_name,
                clap_plugin_id,
                clap_file_path,
                params,
                has_gui,
            )
            .with_sidechain_input(has_sidechain_input),
        );
        r.insert_plugin_index(instance_id, PluginLocator::Master);
    }
    apply_pending_param_overrides(r, instance_id);
    let _ = r.engine
        .send(AudioCommand::SavePluginState { instance_id });
}

/// Mirror an engine-side master chain reorder
/// (`AudioCommand::MovePluginInMaster` -> `AudioEvent::MasterPluginMoved`)
/// onto `Resonance::master_plugins` — the master twin of [`bus_moved`].
pub(super) fn master_moved(r: &mut Resonance, instance_id: PluginInstanceId, to_index: usize) {
    mirror_master_plugin_move(r, instance_id, to_index);
}

/// The mirror itself, shared with the control API's
/// `master.move_effect`, which applies the order immediately so a client
/// can read back what it just set. Applying it twice is harmless: the
/// second call finds the plugin already at `to_index` and returns.
pub(crate) fn mirror_master_plugin_move(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    to_index: usize,
) {
    let Some(from) = r
        .master_plugins
        .iter()
        .position(|p| p.instance_id == instance_id)
    else {
        return;
    };
    // `from` was found, so the chain is non-empty and this cannot wrap.
    let to = to_index.min(r.master_plugins.len() - 1);
    if from == to {
        return;
    }
    let slot = r.master_plugins.remove(from);
    r.master_plugins.insert(to, slot);
}

pub(super) fn master_removed(r: &mut Resonance, instance_id: PluginInstanceId) {
    r.master_plugins.retain(|p| p.instance_id != instance_id);
    if r.mixer.selected_plugin == Some(instance_id) {
        r.mixer.selected_plugin = None;
    }
    r.plugin_state_cache.remove(&instance_id);
    r.remove_plugin_index(instance_id);
}

pub(super) fn master_fx_bypass_changed(r: &mut Resonance, bypassed: bool) {
    r.master_fx_bypassed = bypassed;
}
