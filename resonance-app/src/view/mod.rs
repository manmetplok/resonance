/// View rendering for the Resonance application. The top-level dispatch
/// lives here; concrete surfaces are in sibling modules (transport,
/// mixer, compose, track_header, menus, settings).
pub(crate) mod bounce_dialog;
pub(crate) mod bounce_progress;
pub(crate) mod compose;
pub(crate) mod confirm_delete_track;
pub(crate) mod confirm_quit;
pub(crate) mod controls;
pub(crate) mod knob;
pub(crate) mod menus;
pub(crate) mod mixer;
pub(crate) mod settings;
pub(crate) mod startup;
pub(crate) mod track_header;
pub(crate) mod transport;
pub(crate) mod ui_caches;

use crate::message::*;
use crate::midi_editor::PianoRollCanvas;
use crate::state::*;
use crate::theme;
use crate::timeline::TimelineCanvas;
use iced::widget::{button, canvas, column, container, row, stack, text, Space};
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
        } else if self.mixer.settings_open {
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

        // The piano roll renders the clip at its native pixel rate —
        // total width = keyboard column + every tick of the clip
        // rendered at the current zoom — and lives inside a horizontal
        // scrollable. Two wins: (a) the canvas widget never resizes
        // when the window does, so its geometry cache hits across
        // every paint during a resize, and (b) the notes stay at a
        // stable pixel size instead of being squashed or stretched by
        // window resize. The clip's `duration_ticks` is the natural
        // bound; the extra 4-bar padding leaves room to drag the last
        // note slightly beyond the loop without immediately running
        // out of canvas.
        let extra_ticks: u64 =
            4 * (self.transport.time_sig_num as u64) * (TICKS_PER_QUARTER_NOTE as u64);
        let content_ticks: u64 = clip.duration_ticks.saturating_add(extra_ticks);
        let content_w = crate::midi_editor::KEYBOARD_WIDTH
            + content_ticks as f32 * editor_state.zoom_x;
        let piano_roll = canvas(PianoRollCanvas {
            clip,
            track_id: editor_state.track_id,
            // scroll_x is now driven entirely by the outer Scrollable;
            // the canvas always renders content from origin.
            scroll_x: 0.0,
            scroll_y: editor_state.scroll_y,
            zoom_x: editor_state.zoom_x,
            zoom_y: editor_state.zoom_y,
            snap_ticks: editor_state.snap_ticks,
            selected_note: editor_state.selected_note,
            time_sig_num: self.transport.time_sig_num,
        })
        .width(Length::Fixed(content_w))
        .height(Length::Fill);

        let piano_roll_scroll = iced::widget::Scrollable::with_direction(
            piano_roll,
            iced::widget::scrollable::Direction::Horizontal(
                iced::widget::scrollable::Scrollbar::default(),
            ),
        )
        .width(Length::Fill)
        .height(Length::Fill);

        let editor_panel = column![editor_toolbar, piano_roll_scroll].spacing(0);

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
            self.registry
                .tracks
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
            // Horizontal scrolling is now driven by the outer
            // `Scrollable` — the canvas always renders content from
            // sample-zero. The internal scroll_offset field is kept
            // (so `sample_to_x` and friends keep their signature) but
            // pinned to 0 from the view side.
            scroll_offset: 0.0,
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
            selected_track: self.interaction.selected_track,
            global_tracks_expanded: self.viewport.global_tracks_expanded,
            tempo_map: &self.tempo_map,
            selected_global_event: self.interaction.selected_global_event,
            section_placements: &self.compose.placements,
            section_definitions: &self.compose.definitions,
            selected_placement_id: self.compose.selected_placement_id,
        };

        // Fixed canvas width = full content width. With the canvas no
        // longer set to `Length::Fill`, its `bounds.size()` stays
        // stable across window resizes and `canvas::Cache` keeps
        // hitting instead of re-rasterizing every paint.
        let content_w = timeline_data.content_width_natural();
        let canvas_inner = canvas(timeline_data)
            .width(Length::Fixed(content_w))
            .height(Length::Fill);
        let canvas_el = iced::widget::Scrollable::with_direction(
            canvas_inner,
            iced::widget::scrollable::Direction::Horizontal(
                iced::widget::scrollable::Scrollbar::default(),
            ),
        )
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
