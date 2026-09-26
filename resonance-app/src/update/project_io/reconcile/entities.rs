//! Roadmap group (6): the entities themselves — tracks, busses, the
//! master chain and the track outputs (ARCH-01 A-13f, A-13h).
//!
//! Shaped like `clips::AudioClips`: after a `ClearAll` (`old = None`) the
//! mirror is emptied and every entity is added with every scalar; on the
//! diff path only the scalars that differ from `old` are sent and the
//! mirror is updated in place.
//!
//! Since A-13h the diff path also adds what `old` lacks: a bus (the
//! after-`ClearAll` add body, [`replay_bus`]) and a plugin instance on a
//! chain that stays (the same [`replay_plugins`] body, appended to the
//! live chain); since A-13i a track too ([`replay_track`]), sub-tracks
//! included. What `new` lacks was already removed by
//! `removals::EntityRemovals`, the stage before, and [`EntityOrder`] puts
//! every chain into the target's order last.
//!
//! **Kept and fresh tracks** ([`kept_tracks`]). A track is *kept* by a diff
//! restore when both files have its id with the same type and sub-track
//! link (and, for a sub-track, its parent is kept). Every other track of
//! `new` is *fresh* — added exactly as a load adds it — and every other
//! track of `old` is removed. After a `ClearAll` nothing is kept. The
//! domains whose after-`ClearAll` behaviour differs (track outputs, plugin
//! state, external instruments, freeze, group macros) apply it per fresh
//! track rather than per restore.
//!
//! Each plugin's state (blob, per-slot bypass, params) is
//! `plugin_state::PluginState`'s, the next domain in the stage.
//!
//! `Stage::Entities` runs after `Removals` and before `Routing` on both
//! paths: the engine rejects a send naming an unregistered endpoint, and a
//! key route names a plugin instance id.

use std::collections::{HashMap, HashSet};

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
///   scalar + its plugin chain) and is mirrored in file order. Then the
///   legacy generate-params migration, which reads the replayed track
///   roles.
/// * Diff: per kept track ([`kept_tracks`]), each scalar that differs from
///   `old` is sent; the mirror takes every field (name, order, …) from
///   `new`; each plugin `old` did not have on the chain is added to its
///   end (`EntityOrder` moves it into place). Each fresh track is added as
///   after a `ClearAll`, with its saved `.order`. A removed one is already
///   gone (`removals::EntityRemovals`).
///
/// The main output (`SetTrackOutput`) is [`TrackOutputs`]', after the
/// busses it names exist.
pub(crate) struct Tracks;

impl Reconcile for Tracks {
    const NAME: &'static str = "tracks";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, _ctx: &ReconcileCtx<'_>) {
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
                let order = r.registry.next_track_order;
                replay_track(r, pt, order);
                r.registry.next_track_order += 1;
            }
            // Migrate old generate_params + track roles to lane_generators
            // for projects predating the unified lane generator system.
            // Keyed by track id, so the registry's order (sorted later, by
            // `EntityOrder`) does not matter.
            r.compose.migrate_old_generate_params(&r.registry.tracks);
            return;
        };
        let old_by_id: HashMap<u64, &ProjectTrack> = old.tracks.iter().map(|t| (t.id, t)).collect();
        let kept_tracks = kept_tracks(Some(old), new);
        let kept = kept_plugins(Some(old), new);
        for pt in &new.tracks {
            let Some(&ot) = old_by_id.get(&pt.id).filter(|_| kept_tracks.contains(&pt.id)) else {
                continue;
            };
            apply_track(r, ot, pt);
            let track_id = pt.id;
            let added = replay_plugins(r, fresh(&pt.plugins, &kept), |pp| AudioCommand::AddPlugin {
                track_id,
                clap_file_path: pp.clap_file_path.clone(),
                clap_plugin_id: pp.clap_plugin_id.clone(),
                id: pp.instance_id,
            });
            if let Some(t) = r.registry.tracks.iter_mut().find(|t| t.id == track_id) {
                t.plugins.extend(added);
            }
        }
        // The fresh tracks, as a load adds them — parents before
        // sub-tracks, since the engine files a sub-track under its parent
        // (`CreateSubTrack`). A fresh multi-output instrument's sub-tracks
        // are added here, under their saved ids, before its `PluginAdded`
        // echo can run `ensure_subtracks`, which then finds every
        // (parent, port) taken and adds none. Each keeps its saved
        // `.order` (the fixed point); both id and order counters stay past
        // it, so an add after the restore never collides (D-4).
        let fresh_tracks = new
            .tracks
            .iter()
            .filter(|pt| !kept_tracks.contains(&pt.id));
        let (parents, subs): (Vec<&ProjectTrack>, Vec<&ProjectTrack>) =
            fresh_tracks.partition(|pt| pt.sub_track.is_none());
        for pt in parents.into_iter().chain(subs) {
            if pt.id >= r.registry.next_track_id {
                r.registry.next_track_id = pt.id + 1;
            }
            replay_track(r, pt, pt.order);
            r.registry.next_track_order = r.registry.next_track_order.max(pt.order + 1);
        }
    }
}

