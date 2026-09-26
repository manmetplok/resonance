//! Per-entity replay helpers: one track, one bus, the master chain, one plugin
//! chain. Called from `replay_tracks_and_busses` / `replay_master` in the
//! orchestrator (`mod.rs`).

use resonance_audio::types::*;

use crate::project::{LoadedProject, ProjectBus, ProjectPlugin, ProjectTrack};
use crate::state::*;
use crate::util::db_to_gain;
use crate::Resonance;

pub(super) fn replay_track(r: &mut Resonance, pt: &ProjectTrack, loaded: &LoadedProject) {
    // Repair sub-track id collisions left by buggier prior versions. If
    // the saved id is already in use by an earlier-loaded track, allocate
    // a fresh app-side id from `next_sub_track_id` (which the pre-loop
    // bump already advanced past every saved sub-track id, so this won't
    // collide with later siblings either).
    let track_id = if pt.sub_track.is_some()
        && r.registry.tracks.iter().any(|t| t.id == pt.id)
    {
        let new_id = r.allocate_track_id();
        tracing::warn!(
            "replay_track: sub-track {:?} id {} collided with existing track; remapped to {}",
            pt.name, pt.id, new_id
        );
        new_id
    } else {
        pt.id
    };

    // Register the track / sub-track / instrument-track with the engine.
    if let Some(link) = pt.sub_track {
        let _ = r.engine.send(AudioCommand::CreateSubTrack {
            sub_id: track_id,
            parent_track_id: link.parent_track_id,
            output_port_index: link.output_port_index,
            name: pt.name.clone(),
        });
    } else if pt.track_type == "instrument" {
        let _ = r.engine.send(AudioCommand::AddInstrumentTrack {
            id_hint: Some(track_id),
            name: Some(pt.name.clone()),
        });
    } else if pt.track_type == "vocal" {
        let _ = r.engine.send(AudioCommand::AddVocalTrack {
            id_hint: Some(track_id),
            name: Some(pt.name.clone()),
        });
    } else {
        let _ = r.engine.send(AudioCommand::AddTrack {
            id_hint: Some(track_id),
            name: Some(pt.name.clone()),
        });
    }

    // Set track properties.
    let _ = r.engine.send(AudioCommand::SetTrackVolume {
        track_id,
        volume: db_to_gain(pt.volume),
    });
    let _ = r.engine.send(AudioCommand::SetTrackPan {
        track_id,
        pan: pt.pan,
    });
    let _ = r.engine.send(AudioCommand::SetTrackMute {
        track_id,
        muted: pt.muted,
    });
    let _ = r.engine.send(AudioCommand::SetTrackSolo {
        track_id,
        soloed: pt.soloed,
    });
    let _ = r.engine.send(AudioCommand::SetTrackRecordArm {
        track_id,
        armed: pt.record_armed,
    });
    let _ = r.engine.send(AudioCommand::SetTrackMonitor {
        track_id,
        enabled: pt.monitor_enabled,
    });
    let _ = r.engine.send(AudioCommand::SetTrackPlaybackSource {
        track_id,
        source: pt.playback_source,
    });
    let _ = r.engine.send(AudioCommand::SetTrackMono {
        track_id,
        mono: pt.mono,
    });
    let _ = r.engine.send(AudioCommand::SetTrackFxBypass {
        track_id,
        bypassed: pt.fx_bypassed,
    });
    if let Some(ref device) = pt.input_device_name {
        let _ = r.engine.send(AudioCommand::SetTrackInputDevice {
            track_id,
            device_name: Some(device.clone()),
        });
    }
    if let Some(port_index) = pt.input_port_index {
        let _ = r.engine.send(AudioCommand::SetTrackInputPort {
            track_id,
            port_index,
        });
    }
    if pt.midi_input_device.is_some() {
        let _ = r.engine.send(AudioCommand::SetTrackMidiInput {
            track_id,
            device: pt.midi_input_device.clone(),
            channel: pt.midi_input_channel,
        });
    }
    if pt.midi_output_device.is_some() {
        let _ = r.engine.send(AudioCommand::SetTrackMidiOutput {
            track_id,
            device: pt.midi_output_device.clone(),
            channel: pt.midi_output_channel,
        });
    }

    // Build GUI track state.
    let gui_plugins = replay_plugins(r, &pt.plugins, loaded, |pp| AudioCommand::AddPlugin {
        track_id,
        clap_file_path: pp.clap_file_path.clone(),
        clap_plugin_id: pp.clap_plugin_id.clone(),
        id: pp.instance_id,
    });

    let order = r.registry.next_track_order;
    let mut track = if let Some(link) = pt.sub_track {
        // Sub-tracks are always instrument-typed regardless of what the
        // saved `track_type` says. Earlier buggy saves could land a
        // sub-track in a colliding-id slot whose surviving entry had
        // track_type "vocal"; re-typing here keeps the inspector / mixer
        // rendering the correct controls after the remap above.
        TrackState::new_sub_track(
            track_id,
            order,
            pt.name.clone(),
            link.parent_track_id,
            link.output_port_index,
        )
    } else if pt.track_type == "instrument" {
        TrackState::new_instrument(track_id, order)
    } else if pt.track_type == "vocal" {
        TrackState::new_vocal(track_id, order)
    } else {
        TrackState::new_audio(track_id, order)
    };
    // Projects saved before the order-based default-naming fix
    // (commit ~late-2026) stored auto-generated names like
    // "Track 1000000006" derived from the engine TrackId. Replace
    // those on load with the new order-based form; user-chosen
    // names containing digits are left alone.
    track.name = migrate_auto_name(&pt.name, pt.track_type == "instrument", order);
    track.volume = pt.volume;
    track.pan = pt.pan;
    track.muted = pt.muted;
    track.soloed = pt.soloed;
    track.fx_bypassed = pt.fx_bypassed;
    track.record_armed = pt.record_armed;
    track.monitor_enabled = pt.monitor_enabled;
    track.playback_source = pt.playback_source;
    track.mono = pt.mono;
    track.input_device_name = pt.input_device_name.clone();
    track.plugins = gui_plugins;
    track.output = pt
        .output_bus
        .map(TrackOutput::Bus)
        .unwrap_or(TrackOutput::Master);
    track.instrument_type = pt.instrument_type;
    track.instrument_icon = pt.instrument_icon;
    track.role = pt.role;
    track.sub_track = pt.sub_track;
    track.input_port_index = pt.input_port_index.unwrap_or(0);
    track.midi_input_device = pt.midi_input_device.clone();
    track.midi_input_channel = pt.midi_input_channel;
    track.midi_output_device = pt.midi_output_device.clone();
    track.midi_output_channel = pt.midi_output_channel;
    r.registry.tracks.push(track);
    r.registry.next_track_order += 1;
    // External-instrument mode is restored after every track, by the
    // `ExternalInstruments` reconcile domain (ARCH-01 A-13b).
}

