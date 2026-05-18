//! [`canvas::Program`] impl for [`ComposeTrackCanvas`] — per-frame draw
//! orchestration + click event routing.

use std::time::Instant;

use iced::widget::canvas::{self, Frame};
use iced::{mouse, Point, Rectangle, Renderer, Size, Theme};

use crate::compose::ComposeMessage;
use crate::message::*;
use crate::theme;

use crate::view::compose::lane_side;

use super::{
    lane_kind_for, track_meta_line, ComposeTrackCanvas, ComposeTrackCanvasState,
    DOUBLE_CLICK_MS, NAME_COLUMN_WIDTH,
};

impl<'a> canvas::Program<Message> for ComposeTrackCanvas<'a> {
    type State = ComposeTrackCanvasState;

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::BG);

        if self.section_end <= self.section_start || bounds.width <= 0.0 {
            return vec![frame.into_geometry()];
        }

        let tracks = self.sorted_tracks();
        let is_expanded = self.expanded_track_id.is_some();

        for (idx, track) in tracks.iter().enumerate() {
            let row_rect = self.track_row_rect(idx, bounds);
            if row_rect.y + row_rect.height < 0.0 || row_rect.y > bounds.height {
                continue;
            }

            let is_selected_for_details = self.details_track_id == Some(track.id);
            let is_this_expanded = self.expanded_track_id == Some(track.id);

            if is_expanded {
                // --- Collapsed strip rendering ---
                let side_rect = Rectangle {
                    x: 0.0,
                    y: row_rect.y,
                    width: bounds.width,
                    height: row_rect.height,
                };
                lane_side::draw_compact(
                    &mut frame,
                    side_rect,
                    &track.name,
                    is_selected_for_details,
                    is_this_expanded,
                );

                // Hint text for expanded track
                if is_this_expanded {
                    frame.fill_text(canvas::Text {
                        content: "(editing - double-click to collapse)".to_string(),
                        position: Point::new(
                            NAME_COLUMN_WIDTH + 10.0,
                            row_rect.y + row_rect.height * 0.5 - 6.0,
                        ),
                        color: theme::TEXT_DIM,
                        size: 10.0.into(),
                        ..canvas::Text::default()
                    });
                }

                // Bottom separator
                frame.fill_rectangle(
                    Point::new(0.0, row_rect.y + row_rect.height - 1.0),
                    Size::new(bounds.width, 1.0),
                    theme::SEPARATOR,
                );
            } else {
                // --- Normal full-height rendering ---
                let side_rect = Rectangle {
                    x: 0.0,
                    y: row_rect.y,
                    width: NAME_COLUMN_WIDTH,
                    height: row_rect.height,
                };
                let meta = track_meta_line(track);
                lane_side::draw(
                    &mut frame,
                    side_rect,
                    lane_kind_for(track),
                    &track.name,
                    Some(&meta),
                    is_selected_for_details,
                );

                let clip_rect = Rectangle {
                    x: row_rect.x + NAME_COLUMN_WIDTH,
                    y: row_rect.y,
                    width: (row_rect.width - NAME_COLUMN_WIDTH).max(0.0),
                    height: row_rect.height,
                };

                self.draw_grid_background(&mut frame, clip_rect);
                self.draw_beat_grid(&mut frame, clip_rect);

                let mut has_clip_in_section = false;
                for clip in self.midi_clips.iter().filter(|c| c.track_id == track.id) {
                    if let Some(tick_range) = self.clip_tick_range(clip) {
                        has_clip_in_section = true;
                        let clip_start_tick =
                            self.sample_to_section_tick(clip.start_sample);
                        self.draw_clip_outline(&mut frame, clip_rect, tick_range);
                        self.draw_notes(&mut frame, clip, clip_rect, clip_start_tick);
                    }
                }

                if !has_clip_in_section {
                    self.draw_add_button(&mut frame, clip_rect);
                }

                // Bottom separator between rows
                frame.fill_rectangle(
                    Point::new(0.0, row_rect.y + row_rect.height - 1.0),
                    Size::new(bounds.width, 1.0),
                    theme::SEPARATOR,
                );
            }
        }

        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        if let canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            let Some(pos) = cursor.position_in(bounds) else {
                return (canvas::event::Status::Ignored, None);
            };

            // Determine which track row was clicked.
            let clicked_track = self.hit_test_track(pos, bounds);

            // Double-click detection: expand/collapse a track.
            if let Some(track_id) = clicked_track {
                let now = Instant::now();
                if let Some((prev_time, prev_id)) = state.last_click {
                    if prev_id == track_id
                        && now.duration_since(prev_time).as_millis() < DOUBLE_CLICK_MS as u128
                    {
                        state.last_click = None;
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::Compose(ComposeMessage::ExpandTrack { track_id })),
                        );
                    }
                }
                state.last_click = Some((now, track_id));
            }

            // When tracks are expanded, only handle name-column clicks
            // on collapsed strips (to select for details).
            if self.expanded_track_id.is_some() {
                if pos.x < NAME_COLUMN_WIDTH {
                    if let Some(track_id) = clicked_track {
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::Compose(ComposeMessage::SelectLane(
                                crate::compose::SelectedLane::Instrument(track_id),
                            ))),
                        );
                    }
                }
                return (canvas::event::Status::Ignored, None);
            }

            // Normal (non-expanded) behaviour below.

            // Click on the name column opens the instrument details panel
            // on the right side of the Compose tab.
            if pos.x < NAME_COLUMN_WIDTH {
                if let Some(track_id) = self.hit_test_name_column(pos, bounds) {
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::Compose(ComposeMessage::SelectLane(
                            crate::compose::SelectedLane::Instrument(track_id),
                        ))),
                    );
                }
                return (canvas::event::Status::Ignored, None);
            }

            // "+" hint button first — only hits rows with no clip
            if let Some(track_id) = self.hit_test_add_button(pos, bounds) {
                return (
                    canvas::event::Status::Captured,
                    Some(Message::Compose(ComposeMessage::CreateMidiClipInSection {
                        track_id,
                        start_sample: self.section_start,
                        length_bars: self.section_length_bars,
                    })),
                );
            }
            if let Some(msg) = self.hit_test_note_edit(pos, bounds) {
                return (canvas::event::Status::Captured, Some(msg));
            }
        }
        (canvas::event::Status::Ignored, None)
    }
}
