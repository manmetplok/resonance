//! The insert-chain edits every owner shares — add, add-with-id, remove,
//! reorder, whole-chain bypass — for a track's, a bus's or the master's
//! chain (code review ARCH2-02).
//!
//! `PluginMessage`, `BusMessage` and `MasterMessage` keep their per-owner
//! variants (they classify for undo, describe themselves and are gated per
//! owner), but their handlers all land here, so the engine command, the
//! owed echo and the immediate mirror are written once. The track's
//! reorder resolves the instrument-floor rule (`crate::plugin_chain`)
//! before it gets here; everything after that is owner-neutral.

use resonance_audio::types::{AudioCommand, ChainOwner, PluginInstanceId, ScannedPlugin};

use crate::Resonance;

/// Add `plugin` to the end of `owner`'s chain under a fresh app-allocated
/// id (ARCH-04 D-1), fire-and-forget: no placeholder is mirrored, so the
/// mixer shows nothing until the engine's `PluginAdded` echo lands.
pub(crate) fn add(r: &mut Resonance, owner: ChainOwner, plugin: ScannedPlugin) {
    let id = r.allocate_plugin_id();
    let _ = r.engine.send(AudioCommand::AddPlugin {
        owner,
        clap_file_path: plugin.clap_file_path,
        clap_plugin_id: plugin.clap_plugin_id,
        id,
    });
}

/// Add `plugin` under an id the caller chose up front, mirroring a
/// placeholder slot NOW with an empty param list, exactly as the
/// project-load replay does — so the caller can address the slot without
/// waiting for the echo (ba doc #273, todo #1234). The engine's
/// `PluginAdded` echo finds the slot by `instance_id` and fills in
/// `params`/`has_gui` instead of pushing a second one.
pub(crate) fn add_with_id(
    r: &mut Resonance,
    owner: ChainOwner,
    instance_id: PluginInstanceId,
    plugin: ScannedPlugin,
) {
    let _ = r.engine.send(AudioCommand::AddPlugin {
        owner,
        clap_file_path: plugin.clap_file_path.clone(),
        clap_plugin_id: plugin.clap_plugin_id.clone(),
        id: instance_id,
    });
    if let Some(chain) = r.chain_mut(owner) {
        chain.push(crate::state::PluginSlotState::new(
            instance_id,
            plugin.name,
            plugin.clap_plugin_id,
            plugin.clap_file_path,
            Vec::new(),
            false,
        ));
        r.insert_plugin_index(instance_id, owner);
    }
}

/// Remove `instance_id` from `owner`'s chain. Mirrored now, not on the
/// `PluginRemoved` echo, so an undo pressed before the echo lands sees it
/// (STATE-10 shape, ARCH-01 FU-A13c). The echo is owed, so a late one
/// cannot drop an instance an undo re-added under this id (A-13h).
pub(crate) fn remove(r: &mut Resonance, owner: ChainOwner, instance_id: PluginInstanceId) {
    let _ = r.engine.send(AudioCommand::RemovePlugin { owner, instance_id });
    r.io.restore_echoes.expect_plugin_removed(instance_id);
    crate::engine_events::plugins::removed(r, owner, instance_id);
}

/// Move `instance_id` to `to_index` on `owner`'s chain, and mirror it now
/// rather than waiting for `PluginMoved`: the app's `Vec` order is what
/// the mixer draws and what project serialization writes, and a control
/// client must be able to read back the order it just set. The echo
/// replays the same move and no-ops when it already matches.
pub(crate) fn move_plugin(
    r: &mut Resonance,
    owner: ChainOwner,
    instance_id: PluginInstanceId,
    to_index: usize,
) {
    let _ = r.engine.send(AudioCommand::MovePlugin {
        owner,
        instance_id,
        to_index,
    });
    crate::engine_events::plugins::mirror_plugin_move(r, owner, instance_id, to_index);
}
