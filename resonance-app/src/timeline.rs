/// Timeline canvas rendering for the DAW arrangement view.
use iced::widget::canvas;
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use crate::theme;
use crate::{ClipState, TrackState};

use resonance_audio::types::TrackId;

/// Data passed to the timeline canvas for rendering.
#[derive(Debug, Clone)]
pub struct TimelineCanvas {
    pub tracks: Vec<TrackState>,
    pub clips: Vec<ClipState>,
    pub playhead: u64,
    pub sample_rate: u32,
    pub zoom: f32,
    pub scroll_offset: f32,
    pub recording_tracks: Vec<TrackId>,
    pub recording_start_sample: u64,
}

impl<Message> canvas::Program<Message> for TimelineCanvas {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let ruler_height = 30.0;

        // Draw ruler background
        frame.fill_rectangle(
            Point::new(0.0, 0.0),
            Size::new(bounds.width, ruler_height),
            theme::RULER_BG,
        );

        // Draw time ruler markings
        self.draw_ruler(&mut frame, bounds.width, ruler_height);

        // Draw track backgrounds
        let mut sorted_tracks: Vec<&TrackState> = self.tracks.iter().collect();
        sorted_tracks.sort_by_key(|t| t.order);

        for (i, track) in sorted_tracks.iter().enumerate() {
            let y = ruler_height + i as f32 * theme::TRACK_HEIGHT;
            let bg = if i % 2 == 0 {
                theme::BG
            } else {
                theme::PANEL_DARK
            };
            frame.fill_rectangle(
                Point::new(0.0, y),
                Size::new(bounds.width, theme::TRACK_HEIGHT),
                bg,
            );

            // Recording overlay on armed tracks — starts at recording origin
            if self.recording_tracks.contains(&track.id) {
                let start_seconds =
                    self.recording_start_sample as f32 / self.sample_rate as f32;
                let start_x = start_seconds * self.zoom - self.scroll_offset;
                let playhead_seconds = self.playhead as f32 / self.sample_rate as f32;
                let playhead_x = playhead_seconds * self.zoom - self.scroll_offset;
                let overlay_x = start_x.max(0.0);
                let overlay_w = (playhead_x - overlay_x).max(0.0).min(bounds.width - overlay_x);
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

        // Draw clips
        for clip in &self.clips {
            self.draw_clip(&mut frame, clip, &sorted_tracks, ruler_height);
        }

        // Draw playhead
        let playhead_seconds = self.playhead as f32 / self.sample_rate as f32;
        let playhead_x = playhead_seconds * self.zoom - self.scroll_offset;
        if playhead_x >= 0.0 && playhead_x <= bounds.width {
            let total_height = ruler_height + self.tracks.len() as f32 * theme::TRACK_HEIGHT;
            let height = total_height.max(bounds.height);

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
                Size::new(1.0, height),
                theme::ACCENT,
            );
        }

        vec![frame.into_geometry()]
    }
}

impl TimelineCanvas {
    fn draw_ruler(&self, frame: &mut canvas::Frame, width: f32, ruler_height: f32) {
        // Determine tick spacing based on zoom
        let seconds_per_pixel = 1.0 / self.zoom;
        let min_pixel_spacing = 80.0;
        let min_seconds = seconds_per_pixel * min_pixel_spacing;

        // Choose nice time intervals
        let intervals = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0];
        let major_interval = intervals
            .iter()
            .find(|&&i| i >= min_seconds)
            .copied()
            .unwrap_or(60.0);

        let start_time = self.scroll_offset / self.zoom;
        let end_time = start_time + width / self.zoom;

        let first_tick = (start_time / major_interval).floor() as i64;
        let last_tick = (end_time / major_interval).ceil() as i64;

        for i in first_tick..=last_tick {
            let time = i as f32 * major_interval;
            let x = time * self.zoom - self.scroll_offset;

            if x < -1.0 || x > width + 1.0 {
                continue;
            }

            // Major tick
            frame.fill_rectangle(
                Point::new(x, ruler_height - 10.0),
                Size::new(1.0, 10.0),
                theme::TEXT_DIM,
            );

            // Time label
            let label = if major_interval >= 1.0 {
                let mins = (time / 60.0) as u32;
                let secs = time % 60.0;
                if mins > 0 {
                    format!("{}:{:02.0}", mins, secs)
                } else {
                    format!("{:.0}s", secs)
                }
            } else {
                format!("{:.1}s", time)
            };

            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(x + 3.0, ruler_height - 22.0),
                color: theme::TEXT_DIM,
                size: 11.0.into(),
                ..canvas::Text::default()
            });

            // Minor ticks (4 subdivisions)
            let minor_interval = major_interval / 4.0;
            for j in 1..4 {
                let minor_time = time + j as f32 * minor_interval;
                let minor_x = minor_time * self.zoom - self.scroll_offset;
                if minor_x >= 0.0 && minor_x <= width {
                    frame.fill_rectangle(
                        Point::new(minor_x, ruler_height - 5.0),
                        Size::new(1.0, 5.0),
                        Color::from_rgb(0.25, 0.25, 0.25),
                    );
                }
            }
        }

        // Ruler bottom line
        frame.fill_rectangle(
            Point::new(0.0, ruler_height - 1.0),
            Size::new(width, 1.0),
            theme::SEPARATOR,
        );
    }

    fn draw_clip(
        &self,
        frame: &mut canvas::Frame,
        clip: &ClipState,
        sorted_tracks: &[&TrackState],
        ruler_height: f32,
    ) {
        let track_index = sorted_tracks
            .iter()
            .position(|t| t.id == clip.track_id);

        let track_index = match track_index {
            Some(i) => i,
            None => return,
        };

        let y = ruler_height + track_index as f32 * theme::TRACK_HEIGHT + 2.0;
        let clip_height = theme::TRACK_HEIGHT - 4.0;

        let start_seconds = clip.start_sample as f32 / self.sample_rate as f32;
        let duration_seconds = clip.duration_samples as f32 / self.sample_rate as f32;

        let x = start_seconds * self.zoom - self.scroll_offset;
        let w = duration_seconds * self.zoom;

        // Clip body
        frame.fill_rectangle(
            Point::new(x, y),
            Size::new(w, clip_height),
            theme::CLIP_BODY,
        );

        // Clip header bar
        let header_height = 18.0;
        frame.fill_rectangle(
            Point::new(x, y),
            Size::new(w, header_height),
            theme::CLIP_HEADER,
        );

        // Clip name (truncated safely for multi-byte UTF-8)
        let display_name: String = if clip.name.chars().count() > 20 {
            let mut truncated: String = clip.name.chars().take(17).collect();
            truncated.push_str("...");
            truncated
        } else {
            clip.name.clone()
        };
        frame.fill_text(canvas::Text {
            content: display_name,
            position: Point::new(x + 4.0, y + 2.0),
            color: theme::TEXT,
            size: 11.0.into(),
            ..canvas::Text::default()
        });

        // Clip border
        let border = canvas::Path::rectangle(Point::new(x, y), Size::new(w, clip_height));
        frame.stroke(
            &border,
            canvas::Stroke::default()
                .with_color(Color::from_rgba(0.0, 0.0, 0.0, 0.3))
                .with_width(1.0),
        );
    }
}
