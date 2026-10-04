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

use crate::state::{ChainOwner, PluginSlotState};
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
    let (owner, index) = locate_slot(r, instance_id)?;
    let slot = r.chain(owner)?.get(index)?;
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
    let (owner, index) = locate_slot(r, instance_id)?;
    match kind {
        ReplaceKind::AlreadyLoaded => {}
        ReplaceKind::Relocate => relocate(r, instance_id, owner, plugin),
        ReplaceKind::Swap => {
            swap(r, instance_id, owner, index, plugin);
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
    owner: ChainOwner,
    plugin: ScannedPlugin,
) {
    let clap_file_path = plugin.clap_file_path.clone();
    r.with_plugin_mut(instance_id, |slot| {
        slot.clap_file_path = plugin.clap_file_path.clone();
        // The catalog's spelling of the name is the live one; the slot
        // has been showing whatever the project file recorded.
        slot.plugin_name = plugin.name.clone();
    });
    let _ = r.engine.send(AudioCommand::AddPlugin {
        owner,
        clap_file_path,
        clap_plugin_id: plugin.clap_plugin_id,
        id: instance_id,
    });
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
    owner: ChainOwner,
    index: usize,
    plugin: ScannedPlugin,
) -> PluginInstanceId {
    let new_id = r.allocate_plugin_id();

    // Drop the outgoing instance first. For a missing plugin the engine
    // has nothing to drop and this is only its echo; for a live one it
    // is what stops the plugin processing.
    let _ = r.engine.send(AudioCommand::RemovePlugin {
        owner,
        instance_id: old_id,
    });
    // The removal is mirrored below, so its echo is owed, as on a live
    // delete: an undo landing before it re-adds `old_id`, and the late
    // echo must not drop that slot (and its lanes) again (A-13h). The
    // engine echoes every `RemovePlugin`, a missing instance included,
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
    // Its key route: the engine drops it itself on `RemovePlugin`, for
    // every owner (ARCH2-02), so only the mirror is pruned here.
    r.sidechain.clear_plugin(old_id);
    // The replacement takes the outgoing slot's focus (it sits in the
    // same place); the window closes, as it shows the old instance.
    let was_focused = r.ui.mixer.focused_slot == Some(old_id);
    r.ui.mixer.forget_plugin(old_id);
    if was_focused {
        r.ui.mixer.focused_slot = Some(new_id);
    }

    let replacement = PluginSlotState::new(
        new_id,
        plugin.name.clone(),
        plugin.clap_plugin_id.clone(),
        plugin.clap_file_path.clone(),
        Vec::new(),
        false,
    );
    if let Some(chain) = r.chain_mut(owner) {
        chain[index] = replacement;
    }
    r.remove_plugin_index(old_id);
    r.insert_plugin_index(new_id, owner);

    let _ = r.engine.send(AudioCommand::AddPlugin {
        owner,
        clap_file_path: plugin.clap_file_path,
        clap_plugin_id: plugin.clap_plugin_id,
        id: new_id,
    });
    if let Some(to_index) = live_index(r, owner, index) {
        let _ = r.engine.send(AudioCommand::MovePlugin {
            owner,
            instance_id: new_id,
            to_index,
        });
    }
    new_id
}

/// Which chain holds `instance_id`, and at which index.
pub(crate) fn locate_slot(
    r: &Resonance,
    instance_id: PluginInstanceId,
) -> Option<(ChainOwner, usize)> {
    let position =
        |chain: &[PluginSlotState]| chain.iter().position(|p| p.instance_id == instance_id);
    for track in &r.registry.tracks {
        if let Some(i) = position(&track.plugins) {
            return Some((ChainOwner::Track(track.id), i));
        }
    }
    for bus in &r.registry.busses {
        if let Some(i) = position(&bus.plugins) {
            return Some((ChainOwner::Bus(bus.id), i));
        }
    }
    position(&r.master.plugins).map(|i| (ChainOwner::Master, i))
}

/// The engine-side index that corresponds to app-chain index `index`.
///
/// Just [`plugin_chain::engine_slot_index`] against this owner's
/// chain. The translation rule lives there rather than here because
/// `engine_events::plugins` needs the same answer when a missing plugin
/// comes back, and two copies of a rule that must agree is how they
/// drift apart.
fn live_index(r: &Resonance, owner: ChainOwner, index: usize) -> Option<usize> {
    Some(crate::plugin_chain::engine_slot_index(r.chain(owner)?, index))
}
