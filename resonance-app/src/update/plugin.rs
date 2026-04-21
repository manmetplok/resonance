use iced::Task;
use resonance_audio::types::AudioCommand;

use crate::message::{Message, PluginMessage};
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: PluginMessage) -> Task<Message> {
    match m {
        PluginMessage::AddPluginToTrack(track_id, plugin) => {
            r.engine.send(AudioCommand::AddPlugin {
                track_id,
                clap_file_path: plugin.clap_file_path,
                clap_plugin_id: plugin.clap_plugin_id,
                id_hint: None,
            });
        }
        PluginMessage::RemovePluginFromTrack(track_id, instance_id) => {
            r.engine.send(AudioCommand::RemovePlugin {
                track_id,
                instance_id,
            });
        }
        PluginMessage::TogglePluginPanel(instance_id) => {
            if r.mixer.selected_plugin == Some(instance_id) {
                r.mixer.selected_plugin = None;
            } else {
                r.mixer.selected_plugin = Some(instance_id);
            }
        }
        PluginMessage::SetPluginParam(instance_id, param_id, value) => {
            r.engine.send(AudioCommand::SetPluginParam {
                instance_id,
                param_id,
                value,
            });
            r.with_plugin_mut(instance_id, |p| {
                if let Some(param) = p.params.iter_mut().find(|pp| pp.id == param_id) {
                    param.current_value = value;
                }
            });
        }
        PluginMessage::OpenPluginEditor(instance_id) => {
            r.engine
                .send(AudioCommand::OpenPluginEditor { instance_id });
            r.with_plugin_mut(instance_id, |p| p.editor_open = true);
        }
        PluginMessage::ClosePluginEditor(instance_id) => {
            r.engine
                .send(AudioCommand::ClosePluginEditor { instance_id });
            r.with_plugin_mut(instance_id, |p| p.editor_open = false);
            r.engine.send(AudioCommand::SavePluginState { instance_id });
        }
    }
    Task::none()
}
