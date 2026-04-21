use iced::Task;

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
            return r
                .update(Message::ProjectIo(ProjectIoMessage::SaveProject));
        }
        UiMessage::ConfirmDiscardAndQuit => {
            if let Some(id) = r.confirm_quit.take() {
                return iced::window::close(id);
            }
        }
        UiMessage::CancelQuit => {
            r.confirm_quit = None;
        }
        UiMessage::ToggleGlobalTracks => {
            r.viewport.global_tracks_expanded =
                !r.viewport.global_tracks_expanded;
        }
    }
    Task::none()
}
