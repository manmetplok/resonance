//! Roadmap group (6), first step: the entities themselves — tracks,
//! busses, the master chain and the track outputs (ARCH-01 A-13f).
//!
//! Shaped like `clips::AudioClips`: after a `ClearAll` (`old = None`) the
//! mirror is emptied and every entity is added with every scalar; on the
//! diff path only the scalars that differ from `old` are sent and the
//! mirror is updated in place. The diff path relies on
//! `structurally_compatible`: the track, bus and plugin-instance sets of
//! `old` and `new` are equal (same types, same sub-track links, same chain
//! identity and order), so it never adds or removes an entity. An id
//! missing from `old` is skipped there as defence in depth.
//!
//! `Stage::Entities` runs after `Timeline` and before `Routing` on both
//! paths: the engine rejects a send naming an unregistered endpoint, and a
//! key route names a plugin instance id.

use std::collections::HashMap;

use resonance_audio::types::*;

use super::{Reconcile, ReconcileCtx};
use crate::project::{ProjectBus, ProjectFile, ProjectPlugin, ProjectTrack};
use crate::state::*;
use crate::util::db_to_gain;
use crate::Resonance;

/// The tracks (sub-tracks included) and their `r.registry.tracks` mirror.
///
/// * After a `ClearAll`: the registry, the track-order counter and the
///   plugin side-index are emptied, the app track-id counter is bumped
///   past every saved id, then every track goes out (`AddTrack` /
///   `AddInstrumentTrack` / `AddVocalTrack` / `CreateSubTrack` + every
///   scalar + its plugin chain) and is mirrored. Then the legacy
///   generate-params migration, which reads the replayed track roles.
/// * Diff: per track, each scalar that differs from `old` is sent; the
///   mirror takes every field (name, order, …) from `new`.
///
/// The main output (`SetTrackOutput`) is [`TrackOutputs`]', after the
/// busses it names exist.
pub(crate) struct Tracks;

impl Reconcile for Tracks {
    const NAME: &'static str = "tracks";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        let Some(old) = old else {
            r.registry.tracks.clear();
            r.registry.next_track_order = 0;
            // Rebuilt from the replayed chains by `EntityOrder`; nothing in
            // between looks a plugin up through it.
            r.plugin_mirror.index.clear();
            // Bump the app-side track id counter past every persisted id so
            // a fresh add after this load doesn't collide with a restored
            // track (ARCH-04 D-4: this is the ONLY track-id counter now, so
            // this bump matters for every track, not only ones in the old
            // sub-track range). `replay_track` mirrors each track eagerly as
            // it replays, so this pre-loop bump is belt and braces on top of
            // that eager mirror, not the only thing keeping the next
            // `allocate_track_id` skip loop short.
            for pt in &new.tracks {
                if pt.id >= r.registry.next_track_id {
                    r.registry.next_track_id = pt.id + 1;
                }
            }
            for pt in &new.tracks {
                replay_track(r, pt, ctx);
            }
            // Defensive: older project files weren't guaranteed to be saved
            // in .order sequence, and replay relies on the registry staying
            // sorted by .order for the view layer's invariant.
            r.registry.resort_tracks();
            r.compose.refresh_track_count(&r.registry.tracks);
            // Migrate old generate_params + track roles to lane_generators
            // for projects predating the unified lane generator system.
            r.compose.migrate_old_generate_params(&r.registry.tracks);
            return;
        };
        let old_by_id: HashMap<u64, &ProjectTrack> = old.tracks.iter().map(|t| (t.id, t)).collect();
        for pt in &new.tracks {
            // Defence in depth: `structurally_compatible` should have gated
            // us here, but if it ever drifts we'd rather skip an unmatched
            // id than crash on undo.
            if let Some(&ot) = old_by_id.get(&pt.id) {
                apply_track(r, ot, pt);
            }
        }
    }
}

