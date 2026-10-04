use iced::Task;
use resonance_audio::types::{AudioCommand, ChainOwner, PluginInstanceId, ScannedPlugin};

use crate::message::Message;
use crate::update::chain_edit;
use crate::Resonance;

/// `Message::Master` variants, handled by [`handle`] in this module.
/// Declared here beside its handler and re-exported from `crate::message`
/// (ARCH-01 A1-3).
#[derive(Debug, Clone)]
pub enum MasterMessage {
    ToggleMasterFxBypass,
    AddPluginToMaster(ScannedPlugin),
    /// Add a plugin to the master whose instance id the *app* chose up
    /// front, mirroring a placeholder slot into `Resonance::master.plugins`
    /// immediately so the caller can address it without waiting for the
    /// engine's `PluginAdded` echo. The master twin of
    /// [`BusMessage::AddPluginToBusWithId`](crate::message::BusMessage::AddPluginToBusWithId); `engine_events::plugins::added`
    /// is idempotent, so the echo fills the placeholder's params in
    /// rather than pushing a duplicate. The GUI never sends this.
    AddPluginToMasterWithId {
        instance_id: PluginInstanceId,
        plugin: ScannedPlugin,
    },
    RemovePluginFromMaster(PluginInstanceId),
    /// Reorder the master insert chain: move `instance_id` to
    /// `to_index`, clamped to the last slot. Sends
    /// `AudioCommand::MovePlugin` AND mirrors the new order into
    /// `Resonance::master.plugins`, so a control client reads its own
    /// write back in the same cycle; the engine's `PluginMoved`
    /// echo replays the same move and is then a no-op.
    MovePluginInMaster {
        instance_id: PluginInstanceId,
        to_index: usize,
    },
}

impl MasterMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            // Every master-bus edit (FX bypass, insert chain) is a discrete,
            // persisted edit.
            Self::ToggleMasterFxBypass
            | Self::AddPluginToMaster(..)
            | Self::AddPluginToMasterWithId { .. }
            | Self::RemovePluginFromMaster(..)
            | Self::MovePluginInMaster { .. } => UndoAction::Record,
        }
    }
}

pub fn handle(r: &mut Resonance, m: MasterMessage) -> Task<Message> {
    match m {
        MasterMessage::ToggleMasterFxBypass => {
            r.master.fx_bypassed = !r.master.fx_bypassed;
            let _ = r.engine.send(AudioCommand::SetFxBypass {
                owner: ChainOwner::Master,
                bypassed: r.master.fx_bypassed,
            });
        }
        // The chain edits are owner-neutral (`update::chain_edit`,
        // ARCH2-02); these arms only name the owner.
        MasterMessage::AddPluginToMaster(plugin) => chain_edit::add(r, ChainOwner::Master, plugin),
        MasterMessage::AddPluginToMasterWithId {
            instance_id,
            plugin,
        } => chain_edit::add_with_id(r, ChainOwner::Master, instance_id, plugin),
        MasterMessage::RemovePluginFromMaster(instance_id) => {
            chain_edit::remove(r, ChainOwner::Master, instance_id);
        }
        MasterMessage::MovePluginInMaster {
            instance_id,
            to_index,
        } => chain_edit::move_plugin(r, ChainOwner::Master, instance_id, to_index),
    }
    Task::none()
}
