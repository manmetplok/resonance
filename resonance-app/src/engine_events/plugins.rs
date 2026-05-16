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
    output_port_count: usize,
    output_port_names: Vec<String>,
) {
    // Idempotent: if the plugin slot already exists (created by project load),
    // just update its params and has_gui. Otherwise push a new slot.
    if let Some(track) = r.registry.tracks.iter_mut().find(|t| t.id == track_id) {
        if let Some(slot) = track
            .plugins
            .iter_mut()
            .find(|p| p.instance_id == instance_id)
        {
            slot.params = params;
            slot.has_gui = has_gui;
        } else {
            track.plugins.push(PluginSlotState::new(
                instance_id,
                plugin_name,
                clap_plugin_id,
                clap_file_path,
                params,
                has_gui,
            ));
        }
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
                r.engine
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

    // Seed the undo plugin-state cache with the plugin's initial CLAP
    // state. Snapshots taken before the user interacts with the plugin
    // will have the default blob to restore to, avoiding "undo resets
    // the plugin to uninitialised garbage" UX.
    r.engine
        .send(AudioCommand::SavePluginState { instance_id });

    ensure_subtracks(r, track_id, output_port_count, &output_port_names);
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
        r.engine.send(AudioCommand::CreateSubTrack {
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
) {
    if let Some(bus) = r.registry.busses.iter_mut().find(|b| b.id == bus_id) {
        if let Some(slot) = bus
            .plugins
            .iter_mut()
            .find(|p| p.instance_id == instance_id)
        {
            slot.params = params;
            slot.has_gui = has_gui;
        } else {
            bus.plugins.push(PluginSlotState::new(
                instance_id,
                plugin_name,
                clap_plugin_id,
                clap_file_path,
                params,
                has_gui,
            ));
        }
    }
    r.engine
        .send(AudioCommand::SavePluginState { instance_id });
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
}

pub(super) fn master_added(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    plugin_name: String,
    clap_plugin_id: String,
    clap_file_path: String,
    params: Vec<ParamInfo>,
    has_gui: bool,
) {
    if let Some(slot) = r
        .master_plugins
        .iter_mut()
        .find(|p| p.instance_id == instance_id)
    {
        slot.params = params;
        slot.has_gui = has_gui;
    } else {
        r.master_plugins.push(PluginSlotState::new(
            instance_id,
            plugin_name,
            clap_plugin_id,
            clap_file_path,
            params,
            has_gui,
        ));
    }
    r.engine
        .send(AudioCommand::SavePluginState { instance_id });
}

pub(super) fn master_removed(r: &mut Resonance, instance_id: PluginInstanceId) {
    r.master_plugins.retain(|p| p.instance_id != instance_id);
    if r.mixer.selected_plugin == Some(instance_id) {
        r.mixer.selected_plugin = None;
    }
    r.plugin_state_cache.remove(&instance_id);
}

pub(super) fn master_fx_bypass_changed(r: &mut Resonance, bypassed: bool) {
    r.master_fx_bypassed = bypassed;
}
