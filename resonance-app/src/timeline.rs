//! Timeline canvas: the arrangement view for tracks, audio clips, and
//! MIDI clips. The canvas's three concerns are split across files:
//!
//! - this file: [`TimelineCanvas`] struct, small geometry helpers, and
//!   the [`canvas::Program`] impl that orchestrates per-event dispatch
//!   and per-frame drawing.
//! - [`timeline_input`](crate::timeline_input): pointer / wheel /
//!   keyboard event handling and the [`TimelineState`] drag tracker.
//! - [`timeline_draw`](crate::timeline_draw): pure-draw routines for
//!   the ruler, grid, global tracks, and clips.
//! - [`timeline_snap`](crate::timeline_snap): the snap-to-grid helpers
//!   shared with the clip-drag and seek paths.
use std::time::Instant;

use iced::widget::canvas;
use iced::{keyboard, mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use crate::message::*;
use crate::state::{self, ClipState, MidiClipState, TrackState};
use crate::theme;
use crate::timeline_input::{ClipInteraction, TempoDrag};

use resonance_audio::types::{ClipId, TempoMap, TrackId};

pub mod hit_test;
pub mod scrollbar;

// Snap helpers are external public API for this canvas — re-export them
// from the snap submodule so existing call sites keep working.
pub use crate::timeline_snap::{snap_sample_to_grid, snap_sample_to_grid_tempo};

/// Data passed to the timeline canvas for rendering.
#[derive(Debug)]
pub struct TimelineCanvas<'a> {
    pub tracks: &'a [TrackState],
    pub clips: &'a [ClipState],
    pub playhead: u64,
    pub sample_rate: u32,
    pub zoom: f32,
    pub scroll_offset: f32,
    pub recording_tracks: Vec<TrackId>,
    pub recording_start_sample: u64,
    pub bpm: f32,
    pub time_sig_num: u8,
    pub scroll_offset_y: f32,
    pub loop_enabled: bool,
    pub loop_in: u64,
    pub loop_out: u64,
    pub selected_clip: Option<ClipId>,
    pub midi_clips: &'a [MidiClipState],
    pub selected_midi_clip: Option<ClipId>,
    pub selected_track: Option<TrackId>,
    pub global_tracks_expanded: bool,
    pub tempo_map: &'a TempoMap,
    pub selected_global_event: Option<crate::state::SelectedGlobalEvent>,
}

impl TimelineCanvas<'_> {
    /// Height of the global tracks area (tempo + time signature rows).
    /// Returns 0.0 when collapsed.
    pub(crate) fn global_tracks_height(&self) -> f32 {
        if self.global_tracks_expanded {
            2.0 * theme::GLOBAL_TRACK_ROW_HEIGHT
        } else {
            0.0
        }
    }

    /// Total fixed header height: ruler + global tracks area.
    /// This is the Y offset where regular track rows begin.
    pub(crate) fn fixed_header_height(&self) -> f32 {
        theme::RULER_HEIGHT + self.global_tracks_height()
    }

    /// Convert a sample position to pixel x coordinate.
    pub(crate) fn sample_to_x(&self, sample: u64) -> f32 {
        (sample as f64 / self.sample_rate as f64) as f32 * self.zoom - self.scroll_offset
    }

    /// Rightmost pixel needed to show all content (clips + MIDI clips).
    /// Always returns at least `viewport_width * 1.5` so users can scroll a
    /// bit past the last clip, and never less than `viewport_width` itself.
    pub(crate) fn content_width_px(&self, viewport_width: f32) -> f32 {
        let mut max_sample: u64 = 0;
        for c in self.clips {
            let end = c.start_sample + c.duration_samples;
            if end > max_sample {
                max_sample = end;
            }
        }
        for c in self.midi_clips {
            let end = self.tempo_map.tick_to_abs_sample(
                c.start_sample,
                c.duration_ticks,
                self.sample_rate,
            );
            if end > max_sample {
                max_sample = end;
            }
        }
        let content = (max_sample as f64 / self.sample_rate as f64) as f32 * self.zoom;
        content.max(viewport_width * 1.5).max(viewport_width)
    }

    /// Total vertical content height (tracks + ruler + global tracks).
    /// Excludes sub-tracks since the arrange view hides them entirely.
    pub(crate) fn content_height_px(&self) -> f32 {
        let visible = self.tracks.iter().filter(|t| t.sub_track.is_none()).count();
        self.fixed_header_height() + visible as f32 * theme::TRACK_HEIGHT
    }

    /// Tracks visible in the arrange view, sorted by `order`. Excludes
    /// sub-tracks (rendered only in the mixer view).
    pub(super) fn visible_tracks_sorted(&self) -> Vec<&TrackState> {
        hit_test::sorted_arrange_tracks(self.tracks)
    }
}