/// The busses and their `r.registry.busses` mirror.
///
/// * After a `ClearAll`: the registry and the bus-order counter are
///   emptied, then every bus goes out (`AddBus` + every scalar, its return
///   role, its plugin chain) and is mirrored.
/// * Diff: per bus, each scalar that differs from `old` is sent (including
///   `SetBusName`); the mirror takes every field from `new`.
pub(crate) struct Busses;

impl Reconcile for Busses {
    const NAME: &'static str = "busses";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        let Some(old) = old else {
            r.registry.busses.clear();
            r.registry.next_bus_order = 0;
            for pb in &new.busses {
                replay_bus(r, pb, ctx);
            }
            r.registry.resort_busses();
            // Output-destination picker depends on the bus list.
            r.ui.view_caches.rebuild_output(&r.registry.busses);
            return;
        };
        let old_by_id: HashMap<u64, &ProjectBus> = old.busses.iter().map(|b| (b.id, b)).collect();
        for pb in &new.busses {
            // Defence in depth — see `Tracks`.
            if let Some(&ob) = old_by_id.get(&pb.id) {
                apply_bus(r, ob, pb);
            }
        }
    }
}

/// The master FX chain and its bypass (`r.master`).
///
/// * After a `ClearAll`: `SetMasterFxBypass`, then every master plugin is
///   added; the mirror is rebuilt.
/// * Diff: `SetMasterFxBypass` when it changed; slot names refreshed.
pub(crate) struct Master;

impl Reconcile for Master {
    const NAME: &'static str = "master";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        let Some(old) = old else {
            r.master.fx_bypassed = new.master_fx_bypassed;
            let _ = r.engine.send(AudioCommand::SetMasterFxBypass {
                bypassed: new.master_fx_bypassed,
            });
            r.master.plugins = replay_plugins(r, &new.master_plugins, ctx, |pp| {
                AudioCommand::AddPluginToMaster {
                    clap_file_path: pp.clap_file_path.clone(),
                    clap_plugin_id: pp.clap_plugin_id.clone(),
                    id: pp.instance_id,
                }
            });
            return;
        };
        apply_master(r, old, new);
    }
}

/// Each track's main output (`SetTrackOutput`), after every bus it may
/// name exists. The app mirror (`TrackState::output`) is written by
/// [`Tracks`] with the rest of the track.
///
/// * After a `ClearAll`: sent for every track routed to a bus (the engine
///   default is the master).
/// * Diff: sent for every track whose output differs from `old`'s,
///   including one returning to the master.
pub(crate) struct TrackOutputs;

