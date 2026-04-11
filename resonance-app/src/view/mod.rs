/// View rendering for the Resonance application. The top-level dispatch
/// lives here; concrete surfaces are in sibling modules (transport,
/// mixer, compose, track_header, menus, settings).
pub(crate) mod compose;
pub(crate) mod controls;
pub(crate) mod knob;
pub(crate) mod menus;
pub(crate) mod mixer;
pub(crate) mod settings;
pub(crate) mod track_header;
pub(crate) mod transport;

use crate::message::*;
use crate::midi_editor::PianoRollCanvas;
use crate::state::*;
use crate::theme;
use crate::timeline::TimelineCanvas;
use iced::widget::{
    button, canvas, column, container, row, stack, text, Space,
};
use iced::{alignment, Element, Length};
use resonance_audio::types::*;

impl crate::Resonance {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let transport = transport::view_transport(self);
        let main_area = match self.view_mode {
            ViewMode::Arrange => self.view_main_area(),
            ViewMode::Mixer => self.view_mixer(),
            ViewMode::Compose => self.view_compose(),
        };

        let content: Element<'_, Message> = if let Some(ref err) = self.error_message {
            let error_bar = container(
                row![
                    text(err).size(13).color(iced::Color::WHITE),
                    Space::with_width(Length::Fill),
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

        if self.mixer.settings_open {
            stack![base, settings::view_settings_overlay(self)].into()
        } else if self.mixer.add_track_menu_open {
            stack![base, menus::view_add_track_menu(self)].into()
        } else {
            base
        }
    }

    fn view_main_area(&self) -> Element<'_, Message> {
        let track_headers = track_header::view_track_headers(self);
        let timeline = self.view_timeline();

        let main = row![track_headers, timeline];

        if let Some(editor) = self.view_midi_editor_panel() {
            return column![
                container(main).width(Length::Fill).height(Length::Fill),
                editor,
            ]
            .spacing(0)
            .into();
        }

        container(main)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// Bottom MIDI editor panel shown whenever a clip is open in the piano
    /// roll. Used by both the Arrange and Compose tabs so inline editing
    /// works identically from either view.
    pub(crate) fn view_midi_editor_panel(&self) -> Option<Element<'_, Message>> {
        let editor_state = self.interaction.editing_midi_clip.as_ref()?;
        let clip = self
            .midi_clips
            .iter()
            .find(|c| c.id == editor_state.clip_id)?;

        let close_btn = button(text("Close Editor").size(12).color(theme::TEXT))
            .on_press(Message::MidiEditor(MidiEditorMessage::CloseMidiEditor))
            .style(|_theme, status| theme::transport_button_style(status))
            .padding([4, 8]);
        let editor_label = text(format!("MIDI: {}", clip.name))
            .size(12)
            .color(theme::ACCENT);
        let editor_toolbar = container(
            row![editor_label, Space::with_width(Length::Fill), close_btn]
                .spacing(8)
                .align_y(alignment::Vertical::Center)
                .padding([4, 8]),
        )
        .width(Length::Fill)
        .style(theme::panel_outlined);

        let piano_roll = canvas(PianoRollCanvas {
            clip,
            track_id: editor_state.track_id,
            scroll_x: editor_state.scroll_x,
            scroll_y: editor_state.scroll_y,
            zoom_x: editor_state.zoom_x,
            zoom_y: editor_state.zoom_y,
            snap_ticks: editor_state.snap_ticks,
            selected_note: editor_state.selected_note,
            time_sig_num: self.transport.time_sig_num,
        })
        .width(Length::Fill)
        .height(Length::Fill);

        let editor_panel = column![editor_toolbar, piano_roll].spacing(0);

        Some(
            container(editor_panel)
                .width(Length::Fill)
                .height(250)
                .style(|_theme| container::Style {
                    background: Some(iced::Background::Color(theme::BG)),
                    border: iced::Border {
                        color: theme::SEPARATOR,
                        width: 1.0,
                        radius: 0.0.into(),
                    },
                    ..Default::default()
                })
                .into(),
        )
    }

    fn view_timeline(&self) -> Element<'_, Message> {
        let recording_tracks: Vec<TrackId> = if self.transport.recording {
            self.registry.tracks
                .iter()
                .filter(|t| t.record_armed)
                .map(|t| t.id)
                .collect()
        } else {
            Vec::new()
        };

        let timeline_data = TimelineCanvas {
            tracks: &self.registry.tracks,
            clips: &self.clips,
            playhead: self.transport.playhead,
            sample_rate: self.sample_rate,
            zoom: self.viewport.zoom,
            scroll_offset: self.viewport.scroll_offset,
            recording_tracks,
            recording_start_sample: self.transport.recording_start_sample,
            bpm: self.transport.bpm,
            time_sig_num: self.transport.time_sig_num,
            scroll_offset_y: self.viewport.scroll_offset_y,
            loop_enabled: self.transport.loop_enabled,
            loop_in: self.transport.loop_in,
            loop_out: self.transport.loop_out,
            selected_clip: self.interaction.selected_clip,
            midi_clips: &self.midi_clips,
            selected_midi_clip: self.interaction.selected_midi_clip,
        };

        let canvas_el = canvas(timeline_data)
            .width(Length::Fill)
            .height(Length::Fill);

        // Floating zoom buttons, anchored to the bottom-right corner of the
        // timeline. Using Length::Shrink so the overlay only hit-tests the
        // buttons themselves — clicks elsewhere pass through to the canvas.
        let zoom_out = button(
            theme::icon(theme::fa::MAGNIFYING_GLASS_MINUS)
                .size(12)
                .color(theme::TEXT),
        )
        .on_press(Message::Viewport(ViewportMessage::ZoomOut))
        .padding([6, 8])
        .style(|_theme, status| theme::floating_button_style(status));

        let zoom_in = button(
            theme::icon(theme::fa::MAGNIFYING_GLASS_PLUS)
                .size(12)
                .color(theme::TEXT),
        )
        .on_press(Message::Viewport(ViewportMessage::ZoomIn))
        .padding([6, 8])
        .style(|_theme, status| theme::floating_button_style(status));

        let zoom_group = row![zoom_out, zoom_in].spacing(4);

        let overlay = container(
            column![
                Space::with_height(Length::Fill),
                row![Space::with_width(Length::Fill), zoom_group],
            ]
            .spacing(0),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(iced::Padding {
            top: 0.0,
            right: 20.0,
            bottom: 20.0,
            left: 0.0,
        });

        stack![canvas_el, overlay].into()
    }
}
