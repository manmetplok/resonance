use iced::Task;
use resonance_audio::types::AudioCommand;

use crate::message::{Message, ProjectIoMessage, UiMessage};
use crate::state::ViewMode;
use crate::update::project_io;
use crate::Resonance;

pub fn handle(r: &mut Resonance, m: UiMessage) -> Task<Message> {
    match m {
        UiMessage::SwitchView(mode) => {
            // Track the view to return to when leaving Performance mode.
            // Entering Performance from elsewhere remembers the source;
            // switching to any other view clears the memory. Switching
            // does not touch transport state — playback continues.
            match (r.view_mode, mode) {
                (ViewMode::Performance, ViewMode::Performance) => {}
                (from, ViewMode::Performance) => r.pre_performance_view = Some(from),
                _ => r.pre_performance_view = None,
            }
            r.view_mode = mode;
        }
        UiMessage::TogglePerformanceMode => {
            toggle_performance_mode(r);
        }
        UiMessage::RequestPerformanceToggle => {
            // The unmodified `F` shortcut arrives via the global keyboard
            // subscription, which fires even while a text field is focused.
            // Probe the live widget tree for keyboard focus and only toggle
            // once we know no text input is being edited (see `crate::focus`).
            return crate::focus::any_text_input_focused()
                .map(|editing| Message::Ui(UiMessage::PerformanceToggleResolved { editing }));
        }
        UiMessage::PerformanceToggleResolved { editing } => {
            // Suppress the toggle when `F` was typed into a focused text
            // field; otherwise apply the manual toggle.
            if !editing {
                toggle_performance_mode(r);
            }
        }
        UiMessage::ExitPerformanceMode => {
            // `Esc` only leaves Performance mode; it is a no-op elsewhere so
            // it never steals Escape from other views.
            if r.view_mode == ViewMode::Performance {
                r.view_mode = r.pre_performance_view.take().unwrap_or(ViewMode::Arrange);
            }
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
        UiMessage::ToggleReferencePanel => {
            r.mixer.reference_panel_open = !r.mixer.reference_panel_open;
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
        UiMessage::ToggleMixerInspectorGroup(group) => {
            let set = &mut r.mixer.collapsed_inspector_groups;
            if !set.remove(&group) {
                set.insert(group);
            }
        }
        UiMessage::ToggleMidiClockSend => {
            r.midi_clock_send_enabled = !r.midi_clock_send_enabled;
            let _ = r.engine.send(AudioCommand::SetMidiClockOutput {
                device: r.midi_clock_send_device.clone(),
                enabled: r.midi_clock_send_enabled,
            });
        }
        UiMessage::SetMidiClockSendDevice(device) => {
            r.midi_clock_send_device = device.clone();
            let _ = r.engine.send(AudioCommand::SetMidiClockOutput {
                device,
                enabled: r.midi_clock_send_enabled,
            });
        }
        UiMessage::ToggleMidiClockRecv => {
            r.midi_clock_recv_enabled = !r.midi_clock_recv_enabled;
            let _ = r.engine.send(AudioCommand::SetMidiClockInput {
                device: r.midi_clock_recv_device.clone(),
                enabled: r.midi_clock_recv_enabled,
            });
        }
        UiMessage::SetMidiClockRecvDevice(device) => {
            r.midi_clock_recv_device = device.clone();
            let _ = r.engine.send(AudioCommand::SetMidiClockInput {
                device,
                enabled: r.midi_clock_recv_enabled,
            });
        }
        UiMessage::SetPerformanceTuning(index) => {
            // Footer instrument/tuning pill. Pure view state — the diagram
            // bands re-voice from `r.performance` on the next render.
            r.performance.set_tuning_index(index);
        }
        UiMessage::SetPerformanceCapo(frets) => {
            // Footer capo stepper. The setter clamps to `0..=MAX_CAPO`.
            r.performance.set_capo(frets);
        }
    }
    Task::none()
}

/// Apply the manual Performance-mode toggle: a pure view switch that never
/// auto-opens on record-arm and never disturbs transport. If already in
/// Performance, return to the remembered view; otherwise enter Performance
/// from the current view (remembering it for the return trip).
fn toggle_performance_mode(r: &mut Resonance) {
    if r.view_mode == ViewMode::Performance {
        r.view_mode = r.pre_performance_view.take().unwrap_or(ViewMode::Arrange);
    } else {
        r.pre_performance_view = Some(r.view_mode);
        r.view_mode = ViewMode::Performance;
    }
}