impl Reconcile for TrackOutputs {
    const NAME: &'static str = "track_outputs";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, _ctx: &ReconcileCtx<'_>) {
        let old_outputs: Option<HashMap<u64, Option<BusId>>> =
            old.map(|old| old.tracks.iter().map(|t| (t.id, t.output_bus)).collect());
        for pt in &new.tracks {
            let output = match &old_outputs {
                None => match pt.output_bus {
                    Some(bus_id) => TrackOutput::Bus(bus_id),
                    None => continue,
                },
                Some(old_outputs) => {
                    // Defence in depth — see `Tracks`.
                    match old_outputs.get(&pt.id) {
                        Some(before) if *before != pt.output_bus => {}
                        _ => continue,
                    }
                    pt.output_bus.map(TrackOutput::Bus).unwrap_or(TrackOutput::Master)
                }
            };
            let _ = r.engine.send(AudioCommand::SetTrackOutput {
                track_id: pt.id,
                output,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Full arms: add one entity after a `ClearAll`
// ---------------------------------------------------------------------------

fn replay_track(r: &mut Resonance, pt: &ProjectTrack, ctx: &ReconcileCtx<'_>) {
    // Repair sub-track id collisions left by buggier prior versions. If
    // the saved id is already in use by an earlier-loaded track, allocate
    // a fresh app-side id from `next_track_id` (which the pre-loop bump
    // already advanced past every saved track id, so this won't collide
    // with later siblings either).
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
            id: track_id,
            name: Some(pt.name.clone()),
        });
    } else if pt.track_type == "vocal" {
        let _ = r.engine.send(AudioCommand::AddVocalTrack {
            id: track_id,
            name: Some(pt.name.clone()),
        });
    } else {
        let _ = r.engine.send(AudioCommand::AddTrack {
            id: track_id,
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
    let gui_plugins = replay_plugins(r, &pt.plugins, ctx, |pp| AudioCommand::AddPlugin {
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

fn replay_bus(r: &mut Resonance, pb: &ProjectBus, ctx: &ReconcileCtx<'_>) {
    let _ = r.engine.send(AudioCommand::AddBus {
        id: pb.id,
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
        ctx,
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
fn replay_plugins(
    r: &mut Resonance,
    plugins: &[ProjectPlugin],
    ctx: &ReconcileCtx<'_>,
    mut add_command: impl FnMut(&ProjectPlugin) -> AudioCommand,
) -> Vec<PluginSlotState> {
    let mut gui_plugins = Vec::with_capacity(plugins.len());
    for pp in plugins {
        let _ = r.engine.send(add_command(pp));
        if let Some(state_data) = ctx.plugin_states.get(&pp.instance_id) {
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

// ---------------------------------------------------------------------------
// Diff arms: drive one live entity's scalars to the target
// ---------------------------------------------------------------------------

fn apply_track(r: &mut Resonance, a: &ProjectTrack, b: &ProjectTrack) {
    let track_id = b.id;
    if a.volume != b.volume {
        let _ = r.engine.send(AudioCommand::SetTrackVolume {
            track_id,
            volume: db_to_gain(b.volume),
        });
    }
    if a.pan != b.pan {
        let _ = r.engine.send(AudioCommand::SetTrackPan {
            track_id,
            pan: b.pan,
        });
    }
    if a.muted != b.muted {
        let _ = r.engine.send(AudioCommand::SetTrackMute {
            track_id,
            muted: b.muted,
        });
    }
    if a.soloed != b.soloed {
        let _ = r.engine.send(AudioCommand::SetTrackSolo {
            track_id,
            soloed: b.soloed,
        });
    }
    if a.record_armed != b.record_armed {
        let _ = r.engine.send(AudioCommand::SetTrackRecordArm {
            track_id,
            armed: b.record_armed,
        });
    }
    if a.monitor_enabled != b.monitor_enabled {
        let _ = r.engine.send(AudioCommand::SetTrackMonitor {
            track_id,
            enabled: b.monitor_enabled,
        });
    }
    if a.playback_source != b.playback_source {
        let _ = r.engine.send(AudioCommand::SetTrackPlaybackSource {
            track_id,
            source: b.playback_source,
        });
    }
    if a.mono != b.mono {
        let _ = r.engine.send(AudioCommand::SetTrackMono {
            track_id,
            mono: b.mono,
        });
    }
    if a.fx_bypassed != b.fx_bypassed {
        let _ = r.engine.send(AudioCommand::SetTrackFxBypass {
            track_id,
            bypassed: b.fx_bypassed,
        });
    }
    if a.input_device_name != b.input_device_name {
        let _ = r.engine.send(AudioCommand::SetTrackInputDevice {
            track_id,
            device_name: b.input_device_name.clone(),
        });
    }
    if a.input_port_index != b.input_port_index {
        if let Some(port_index) = b.input_port_index {
            let _ = r.engine.send(AudioCommand::SetTrackInputPort {
                track_id,
                port_index,
            });
        }
    }
    if a.midi_input_device != b.midi_input_device || a.midi_input_channel != b.midi_input_channel {
        let _ = r.engine.send(AudioCommand::SetTrackMidiInput {
            track_id,
            device: b.midi_input_device.clone(),
            channel: b.midi_input_channel,
        });
    }
    if a.midi_output_device != b.midi_output_device || a.midi_output_channel != b.midi_output_channel
    {
        let _ = r.engine.send(AudioCommand::SetTrackMidiOutput {
            track_id,
            device: b.midi_output_device.clone(),
            channel: b.midi_output_channel,
        });
    }
    // `SetTrackOutput` is `TrackOutputs`'.

    // Mirror onto GUI track state. The structural check guarantees the
    // track exists in `r.registry.tracks`.
    if let Some(t) = r.registry.tracks.iter_mut().find(|t| t.id == track_id) {
        t.name = b.name.clone();
        t.order = b.order;
        t.volume = b.volume;
        t.pan = b.pan;
        t.muted = b.muted;
        t.soloed = b.soloed;
        t.fx_bypassed = b.fx_bypassed;
        t.record_armed = b.record_armed;
        t.monitor_enabled = b.monitor_enabled;
        t.playback_source = b.playback_source;
        t.mono = b.mono;
        t.input_device_name = b.input_device_name.clone();
        t.input_port_index = b.input_port_index.unwrap_or(0);
        t.output = b
            .output_bus
            .map(TrackOutput::Bus)
            .unwrap_or(TrackOutput::Master);
        t.instrument_type = b.instrument_type;
        t.instrument_icon = b.instrument_icon;
        t.role = b.role;
        t.midi_input_device = b.midi_input_device.clone();
        t.midi_input_channel = b.midi_input_channel;
        t.midi_output_device = b.midi_output_device.clone();
        t.midi_output_channel = b.midi_output_channel;
        // Plugin slot metadata: instance_id/clap identity are
        // guaranteed stable by the structural check, but the
        // human-visible name may change.
        for (slot, pp) in t.plugins.iter_mut().zip(b.plugins.iter()) {
            slot.plugin_name = pp.plugin_name.clone();
        }
        apply_plugin_bypass(&r.engine, &mut t.plugins, &b.plugins);
    }
}

fn apply_bus(r: &mut Resonance, a: &ProjectBus, b: &ProjectBus) {
    let bus_id = b.id;
    if a.volume != b.volume {
        let _ = r.engine.send(AudioCommand::SetBusVolume {
            bus_id,
            volume: db_to_gain(b.volume),
        });
    }
    if a.pan != b.pan {
        let _ = r.engine.send(AudioCommand::SetBusPan { bus_id, pan: b.pan });
    }
    if a.muted != b.muted {
        let _ = r.engine.send(AudioCommand::SetBusMute {
            bus_id,
            muted: b.muted,
        });
    }
    if a.fx_bypassed != b.fx_bypassed {
        let _ = r.engine.send(AudioCommand::SetBusFxBypass {
            bus_id,
            bypassed: b.fx_bypassed,
        });
    }
    if a.name != b.name {
        let _ = r.engine.send(AudioCommand::SetBusName {
            bus_id,
            name: b.name.clone(),
        });
    }
    if a.is_return != b.is_return {
        let _ = r.engine.send(AudioCommand::SetBusRole {
            bus_id,
            is_return: b.is_return,
        });
    }
    if let Some(bus) = r.registry.busses.iter_mut().find(|x| x.id == bus_id) {
        bus.name = b.name.clone();
        bus.order = b.order;
        bus.volume = b.volume;
        bus.pan = b.pan;
        bus.muted = b.muted;
        bus.fx_bypassed = b.fx_bypassed;
        bus.is_return = b.is_return;
        for (slot, pp) in bus.plugins.iter_mut().zip(b.plugins.iter()) {
            slot.plugin_name = pp.plugin_name.clone();
        }
        apply_plugin_bypass(&r.engine, &mut bus.plugins, &b.plugins);
    }
}

fn apply_master(r: &mut Resonance, a: &ProjectFile, b: &ProjectFile) {
    if a.master_fx_bypassed != b.master_fx_bypassed {
        r.master.fx_bypassed = b.master_fx_bypassed;
        let _ = r.engine.send(AudioCommand::SetMasterFxBypass {
            bypassed: b.master_fx_bypassed,
        });
    }
    for (slot, pp) in r.master.plugins.iter_mut().zip(b.master_plugins.iter()) {
        slot.plugin_name = pp.plugin_name.clone();
    }
    apply_plugin_bypass(&r.engine, &mut r.master.plugins, &b.master_plugins);
}

/// Apply the saved per-slot bypass to one chain, telling the engine about
/// every slot that actually moved (ba todo #1305).
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
