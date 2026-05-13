use iced::widget::{column, container, row, scrollable, text, Space};
use iced::{alignment, Color, Element, Length};

use crate::compose::SelectedLane;
use crate::message::Message;
use crate::theme;

pub mod chord_lane;
pub mod drumroll;
pub mod expanded_editor;
pub mod global_tracks;
pub mod lane_inspector;
pub mod lane_side;
pub mod manual_motif_canvas;
pub mod popover;
pub mod scale_stripe;
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
                let global_tracks_row = row![
                    Space::with_width(Length::Fixed(tracks::NAME_COLUMN_WIDTH)),
                    global_tracks::view(
                        &self.tempo_map,
                        placement.start_bar,
                        definition.length_bars,
                    ),
                ];

                let chords_selected =
                    matches!(self.compose.selected_lane, SelectedLane::Chords);
                let scale_stripe = container(scale_stripe::view(definition))
                    .width(Length::Fill)
                    .padding([8, 12]);
                let chord_lane = chord_lane::view(
                    definition,
                    &self.tempo_map,
                    placement.start_bar,
                    self.compose.selected_chord_id,
                    chords_selected,
                );

                let editor: Element<'_, Message> = match self.compose.selected_chord_id {
                    Some(chord_id) if definition.chords.iter().any(|c| c.id == chord_id) => {
                        popover::view(definition, chord_id)
                    }
                    _ => container(Space::with_height(0)).width(Length::Fill).into(),
                };

                let synth_tracks = tracks::view(self, placement, definition);
                let drum_tracks = drumroll::view(self, placement, definition);

                let section_sub = format!(
                    "{} \u{00b7} {}\u{2013}{}",
                    definition.name,
                    placement.start_bar + 1,
                    placement.start_bar + definition.length_bars,
                );
                let section_group =
                    group_header("SECTION", section_sub, "2 lanes", GroupKind::Section);
                let track_count = self
                    .registry
                    .tracks
                    .iter()
                    .filter(|t| {
                        matches!(
                            t.track_type,
                            resonance_audio::types::TrackType::Instrument
                        ) && t.sub_track.is_none()
                    })
                    .count();
                let tracks_sub = format!("{} tracks \u{00b7} monophonic", track_count);
                let tracks_group =
                    group_header("TRACKS", tracks_sub, "", GroupKind::Tracks);

                let left_column: Element<'_, Message> = match self.compose.expanded_track_id {
                    Some(track_id) if self.registry.tracks.iter().any(|t| t.id == track_id) => {
                        let expanded = expanded_editor::view(self, track_id, placement, definition);
                        let inner = column![
                            section_group,
                            scale_stripe,
                            global_tracks_row,
                            chord_lane,
                            editor,
                            tracks_group,
                            synth_tracks,
                            expanded
                        ]
                        .spacing(0)
                        .width(Length::Fill);
                        scrollable(inner)
                            .height(Length::Fill)
                            .width(Length::Fill)
                            .into()
                    }
                    _ => {
                        let inner = column![
                            section_group,
                            scale_stripe,
                            global_tracks_row,
                            chord_lane,
                            editor,
                            tracks_group,
                            synth_tracks,
                            drum_tracks
                        ]
                        .spacing(0)
                        .width(Length::Fill);
                        scrollable(inner)
                            .height(Length::Fill)
                            .width(Length::Fill)
                            .into()
                    }
                };

                // Unified right-hand inspector panel: context-sensitive
                // based on which lane is selected.
                let clip_id_for_drum = match &self.compose.selected_lane {
                    SelectedLane::Drums(track_id) => {
                        drumroll::clip_for_track(self, placement, definition, *track_id)
                    }
                    _ => None,
                };

                let right_panel = lane_inspector::view(
                    definition,
                    &self.compose.selected_lane,
                    &self.registry.tracks,
                    &self.compose.drumroll,
                    clip_id_for_drum,
                    &self.table_registry,
                );

                row![left_column, right_panel]
                    .spacing(0)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
            None => empty_state(),
        };

        column![strip, status, body]
            .spacing(0)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

/// Which color family a group header reads as. Section-level groups
/// (chord lane / scale) use the lavender accent; track-level groups (synth
/// + drum lanes) use the warm amber accent.
#[derive(Debug, Clone, Copy)]
enum GroupKind {
    Section,
    Tracks,
}

impl GroupKind {
    fn accent(self) -> Color {
        match self {
            GroupKind::Section => theme::ACCENT_SOFT,
            GroupKind::Tracks => theme::WARM,
        }
    }

    fn dot(self) -> Color {
        match self {
            GroupKind::Section => theme::ACCENT,
            GroupKind::Tracks => theme::WARM,
        }
    }
}

/// Group separator inserted between SECTION lanes (scale + chords) and
/// TRACKS lanes (synth + drums). Reads as a slim, dim banner — colored
/// bullet, letter-spaced uppercase tag, dim subtitle, and an optional
/// trailing count on the right.
fn group_header<'a>(
    tag: impl Into<String>,
    sub: impl Into<String>,
    count: impl Into<String>,
    kind: GroupKind,
) -> Element<'a, Message> {
    let dot = container(Space::new(Length::Fixed(6.0), Length::Fixed(6.0))).style(
        move |_theme| container::Style {
            background: Some(iced::Background::Color(kind.dot())),
            border: iced::Border {
                color: kind.dot(),
                width: 0.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        },
    );
    let tag_text = text(tag.into())
        .size(10)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(kind.accent());
    let sub_text = text(sub.into()).size(11).color(theme::TEXT_3);
    let mut head = row![dot, tag_text, sub_text]
        .spacing(8)
        .align_y(alignment::Vertical::Center);
    let count_str = count.into();
    if !count_str.is_empty() {
        head = head.push(Space::with_width(Length::Fill));
        head = head.push(
            text(count_str)
                .size(10)
                .font(theme::MONO_FONT)
                .color(theme::TEXT_3),
        );
    }
    container(head)
        .padding([8, 14])
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
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
