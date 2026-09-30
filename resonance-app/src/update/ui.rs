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
            match (r.ui.view_mode, mode) {
                (ViewMode::Performance, ViewMode::Performance) => {}
                (from, ViewMode::Performance) => r.ui.pre_performance_view = Some(from),
                _ => r.ui.pre_performance_view = None,
            }
            r.ui.view_mode = mode;
        }
        UiMessage::TogglePerformanceMode => {
            toggle_performance_mode(r);
        }
        UiMessage::ExitPerformanceMode => {
            // Leaves Performance mode only; a no-op elsewhere.
            if r.ui.view_mode == ViewMode::Performance {
                r.ui.view_mode = r.ui.pre_performance_view.take().unwrap_or(ViewMode::Arrange);
            }
        }
        UiMessage::OpenSettings => {
            r.ui.mixer.settings_open = true;
        }
        UiMessage::CloseSettings => {
            r.ui.mixer.settings_open = false;
            r.ui.keymap_editor.capturing = None;
            r.ui.keymap_editor.conflict = None;
        }
        UiMessage::OpenAddTrackMenu => {
            r.ui.mixer.add_track_menu_open = true;
        }
        UiMessage::CloseAddTrackMenu => {
            r.ui.mixer.add_track_menu_open = false;
        }
        UiMessage::ToggleReferencePanel => {
            r.ui.mixer.reference_panel_open = !r.ui.mixer.reference_panel_open;
        }
        UiMessage::DismissError => {
            r.banners.error_message = None;
        }
        UiMessage::DismissImportProgress => {
            r.media.import_progress_modal_open = false;
            r.media.import_progress.clear();
        }
        UiMessage::DismissMissingPlugins => {
            r.missing_plugins.dismiss();
        }
        UiMessage::ShowMissingPlugins => {
            r.missing_plugins.show();
            r.ui.mixer.settings_open = false;
        }
        UiMessage::StartNewProject => {
            if r.refuse_project_switch_during_render() {
                return Task::none();
            }
            return project_io::save_project_as_dialog();
        }
        UiMessage::NewEmptyProject => {
            if r.session.dirty {
                r.banners.error_message =
                    Some("Save the project first: it has unsaved changes".into());
                return Task::none();
            }
            if r.io.loading || r.io.saving || r.io.save_state.is_some() {
                return Task::none();
            }
            if r.refuse_project_switch_during_render() {
                return Task::none();
            }
            crate::update::project_io::instantiate_builtin(
                r,
                crate::update::project_io::BuiltinTemplateId::Empty,
            );
        }
        UiMessage::SelectTrack(id) => {
            // A track and a bus can't both be selected — the inspector
            // describes one channel.
            if id.is_some() {
                r.ui.mixer.selected_bus = None;
            }
            match id {
                // An additive (Cmd/Shift) click on a track toggles it in the
                // multi-selection and leaves any clip selection alone.
                Some(track_id) if r.ui.interaction.select_additive => {
                    r.ui.interaction.toggle_track_selection(track_id);
                }
                // A plain click (or an explicit deselect-all) replaces the
                // selection and drops the clip selection, as before.
                _ => {
                    r.ui.interaction.select_single_track(id);
                    r.ui.interaction.selected_clip = None;
                    r.ui.interaction.selected_midi_clip = None;
                }
            }
        }
        UiMessage::SelectBus(id) => {
            r.ui.mixer.selected_bus = id;
            // Selecting a bus takes the highlight off whatever track had
            // it, so the mixer never shows two selected strips while the
            // inspector describes one of them.
            if id.is_some() {
                r.ui.interaction.select_single_track(None);
                r.ui.interaction.selected_clip = None;
                r.ui.interaction.selected_midi_clip = None;
            }
        }
        UiMessage::OpenTrackMenu { id, x, y } => {
            // Right-click selects the track (so the menu's "Freeze selected
            // tracks" entry targets what was clicked) and opens the context
            // menu anchored at the row (design doc #181, todo #581).
            r.ui.interaction.select_single_track(Some(id));
            r.ui.interaction.track_menu = Some(crate::state::TrackMenuState {
                track_id: id,
                x,
                y,
            });
        }
        UiMessage::CloseTrackMenu => {
            r.ui.interaction.track_menu = None;
        }
        UiMessage::ModifiersChanged(mods) => {
            // Cmd (macOS) / Ctrl (other platforms) and Shift both extend the
            // track selection. Mirroring the live state here lets the
            // modifier-less mouse press decide single vs additive.
            r.ui.interaction.select_additive = mods.command() || mods.shift();
        }
        UiMessage::ConfirmSaveAndQuit => {
            let window_id = r.modals.confirm_quit.take();
            r.modals.quit_after_save = window_id;
            return r.update(Message::ProjectIo(ProjectIoMessage::SaveProject));
        }
        UiMessage::ConfirmDiscardAndQuit => {
            if let Some(id) = r.modals.confirm_quit.take() {
                crate::update::project_io::recovery::close_session(r);
                r.engine.shutdown(std::time::Duration::from_millis(150));
                return iced::window::close(id);
            }
        }
        UiMessage::CancelQuit => {
            r.modals.confirm_quit = None;
        }
        UiMessage::ToggleGlobalTracks => {
            r.viewport.global_tracks_expanded = !r.viewport.global_tracks_expanded;
        }
        UiMessage::ToggleMixerInspectorGroup(group) => {
            let set = &mut r.ui.mixer.collapsed_inspector_groups;
            if !set.remove(&group) {
                set.insert(group);
            }
        }
        UiMessage::ToggleTakeLane(track_id) => {
            // Everything defaults to *folded* here (unlike the collapsible
            // panels, which store the collapsed set): a take lane only
            // exists after cycle recording, and unfolding every stack the
            // moment a pass lands would shove the arrangement down.
            let set = &mut r.ui.interaction.take_lane_expanded_tracks;
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
            r.devices.midi.midi_clock_send_enabled = !r.devices.midi.midi_clock_send_enabled;
            let _ = r.engine.send(AudioCommand::SetMidiClockOutput {
                device: r.devices.midi.midi_clock_send_device.clone(),
                enabled: r.devices.midi.midi_clock_send_enabled,
            });
        }
        UiMessage::SetMidiClockSendDevice(device) => {
            r.devices.midi.midi_clock_send_device = device.clone();
            let _ = r.engine.send(AudioCommand::SetMidiClockOutput {
                device,
                enabled: r.devices.midi.midi_clock_send_enabled,
            });
        }
        UiMessage::ToggleMidiClockRecv => {
            r.devices.midi.midi_clock_recv_enabled = !r.devices.midi.midi_clock_recv_enabled;
            let _ = r.engine.send(AudioCommand::SetMidiClockInput {
                device: r.devices.midi.midi_clock_recv_device.clone(),
                enabled: r.devices.midi.midi_clock_recv_enabled,
            });
        }
        UiMessage::SetMidiClockRecvDevice(device) => {
            r.devices.midi.midi_clock_recv_device = device.clone();
            let _ = r.engine.send(AudioCommand::SetMidiClockInput {
                device,
                enabled: r.devices.midi.midi_clock_recv_enabled,
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
            r.ui.mixer.markers_overview_open = !r.ui.mixer.markers_overview_open;
        }
        UiMessage::CloseMarkersOverview => {
            r.ui.mixer.markers_overview_open = false;
        }
        UiMessage::RequestShortcut(message) => {
            // The typing gate for the held-`B` audition, which is not a
            // registry command: probe focus first, act only when no text
            // field is being edited (UPD-11).
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
        UiMessage::ShortcutKey {
            chord,
            repeat,
            captured,
        } => {
            return crate::update::shortcuts::handle_key(r, chord, repeat, captured);
        }
        UiMessage::ShortcutProbed { command, editing } => {
            return crate::update::shortcuts::probed(r, command, editing);
        }
        UiMessage::OpenPalette(mode) => {
            return crate::update::palette::open(r, mode);
        }
        UiMessage::ClosePalette => {
            return crate::update::palette::close(r);
        }
        UiMessage::Palette(msg) => {
            return crate::update::palette::handle(r, msg);
        }
        UiMessage::Keymap(msg) => {
            return crate::update::keymap::handle(r, msg);
        }
    }
    Task::none()
}

/// Apply the manual Performance-mode toggle: a pure view switch that never
/// auto-opens on record-arm and never disturbs transport. If already in
/// Performance, return to the remembered view; otherwise enter Performance
/// from the current view (remembering it for the return trip).
fn toggle_performance_mode(r: &mut Resonance) {
    if r.ui.view_mode == ViewMode::Performance {
        r.ui.view_mode = r.ui.pre_performance_view.take().unwrap_or(ViewMode::Arrange);
    } else {
        r.ui.pre_performance_view = Some(r.ui.view_mode);
        r.ui.view_mode = ViewMode::Performance;
    }
}
