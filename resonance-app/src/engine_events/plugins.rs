//! App-side handlers for plugin lifecycle events from the engine — one
//! set for every chain owner (a track's, a bus's or the master's chain;
//! code review ARCH2-02), plus the sub-track auto-creation policy that a
//! track's `PluginAdded` triggers.

use resonance_audio::types::*;

use crate::state::*;
use crate::Resonance;

/// The `PluginAdded` echo for any chain owner.
#[allow(clippy::too_many_arguments)]
pub(super) fn added(
    r: &mut Resonance,
    owner: ChainOwner,
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
    // The add a diff restore owed, first: the echo answers it whether or
    // not the instance is still wanted (FU-A13d).
    r.io.restore_echoes.settle_plugin_added(instance_id);
    // A diff restore removed this instance after adding it, before this
    // echo arrived: the engine has already dropped it (ARCH-01 A-13h)…
    if r.io.restore_echoes.plugin_removal_owed(instance_id)
        // …or removed its track or bus (A-13i), which drops the chain
        // with it.
        || owner_removal_owed(r, owner)
    {
        return;
    }
    // Idempotent: if the plugin slot already exists (created by project load),
    // just update its params and has_gui. Otherwise push a new slot.
    let mut inserted = false;
    let mut recovered = false;
    if let Some(chain) = r.chain_mut(owner) {
        if let Some(slot) = chain.iter_mut().find(|p| p.instance_id == instance_id) {
            recovered = adopt_live_instance(slot, params, has_gui, has_sidechain_input);
        } else {
            chain.push(
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
        r.insert_plugin_index(instance_id, owner);
    }
    if recovered {
        let live = r
            .chain(owner)
            .and_then(|chain| live_slot_index(chain, instance_id));
        restore_after_recovery(
            r,
            instance_id,
            live.map(|to_index| AudioCommand::MovePlugin {
                owner,
                instance_id,
                to_index,
            }),
        );
    }

    // If this plugin was added as part of a track preset, load the saved
    // plugin state blob (if any). Pop the first entry from the pending
    // list to stay in order. Presets are per track; a bus or master add
    // never carries one.
    if let ChainOwner::Track(track_id) = owner {
        if let Some((pending_track, ref mut states)) = r.presets.pending_preset_plugin_states {
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
        if r.presets.pending_preset_plugin_states
            .as_ref()
            .map(|(_, s)| s.is_empty())
            .unwrap_or(false)
        {
            r.presets.pending_preset_plugin_states = None;
        }
    }

    apply_pending_param_overrides(r, instance_id);
    crate::update::control::plugin_presets::apply_pending_preset(r, instance_id);

    // Seed the undo plugin-state cache with the plugin's initial CLAP
    // state. Snapshots taken before the user interacts with the plugin
    // will have the default blob to restore to, avoiding "undo resets
    // the plugin to uninitialised garbage" UX.
    let _ = r.engine
        .send(AudioCommand::SavePluginState { instance_id });

    // The port layout, for the sub-track policy. Only a track chain has
    // sub-tracks ([`ensure_instance_subtracks`] returns at once for any
    // other owner), so the layout is kept for a track's plugin only.
    if owner.track_id().is_some() {
        let mut port_names = output_port_names;
        port_names.resize_with(output_port_count.max(port_names.len()), String::new);
        port_names.truncate(output_port_count);
        r.plugin_mirror.output_ports.insert(instance_id, port_names);
        ensure_instance_subtracks(r, instance_id);
    }
}

/// Whether a diff restore has removed `owner` itself and is still owed
/// the echo (ARCH-01 A-13i): an echo for a plugin on it is then stale.
fn owner_removal_owed(r: &Resonance, owner: ChainOwner) -> bool {
    match owner {
        ChainOwner::Track(track_id) => r.io.restore_echoes.track_removal_owed(track_id),
        ChainOwner::Bus(bus_id) => r.io.restore_echoes.bus_removal_owed(bus_id),
        ChainOwner::Master => false,
    }
}

/// Run the sub-track policy ([`ensure_subtracks`]) for one track plugin,
/// from its stored port layout — when it is added, when the drums'
/// `output_mode` is edited, and when a rescan reports it moved Stereo ->
/// Multi (drums-plugin-rework.md §8, E11). Never on a rescan that finds it
/// already in Multi: that would re-create a sub-track the user deleted.
///
/// A Resonance Drums instance gets its per-pad sub-tracks only in **Multi**
/// output mode: in Stereo (the default for a new instance) every pad sums
/// to the main output and the other ports are silent, so sub-tracks would
/// be dead faders. Switching to Multi creates the missing ones. Switching
/// back to Stereo **keeps** the ones that exist: they are the user's
/// tracks (fader, routing, names) and go quiet rather than vanish; delete
/// them by hand, or undo the switch, which takes back the sub-tracks it
/// created with it. Every other multi-output plugin gets its sub-tracks as
/// soon as it is added, as before.
pub(crate) fn ensure_instance_subtracks(r: &mut Resonance, instance_id: PluginInstanceId) {
    let Some(ChainOwner::Track(track_id)) = r.plugin_mirror.index.get(&instance_id).copied()
    else {
        return;
    };
    let Some(ports) = r.plugin_mirror.output_ports.get(&instance_id).cloned() else {
        return;
    };
    let routes = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .and_then(|t| t.plugins.iter().find(|s| s.instance_id == instance_id))
        .is_some_and(crate::drums_mirror::routes_to_ports);
    if routes {
        ensure_subtracks(r, track_id, ports.len(), &ports);
    }
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
/// `reposition` is the owner's `MovePlugin`, already aimed at the live
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
    if let Some(data) = r.plugin_mirror.state_cache.get(&instance_id) {
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
/// Called from the `PluginAdded` handler, for every chain owner,
/// *after* it has written the event's `params` into the slot,
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
/// the user just typed. Ordering is safe: the load queues
/// `LoadPluginState` (the `PluginState` reconcile domain) inside the same
/// synchronous restore that parks these values, and this event is only
/// handled after that restore returns, so the per-param sends land after
/// the state blob and win over it.
///
/// Ids not present on the instantiated plugin are skipped: a plugin that
/// dropped or renumbered a parameter between versions must not have a
/// stale id pushed at it. So are params the host does not persist
/// ([`ParamInfo::host_persisted`]): a project written before the plugin
/// declared one state-excluded (the drums' `kit_select`) still carries
/// it, and re-sending it after the blob would override the kit the blob
/// just recalled.
///
/// [`ParamInfo::host_persisted`]: resonance_audio::types::ParamInfo::host_persisted
fn apply_pending_param_overrides(r: &mut Resonance, instance_id: PluginInstanceId) {
    let Some(overrides) = r.presets.pending_plugin_param_overrides.remove(&instance_id) else {
        return;
    };
    let applied = r
        .with_plugin_mut(instance_id, |slot| {
            let mut applied = Vec::new();
            for saved in &overrides {
                if let Some(param) = slot
                    .params
                    .iter_mut()
                    .find(|p| p.id == saved.id && p.host_persisted())
                {
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
/// part of event handling. It is called from [`added`] after PluginAdded,
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
    let Some((parent_name, parent_color)) = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == parent_track_id)
        .map(|t| (t.name.clone(), t.color))
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
        let mut sub = TrackState::new_sub_track(
            sub_id,
            order,
            sub_name,
            parent_track_id,
            port_idx as u32,
        );
        // A sub-track is one tap of its parent's instrument, so it reads
        // as the same track: it inherits the parent's colour.
        sub.color = parent_color;
        r.registry.tracks.push(sub);
    }
}

/// The `PluginRemoved` echo. Swallowed when a diff restore or a live
/// delete already mirrored it — the id may by now be an instance a later
/// restore re-added (ARCH-01 A-13h) — mirrored otherwise.
pub(super) fn removed_echo(r: &mut Resonance, owner: ChainOwner, instance_id: PluginInstanceId) {
    if r.io.restore_echoes.settle_plugin_removed(instance_id) {
        return;
    }
    removed(r, owner, instance_id);
}

/// Mirror a plugin's removal from `owner`'s chain: the slot, its cached
/// blob, parked params, side-index entry, selection, key route and
/// automation lanes. Called by the live delete at once (STATE-10 shape,
/// FU-A13c) and by [`removed_echo`] for a removal nobody mirrored yet.
pub(crate) fn removed(r: &mut Resonance, owner: ChainOwner, instance_id: PluginInstanceId) {
    r.ui.mixer.forget_plugin(instance_id);
    if let Some(chain) = r.chain_mut(owner) {
        chain.retain(|p| p.instance_id != instance_id);
    }
    r.plugin_mirror.state_cache.remove(&instance_id);
    r.plugin_mirror.owed_blobs.remove(&instance_id);
    r.plugin_mirror.kit_info.remove(&instance_id);
    r.plugin_mirror.output_ports.remove(&instance_id);
    // Drop the load-time copies too, so a removed slot can neither
    // resurrect a `plugin_*.bin` nothing references nor lend its parked
    // parameter list to a later instance that reuses the id.
    r.presets.pending_plugin_param_overrides.remove(&instance_id);
    crate::update::plugin_preset_ui::forget_instance(r, instance_id);
    r.remove_plugin_index(instance_id);
    // The engine's `RemovePlugin` arm drops this instance's key route
    // itself, on every owner (ARCH2-02), so only the mirror needs pruning
    // here — but prune it we must, or the route is written to the next
    // save and reloads onto whatever plugin later occupies this instance
    // id (ba todo #1311).
    r.sidechain.clear_plugin(instance_id);
    drop_plugin_lanes(r, instance_id);
}

/// Drop every automation lane aimed at a plugin instance that has just
/// left its chain, in the mirror and in the engine (automation-control-api
/// D1). The engine keys lanes by target and does not prune them on a
/// plugin removal; an orphaned lane is inert but saved, and reloads onto
/// whatever plugin later occupies the id — the same leak #1311 closed for
/// key routes. Track deletion does the same for a whole chain
/// (`engine_events::tracks::drop_track_references`).
///
/// The removal is a recorded edit, so an undo brings the lanes back: the
/// snapshot carries them and `restore_automation_lanes` re-sends every
/// lane the mirror lacks. A missing plugin's restore (a relocate) keeps
/// its instance id and never comes through here, so its lanes survive.
pub(crate) fn drop_plugin_lanes(r: &mut Resonance, instance_id: PluginInstanceId) {
    use resonance_common::AutomationTarget as T;

    let stale: Vec<T> = r
        .automation
        .lanes
        .keys()
        .filter(|t| matches!(t, T::PluginParam { instance, .. } if *instance == instance_id))
        .cloned()
        .collect();
    for target in stale {
        r.automation.lanes.remove(&target);
        r.automation.live_values.remove(&target);
        let _ = r.engine.send(AudioCommand::ClearAutomationLane { target });
    }
}

/// Mirror an engine-side chain reorder (`AudioCommand::MovePlugin` ->
/// `AudioEvent::PluginMoved`, ba todo #1224 / doc #273 todo #1237) onto
/// the app's chain for `owner`.
///
/// The engine is the source of truth for chain order and reports the slot
/// the plugin actually landed on *after* its own clamping, so this replays
/// that index rather than re-deriving it. Mirroring matters beyond the
/// display: the `Vec`'s order is what project serialization writes, so a
/// reorder only survives save/load if it lands here too. `plugin_mirror.index` is
/// keyed by owner, not by slot, so it needs no update.
pub(super) fn moved(
    r: &mut Resonance,
    owner: ChainOwner,
    instance_id: PluginInstanceId,
    to_index: usize,
) {
    // A diff restore's reorder, already mirrored (ARCH-01 A-13h).
    if r.io.restore_echoes.settle_plugin_moved(instance_id, to_index) {
        return;
    }
    mirror_plugin_move(r, owner, instance_id, to_index);
}

/// The mirror itself, shared with the control API's
/// `track/bus/master.move_effect` (ba doc #273, todo #1225), which applies
/// the order immediately so a client can read back what it just set
/// instead of waiting a cycle for `PluginMoved`. Applying it twice is
/// harmless: the second call finds the plugin already at `to_index` and
/// returns.
pub(crate) fn mirror_plugin_move(
    r: &mut Resonance,
    owner: ChainOwner,
    instance_id: PluginInstanceId,
    to_index: usize,
) {
    let Some(chain) = r.chain_mut(owner) else {
        return;
    };
    let Some(from) = chain.iter().position(|p| p.instance_id == instance_id) else {
        return;
    };
    // `from` was found, so the chain is non-empty and this cannot wrap.
    let to = to_index.min(chain.len() - 1);
    if from == to {
        return;
    }
    let slot = chain.remove(from);
    chain.insert(to, slot);
}

pub(super) fn scanned(r: &mut Resonance, plugins: Vec<ScannedPlugin>) {
    r.plugin_catalog.available_plugins = plugins;
    r.ui.view_caches.rebuild_plugins(&r.plugin_catalog.available_plugins);
    crate::plugin_preset_library::register_scanned(r);
    crate::update::plugin_preset_ui::rebuild_caches(r);
    // A scan answers exactly one `PluginsScanned`, whether it was the
    // startup scan or a live rescan, so this is where a rescan ends
    // (ba todo #1307).
    r.plugin_catalog.plugin_scan_in_progress = false;
}

/// An add that produced no instance (ba doc #275 P5, todo #1309).
///
/// Since ARCH-04 D-1 every add carries an app-allocated id, so
/// `instance_id` is always `Some` here — what still varies is whether a
/// slot was already mirrored *for* that id when the failure lands. Two
/// outcomes, decided by whether marking a slot missing actually finds one:
///
/// * **it does** — the project-load replay and the control API's
///   `...WithId` paths mirror a placeholder slot up front, so this
///   instance is marked [`PluginAvailability::Missing`] and the load
///   warning is raised. The slot KEEPS its position in the chain and
///   keeps the preserved settings hanging off it (ba todo #1308); only an
///   explicit remove throws those away.
/// * **it does not** — the mixer's "+ FX" picker (`AddPluginToTrack` and
///   its bus/master siblings) doesn't mirror anything until the
///   `PluginAdded` echo lands, so a failure there finds no slot to mark
///   even though the id was known. It surfaces as a plain error, which
///   is all it ever was.
pub(super) fn load_failed(
    r: &mut Resonance,
    instance_id: Option<PluginInstanceId>,
    clap_plugin_id: String,
    clap_file_path: String,
    reason: String,
) {
    // A diff restore's add (FU-A13d): the restore counted this instance
    // as live when it named the chain's engine indices.
    let restored =
        instance_id.is_some_and(|instance_id| r.io.restore_echoes.settle_plugin_added(instance_id));
    let marked = instance_id.is_some_and(|instance_id| {
        // Not `with_plugin_mut`: its `debug_assert` treats an unknown id
        // as a bug, and here an id with no slot is an ordinary outcome —
        // the plugin was removed while the add was in flight.
        mark_slot_missing(r, instance_id, &reason)
    });
    if let Some(instance_id) = instance_id.filter(|_| restored) {
        r.io.restore_echoes.forget_plugin_moves(instance_id);
        if marked {
            reposition_after_missing(r, instance_id);
        }
    }
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

/// Put every live slot after `missing` back where the app's chain says,
/// now that `missing` is known to be absent from the engine's (FU-A13d).
///
/// A diff restore re-adds the plugins its target has and the live chain
/// lacks, then moves each into place by **engine** index
/// ([`crate::plugin_chain::engine_slot_index`]). A fresh instance is
/// `Available` until its echo lands, so every move that restore named for
/// a slot after one that then fails to load counted it, and is one engine
/// slot too far right: two plugins re-added by one restore, the first of
/// them missing, came back in swapped order in the engine while the
/// mixer showed them right.
///
/// The moves only ever misplace slots after the missing one (the reorder
/// runs left to right, so a slot moved past it stays past it), and the
/// engine runs these after every command the restore sent, so moving
/// each of those slots, left to right, to its engine index restores the
/// order whatever the engine made of the earlier moves. The mirror is
/// already in that order: each echo is owed and swallowed rather than
/// replayed as an app index. A fresh plugin further right that fails
/// too is still counted here; its own failure runs this again.
fn reposition_after_missing(r: &mut Resonance, missing: PluginInstanceId) {
    let Some(&owner) = r.plugin_mirror.index.get(&missing) else {
        return;
    };
    let chain: &[PluginSlotState] = r.chain(owner).unwrap_or(&[]);
    let Some(at) = chain.iter().position(|p| p.instance_id == missing) else {
        return;
    };
    let moves: Vec<(PluginInstanceId, usize)> = chain
        .iter()
        .enumerate()
        .skip(at + 1)
        .filter(|(_, p)| !p.availability.is_missing())
        .map(|(i, p)| (p.instance_id, crate::plugin_chain::engine_slot_index(chain, i)))
        .collect();
    for (instance_id, to_index) in moves {
        let _ = r.engine.send(AudioCommand::MovePlugin {
            owner,
            instance_id,
            to_index,
        });
        r.io.restore_echoes.expect_plugin_moved(instance_id, to_index);
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
        .master.plugins
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
    // Feeds the undo system's plugin-state cache so snapshots can replay
    // internal CLAP state on restore. The project-save path drains the
    // cache via `SaveAllPluginStates` separately.
    let blob: std::sync::Arc<[u8]> = data.into();
    // A refresh that was owed (an editor kit pick, STATE2-07): this echo
    // is the state after the change it was asked for, and fills the
    // snapshots taken while waiting for it — the same `Arc` the cache
    // takes, so restoring one of them finds the blob live and pushes
    // nothing.
    let mut superseded = false;
    if let Some(owed) = r.plugin_mirror.owed_blobs.get_mut(&instance_id) {
        owed.echoes += 1;
        let echo = owed.echoes;
        if let Ok(mut slots) = owed.slots.lock() {
            slots.retain(|(mark, late)| {
                if *mark != echo {
                    return true;
                }
                if let Ok(mut slot) = late.lock() {
                    *slot = Some(blob.clone());
                }
                false
            });
        }
        superseded = owed.superseded;
        if owed.echoes >= owed.marks {
            r.plugin_mirror.owed_blobs.remove(&instance_id);
        }
    }
    // An undo / redo restored a state since the refresh was asked for:
    // this blob describes the state it replaced.
    if !superseded {
        r.plugin_mirror.state_cache.insert(instance_id, blob);
    }
}

/// A `*.save_plugin_preset` (or a host bar's Save) armed this capture:
/// the preset form of the plugin's state — its sound, including what it
/// keeps outside its parameters (an amp's model by content id, an IR's
/// file), and none of its session state.
pub(super) fn preset_state_saved(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    data: Vec<u8>,
    preset_form: bool,
    first_party: bool,
) {
    let Some(pending) = r.presets.pending_plugin_preset_saves.remove(&instance_id) else {
        return;
    };
    let saved = crate::update::control::write_plugin_preset(
        r,
        &pending,
        &data,
        crate::update::control::SavedStateKind {
            first_party,
            preset_form,
        },
    );
    crate::update::plugin_preset_ui::library_changed(r);
    if let Err(e) = saved {
        r.banners.error_message = Some(format!("Could not save preset {:?}: {e}", pending.name));
    }
}

/// The plugin says it loaded a preset (`clap_host_preset_load.loaded`).
/// For a plugin that reports its identity itself this adds nothing; for
/// any other it is the identity: a factory preset by its load key, a file
/// by its path (slice P5; P8 names discovered presets properly).
pub(super) fn preset_loaded(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    location: resonance_audio::types::PluginPresetLocation,
    load_key: Option<String>,
) {
    use resonance_audio::types::PluginPresetLocation as L;
    use resonance_plugin::presets::PresetSource as PluginPresetSource;
    if r
        .presets
        .plugin_preset_identity
        .get(&instance_id)
        .is_some_and(|i| i.reported)
    {
        return;
    }
    // A preset the plugin's discovery listed is named the way the library
    // lists it (its stable id and discovered name), so a `loaded()` echo of
    // a host load confirms that identity instead of replacing it.
    let clap_id = r.with_plugin_mut(instance_id, |slot| slot.clap_plugin_id.clone());
    let as_discovered = match &location {
        L::Plugin => resonance_audio::types::DiscoveredLocation::Plugin,
        L::File(p) => resonance_audio::types::DiscoveredLocation::File(p.clone()),
    };
    let discovered = clap_id
        .and_then(|id| r.presets.discovered.get(&id))
        .and_then(|list| list.iter().find(|p| p.is_at(&as_discovered, load_key.as_deref())))
        .map(|p| (p.stable_id(), p.name.clone()));
    let (source, id, name) = match (discovered, location) {
        (Some((id, name)), _) => (PluginPresetSource::Factory, id, name),
        (None, L::Plugin) => {
            let key = load_key.unwrap_or_default();
            (PluginPresetSource::Factory, key.clone(), key)
        }
        (None, L::File(path)) => {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let id = match load_key {
                Some(k) => format!("{}#{k}", path.display()),
                None => path.display().to_string(),
            };
            (PluginPresetSource::User, id, stem)
        }
    };
    r.presets.plugin_preset_identity.insert(
        instance_id,
        crate::state::presets::SlotPresetIdentity {
            source,
            id,
            name,
            modified: false,
            reported: false,
        },
    );
}

/// The full state a preset load saved before loading: it fills the undo
/// entries and audition origins waiting on `token`, and completes a revert
/// that happened before it arrived.
pub(super) fn state_captured(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    token: u64,
    data: Vec<u8>,
    after: bool,
) {
    let blob: std::sync::Arc<[u8]> = data.into();
    if after {
        // The state the load left: the live cache (a fresh `Arc`, so an
        // undo pushes its own blob over it) and the redo snapshots taken
        // while it was owed.
        let mut superseded = false;
        if let Some(owed) = r.presets.pending_after.remove(&token) {
            // An undo / redo since the load: the live state is no longer
            // this one; it only fills the snapshots waiting on it.
            superseded = owed.superseded;
            let slots = owed.slots.lock().map(|s| s.clone()).unwrap_or_default();
            for late in slots {
                if let Ok(mut slot) = late.lock() {
                    *slot = Some(blob.clone());
                }
            }
        }
        if !superseded && r.plugin_slot(instance_id).is_some() {
            r.plugin_mirror.state_cache.insert(instance_id, blob);
        }
        return;
    }
    for late in r.presets.pending_captures.remove(&token).unwrap_or_default() {
        if let Ok(mut slot) = late.lock() {
            *slot = Some(blob.clone());
        }
    }
    if r.presets.revert_on_capture.remove(&token) == Some(instance_id) {
        let _ = r.engine.send(AudioCommand::LoadPluginState {
            instance_id,
            data: blob.to_vec(),
        });
        r.plugin_mirror.state_cache.insert(instance_id, blob);
    }
}

/// A plugin's preset-discovery factory listed its presets (slice P8): they
/// join the library as read-only factory presets (after any compiled-in
/// bank), loadable through `clap.preset-load`. A preset the provider flags
/// as a favourite is starred once, when the user has never marked it.
pub(super) fn presets_discovered(
    r: &mut Resonance,
    plugin_id: String,
    presets: Vec<resonance_audio::types::DiscoveredPreset>,
) {
    use resonance_plugin::presets::FactoryEntry;
    let lib = crate::plugin_preset_library::library(r);
    let mut entries: Vec<FactoryEntry> = r
        .plugin_catalog
        .available_plugins
        .iter()
        .find(|p| p.clap_plugin_id == plugin_id)
        .map(|p| {
            p.factory_presets
                .iter()
                .map(|e| FactoryEntry::from_parts(&e.id, &e.name, &e.json, e.meta.as_deref()))
                .collect()
        })
        .unwrap_or_default();
    for p in &presets {
        let meta = serde_json::json!({
            "author": (!p.creators.is_empty()).then(|| p.creators.join(", ")),
            "description": p.description,
            "tags": p.features,
        });
        entries.push(FactoryEntry::from_parts(
            &p.stable_id(),
            &p.name,
            r#"{"version":1,"params":{}}"#,
            Some(&meta.to_string()),
        ));
    }
    lib.register_factory_entries(&plugin_id, entries);
    // A provider's favourite is starred the first time it is seen, and
    // only then: the mark records that, so un-starring it sticks.
    const SEEDED: &str = "discovery_favorite_seeded";
    for p in presets.iter().filter(|p| p.is_favorite()) {
        let key = resonance_plugin::presets::mark_key(&plugin_id, &p.stable_id());
        let seeded = lib.marks().marks(&key).extra.contains_key(SEEDED);
        if !seeded {
            let _ = lib.marks().update(&key, &|m| {
                m.favorite = true;
                m.extra.insert(SEEDED.to_string(), serde_json::Value::Bool(true));
            });
        }
    }
    r.presets.discovered.insert(plugin_id, presets);
    crate::update::plugin_preset_ui::rebuild_caches(r);
}

/// The plugin's params after a preset state load: the mirror takes every
/// value and its text (a third-party preset's values are only known to the
/// plugin, slice P7). Ids the mirror does not have are ignored.
pub(super) fn params_refreshed(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    params: Vec<ParamInfo>,
) {
    let (drums, to_multi) = r
        .with_plugin_mut(instance_id, |slot| {
            let was = crate::drums_mirror::routes_to_ports(slot);
            for fresh in &params {
                if let Some(p) = slot.params.iter_mut().find(|p| p.id == fresh.id) {
                    p.current_value = fresh.current_value;
                    p.text = fresh.text.clone();
                }
            }
            let now = crate::drums_mirror::routes_to_ports(slot);
            (crate::drums_mirror::is_drums(slot), !was && now)
        })
        .unwrap_or((false, false));
    if drums {
        // A state load may set the kit, and a whole state blob (a
        // project's, an undo's) the output mode too — a plugin preset
        // leaves `output_mode` alone. The sub-track policy runs only on
        // the Stereo -> Multi transition (see `param_values_changed`).
        crate::update::compose::refresh_kit_pads(r);
        if to_multi {
            ensure_instance_subtracks(r, instance_id);
        }
    }
}

/// A plugin's values rescan: the params that moved, with their text (a
/// load progress, a selection the plugin derived itself). The mirror takes
/// them; ids it does not have are ignored.
pub(super) fn param_values_changed(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    values: Vec<resonance_audio::types::ParamValueUpdate>,
) {
    let (drums, to_multi) = r
        .with_plugin_mut(instance_id, |slot| {
            let was = crate::drums_mirror::routes_to_ports(slot);
            for fresh in values {
                if let Some(p) = slot.params.iter_mut().find(|p| p.id == fresh.id) {
                    p.current_value = fresh.value;
                    p.text = fresh.text;
                }
            }
            let now = crate::drums_mirror::routes_to_ports(slot);
            (crate::drums_mirror::is_drums(slot), !was && now)
        })
        .unwrap_or((false, false));
    if drums {
        // `kit_select`'s text names the kit the picker shows, and a state
        // load may have moved `output_mode` (a v1 state loads as Multi).
        crate::update::compose::refresh_kit_pads(r);
        // The sub-track policy runs on the Stereo -> Multi transition
        // only. A rescan that finds the instance already in Multi (a kit
        // load's stages, a selection the plugin derived) must not bring
        // back a sub-track the user deleted — and it would, outside undo.
        if to_multi {
            ensure_instance_subtracks(r, instance_id);
        }
    }
}

/// A Resonance Drums instance reported the pads of the kit it now plays
/// (`com.resonance.kit-info`): mirror it, and re-derive the kit picker.
pub(super) fn kit_info(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    info: resonance_common::kit_info::KitInfo,
) {
    r.plugin_mirror.kit_info.insert(instance_id, info);
    crate::update::compose::refresh_kit_pads(r);
}

/// The plugin changed a param itself and reported it (CLAP output
/// parameter events). An ordinary param goes through `update` as
/// [`PluginMessage::ParamEditedByPlugin`], so it takes an undo entry, a
/// revision and the dirty flag like any edit. A read-only output only
/// moves the mirror (there is nothing to undo), and so does any edit a
/// gate would refuse — no project open, a bounce running: the plugin has
/// it either way, and the mirror must not fall behind.
///
/// [`PluginMessage::ParamEditedByPlugin`]: crate::message::PluginMessage::ParamEditedByPlugin
pub(super) fn param_edited(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    edit: resonance_audio::types::PluginParamEdit,
) -> iced::Task<crate::message::Message> {
    let read_only = r
        .with_plugin_mut(instance_id, |slot| {
            slot.params
                .iter()
                .find(|p| p.id == edit.param_id)
                .map(|p| p.read_only)
        })
        .flatten();
    let Some(read_only) = read_only else {
        return iced::Task::none();
    };
    let message = crate::message::Message::Plugin(
        crate::message::PluginMessage::ParamEditedByPlugin {
            instance_id,
            param_id: edit.param_id,
            value: edit.value,
            text: edit.text.clone(),
            gesture: edit.gesture,
        },
    );
    if read_only || r.gates_message(&message) {
        param_values_changed(
            r,
            instance_id,
            vec![resonance_audio::types::ParamValueUpdate {
                id: edit.param_id,
                value: edit.value,
                text: edit.text,
            }],
        );
        return iced::Task::none();
    }
    r.update(message)
}

/// A Resonance plugin reported its loaded preset and modified flag
/// (`com.resonance.preset-session`). The report is the truth from now on.
pub(super) fn preset_identity(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    identity: Option<resonance_common::preset_session::IdentityReport>,
) {
    use resonance_plugin::presets::PresetSource as PluginPresetSource;
    match identity {
        Some(report) => {
            let source = if report.source == "factory" {
                PluginPresetSource::Factory
            } else {
                PluginPresetSource::User
            };
            r.presets.plugin_preset_identity.insert(
                instance_id,
                crate::state::presets::SlotPresetIdentity {
                    source,
                    id: report.id,
                    name: report.name,
                    modified: report.modified,
                    reported: true,
                },
            );
        }
        None => {
            r.presets.plugin_preset_identity.remove(&instance_id);
        }
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

/// The `FxBypassChanged` echo: `owner`'s whole-chain bypass moved.
///
/// A track's echo is dropped when the track's removal is still owed
/// (ARCH-01 A-13i): a late echo of a command sent to the *old*
/// incarnation of this id — FIFO puts it before the removal a diff restore
/// or a live delete already mirrored, so it names a track that either no
/// longer exists or has already been replaced by a fresh one under the
/// same id, whose own restored value this must not clobber.
pub(super) fn fx_bypass_changed(r: &mut Resonance, owner: ChainOwner, bypassed: bool) {
    match owner {
        ChainOwner::Track(track_id) => {
            if r.io.restore_echoes.track_removal_owed(track_id) {
                return;
            }
            if let Some(track) = r.registry.tracks.iter_mut().find(|t| t.id == track_id) {
                track.fx_bypassed = bypassed;
            }
        }
        ChainOwner::Bus(bus_id) => {
            if let Some(bus) = r.registry.busses.iter_mut().find(|b| b.id == bus_id) {
                bus.fx_bypassed = bypassed;
            }
        }
        ChainOwner::Master => r.master.fx_bypassed = bypassed,
    }
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
    // The echo can trail the slot's removal (the editor closed because
    // the plugin went): there is nothing left to mirror, and no window to
    // fall back to.
    if r.plugin_slot(instance_id).is_none() {
        return;
    }
    r.with_plugin_mut(instance_id, |slot| slot.editor_open = open);
    if failure.is_some() {
        crate::update::plugin_window::open_generic(r, instance_id);
    }
}