/// The busses and their `r.registry.busses` mirror.
///
/// * After a `ClearAll`: the registry and the bus-order counter are
///   emptied, then every bus goes out (`AddBus` + every scalar, its return
///   role, its plugin chain) and is mirrored.
/// * Diff: a bus `old` lacks is added exactly as after a `ClearAll` (with
///   its saved `.order`); per bus `old` has, each scalar that differs is
///   sent (including `SetBusName`), the mirror takes every field from
///   `new`, and each plugin `old` did not have on the chain is added (to
///   the end; [`EntityOrder`] moves it into place). A bus `new` lacks is
///   already gone (`removals::EntityRemovals`).
pub(crate) struct Busses;

impl Reconcile for Busses {
    const NAME: &'static str = "busses";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, _ctx: &ReconcileCtx<'_>) {
        let Some(old) = old else {
            r.registry.busses.clear();
            r.registry.next_bus_order = 0;
            for pb in &new.busses {
                replay_bus(r, pb);
            }
            return;
        };
        let old_by_id: HashMap<u64, &ProjectBus> = old.busses.iter().map(|b| (b.id, b)).collect();
        let kept = kept_plugins(Some(old), new);
        for pb in &new.busses {
            let Some(&ob) = old_by_id.get(&pb.id) else {
                replay_bus(r, pb);
                // `replay_bus` numbers a bus by replay position; a diff add
                // keeps the snapshot's `.order` (the fixed point), and the
                // counter stays past it.
                if let Some(bus) = r.registry.busses.iter_mut().find(|b| b.id == pb.id) {
                    bus.order = pb.order;
                }
                r.registry.next_bus_order = r.registry.next_bus_order.max(pb.order + 1);
                continue;
            };
            apply_bus(r, ob, pb);
            let bus_id = pb.id;
            let added = replay_plugins(r, fresh(&pb.plugins, &kept), |pp| AudioCommand::AddPluginToBus {
                bus_id,
                clap_file_path: pp.clap_file_path.clone(),
                clap_plugin_id: pp.clap_plugin_id.clone(),
                id: pp.instance_id,
            });
            if let Some(bus) = r.registry.busses.iter_mut().find(|b| b.id == bus_id) {
                bus.plugins.extend(added);
            }
        }
    }
}

/// The master FX chain and its bypass (`r.master`).
///
/// * After a `ClearAll`: `SetMasterFxBypass`, then every master plugin is
///   added; the mirror is rebuilt.
/// * Diff: `SetMasterFxBypass` when it changed; slot names refreshed;
///   each plugin `old` did not have is added to the end of the chain.
///
/// The per-slot bypass, blobs and params of every chain are
/// `plugin_state::PluginState`'s.
pub(crate) struct Master;

