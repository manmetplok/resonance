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
        UiMessage::DismissImportProgress => {
            r.import_progress_modal_open = false;
            r.import_progress.clear();
        }
        UiMessage::DismissMissingPlugins => {
            r.missing_plugins.dismiss();
        }
        UiMessage::ShowMissingPlugins => {
            r.missing_plugins.show();
            r.mixer.settings_open = false;
        }
        UiMessage::StartNewProject => {
            if r.refuse_project_switch_during_render() {
                return Task::none();
            }
            return project_io::save_project_as_dialog();
        }
        UiMessage::SelectTrack(id) => {
            // A track and a bus can't both be selected — the inspector
            // describes one channel.
            if id.is_some() {
                r.mixer.selected_bus = None;
            }
            match id {
                // An additive (Cmd/Shift) click on a track toggles it in the
                // multi-selection and leaves any clip selection alone.
                Some(track_id) if r.interaction.select_additive => {
                    r.interaction.toggle_track_selection(track_id);
                }
                // A plain click (or an explicit deselect-all) replaces the
                // selection and drops the clip selection, as before.
                _ => {
                    r.interaction.select_single_track(id);
                    r.interaction.selected_clip = None;
                    r.interaction.selected_midi_clip = None;
                }
            }
        }
        UiMessage::SelectBus(id) => {
            r.mixer.selected_bus = id;
            // Selecting a bus takes the highlight off whatever track had
            // it, so the mixer never shows two selected strips while the
            // inspector describes one of them.
            if id.is_some() {
                r.interaction.select_single_track(None);
                r.interaction.selected_clip = None;
                r.interaction.selected_midi_clip = None;
            }
        }
        UiMessage::OpenTrackMenu { id, x, y } => {
            // Right-click selects the track (so the menu's "Freeze selected
            // tracks" entry targets what was clicked) and opens the context
            // menu anchored at the row (design doc #181, todo #581).
            r.interaction.select_single_track(Some(id));
            r.interaction.track_menu = Some(crate::state::TrackMenuState {
                track_id: id,
                x,
                y,
            });
        }
        UiMessage::CloseTrackMenu => {
            r.interaction.track_menu = None;
        }
        UiMessage::ModifiersChanged(mods) => {
            // Cmd (macOS) / Ctrl (other platforms) and Shift both extend the
            // track selection. Mirroring the live state here lets the
            // modifier-less mouse press decide single vs additive.
            r.interaction.select_additive = mods.command() || mods.shift();
        }
        UiMessage::ConfirmSaveAndQuit => {
            let window_id = r.confirm_quit.take();
            r.quit_after_save = window_id;
            return r.update(Message::ProjectIo(ProjectIoMessage::SaveProject));
        }
        UiMessage::ConfirmDiscardAndQuit => {
            if let Some(id) = r.confirm_quit.take() {
                crate::update::project_io::recovery::close_session(r);
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
        UiMessage::ToggleTakeLane(track_id) => {
            // Everything defaults to *folded* here (unlike the collapsible
            // panels, which store the collapsed set): a take lane only
            // exists after cycle recording, and unfolding every stack the
            // moment a pass lands would shove the arrangement down.
            let set = &mut r.interaction.take_lane_expanded_tracks;
            if !set.remove(&track_id) {
                set.insert(track_id);
            }
        }
        UiMessage::ToggleAutosave => {
            let enabled = &mut r.settings.autosave.enabled;
            *enabled = !*enabled;
            crate::settings::persist(&r.settings);
        }
        UiMessage::SetAutosaveInterval(secs) => {
            // The trigger reads the setting on every tick, so the new
            // spacing applies from the next one.
            r.settings.autosave.interval_secs = secs.max(1);
            crate::settings::persist(&r.settings);
        }
        UiMessage::ToggleFollowPlayhead => {
            let follow = &mut r.settings.arrange.follow_playhead;
            *follow = !*follow;
            // Switching it (back) on means "follow now", not "follow
            // once the current manual-scroll pause ends".
            r.viewport.follow_paused = false;
            crate::settings::persist(&r.settings);
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
        UiMessage::ToggleMarkersOverview => {
            r.mixer.markers_overview_open = !r.mixer.markers_overview_open;
        }
        UiMessage::CloseMarkersOverview => {
            r.mixer.markers_overview_open = false;
        }
        UiMessage::RequestMarkerNav { forward } => {
            // The bare `.`/`,` shortcut arrives via the global keyboard
            // subscription, which fires even while a text field is focused.
            // Probe for keyboard focus and only navigate once we know no
            // text input is being edited (see `crate::focus`), mirroring the
            // `F` performance-toggle gate.
            return crate::focus::any_text_input_focused()
                .map(move |editing| Message::Ui(UiMessage::MarkerNavResolved { forward, editing }));
        }
        UiMessage::RequestShortcut(message) => {
            // Same gate as `F` and `.`/`,`, for the shortcuts that are also
            // typing keys (Enter, `B`, Cmd-Z/Y): probe focus first, act
            // only when no text field is being edited (UPD-11).
            return crate::focus::any_text_input_focused().map(move |editing| {
                Message::Ui(UiMessage::ShortcutResolved {
                    message: message.clone(),
                    editing,
                })
            });
        }
        UiMessage::ShortcutResolved { message, editing } => {
            // Re-enter `update()` so the wrapped message meets every gate
            // (startup modal, render in flight) exactly as if the key had
            // dispatched it directly.
            if !editing {
                return r.update(*message);
            }
        }
        UiMessage::MarkerNavResolved { forward, editing } => {
            // Suppress navigation when the key was typed into a focused text
            // field; otherwise jump to the adjacent marker.
            if !editing {
                let nav = if forward {
                    crate::message::MarkerMessage::JumpToNext
                } else {
                    crate::message::MarkerMessage::JumpToPrev
                };
                return r.update(Message::Marker(nav));
            }
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
