//! Putting a different plugin in a slot **without moving the slot** (ba
//! doc #275 P5, todo #1309).
//!
//! This is the recovery half of "a missing plugin is an
//! indistinguishable dead slot". Until now a chain could only gain a
//! plugin at the end and lose one entirely, so the only way to deal with
//! a plugin that would not load was to remove it and add its
//! replacement — which put the new plugin last in the chain (a different
//! sound: order is audible), and threw away everything the dead slot was
//! holding on the way past.
//!
//! Both surfaces come through here — the mixer inspector's per-slot
//! picker and the control API's `track/bus/master.replace_effect` — so
//! neither can grow rules the other does not have. It works on all three
//! chains from one entry point because plugin instance ids are unique
//! across tracks, busses and master (the same property `with_plugin_mut`
//! relies on).
//!
//! # The two things a replace can mean
//!
//! **Relocate** — the caller names the plugin that is *already* in the
//! slot, at whatever path the catalog now has for it. This is the
//! "I moved my plugin folder, then rescanned" case. The slot keeps its
//! instance id, and therefore keeps the settings parked against that id:
//! the opaque CLAP blob in `plugin_mirror.state_cache` and the parameter values
//! in `pending_plugin_param_overrides` (ba todo #1308). When the engine
//! answers `PluginAdded` for that id, `engine_events::plugins` pushes
//! both into the new instance and the plugin comes back exactly as it
//! was saved.
//!
//! **Swap** — a *different* plugin takes the position. The preserved
//! state belongs to the plugin that is leaving and cannot mean anything
//! to the one arriving, so it is dropped, and the replacement gets a
//! fresh instance id rather than inheriting one that automation lanes,
//! key routes and per-slot state are keyed by.
//!
//! Neither is "remove, then add". Removing a slot is the one gesture
//! that discards a missing plugin's preserved settings, and it is left
//! as an explicit user action for exactly that reason.

use resonance_audio::types::{AudioCommand, PluginInstanceId, ScannedPlugin};

use crate::state::{PluginLocator, PluginSlotState};
use crate::Resonance;

/// What a replace turns out to mean for a given slot.
///
/// A pure function of the slot's current state and the plugin asked
/// for — see [`classify`] — so the control layer can word its reply
/// from the same rule the mutation applies, instead of restating it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceKind {
    /// The slot's own plugin, re-instantiated from wherever the catalog
    /// now finds it. Keeps the instance, and with it the settings
    /// preserved against that instance.
    Relocate,
    /// A different plugin takes the slot's position. The outgoing
    /// plugin's preserved state goes with it.
    Swap,
    /// The slot already carries this plugin and it is loaded. Nothing
    /// to do — reported rather than silently ignored so a caller can
    /// tell "already the case" from "did not happen".
    AlreadyLoaded,
}

/// What replacing this slot's plugin with `new_clap_plugin_id` would
/// mean. `None` when no chain carries `instance_id`.
pub(crate) fn classify(
    r: &Resonance,
    instance_id: PluginInstanceId,
    new_clap_plugin_id: &str,
) -> Option<ReplaceKind> {
    let (locator, index) = locate_slot(r, instance_id)?;
    let slot = chain(r, locator)?.get(index)?;
    Some(
        match (
            slot.clap_plugin_id == new_clap_plugin_id,
            slot.availability.is_missing(),
        ) {
            (true, false) => ReplaceKind::AlreadyLoaded,
            (true, true) => ReplaceKind::Relocate,
            (false, _) => ReplaceKind::Swap,
        },
    )
}

/// Replace the plugin in the slot holding `instance_id` with `plugin`,
/// keeping the slot's position in its chain.
///
/// `None` when no chain carries `instance_id`.
pub(crate) fn replace_plugin_slot(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    plugin: ScannedPlugin,
) -> Option<ReplaceKind> {
    let kind = classify(r, instance_id, &plugin.clap_plugin_id)?;
    let (locator, index) = locate_slot(r, instance_id)?;
    match kind {
        ReplaceKind::AlreadyLoaded => {}
        ReplaceKind::Relocate => relocate(r, instance_id, locator, plugin),
        ReplaceKind::Swap => {
            swap(r, instance_id, locator, index, plugin);
        }
    }
    Some(kind)
}

