use iced::widget::{column, container, row, text, Space};
use iced::{alignment, Element, Length};

use crate::message::Message;
use crate::state::InstrumentType;
use crate::theme;

pub mod chord_lane;
pub mod drumroll;
pub mod expanded_editor;
pub mod instrument_panel;
pub mod popover;
pub mod scale_panel;
pub mod strip;
pub mod tracks;

impl crate::Resonance {
    pub(crate) fn view_compose(&self) -> Element<'_, Message> {
        let strip = strip::view(&self.compose);

        let selected = self.compose.selected_placement().and_then(|p| {
            self.compose
                .find_definition(p.definition_id)
                .map(|d| (p, d))
        });

        let status: Element<'_, Message> = match &self.compose.last_error {
            Some(err) => container(
                text(err)
                    .size(12)
                    .color(iced::Color::from_rgb(1.0, 0.6, 0.5)),
            )
            .padding([4, 10])
            .width(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::PANEL_DARK)),
                ..Default::default()
            })
            .into(),
            None => container(Space::with_height(0)).width(Length::Fill).into(),
        };

        let body: Element<'_, Message> = match selected {
            Some((placement, definition)) => {
                // The chord lane and the track rows share a beat grid. The
                // track area reserves `NAME_COLUMN_WIDTH` on its left for the
                // track name labels, so the chord lane is padded the same
                // amount to keep bar 1 aligned in both.
                let chord_lane = row![
                    Space::with_width(Length::Fixed(tracks::NAME_COLUMN_WIDTH)),
                    chord_lane::view(
                        definition,
                        self.transport.time_sig_num,
                        self.compose.selected_chord_id,
                    ),
                ];

                let editor: Element<'_, Message> = match self.compose.selected_chord_id {
                    Some(chord_id)
                        if definition.chords.iter().any(|c| c.id == chord_id) =>
                    {
                        popover::view(definition, chord_id)
                    }
                    _ => container(Space::with_height(0)).width(Length::Fill).into(),
                };

                let synth_tracks = tracks::view(self, placement, definition);
                let drum_tracks = drumroll::view(self, placement, definition);

                let left_column = match self.compose.expanded_track_id {
                    Some(track_id)
                        if self
                            .registry
                            .tracks
                            .iter()
                            .any(|t| t.id == track_id) =>
                    {
                        let expanded =
                            expanded_editor::view(self, track_id, placement, definition);
                        column![chord_lane, editor, synth_tracks, expanded]
                            .spacing(0)
                            .width(Length::Fill)
                            .height(Length::Fill)
                    }
                    _ => {
                        column![chord_lane, editor, synth_tracks, drum_tracks]
                            .spacing(0)
                            .width(Length::Fill)
                            .height(Length::Fill)
                    }
                };

                // The right-side panel swaps between scale info for the
                // section (default), synth instrument details, and the
                // drumroll pattern controls for drum tracks.
                let right_panel: Element<'_, Message> = match self
                    .compose
                    .details_track_id
                    .and_then(|id| self.registry.tracks.iter().find(|t| t.id == id))
                {
                    Some(track) if track.instrument_type == InstrumentType::Drum => {
                        let clip_id =
                            drumroll::clip_for_track(self, placement, definition, track.id);
                        drumroll::controls::view(&self.compose.drumroll, track, clip_id)
                    }
                    Some(track) => instrument_panel::view(track),
                    None => scale_panel::view(definition, &self.registry.tracks),
                };

                row![left_column, right_panel]
                    .spacing(0)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
            None => empty_state(),
        };

        column![strip, status, body].spacing(0).into()
    }
}

fn empty_state<'a>() -> Element<'a, Message> {
    container(
        column![
            text("No sections yet").size(18).color(theme::TEXT),
            text("Use the + button above to create your first section.")
                .size(12)
                .color(theme::TEXT_DIM),
        ]
        .spacing(8)
        .align_x(alignment::Horizontal::Center),
    )
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into()
}