/// Local state for the timeline canvas, tracking active drag operations.
#[derive(Debug, Default)]
pub struct TimelineState {
    pub(super) dragging_loop: bool,
    pub(super) clip_interaction: Option<ClipInteraction>,
    pub(super) last_reported_width: f32,
    pub(super) last_reported_content_width: f32,
    pub(super) last_reported_content_height: f32,
    /// Tracks the most recent click on a MIDI clip for double-click detection.
    pub(super) last_midi_click: Option<(Instant, ClipId)>,
    /// Horizontal scrollbar drag in progress. Stores the x-offset of the
    /// grab point relative to the left edge of the thumb (in track pixels).
    pub(super) h_scrollbar_grab: Option<f32>,
    /// Vertical scrollbar drag in progress (y-offset within the thumb).
    pub(super) v_scrollbar_grab: Option<f32>,
    /// Tracks the most recent click on a global track for double-click detection.
    pub(super) last_global_click: Option<(Instant, state::GlobalTrackKind)>,
    /// Active tempo-event drag.
    pub(super) tempo_drag: Option<TempoDrag>,
}

impl canvas::Program<Message> for TimelineCanvas<'_> {
    type State = TimelineState;

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        let result = match event {
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                self.handle_wheel(delta, bounds, cursor)
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                self.handle_press(state, bounds, cursor)
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                self.handle_move(state, bounds, cursor)
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                self.handle_release(state)
            }
            canvas::Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
                self.handle_key(&key)
            }
            _ => (canvas::event::Status::Ignored, None),
        };
        if let (_, Some(_)) = result {
            return result;
        }
        if result.0 == canvas::event::Status::Captured {
            return result;
        }
        if let Some(msg) = self.report_viewport(state, bounds) {
            return (canvas::event::Status::Ignored, Some(msg));
        }
        (canvas::event::Status::Ignored, None)
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let ruler_height = theme::RULER_HEIGHT;
        let header_height = self.fixed_header_height();
        let y_off = self.scroll_offset_y;

        // Draw ruler background
        frame.fill_rectangle(
            Point::new(0.0, 0.0),
            Size::new(bounds.width, ruler_height),
            theme::RULER_BG,
        );

        // Draw global tracks area (tempo + time signature) between ruler and tracks
        self.draw_global_tracks(&mut frame, bounds.width, ruler_height);

        // Draw track backgrounds. Only non-sub-tracks are rendered; the
        // mixer view is where sub-track lanes live.
        let sorted_tracks = self.visible_tracks_sorted();
        let track_area_height = sorted_tracks.len() as f32 * theme::TRACK_HEIGHT;

        for (i, track) in sorted_tracks.iter().enumerate() {
            let y = header_height + i as f32 * theme::TRACK_HEIGHT - y_off;

            // Skip tracks entirely above or below the visible area
            if y + theme::TRACK_HEIGHT < header_height || y > bounds.height {
                continue;
            }

            let is_selected = self.selected_track == Some(track.id);
            let bg = if is_selected {
                theme::PANEL_SELECTED
            } else if i % 2 == 0 {
                theme::BG
            } else {
                theme::PANEL_DARK
            };
            frame.fill_rectangle(
                Point::new(0.0, y),
                Size::new(bounds.width, theme::TRACK_HEIGHT),
                bg,
            );

            // Recording overlay on armed tracks
            if self.recording_tracks.contains(&track.id) {
                let (overlay_start, overlay_end) = if self.loop_enabled {
                    (self.loop_in, self.playhead.min(self.loop_out))
                } else {
                    (self.recording_start_sample, self.playhead)
                };
                let start_x = self.sample_to_x(overlay_start);
                let end_x = self.sample_to_x(overlay_end);
                let overlay_x = start_x.max(0.0);
                let overlay_w = (end_x - overlay_x).max(0.0).min(bounds.width - overlay_x);
                if overlay_w > 0.0 {
                    frame.fill_rectangle(
                        Point::new(overlay_x, y),
                        Size::new(overlay_w, theme::TRACK_HEIGHT),
                        Color::from_rgba(0.8, 0.2, 0.2, 0.08),
                    );
                }
            }

            // Track separator line
            frame.fill_rectangle(
                Point::new(0.0, y + theme::TRACK_HEIGHT - 1.0),
                Size::new(bounds.width, 1.0),
                theme::TRACK_LINE,
            );
        }

        // Draw bar/beat grid lines through track area
        self.draw_grid_lines(
            &mut frame,
            bounds.width,
            header_height,
            track_area_height,
            y_off,
        );

        // Draw bar/beat ruler
        self.draw_ruler(&mut frame, bounds.width, ruler_height);

        // Draw audio clips
        for clip in self.clips {
            self.draw_clip(
                &mut frame,
                clip,
                &sorted_tracks,
                header_height,
                y_off,
                bounds.height,
            );
        }

        // Draw MIDI clips
        for clip in self.midi_clips {
            self.draw_midi_clip(
                &mut frame,
                clip,
                &sorted_tracks,
                header_height,
                y_off,
                bounds.height,
            );
        }

        // Draw loop in/out markers
        if self.loop_enabled {
            let loop_in_x = self.sample_to_x(self.loop_in);
            let loop_out_x = self.sample_to_x(self.loop_out);
            let total_height = (header_height + track_area_height - y_off).max(bounds.height);
            let loop_color = theme::LOOP_MARKER;

            // Dim overlay outside loop range (over track area only)
            if loop_in_x > 0.0 {
                frame.fill_rectangle(
                    Point::new(0.0, header_height),
                    Size::new(loop_in_x.min(bounds.width), total_height - header_height),
                    Color::from_rgba(0.0, 0.0, 0.0, 0.15),
                );
            }
            if loop_out_x < bounds.width {
                let right_start = loop_out_x.max(0.0);
                frame.fill_rectangle(
                    Point::new(right_start, header_height),
                    Size::new(
                        (bounds.width - right_start).max(0.0),
                        total_height - header_height,
                    ),
                    Color::from_rgba(0.0, 0.0, 0.0, 0.15),
                );
            }

            // Amber range fill in ruler area
            let range_x = loop_in_x.max(0.0);
            let range_w = (loop_out_x - range_x).max(0.0).min(bounds.width - range_x);
            if range_w > 0.0 {
                frame.fill_rectangle(
                    Point::new(range_x, 0.0),
                    Size::new(range_w, ruler_height),
                    Color::from_rgba(0.9, 0.72, 0.1, 0.15),
                );
            }

            // Loop In line + handle
            if loop_in_x >= -1.0 && loop_in_x <= bounds.width + 1.0 {
                frame.fill_rectangle(
                    Point::new(loop_in_x - 0.5, 0.0),
                    Size::new(1.0, total_height),
                    loop_color,
                );
                let tri = canvas::Path::new(|b| {
                    b.move_to(Point::new(loop_in_x - 6.0, 0.0));
                    b.line_to(Point::new(loop_in_x + 6.0, 0.0));
                    b.line_to(Point::new(loop_in_x, 8.0));
                    b.close();
                });
                frame.fill(&tri, loop_color);
            }

            // Loop Out line + handle
            if loop_out_x >= -1.0 && loop_out_x <= bounds.width + 1.0 {
                frame.fill_rectangle(
                    Point::new(loop_out_x - 0.5, 0.0),
                    Size::new(1.0, total_height),
                    loop_color,
                );
                let tri = canvas::Path::new(|b| {
                    b.move_to(Point::new(loop_out_x - 6.0, 0.0));
                    b.line_to(Point::new(loop_out_x + 6.0, 0.0));
                    b.line_to(Point::new(loop_out_x, 8.0));
                    b.close();
                });
                frame.fill(&tri, loop_color);
            }
        }

        // Draw playhead
        let playhead_seconds = (self.playhead as f64 / self.sample_rate as f64) as f32;
        let playhead_x = playhead_seconds * self.zoom - self.scroll_offset;
        if playhead_x >= 0.0 && playhead_x <= bounds.width {
            let total_height = (header_height + track_area_height - y_off).max(bounds.height);

            // Playhead triangle at top
            let triangle = canvas::Path::new(|builder| {
                builder.move_to(Point::new(playhead_x - 6.0, 0.0));
                builder.line_to(Point::new(playhead_x + 6.0, 0.0));
                builder.line_to(Point::new(playhead_x, 8.0));
                builder.close();
            });
            frame.fill(&triangle, theme::ACCENT);

            // Playhead line
            frame.fill_rectangle(
                Point::new(playhead_x - 0.5, 0.0),
                Size::new(1.0, total_height),
                theme::ACCENT,
            );
        }

        // Smart scrollbars — drawn last so they sit above clips + playhead.
        let (h_rects, v_rects) = self.scrollbar_rects(bounds);

        let track_color = Color::from_rgba(0.08, 0.08, 0.08, 0.8);
        let thumb_color = Color::from_rgba(0.45, 0.45, 0.45, 0.85);

        if let Some(sb) = h_rects {
            frame.fill_rectangle(
                Point::new(sb.track.x, sb.track.y),
                Size::new(sb.track.width, sb.track.height),
                track_color,
            );
            frame.fill_rectangle(
                Point::new(sb.thumb.x, sb.thumb.y),
                Size::new(sb.thumb.width, sb.thumb.height),
                thumb_color,
            );
        }
        if let Some(sb) = v_rects {
            frame.fill_rectangle(
                Point::new(sb.track.x, sb.track.y),
                Size::new(sb.track.width, sb.track.height),
                track_color,
            );
            frame.fill_rectangle(
                Point::new(sb.thumb.x, sb.thumb.y),
                Size::new(sb.thumb.width, sb.thumb.height),
                thumb_color,
            );
        }

        vec![frame.into_geometry()]
    }
}