/// Re-instantiate the slot's own plugin from a new file, under the same
/// instance id.
///
/// Nothing is removed and nothing is repositioned here: the engine never
/// had this instance (the slot is missing, which is the only state a
/// relocate is offered in), so there is no instance to drop and no
/// engine-side chain entry to move. The reposition happens on the way
/// back, in `engine_events::plugins`, once the add has actually produced
/// an instance — the engine appends to a chain that has been running
/// without this plugin, so the slot's position has to be re-imposed
/// *after* the add succeeds, not hoped for before it.
fn relocate(
    r: &mut Resonance,
    instance_id: PluginInstanceId,
    locator: PluginLocator,
    plugin: ScannedPlugin,
) {
    let clap_file_path = plugin.clap_file_path.clone();
    r.with_plugin_mut(instance_id, |slot| {
        slot.clap_file_path = plugin.clap_file_path.clone();
        // The catalog's spelling of the name is the live one; the slot
        // has been showing whatever the project file recorded.
        slot.plugin_name = plugin.name.clone();
    });
    let _ = r.engine.send(add_command(
        locator,
        &plugin.clap_plugin_id,
        &clap_file_path,
        instance_id,
    ));
}

/// Put a *different* plugin in the slot's position.
///
/// The app's chain is rewritten immediately — the replacement slot is
/// dropped in at the same index rather than appended — so the mixer
/// shows the new plugin where the old one was in the same frame, and a
/// control client can read back the chain it just edited without waiting
/// for an echo. The engine gets the same shape in three commands:
/// remove the old instance, add the new one (which appends), move it to
/// the position the slot occupies.
///
/// Returns the replacement's instance id.
fn swap(
    r: &mut Resonance,
    old_id: PluginInstanceId,
    locator: PluginLocator,
    index: usize,
    plugin: ScannedPlugin,
) -> PluginInstanceId {
    let new_id = r.allocate_plugin_id();

    // Drop the outgoing instance first. For a missing plugin the engine
    // has nothing to drop and this is only its echo; for a live one it
    // is what stops the plugin processing.
    let _ = r.engine.send(remove_command(locator, old_id));
    // The removal is mirrored below, so its echo is owed, as on a live
    // delete: an undo landing before it re-adds `old_id`, and the late
    // echo must not drop that slot (and its lanes) again (A-13h). The
    // engine echoes every `RemovePlugin*`, a missing instance included,
    // so the debt always settles.
    r.io.restore_echoes.expect_plugin_removed(old_id);

    // The old plugin's preserved settings go with it. Done here rather
    // than left to the `PluginRemoved` echo so that a save taken in the
    // same frame cannot still write a `plugin_*.bin` for a slot that no
    // longer exists (the echo repeats it, idempotently).
    r.plugin_mirror.state_cache.remove(&old_id);
    r.plugin_mirror.kit_info.remove(&old_id);
    r.plugin_mirror.output_ports.remove(&old_id);
    r.presets.pending_plugin_param_overrides.remove(&old_id);
    // Its automation lanes too: they are keyed by the outgoing instance
    // id, which the replacement does not inherit (automation-control-api
    // D1). Undo of the swap brings them back with the old slot.
    crate::engine_events::plugins::drop_plugin_lanes(r, old_id);
    if r.sidechain.clear_plugin(old_id) && !matches!(locator, PluginLocator::Track(_)) {
        // The engine drops a key route itself on `RemovePlugin`, but not
        // on the bus/master removals — same asymmetry
        // `engine_events::plugins::drop_route_onto_removed_chain_plugin`
        // exists for.
        let _ = r
            .engine
            .send(AudioCommand::ClearSidechainRoute { plugin: old_id });
    }
    if r.ui.mixer.selected_plugin == Some(old_id) {
        // The parameter panel was showing the outgoing plugin's
        // parameters. The replacement has its own, and none of them have
        // arrived yet.
        r.ui.mixer.selected_plugin = None;
    }

    let replacement = PluginSlotState::new(
        new_id,
        plugin.name.clone(),
        plugin.clap_plugin_id.clone(),
        plugin.clap_file_path.clone(),
        Vec::new(),
        false,
    );
    if let Some(chain) = chain_mut(r, locator) {
        chain[index] = replacement;
    }
    r.remove_plugin_index(old_id);
    r.insert_plugin_index(new_id, locator);

    let _ = r.engine.send(add_command(
        locator,
        &plugin.clap_plugin_id,
        &plugin.clap_file_path,
        new_id,
    ));
    if let Some(to_index) = live_index(r, locator, index) {
        let _ = r.engine.send(move_command(locator, new_id, to_index));
    }
    new_id
}