impl Reconcile for Master {
    const NAME: &'static str = "master";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, _ctx: &ReconcileCtx<'_>) {
        let Some(old) = old else {
            r.master.fx_bypassed = new.master_fx_bypassed;
            let _ = r.engine.send(AudioCommand::SetMasterFxBypass {
                bypassed: new.master_fx_bypassed,
            });
            r.master.plugins = replay_plugins(r, &new.master_plugins, |pp| {
                AudioCommand::AddPluginToMaster {
                    clap_file_path: pp.clap_file_path.clone(),
                    clap_plugin_id: pp.clap_plugin_id.clone(),
                    id: pp.instance_id,
                }
            });
            return;
        };
        apply_master(r, old, new);
        let kept = kept_plugins(Some(old), new);
        let added = replay_plugins(r, fresh(&new.master_plugins, &kept), |pp| {
            AudioCommand::AddPluginToMaster {
                clap_file_path: pp.clap_file_path.clone(),
                clap_plugin_id: pp.clap_plugin_id.clone(),
                id: pp.instance_id,
            }
        });
        r.master.plugins.extend(added);
    }
}

/// Each track's main output (`SetTrackOutput`), after every bus it may
/// name exists. The app mirror (`TrackState::output`) is written by
/// [`Tracks`] with the rest of the track.
///
/// * After a `ClearAll`: sent for every track routed to a bus (the engine
///   default is the master).
/// * Diff: sent for every kept track whose output differs from `old`'s,
///   including one returning to the master; for a fresh track, as after a
///   `ClearAll`.
pub(crate) struct TrackOutputs;

impl Reconcile for TrackOutputs {
    const NAME: &'static str = "track_outputs";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, _ctx: &ReconcileCtx<'_>) {
        let kept = kept_tracks(old, new);
        let old_outputs: HashMap<u64, Option<BusId>> = old
            .map(|old| old.tracks.iter().map(|t| (t.id, t.output_bus)).collect())
            .unwrap_or_default();
        for pt in &new.tracks {
            let output = if kept.contains(&pt.id) {
                if old_outputs.get(&pt.id) == Some(&pt.output_bus) {
                    continue;
                }
                pt.output_bus.map(TrackOutput::Bus).unwrap_or(TrackOutput::Master)
            } else {
                // Fresh: the engine default is the master.
                match pt.output_bus {
                    Some(bus_id) => TrackOutput::Bus(bus_id),
                    None => continue,
                }
            };
            let _ = r.engine.send(AudioCommand::SetTrackOutput {
                track_id: pt.id,
                output,
            });
        }
    }
}

/// Last in `Entities`, on every origin: the registry resorted by `.order`
/// (the view layer's invariant — older files were not guaranteed to be
/// saved in order, and a diff restore may have changed an `.order`), the
/// output-destination picker rebuilt from the bus list, the compose
/// instrument-lane count refreshed.
///
/// Then each chain is put into the target's order:
///
/// * After a `ClearAll`: re-sorted app-side into its saved order (stable,
///   so a no-op when the placeholders went in in order) — the engine's
///   chains were built in that order.
/// * Diff: the live chain is the kept slots in their old order, then the
///   slots the entity domains appended, which is also the engine's order.
///   Each slot out of place is moved with `MovePlugin` /
///   `MovePluginInBus` / `MovePluginInMaster` and mirrored at once (see
///   [`order_chain`]).
///
/// Last, on every origin, the plugin side-index is rebuilt from the
/// chains (a diff restore may have added and removed slots).
pub(crate) struct EntityOrder;

impl Reconcile for EntityOrder {
    const NAME: &'static str = "entity_order";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, _ctx: &ReconcileCtx<'_>) {
        r.registry.resort_tracks();
        r.registry.resort_busses();
        r.ui.view_caches.rebuild_output(&r.registry.busses);
        r.compose.refresh_track_count(&r.registry.tracks);
        if let Some(old) = old {
            order_live_chains(r, old, new);
            r.rebuild_plugin_index();
            return;
        }
        let saved_ids = |plugins: &[ProjectPlugin]| -> Vec<u64> {
            plugins.iter().map(|p| p.instance_id).collect()
        };
        let tracks: HashMap<TrackId, Vec<u64>> =
            new.tracks.iter().map(|pt| (pt.id, saved_ids(&pt.plugins))).collect();
        let busses: HashMap<BusId, Vec<u64>> =
            new.busses.iter().map(|pb| (pb.id, saved_ids(&pb.plugins))).collect();
        for track in &mut r.registry.tracks {
            if let Some(order) = tracks.get(&track.id) {
                sort_plugins_by_saved_order(&mut track.plugins, order);
            }
        }
        for bus in &mut r.registry.busses {
            if let Some(order) = busses.get(&bus.id) {
                sort_plugins_by_saved_order(&mut bus.plugins, order);
            }
        }
        sort_plugins_by_saved_order(&mut r.master.plugins, &saved_ids(&new.master_plugins));
        // Re-populate the `with_plugin_mut` side-index from the wholesale
        // replay. Per-slot inserts would also work but a single rebuild is
        // simpler and keeps the add bodies focused on their own concern.
        r.rebuild_plugin_index();
    }
}