pub(super) fn replay_bus(r: &mut Resonance, pb: &ProjectBus, loaded: &LoadedProject) {
    let _ = r.engine.send(AudioCommand::AddBus {
        id_hint: Some(pb.id),
        name: Some(pb.name.clone()),
    });
    let _ = r.engine.send(AudioCommand::SetBusVolume {
        bus_id: pb.id,
        volume: db_to_gain(pb.volume),
    });
    let _ = r.engine.send(AudioCommand::SetBusPan {
        bus_id: pb.id,
        pan: pb.pan,
    });
    let _ = r.engine.send(AudioCommand::SetBusMute {
        bus_id: pb.id,
        muted: pb.muted,
    });
    let _ = r.engine.send(AudioCommand::SetBusFxBypass {
        bus_id: pb.id,
        bypassed: pb.fx_bypassed,
    });
    // Return role — the destination half of the saved send graph (ba doc
    // #273). Sent unconditionally (the engine no-ops on an unknown bus,
    // and this one exists), so a bus demoted back to a plain sub-mix
    // reloads as one.
    let _ = r.engine.send(AudioCommand::SetBusRole {
        bus_id: pb.id,
        is_return: pb.is_return,
    });

    let gui_plugins = replay_plugins(
        r,
        &pb.plugins,
        loaded,
        |pp| AudioCommand::AddPluginToBus {
            bus_id: pb.id,
            clap_file_path: pp.clap_file_path.clone(),
            clap_plugin_id: pp.clap_plugin_id.clone(),
            id: pp.instance_id,
        },
    );

    let mut bus = BusState::new(pb.id, r.registry.next_bus_order, pb.name.clone());
    bus.volume = pb.volume;
    bus.pan = pb.pan;
    bus.muted = pb.muted;
    bus.fx_bypassed = pb.fx_bypassed;
    bus.is_return = pb.is_return;
    bus.plugins = gui_plugins;
    r.registry.busses.push(bus);
    r.registry.next_bus_order += 1;
}

pub(super) fn replay_master(
    r: &mut Resonance,
    project: &crate::project::ProjectFile,
    loaded: &LoadedProject,
) {
    r.master_fx_bypassed = project.master_fx_bypassed;
    let _ = r.engine.send(AudioCommand::SetMasterFxBypass {
        bypassed: project.master_fx_bypassed,
    });

    r.master_plugins = replay_plugins(r, &project.master_plugins, loaded, |pp| {
        AudioCommand::AddPluginToMaster {
            clap_file_path: pp.clap_file_path.clone(),
            clap_plugin_id: pp.clap_plugin_id.clone(),
            id: pp.instance_id,
        }
    });
}

