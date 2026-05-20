//! Compose tab body — the central scrolling workspace plus the
//! right-rail inspector. Wired by `Resonance::view_compose`. Split from
//! `view/compose/mod.rs` so the module file stays a thin
//! declarations-and-re-exports surface.

use iced::widget::{column, container, row, scrollable, text, Space};
use iced::{alignment, Element, Length};

use crate::compose::SelectedLane;
use crate::message::Message;
use crate::theme;

use super::group_header::{group_header, GroupKind};
use super::{
    chord_lane, drumroll, expanded_editor, global_tracks, lane_inspector, popover, scale_stripe,
    strip, tracks, vocal_lane, workspace_width,
};

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
            None => container(Space::new().height(0)).width(Length::Fill).into(),
        };

        let body: Element<'_, Message> = match selected {
            Some((placement, definition)) => {
                // The chord lane and the track rows share a beat grid. The
                // track area reserves `NAME_COLUMN_WIDTH` on its left for the
                // track name labels, so the chord lane is padded the same
                // amount to keep bar 1 aligned in both.
                let global_tracks_row = row![
                    Space::new().width(Length::Fixed(tracks::NAME_COLUMN_WIDTH)),
                    global_tracks::view(
                        &self.tempo_map,
                        placement.start_bar,
                        definition.length_bars,
                    ),
                ];

                let chords_selected =
                    matches!(self.compose.selected_lane, SelectedLane::Chords);
                // Fixed workspace width — every lane (chord, tracks, drum,
                // global, scale stripe, section/tracks headers) sits in the
                // same column so cells stay aligned and don't stretch with
                // window width. The horizontal `scrollable` below handles
                // overflow when the OS window is narrower than this width.
                let ws_width = workspace_width(
                    &self.tempo_map,
                    placement.start_bar,
                    definition.length_bars,
                );
                let scale_stripe = container(scale_stripe::view(definition))
                    .width(Length::Fixed(ws_width))
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
                    _ => container(Space::new().height(0))
                        .width(Length::Fixed(ws_width))
                        .into(),
                };

                let vocal_tracks = vocal_lane::view(self, placement, definition);
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
                        use resonance_audio::types::TrackType;
                        matches!(t.track_type, TrackType::Instrument | TrackType::Vocal)
                            && t.sub_track.is_none()
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
                            vocal_tracks,
                            synth_tracks,
                            expanded
                        ]
                        .spacing(0)
                        .width(Length::Fixed(ws_width));
                        scrollable(inner)
                            .direction(scrollable::Direction::Both {
                                vertical: scrollable::Scrollbar::default(),
                                horizontal: scrollable::Scrollbar::default(),
                            })
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
                            vocal_tracks,
                            synth_tracks,
                            drum_tracks
                        ]
                        .spacing(0)
                        .width(Length::Fixed(ws_width));
                        scrollable(inner)
                            .direction(scrollable::Direction::Both {
                                vertical: scrollable::Scrollbar::default(),
                                horizontal: scrollable::Scrollbar::default(),
                            })
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

                let section_drum_groups = self.compose.groups_for_definition(definition);
                let right_panel = lane_inspector::view(
                    definition,
                    &self.compose.selected_lane,
                    &self.registry.tracks,
                    &self.compose.drumroll,
                    section_drum_groups,
                    clip_id_for_drum,
                    &self.table_registry,
                    &self.compose.vocal_bulk_lyrics,
                );

                row![left_column, right_panel]
                    .spacing(0)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            }
            None => empty_state(),
        };

        // Bottom editor panel — the piano roll (instrument tracks) or
        // vocal roll (vocal tracks), opened by double-clicking a clip.
        // Mirrors the wiring in `view_main_area` so the editor surface
        // is reachable from either tab.
        let body_with_editor: Element<'_, Message> =
            if let Some(editor) = self.view_midi_editor_panel() {
                column![
                    container(body).width(Length::Fill).height(Length::Fill),
                    editor,
                ]
                .spacing(0)
                .into()
            } else {
                body
            };

        column![strip, status, body_with_editor]
            .spacing(0)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
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