// ---------------------------------------------------------------------------
// Full arms: add one entity after a `ClearAll`
// ---------------------------------------------------------------------------

/// Add one saved track — engine add command, every scalar, its plugin
/// chain — and mirror it at `order`. The caller keeps the order counter.
fn replay_track(r: &mut Resonance, pt: &ProjectTrack, order: usize) {
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
    let gui_plugins = replay_plugins(r, &pt.plugins, |pp| AudioCommand::AddPlugin {
        track_id,
        clap_file_path: pp.clap_file_path.clone(),
        clap_plugin_id: pp.clap_plugin_id.clone(),
        id: pp.instance_id,
    });

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
    // External-instrument mode is restored after every track, by the
    // `ExternalInstruments` reconcile domain (ARCH-01 A-13b).
}

fn replay_bus(r: &mut Resonance, pb: &ProjectBus) {
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

/// Add one saved plugin chain: instantiate each plugin on the engine
/// (via the target-specific `add_command`) and collect placeholder GUI
/// slots. The placeholders' params + has_gui are overwritten when the
/// instance's `PluginAdded` echo arrives from the engine, after this
/// restore returns; `adopt_live_instance` leaves the rest (name, bypass)
/// alone. The saved state blob, the bypass command and the param
/// overrides follow in `PluginState`, once every chain is added.
///
/// **A plugin that never comes back.** `AddPlugin` is fire-and-forget:
/// if the `.clap` isn't on this machine the engine replies with a
/// generic `AudioEvent::Error` and no `PluginAdded`, so the placeholder
/// stays in the chain with an empty `params` mirror and the engine has
/// no instance to save state from (see `PluginState` for what keeps its
/// settings alive).
fn replay_plugins<'p>(
    r: &mut Resonance,
    plugins: impl IntoIterator<Item = &'p ProjectPlugin>,
    mut add_command: impl FnMut(&ProjectPlugin) -> AudioCommand,
) -> Vec<PluginSlotState> {
    let mut gui_plugins = Vec::new();
    for pp in plugins {
        let _ = r.engine.send(add_command(pp));
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
        // `PluginState` sends the matching `SetPluginBypass`.
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
        // Plugin slot metadata: the human-visible name may change.
        // Matched by id: the chain still holds the old order (and the
        // fresh slots come after this). The per-slot bypass is
        // `PluginState`'s.
        rename_slots(&mut t.plugins, &b.plugins);
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
        rename_slots(&mut bus.plugins, &b.plugins);
    }
}

fn apply_master(r: &mut Resonance, a: &ProjectFile, b: &ProjectFile) {
    if a.master_fx_bypassed != b.master_fx_bypassed {
        r.master.fx_bypassed = b.master_fx_bypassed;
        let _ = r.engine.send(AudioCommand::SetMasterFxBypass {
            bypassed: b.master_fx_bypassed,
        });
    }
    rename_slots(&mut r.master.plugins, &b.master_plugins);
}

