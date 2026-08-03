use iced::Task;
use resonance_audio::types::AudioCommand;

use crate::message::{MasterMessage, Message};
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: MasterMessage) -> Task<Message> {
    match m {
        MasterMessage::ToggleMasterFxBypass => {
            r.master_fx_bypassed = !r.master_fx_bypassed;
            let _ = r.engine.send(AudioCommand::SetMasterFxBypass {
                bypassed: r.master_fx_bypassed,
            });
        }
        MasterMessage::AddPluginToMaster(plugin) => {
            let _ = r.engine.send(AudioCommand::AddPluginToMaster {
                clap_file_path: plugin.clap_file_path,
                clap_plugin_id: plugin.clap_plugin_id,
                id_hint: None,
            });
        }
        MasterMessage::AddPluginToMasterWithId {
            instance_id,
            plugin,
        } => {
            let _ = r.engine.send(AudioCommand::AddPluginToMaster {
                clap_file_path: plugin.clap_file_path.clone(),
                clap_plugin_id: plugin.clap_plugin_id.clone(),
                id_hint: Some(instance_id),
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
