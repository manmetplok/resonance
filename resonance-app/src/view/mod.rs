/// View rendering for the Resonance application. The top-level dispatch
/// lives here; concrete surfaces are in sibling modules (transport,
/// mixer, compose, track_header, menus, settings, editor_panel,
/// timeline_panel, timeline, piano_roll, midi_editor).
pub(crate) mod bounce_dialog;
pub(crate) mod clip_inspector;
pub(crate) mod export_dialog;
pub(crate) mod bounce_progress;
pub(crate) mod compose;
pub(crate) mod confirm_delete_track;
pub(crate) mod confirm_quit;
pub(crate) mod controls;
pub(crate) mod editor_panel;
pub(crate) mod import_dialog;
pub(crate) mod knob;
pub(crate) mod markers_overview;
pub(crate) mod menus;
pub mod midi_editor;
pub(crate) mod midi_quantize;
pub(crate) mod mixer;
pub(crate) mod relink_dialog;
pub mod performance;
pub mod piano_roll;
pub(crate) mod settings;
pub(crate) mod startup;
pub mod timeline;
pub(crate) mod timeline_panel;
pub(crate) mod track_header;
pub(crate) mod transport;
pub(crate) mod transport_labels;
pub(crate) mod ui_caches;

use crate::message::*;
use crate::state::*;
use crate::theme;
use iced::widget::{button, column, container, row, stack, text, Space};
use iced::{alignment, Element, Length};

impl crate::Resonance {
    pub fn view(&self) -> Element<'_, Message> {
        // Performance mode is a full-bleed, distraction-free surface: it
        // owns its own status bar / footer (built in follow-up todos) and
        // intentionally hides the normal transport chrome below.
        if matches!(self.view_mode, ViewMode::Performance) {
            return self.view_performance_shell();
        }

        let transport = transport::view_transport(self);
        let main_area = match self.view_mode {
            ViewMode::Arrange => self.view_main_area(),
            ViewMode::Mixer => self.view_mixer(),
            ViewMode::Compose => self.view_compose(),
            // Unreachable: Performance returns early above via
            // `view_performance_shell`; kept so the match stays exhaustive.
            ViewMode::Performance => self.view_performance_shell(),
        };

        let content: Element<'_, Message> = if let Some(ref err) = self.error_message {
            let error_bar = container(
                row![
                    text(err).size(13).color(iced::Color::WHITE),
                    Space::new().width(Length::Fill),
                    button(text("\u{00d7}").size(14).color(iced::Color::WHITE))
                        .on_press(Message::Ui(UiMessage::DismissError))
                        .style(|_theme, _status| iced::widget::button::Style {
                            background: Some(iced::Background::Color(iced::Color::TRANSPARENT)),
                            text_color: iced::Color::WHITE,
                            ..Default::default()
                        })
                ]
                .spacing(8)
                .align_y(alignment::Vertical::Center)
                .padding(8),
            )
            .width(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::RECORD_RED)),
                ..Default::default()
            });
            column![transport, error_bar, main_area].spacing(0).into()
        } else {
            column![transport, main_area].spacing(0).into()
        };

        let base: Element<'_, Message> = container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::base_bg)
            .into();

        if !self.io.has_active_project {
            stack![base, startup::view_startup_overlay(self)].into()
        } else if self.bounce_in_progress.is_some() {
            // The bounce progress modal sits above any other overlay
            // because it gates user input until the engine finishes the
            // current bounce — letting the quit-confirm or delete-track
            // dialog appear over it would invite the user into a state
            // change the engine isn't ready for.
            stack![
                base,
                bounce_progress::view_bounce_progress_overlay(self)
            ]
            .into()
        } else if self.confirm_quit.is_some() {
            stack![base, confirm_quit::view_confirm_quit_overlay(self)].into()
        } else if let Some(track_id) = self.confirm_delete_track {
            stack![
                base,
                confirm_delete_track::view_confirm_delete_track_overlay(self, track_id)
            ]
            .into()
        } else if self.bounce_dialog.is_some() {
            stack![base, bounce_dialog::view_bounce_dialog_overlay(self)].into()
        } else if self.export_dialog.is_some() {
            stack![base, export_dialog::view_export_dialog_overlay(self)].into()
        } else if self.import_dialog.is_some() {
            stack![base, import_dialog::view_import_dialog_overlay(self)].into()
        } else if self.relink.modal_open && !self.relink.modal_targets.is_empty() {
            // Missing-files relink modal (doc #175, todo #607): surfaced on
            // load when the project references audio that's gone, and
            // re-openable from the Pool tab's inline `relink` chip.
            stack![base, relink_dialog::view_relink_dialog_overlay(self)].into()
        } else if self.mixer.settings_open {
            stack![base, settings::view_settings_overlay(self)].into()
        } else if self.mixer.add_track_menu_open {
            stack![base, menus::view_add_track_menu(self)].into()
        } else if self.mixer.markers_overview_open {
            stack![base, markers_overview::view_markers_overview_overlay(self)].into()
        } else if self.compose.drumroll.manager_open
            && matches!(self.view_mode, ViewMode::Compose)
        {
            stack![base, compose::drum_groups_manager::view(self)].into()
        } else if self.interaction.marker_menu.is_some()
            || self.interaction.marker_rename.is_some()
        {
            // Arrangement-marker context menu / inline rename float above the
            // arrange timeline (todo #369). Only reachable from the ruler, so
            // guarding on the state alone is enough.
            stack![base, menus::view_marker_overlay(self)].into()
        } else {
            base
        }
    }

    fn view_main_area(&self) -> Element<'_, Message> {
        let track_headers = track_header::view_track_headers(self);
        let timeline = self.view_timeline();

        let main = row![track_headers, timeline];

        let base: Element<'_, Message> = if let Some(editor) = self.view_midi_editor_panel() {
            column![
                container(main).width(Length::Fill).height(Length::Fill),
                editor,
            ]
            .spacing(0)
            .into()
        } else {
            container(main)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };

        // The clip fade/gain inspector floats over the top-right of the
        // arrange area for the selected editable audio clip (epic #18).
        if let Some(flyout) = self.view_clip_inspector_flyout() {
            let overlay = container(flyout)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(alignment::Horizontal::Right)
                .align_y(alignment::Vertical::Top)
                .padding(10);
            stack![base, overlay].into()
        } else {
            base
        }
    }
}