fn rename_slots(slots: &mut [PluginSlotState], saved: &[ProjectPlugin]) {
    for slot in slots {
        if let Some(pp) = saved.iter().find(|p| p.instance_id == slot.instance_id) {
            slot.plugin_name = pp.plugin_name.clone();
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin instances: which a diff restore keeps, and chain order
// ---------------------------------------------------------------------------

/// Every plugin instance `file` carries, with the chain that holds it.
pub(super) fn plugin_owners(file: &ProjectFile) -> HashMap<u64, (PluginLocator, &ProjectPlugin)> {
    let mut out = HashMap::new();
    for pt in &file.tracks {
        for pp in &pt.plugins {
            out.insert(pp.instance_id, (PluginLocator::Track(pt.id), pp));
        }
    }
    for pb in &file.busses {
        for pp in &pb.plugins {
            out.insert(pp.instance_id, (PluginLocator::Bus(pb.id), pp));
        }
    }
    for pp in &file.master_plugins {
        out.insert(pp.instance_id, (PluginLocator::Master, pp));
    }
    out
}

/// The tracks a diff restore keeps live (ARCH-01 A-13i): in both files,
/// with the same type and the same sub-track link, and — for a sub-track —
/// a kept parent (the engine's `RemoveTrack` drops a parent's sub-tracks
/// with it). Every other track of `new` is *fresh*: added as a load adds
/// it, its chain, output, external-instrument config, freeze source and
/// group-macro flags treated as after a `ClearAll`. Every other track of
/// `old` is removed. So a type change is a remove + add under the same
/// id, which is what the full replay did to it. Empty after a `ClearAll`.
pub(super) fn kept_tracks(old: Option<&ProjectFile>, new: &ProjectFile) -> HashSet<TrackId> {
    let Some(old) = old else {
        return HashSet::new();
    };
    let before: HashMap<TrackId, &ProjectTrack> = old.tracks.iter().map(|t| (t.id, t)).collect();
    let same: HashMap<TrackId, &ProjectTrack> = new
        .tracks
        .iter()
        .filter(|pt| {
            before.get(&pt.id).is_some_and(|was| {
                was.track_type == pt.track_type && was.sub_track == pt.sub_track
            })
        })
        .map(|pt| (pt.id, pt))
        .collect();
    same.iter()
        .filter(|(_, pt)| {
            pt.sub_track
                .is_none_or(|link| same.get(&link.parent_track_id).is_some_and(|p| p.sub_track.is_none()))
        })
        .map(|(id, _)| *id)
        .collect()
}

/// The plugin instances a diff restore keeps live (ARCH-01 A-13h): in
/// both files, on the same chain — a kept track's (A-13i), a bus's, or
/// the master's — with the same `.clap` identity. Every
/// other instance of `new` is *fresh* — added, then given its blob,
/// bypass and parked params as a load does — and every other instance of
/// `old` is removed. So an id whose identity changed (a relocated missing
/// plugin, `update::plugin_replace`) is removed and re-added, which is
/// what the full replay did to it. Empty after a `ClearAll`: everything
/// is fresh.
pub(super) fn kept_plugins(old: Option<&ProjectFile>, new: &ProjectFile) -> HashSet<u64> {
    let Some(old) = old else {
        return HashSet::new();
    };
    let before = plugin_owners(old);
    let tracks = kept_tracks(Some(old), new);
    plugin_owners(new)
        .into_iter()
        .filter(|(id, (owner, pp))| {
            let chain_kept = match owner {
                PluginLocator::Track(track_id) => tracks.contains(track_id),
                PluginLocator::Bus(_) | PluginLocator::Master => true,
            };
            chain_kept
                && before.get(id).is_some_and(|(was_owner, was)| {
                    was_owner == owner
                        && was.clap_plugin_id == pp.clap_plugin_id
                        && was.clap_file_path == pp.clap_file_path
                })
        })
        .map(|(id, _)| id)
        .collect()
}

/// The plugins of `chain` a diff restore adds, in chain order.
fn fresh<'p>(
    chain: &'p [ProjectPlugin],
    kept: &'p HashSet<u64>,
) -> impl Iterator<Item = &'p ProjectPlugin> + 'p {
    chain.iter().filter(|pp| !kept.contains(&pp.instance_id))
}

/// Diff arm of [`EntityOrder`]: every chain of `new` whose live order
/// differs is moved into it, engine and mirror alike.
fn order_live_chains(r: &mut Resonance, old: &ProjectFile, new: &ProjectFile) {
    let kept = kept_plugins(Some(old), new);
    let ids = |plugins: &[ProjectPlugin]| -> Vec<u64> {
        plugins.iter().map(|p| p.instance_id).collect()
    };
    let Resonance {
        registry,
        master,
        engine,
        io,
        ..
    } = r;
    for pt in &new.tracks {
        if let Some(t) = registry.tracks.iter_mut().find(|t| t.id == pt.id) {
            let track_id = t.id;
            order_chain(&mut t.plugins, &ids(&pt.plugins), &kept, |instance_id, to_index| {
                let _ = engine.send(AudioCommand::MovePlugin {
                    track_id,
                    instance_id,
                    to_index,
                });
                io.restore_echoes.expect_plugin_moved(instance_id, to_index);
            });
        }
    }
    for pb in &new.busses {
        if let Some(b) = registry.busses.iter_mut().find(|b| b.id == pb.id) {
            let bus_id = b.id;
            order_chain(&mut b.plugins, &ids(&pb.plugins), &kept, |instance_id, to_index| {
                let _ = engine.send(AudioCommand::MovePluginInBus {
                    bus_id,
                    instance_id,
                    to_index,
                });
                io.restore_echoes.expect_plugin_moved(instance_id, to_index);
            });
        }
    }
    order_chain(&mut master.plugins, &ids(&new.master_plugins), &kept, |instance_id, to_index| {
        let _ = engine.send(AudioCommand::MovePluginInMaster {
            instance_id,
            to_index,
        });
        io.restore_echoes.expect_plugin_moved(instance_id, to_index);
    });
}

/// Move `slots` into `target`'s order, one `send_move(instance, engine
/// index)` per slot that has to move, mirroring each move at once.
///
/// On entry the chain is what the removals and the entity domains left:
/// the kept slots in their old relative order, then the fresh ones in the
/// order they were appended — the engine's order too, since it appends an
/// add. Two passes, left to right:
///
/// 1. the kept slots into their target relative order (the fresh ones
///    stay at the tail);
/// 2. each fresh slot into its target position.
///
/// A move names an **engine** index ([`crate::plugin_chain::engine_slot_index`]):
/// a slot whose plugin is missing on this machine holds its place in the
/// app's chain but not in the engine's, and moving one sends nothing.
/// Kept slots go first because they are the ones known to exist: a fresh
/// slot is `Available` until its echo says otherwise, so a fresh plugin
/// that turns out missing is still counted, and pass 2 can then place a
/// later fresh plugin one engine slot off. That needs two plugins re-added
/// by one restore, one of them missing and out of append order; the
/// recovery path re-positions the missing one if it ever loads.
///
/// Sends nothing for a chain already in order. Ids in `target` that are
/// not in `slots` are skipped; slots not in `target` stay at the end.
pub(super) fn order_chain(
    slots: &mut [PluginSlotState],
    target: &[u64],
    kept: &HashSet<u64>,
    mut send_move: impl FnMut(PluginInstanceId, usize),
) {
    let present: Vec<u64> = target
        .iter()
        .copied()
        .filter(|id| slots.iter().any(|s| s.instance_id == *id))
        .collect();
    let kept_order: Vec<u64> = present.iter().copied().filter(|id| kept.contains(id)).collect();
    for (i, &id) in kept_order.iter().enumerate() {
        place(slots, id, i, &mut send_move);
    }
    for (i, &id) in present.iter().enumerate() {
        place(slots, id, i, &mut send_move);
    }
}

fn place(
    slots: &mut [PluginSlotState],
    id: u64,
    to: usize,
    send_move: &mut impl FnMut(PluginInstanceId, usize),
) {
    let Some(from) = slots.iter().position(|s| s.instance_id == id) else {
        return;
    };
    if from == to {
        return;
    }
    if from > to {
        slots[to..=from].rotate_right(1);
    } else {
        slots[from..=to].rotate_left(1);
    }
    // The engine index is the number of live plugins ahead of the slot
    // once it has moved.
    if !slots[to].availability.is_missing() {
        send_move(id, crate::plugin_chain::engine_slot_index(slots, to));
    }
}
