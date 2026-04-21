use iced::Task;
use resonance_audio::types::AudioCommand;

use crate::message::{MasterMessage, Message};
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: MasterMessage) -> Task<Message> {
    match m {
        MasterMessage::ToggleMasterFxBypass => {
            r.master_fx_bypassed = !r.master_fx_bypassed;
            r.engine.send(AudioCommand::SetMasterFxBypass {
                bypassed: r.master_fx_bypassed,
            });
        }
        MasterMessage::AddPluginToMaster(plugin) => {
            r.engine.send(AudioCommand::AddPluginToMaster {
                clap_file_path: plugin.clap_file_path,
                clap_plugin_id: plugin.clap_plugin_id,
                id_hint: None,
            });
        }
        MasterMessage::RemovePluginFromMaster(instance_id) => {
            r.engine
                .send(AudioCommand::RemovePluginFromMaster { instance_id });
        }
    }
    Task::none()
}