/// Replay one saved plugin chain: instantiate each plugin on the engine
/// (via the target-specific `add_command`), restore its saved state
/// blob, and collect placeholder GUI slots. The placeholders' params +
/// has_gui are overwritten when the subsequent PluginAdded event
/// arrives from the engine.
///
/// **A plugin that never comes back.** `AddPlugin` is fire-and-forget:
/// if the `.clap` isn't on this machine the engine replies with a
/// generic `AudioEvent::Error` and no `PluginAdded`, so the placeholder
/// stays in the chain with an empty `params` mirror and the engine has
/// no instance to save state from. Everything this loop parks app-side —
/// the opaque blob in `plugin_mirror.state_cache`, the parameter values in
/// `pending_plugin_param_overrides` — is therefore the *only* surviving
/// copy of that plugin's settings, and every project-writing path reads
/// it back so a Save As on a machine without the plugin no longer
/// destroys them (ba doc #275, P5). Both are dropped again the moment
/// the instance does turn up (`engine_events::plugins`), where the
/// engine becomes the source of truth.
pub(super) fn replay_plugins(
    r: &mut Resonance,
    plugins: &[ProjectPlugin],
    loaded: &LoadedProject,
    mut add_command: impl FnMut(&ProjectPlugin) -> AudioCommand,
) -> Vec<PluginSlotState> {
    let mut gui_plugins = Vec::with_capacity(plugins.len());
    for pp in plugins {
        let _ = r.engine.send(add_command(pp));
        if let Some(state_data) = loaded.plugin_states.get(&pp.instance_id) {
            let _ = r.engine.send(AudioCommand::LoadPluginState {
                instance_id: pp.instance_id,
                data: state_data.to_vec(),
            });
            // Keep the blob app-side, byte for byte, as the plugin's
            // last known state. A live plugin overwrites this entry with
            // its own fresh blob on the `PluginStateSaved` echo that
            // follows `PluginAdded`; a missing one never does, and this
            // copy is what the next save writes.
            r.plugin_mirror.state_cache.insert(pp.instance_id, std::sync::Arc::clone(state_data));
        }
        // Park the saved parameter overrides until `PluginAdded` reports
        // this instance's param list. Applying them here would be undone:
        // that event carries the values the plugin instantiated with (its
        // defaults) and overwrites `slot.params` wholesale. See
        // `Resonance::pending_plugin_param_overrides`.
        if !pp.params.is_empty() {
            r.presets.pending_plugin_param_overrides
                .insert(pp.instance_id, pp.params.clone());
        }
        // Restore a bypassed slot through the very same command a user
        // toggle sends, so there is one path into the engine. Only when
        // it is actually bypassed: the engine's default is running, and
        // a command per slot on every load would be noise.
        if pp.bypassed {
            let _ = r.engine.send(AudioCommand::SetPluginBypass {
                instance_id: pp.instance_id,
                bypassed: true,
            });
        }
        let mut slot = PluginSlotState::new(
            pp.instance_id,
            pp.plugin_name.clone(),
            pp.clap_plugin_id.clone(),
            pp.clap_file_path.clone(),
            Vec::new(),
            false,
        );
        // Seeded rather than left to the echo: the mixer draws before
        // the engine answers, and a slot that flashed un-bypassed for a
        // frame would read as the project having lost the setting.
        slot.bypassed = pp.bypassed;
        gui_plugins.push(slot);
    }
    gui_plugins
}

/// Reorder `plugins` to match the saved instance-id sequence, leaving
/// any slot whose `instance_id` isn't in `saved` at the end in its
/// current relative order. Missing entries in `saved` (plugins that
/// failed to load and therefore have no live slot) are silently
/// filtered — the absent ids never reach the comparator. Stable, so a
/// chain already in saved order is unchanged.
pub fn sort_plugins_by_saved_order(plugins: &mut [PluginSlotState], saved: &[u64]) {
    if plugins.len() < 2 || saved.is_empty() {
        return;
    }
    // O(n) index lookup; `saved` is bounded by the per-chain plugin
    // count (single digits in practice, dozens worst-case).
    let position = |id: u64| -> usize {
        saved
            .iter()
            .position(|&s| s == id)
            .unwrap_or(usize::MAX)
    };
    plugins.sort_by_key(|p| position(p.instance_id));
}

/// Rewrite legacy auto-generated track names like "Track 1000000006" or
/// "Instrument 1234567890" to the new order-based form ("Track 4").
/// Names that aren't a single Track/Instrument + a 7-or-more digit
/// number pass through unchanged so user labels like "Track 2 (lead)"
/// or short numbered names like "Track 12" are preserved.
pub fn migrate_auto_name(name: &str, is_instrument: bool, order: usize) -> String {
    let prefix = if is_instrument {
        "Instrument "
    } else {
        "Track "
    };
    if let Some(rest) = name.strip_prefix(prefix) {
        if rest.len() >= 7 && rest.chars().all(|c| c.is_ascii_digit()) {
            return format!("{}{}", prefix, order + 1);
        }
    }
    name.to_string()
}
