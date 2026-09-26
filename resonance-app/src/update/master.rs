use iced::Task;
use resonance_audio::types::{AudioCommand, PluginInstanceId, ScannedPlugin};

use crate::message::Message;
use crate::Resonance;

/// `Message::Master` variants, handled by [`handle`] in this module.
/// Declared here beside its handler and re-exported from `crate::message`
/// (ARCH-01 A1-3).
#[derive(Debug, Clone)]
pub enum MasterMessage {
    ToggleMasterFxBypass,
    AddPluginToMaster(ScannedPlugin),
    /// Add a plugin to the master whose instance id the *app* chose up
    /// front, mirroring a placeholder slot into `Resonance::master_plugins`
    /// immediately so the caller can address it without waiting for the
    /// engine's `MasterPluginAdded` echo. The master twin of
    /// [`BusMessage::AddPluginToBusWithId`](crate::message::BusMessage::AddPluginToBusWithId); `engine_events::plugins::master_added`
    /// is idempotent, so the echo fills the placeholder's params in
    /// rather than pushing a duplicate. The GUI never sends this.
    AddPluginToMasterWithId {
        instance_id: PluginInstanceId,
        plugin: ScannedPlugin,
    },
    RemovePluginFromMaster(PluginInstanceId),
    /// Reorder the master insert chain: move `instance_id` to
    /// `to_index`, clamped to the last slot. Sends
    /// `AudioCommand::MovePluginInMaster` AND mirrors the new order into
    /// `Resonance::master_plugins`, so a control client reads its own
    /// write back in the same cycle; the engine's `MasterPluginMoved`
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
            r.master_fx_bypassed = !r.master_fx_bypassed;
            let _ = r.engine.send(AudioCommand::SetMasterFxBypass {
                bypassed: r.master_fx_bypassed,
            });
        }
        MasterMessage::AddPluginToMaster(plugin) => {
            // App-allocated (ARCH-04 D-1); still no eager mirror, same as
            // the track/bus GUI adds — the chain waits for
            // `MasterPluginAdded`.
            let id = r.allocate_plugin_id();
            let _ = r.engine.send(AudioCommand::AddPluginToMaster {
                clap_file_path: plugin.clap_file_path,
                clap_plugin_id: plugin.clap_plugin_id,
                id,
            });
        }
        MasterMessage::AddPluginToMasterWithId {
            instance_id,
            plugin,
        } => {
            let _ = r.engine.send(AudioCommand::AddPluginToMaster {
                clap_file_path: plugin.clap_file_path.clone(),
                clap_plugin_id: plugin.clap_plugin_id.clone(),
                id: instance_id,
            });
            // Mirror the slot NOW with an empty param list, as the
            // project-load replay does; `master_added` finds it by
            // `instance_id` on the echo and fills in params/has_gui
            // instead of pushing a second one.
            r.master_plugins.push(crate::state::PluginSlotState::new(
                instance_id,
                plugin.name,
                plugin.clap_plugin_id,
                plugin.clap_file_path,
                Vec::new(),
                false,
            ));
            r.insert_plugin_index(instance_id, crate::state::PluginLocator::Master);
        }
        MasterMessage::RemovePluginFromMaster(instance_id) => {
            let _ = r.engine
                .send(AudioCommand::RemovePluginFromMaster { instance_id });
        }
        MasterMessage::MovePluginInMaster {
            instance_id,
            to_index,
        } => {
            let _ = r.engine.send(AudioCommand::MovePluginInMaster {
                instance_id,
                to_index,
            });
            crate::engine_events::plugins::mirror_master_plugin_move(r, instance_id, to_index);
        }
    }
    Task::none()
}
