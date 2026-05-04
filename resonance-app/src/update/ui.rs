use iced::Task;
use resonance_audio::types::AudioCommand;

use crate::message::{Message, ProjectIoMessage, UiMessage};
use crate::update::project_io;
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: UiMessage) -> Task<Message> {
    match m {
        UiMessage::SwitchView(mode) => {
            r.view_mode = mode;
        }
        UiMessage::OpenSettings => {
            r.mixer.settings_open = true;
        }
        UiMessage::CloseSettings => {
            r.mixer.settings_open = false;
        }
        UiMessage::OpenAddTrackMenu => {
            r.mixer.add_track_menu_open = true;
        }
        UiMessage::CloseAddTrackMenu => {
            r.mixer.add_track_menu_open = false;
        }
        UiMessage::DismissError => {
            r.error_message = None;
        }
        UiMessage::StartNewProject => {
            return project_io::save_project_as_dialog();
        }
        UiMessage::SelectTrack(id) => {
            r.interaction.selected_track = id;
            r.interaction.selected_clip = None;
            r.interaction.selected_midi_clip = None;
        }
        UiMessage::ConfirmSaveAndQuit => {
            let window_id = r.confirm_quit.take();
            r.quit_after_save = window_id;
            return r.update(Message::ProjectIo(ProjectIoMessage::SaveProject));
        }
        UiMessage::ConfirmDiscardAndQuit => {
            if let Some(id) = r.confirm_quit.take() {
                r.engine.shutdown(std::time::Duration::from_millis(150));
                return iced::window::close(id);
            }
        }
        UiMessage::CancelQuit => {
            r.confirm_quit = None;
        }
        UiMessage::ToggleGlobalTracks => {
            r.viewport.global_tracks_expanded = !r.viewport.global_tracks_expanded;
        }
        UiMessage::ToggleMidiClockSend => {
            r.midi_clock_send_enabled = !r.midi_clock_send_enabled;
            r.engine.send(AudioCommand::SetMidiClockOutput {
                device: r.midi_clock_send_device.clone(),
                enabled: r.midi_clock_send_enabled,
            });
        }
        UiMessage::SetMidiClockSendDevice(device) => {
            r.midi_clock_send_device = device.clone();
            r.engine.send(AudioCommand::SetMidiClockOutput {
                device,
                enabled: r.midi_clock_send_enabled,
            });
        }
        UiMessage::ToggleMidiClockRecv => {
            r.midi_clock_recv_enabled = !r.midi_clock_recv_enabled;
            r.engine.send(AudioCommand::SetMidiClockInput {
                device: r.midi_clock_recv_device.clone(),
                enabled: r.midi_clock_recv_enabled,
            });
        }
        UiMessage::SetMidiClockRecvDevice(device) => {
            r.midi_clock_recv_device = device.clone();
            r.engine.send(AudioCommand::SetMidiClockInput {
                device,
                enabled: r.midi_clock_recv_enabled,
            });
        }
    }
    Task::none()
}
