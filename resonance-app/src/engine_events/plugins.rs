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
    let mut recovered = false;
    if let Some(track) = r.registry.tracks.iter_mut().find(|t| t.id == track_id) {
        if let Some(slot) = track
            .plugins
            .iter_mut()
            .find(|p| p.instance_id == instance_id)
        {
            recovered = adopt_live_instance(slot, params, has_gui, has_sidechain_input);
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
    if recovered {
        let live = r
            .registry
            .tracks
            .iter()
            .find(|t| t.id == track_id)
            .and_then(|t| live_slot_index(&t.plugins, instance_id));
        restore_after_recovery(
            r,
            instance_id,
            live.map(|to_index| AudioCommand::MovePlugin {
                track_id,
                instance_id,
                to_index,
            }),
        );
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

/// Write a live instance's report onto the slot that was waiting for it,
/// and say whether that slot was **missing** until now.
///
/// The plain case is a project load or an add whose echo arrived: the
/// slot simply adopts the engine's parameter list and capability flags.
/// The interesting case is recovery — a slot the engine had already
/// refused (`PluginLoadFailed`) that now instantiates, because the user
/// relocated it, replaced it, or installed the plugin and rescanned. A
/// recovered slot needs two things a fresh one does not, and
/// [`restore_after_recovery`] does both: its preserved state pushed into
/// the new instance, and its POSITION restored, because the engine
/// appends to a chain that has been running without it.
fn adopt_live_instance(
    slot: &mut PluginSlotState,
    params: Vec<ParamInfo>,
    has_gui: bool,
    has_sidechain_input: bool,
) -> bool {
    let was_missing = slot.availability.is_missing();
    slot.params = params;
    slot.has_gui = has_gui;
    slot.has_sidechain_input = has_sidechain_input;
    slot.availability = PluginAvailability::Available;
    was_missing
}

/// Where the engine has to put this instance for the app's order and
/// the engine's processing order to agree.
///
/// Just [`plugin_chain::engine_slot_index`] with the slot located by id
/// first. The translation rule itself lives there, shared with
/// `update::plugin_replace`, because the two paths that name a position
/// to the engine — a recovered plugin and a replacement — must not be
/// able to disagree about what a position is.
///
/// `None` when `instance_id` isn't in the chain.
fn live_slot_index(chain: &[PluginSlotState], instance_id: PluginInstanceId) -> Option<usize> {
    let position = chain.iter().position(|p| p.instance_id == instance_id)?;
    Some(crate::plugin_chain::engine_slot_index(chain, position))
}

/// Finish a missing → available recovery: hand the new instance the
/// state the slot has been holding for it, and put it back where it
/// belongs in the engine's chain.
///
/// `reposition` is the chain-specific move command (`MovePlugin` /
/// `MovePluginInBus` / `MovePluginInMaster`), already aimed at the live
/// index. The engine clamps and no-ops a move that changes nothing, so
/// sending it unconditionally costs a command and nothing else.
///
/// The blob goes first and the parked parameter values go after it (the
/// caller runs [`apply_pending_param_overrides`] next), the same order
/// the load path uses: the blob is the plugin's whole state, the
/// parameter list is the user's explicit edits on top of it.
fn restore_after_recovery(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    reposition: Option<AudioCommand>,
) {
    if let Some(data) = r.plugin_state_cache.get(&instance_id) {
        let _ = r.engine.send(AudioCommand::LoadPluginState {
            instance_id,
            data: data.to_vec(),
        });
    }
    if let Some(cmd) = reposition {
        let _ = r.engine.send(cmd);
    }
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
            for saved in &overrides {
                if let Some(param) = slot.params.iter_mut().find(|p| p.id == saved.id) {
                    param.current_value = saved.value;
                    applied.push((saved.id, saved.value));
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
        let sub_id = r.allocate_track_id();
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
    // Drop the load-time copies too, so a removed slot can neither
    // resurrect a `plugin_*.bin` nothing references nor lend its parked
    // parameter list to a later instance that reuses the id.
    r.pending_plugin_param_overrides.remove(&instance_id);
    r.remove_plugin_index(instance_id);
    // The engine's `RemovePlugin` arm already dropped this instance's key
    // route, so only the mirror needs pruning here — but prune it we must,
    // or the route is written to the next save and reloads onto whatever
    // plugin later occupies this instance id (ba todo #1311).
    r.sidechain.clear_plugin(instance_id);
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
    r.plugin_catalog.available_plugins = plugins;
    r.view_caches.rebuild_plugins(&r.plugin_catalog.available_plugins);
    // A scan answers exactly one `PluginsScanned`, whether it was the
    // startup scan or a live rescan, so this is where a rescan ends
    // (ba todo #1307).
    r.plugin_catalog.plugin_scan_in_progress = false;
}

/// An add that produced no instance (ba doc #275 P5, todo #1309).
///
/// Two outcomes, decided by whether the failure names a slot:
///
/// * **it does** — the project-load replay and the control API both
///   pass an `id_hint`, so the slot that was created optimistically for
///   this instance is marked [`PluginAvailability::Missing`] and the
///   load warning is raised. The slot KEEPS its position in the chain
///   and keeps the preserved settings hanging off it (ba todo #1308);
///   only an explicit remove throws those away.
/// * **it does not** — the mixer's "+ FX" picker lets the engine
///   allocate, so a failure there added nothing and there is no slot to
///   mark. It surfaces as a plain error, which is all it ever was.
pub(super) fn load_failed(
    r: &mut Resonance,
    instance_id: Option<PluginInstanceId>,
    clap_plugin_id: String,
    clap_file_path: String,
    reason: String,
) {
    let marked = instance_id.is_some_and(|instance_id| {
        // Not `with_plugin_mut`: its `debug_assert` treats an unknown id
        // as a bug, and here an id with no slot is an ordinary outcome —
        // the plugin was removed while the add was in flight.
        mark_slot_missing(r, instance_id, &reason)
    });
    if marked {
        r.missing_plugins.note_failure();
    } else {
        r.banners.error_message = Some(format!(
            "Could not load plugin {}{}: {reason}",
            if clap_plugin_id.is_empty() {
                clap_file_path.as_str()
            } else {
                clap_plugin_id.as_str()
            },
            if clap_plugin_id.is_empty() {
                String::new()
            } else {
                format!(" ({clap_file_path})")
            }
        ));
    }
}

/// Flip one slot to [`PluginAvailability::Missing`], wherever it lives.
/// `false` when no chain carries `instance_id`.
fn mark_slot_missing(r: &mut Resonance, instance_id: PluginInstanceId, reason: &str) -> bool {
    let apply = |slot: &mut PluginSlotState| {
        slot.availability = PluginAvailability::Missing {
            reason: reason.to_owned(),
        };
        // A slot with no instance behind it has no editor window to
        // open; leaving the flag set would offer a control that reaches
        // nothing.
        slot.editor_open = false;
        slot.has_gui = false;
    };
    for track in &mut r.registry.tracks {
        if let Some(slot) = track
            .plugins
            .iter_mut()
            .find(|p| p.instance_id == instance_id)
        {
            apply(slot);
            return true;
        }
    }
    for bus in &mut r.registry.busses {
        if let Some(slot) = bus.plugins.iter_mut().find(|p| p.instance_id == instance_id) {
            apply(slot);
            return true;
        }
    }
    if let Some(slot) = r
        .master_plugins
        .iter_mut()
        .find(|p| p.instance_id == instance_id)
    {
        apply(slot);
        return true;
    }
    false
}

/// Bundles the scan could not load (ba todo #1307).
///
/// Recorded rather than logged: a `.clap` that fails to load is simply
/// absent from the catalog, which a user reads as "not installed" — and
/// then reinstalls it, and it fails the same way. Arrives before
/// `PluginsScanned`, so the list is in place by the time the refreshed
/// catalog lands.
pub(super) fn scan_failed(r: &mut Resonance, failures: Vec<PluginScanFailure>) {
    r.plugin_catalog.plugin_scan_failures = failures;
}

/// Adopt the plugin's own formatting of a parameter it was just given
/// (ba todo #1290, finding X8).
///
/// The app mirrors a parameter's *number* the moment it sends the change
/// — that is what keeps a knob under the cursor — but only the plugin
/// can turn that number into `"Low-pass"` or `"40 %"`, and the mirror's
/// text was captured once, at instantiation. This is the echo that keeps
/// the two in step, for the generic panel and for
/// `track/bus/master.plugin_params` alike.
///
/// A stale echo is dropped rather than applied: a knob drag issues one
/// set per frame, so an echo for a value the parameter has already left
/// would paint text that disagrees with the number beside it. Matching
/// on the value the change was made with is exact — the app stored that
/// same f64 — so no tolerance is needed.
pub(super) fn param_text(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    param_id: u32,
    value: f64,
    text: String,
) {
    r.with_plugin_mut(instance_id, |slot| {
        if let Some(param) = slot.params.iter_mut().find(|p| p.id == param_id) {
            if param.current_value == value {
                // A unit belongs to the parameter, not to the value, so
                // adopt one the plugin reveals here (a fader that read
                // "-inf dB" when it loaded had none to take) but never
                // forget one it has already given.
                let unit = resonance_audio::unit_from_text(&text);
                if !unit.is_empty() {
                    param.unit = unit.to_string();
                }
                param.text = text;
            }
        }
    });
}

pub(super) fn state_saved(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    data: Vec<u8>,
) {
    // Also feeds the undo system's plugin-state cache so snapshots can
    // replay internal CLAP state on restore. The project-save path
    // drains the cache via `SaveAllPluginStates` separately.
    // A `*.save_plugin_preset` armed this capture: the blob that just
    // arrived IS the sound, including whatever the plugin keeps outside
    // its parameters (an amp's model path, an IR's file). Write it before
    // the cache insert so a failure is reported against the request that
    // asked for it.
    if r
        .pending_plugin_preset_save
        .as_ref()
        .is_some_and(|p| p.instance_id == instance_id)
    {
        let pending = r.pending_plugin_preset_save.take().expect("just checked");
        if let Err(e) = crate::update::control::write_plugin_preset(
            r,
            &pending.clap_id,
            &pending.name,
            &data,
        ) {
            r.banners.error_message =
                Some(format!("Could not save preset {:?}: {e}", pending.name));
        }
    }

    r.plugin_state_cache.insert(instance_id, data.into());
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
    let mut recovered = false;
    if let Some(bus) = r.registry.busses.iter_mut().find(|b| b.id == bus_id) {
        if let Some(slot) = bus
            .plugins
            .iter_mut()
            .find(|p| p.instance_id == instance_id)
        {
            recovered = adopt_live_instance(slot, params, has_gui, has_sidechain_input);
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
    if recovered {
        let live = r
            .registry
            .busses
            .iter()
            .find(|b| b.id == bus_id)
            .and_then(|b| live_slot_index(&b.plugins, instance_id));
        restore_after_recovery(
            r,
            instance_id,
            live.map(|to_index| AudioCommand::MovePluginInBus {
                bus_id,
                instance_id,
                to_index,
            }),
        );
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
    // Drop the load-time copies too, so a removed slot can neither
    // resurrect a `plugin_*.bin` nothing references nor lend its parked
    // parameter list to a later instance that reuses the id.
    r.pending_plugin_param_overrides.remove(&instance_id);
    r.remove_plugin_index(instance_id);
    drop_route_onto_removed_chain_plugin(r, instance_id);
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
    let mut recovered = false;
    if let Some(slot) = r
        .master_plugins
        .iter_mut()
        .find(|p| p.instance_id == instance_id)
    {
        recovered = adopt_live_instance(slot, params, has_gui, has_sidechain_input);
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
    if recovered {
        let live = live_slot_index(&r.master_plugins, instance_id);
        restore_after_recovery(
            r,
            instance_id,
            live.map(|to_index| AudioCommand::MovePluginInMaster {
                instance_id,
                to_index,
            }),
        );
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
    // Drop the load-time copies too, so a removed slot can neither
    // resurrect a `plugin_*.bin` nothing references nor lend its parked
    // parameter list to a later instance that reuses the id.
    r.pending_plugin_param_overrides.remove(&instance_id);
    r.remove_plugin_index(instance_id);
    drop_route_onto_removed_chain_plugin(r, instance_id);
}

/// Drop the key route onto a plugin that has just come off a **bus** or
/// the **master** chain, and tell the engine to drop it as well.
///
/// The extra command is what separates this from the track case. The
/// engine's dispatcher drops a route on `RemovePlugin` but *not* on
/// `RemovePluginFromBus` / `RemovePluginFromMaster`, so pruning only the
/// mirror would leave the engine still keying an instance id that the
/// next `bus.add_effect` can be handed — the recycled-id misroute the
/// engine's own `drop_plugin_route` exists to prevent. `ClearSidechainRoute`
/// is idempotent (the engine echoes only when a route was present), so
/// sending it unconditionally costs nothing when there was no route.
fn drop_route_onto_removed_chain_plugin(r: &mut Resonance, instance_id: PluginInstanceId) {
    if r.sidechain.clear_plugin(instance_id) {
        let _ = r.engine.send(AudioCommand::ClearSidechainRoute {
            plugin: instance_id,
        });
    }
}

/// Reconcile the GUI key-route mirror to the engine's echo (ba todo
/// #1311). `source: None` means the route was cleared.
///
/// The engine is the authority: it stores the route, decides whether it
/// survives a plugin or source removal, and only ever echoes resolved
/// state. Mirroring the echo — rather than leaving the optimistic write
/// from `PluginMessage::SetPluginSidechain` as the last word — is what
/// keeps a route the engine silently dropped out of the next save.
pub(super) fn sidechain_route_changed(
    r: &mut Resonance,
    plugin: PluginInstanceId,
    source: Option<SendSource>,
    enabled: bool,
) {
    match source {
        Some(source) => r.sidechain.upsert(SidechainRoute {
            plugin,
            source,
            enabled,
        }),
        None => {
            r.sidechain.clear_plugin(plugin);
        }
    }
}

pub(super) fn master_fx_bypass_changed(r: &mut Resonance, bypassed: bool) {
    r.master_fx_bypassed = bypassed;
}

/// Adopt the engine's per-slot bypass echo (ba todo #1305).
///
/// The app never sets this optimistically. `SetPluginBypass` starts a
/// crossfade rather than flipping a switch, so the echo is the only point
/// at which the flag has actually moved — and the same echo carries a
/// restore issued by project load, a GUI click and a control-API call
/// alike, so all three land through one path and cannot disagree.
///
/// `own_bypass_param` is reported but not stored: it says whether the
/// plugin keeps running (and so keeps its latency in the chain) rather
/// than being skipped, which is the engine's business, not a fact the
/// app mirrors.
pub(super) fn bypass_changed(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    bypassed: bool,
    _own_bypass_param: bool,
) {
    r.with_plugin_mut(instance_id, |slot| slot.bypassed = bypassed);
}

/// Adopt the engine's editor-state echo (ba todo #1347).
///
/// `editor_open` now reflects only what the engine reported. Three things
/// reach here: a successful open, a close (host- or user-initiated,
/// including a window closed from its own titlebar), and a failed open.
///
/// On a FAILURE the slot is also selected into the generic parameter
/// panel. That panel is reachable for every plugin since ba todo #1306,
/// and it is the only thing left to fall back on when a floating editor
/// refuses — otherwise the press appears to do nothing at all. The
/// error banner still fires from the accompanying `AudioEvent::Error`,
/// so the user gets both the reason and somewhere to go.
pub(super) fn editor_state(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    open: bool,
    failure: Option<resonance_audio::types::PluginEditorFailure>,
) {
    r.with_plugin_mut(instance_id, |slot| slot.editor_open = open);
    if failure.is_some() {
        r.mixer.selected_plugin = Some(instance_id);
    }
}