/// Which chain holds `instance_id`, and at which index.
fn locate_slot(r: &Resonance, instance_id: PluginInstanceId) -> Option<(PluginLocator, usize)> {
    let position =
        |chain: &[PluginSlotState]| chain.iter().position(|p| p.instance_id == instance_id);
    for track in &r.registry.tracks {
        if let Some(i) = position(&track.plugins) {
            return Some((PluginLocator::Track(track.id), i));
        }
    }
    for bus in &r.registry.busses {
        if let Some(i) = position(&bus.plugins) {
            return Some((PluginLocator::Bus(bus.id), i));
        }
    }
    position(&r.master.plugins).map(|i| (PluginLocator::Master, i))
}

fn chain(r: &Resonance, locator: PluginLocator) -> Option<&[PluginSlotState]> {
    match locator {
        PluginLocator::Track(track_id) => r
            .registry
            .tracks
            .iter()
            .find(|t| t.id == track_id)
            .map(|t| t.plugins.as_slice()),
        PluginLocator::Bus(bus_id) => r
            .registry
            .busses
            .iter()
            .find(|b| b.id == bus_id)
            .map(|b| b.plugins.as_slice()),
        PluginLocator::Master => Some(r.master.plugins.as_slice()),
    }
}

fn chain_mut(r: &mut Resonance, locator: PluginLocator) -> Option<&mut Vec<PluginSlotState>> {
    match locator {
        PluginLocator::Track(track_id) => r
            .registry
            .tracks
            .iter_mut()
            .find(|t| t.id == track_id)
            .map(|t| &mut t.plugins),
        PluginLocator::Bus(bus_id) => r
            .registry
            .busses
            .iter_mut()
            .find(|b| b.id == bus_id)
            .map(|b| &mut b.plugins),
        PluginLocator::Master => Some(&mut r.master.plugins),
    }
}

/// The engine-side index that corresponds to app-chain index `index`.
///
/// Just [`plugin_chain::engine_slot_index`] against this locator's
/// chain. The translation rule lives there rather than here because
/// `engine_events::plugins` needs the same answer when a missing plugin
/// comes back, and two copies of a rule that must agree is how they
/// drift apart.
fn live_index(r: &Resonance, locator: PluginLocator, index: usize) -> Option<usize> {
    Some(crate::plugin_chain::engine_slot_index(
        chain(r, locator)?,
        index,
    ))
}

fn add_command(
    locator: PluginLocator,
    clap_plugin_id: &str,
    clap_file_path: &str,
    id: PluginInstanceId,
) -> AudioCommand {
    let clap_plugin_id = clap_plugin_id.to_owned();
    let clap_file_path = clap_file_path.to_owned();
    match locator {
        PluginLocator::Track(track_id) => AudioCommand::AddPlugin {
            track_id,
            clap_file_path,
            clap_plugin_id,
            id,
        },
        PluginLocator::Bus(bus_id) => AudioCommand::AddPluginToBus {
            bus_id,
            clap_file_path,
            clap_plugin_id,
            id,
        },
        PluginLocator::Master => AudioCommand::AddPluginToMaster {
            clap_file_path,
            clap_plugin_id,
            id,
        },
    }
}

fn remove_command(locator: PluginLocator, instance_id: PluginInstanceId) -> AudioCommand {
    match locator {
        PluginLocator::Track(track_id) => AudioCommand::RemovePlugin {
            track_id,
            instance_id,
        },
        PluginLocator::Bus(bus_id) => AudioCommand::RemovePluginFromBus {
            bus_id,
            instance_id,
        },
        PluginLocator::Master => AudioCommand::RemovePluginFromMaster { instance_id },
    }
}

fn move_command(
    locator: PluginLocator,
    instance_id: PluginInstanceId,
    to_index: usize,
) -> AudioCommand {
    match locator {
        PluginLocator::Track(track_id) => AudioCommand::MovePlugin {
            track_id,
            instance_id,
            to_index,
        },
        PluginLocator::Bus(bus_id) => AudioCommand::MovePluginInBus {
            bus_id,
            instance_id,
            to_index,
        },
        PluginLocator::Master => AudioCommand::MovePluginInMaster {
            instance_id,
            to_index,
        },
    }
}
