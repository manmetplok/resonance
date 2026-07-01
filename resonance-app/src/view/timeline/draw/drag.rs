//! Drag-to-timeline placement affordances.
use iced::widget::canvas;
use iced::{Color, Point, Size};

use crate::message::DropTarget;
use crate::state::DragPlacement;
use crate::theme;
use super::TimelineCanvas;

impl TimelineCanvas<'_> {
    /// Draw the drag-to-timeline placement affordances (doc #175, todo
    /// #605) over the lanes: the lit target lane, a dashed grid-snapped
    /// ghost clip, the "create a new audio track" drop zone below the last
    /// lane, and the drag pill + drop tooltip trailing the cursor.
    ///
    /// Runs in the uncached overlay pass so it repaints as the cursor moves.
    /// All coordinates are canvas content-space (`sample_to_x`'s space).
    pub(in crate::view::timeline) fn draw_drag_placement(
        &self,
        frame: &mut canvas::Frame,
        bounds: iced::Rectangle,
        drag: &DragPlacement,
    ) {
        const DASH: canvas::LineDash<'static> = canvas::LineDash {
            segments: &[5.0, 3.0],
            offset: 0,
        };

        let header_height = self.fixed_header_height();
        let y_off = self.scroll_offset_y;
        let track_h = theme::TRACK_HEIGHT;
        let inset = theme::CLIP_LANE_INSET;
        let sorted_tracks = self.visible_tracks_sorted();
        let lane_count = sorted_tracks.len();

        let lane_index = drag.resolved.as_ref().and_then(|r| r.lane_index);
        let start_sample = drag.resolved.as_ref().map(|r| match r.target {
            DropTarget::ExistingTrack { start_sample, .. }
            | DropTarget::NewTrack { start_sample } => start_sample,
        });
        let targeting_new = drag.resolved.as_ref().is_some_and(|r| r.lane_index.is_none());

        // --- New-audio-track drop zone, below the last lane ---------------
        let zone_h = track_h.min(72.0);
        let zone_y = header_height + lane_count as f32 * track_h - y_off;
        if zone_y < bounds.height && zone_y + zone_h > header_height {
            let zone = canvas::Path::rounded_rectangle(
                Point::new(6.0, zone_y + 6.0),
                Size::new((bounds.width - 12.0).max(0.0), (zone_h - 12.0).max(0.0)),
                theme::RADIUS_LG.into(),
            );
            if targeting_new {
                frame.fill(&zone, theme::ACCENT_DIM);
            }
            frame.stroke(
                &zone,
                canvas::Stroke {
                    line_dash: DASH,
                    ..canvas::Stroke::default()
                        .with_color(if targeting_new {
                            theme::ACCENT_SOFT
                        } else {
                            theme::ACCENT_LINE
                        })
                        .with_width(1.5)
                },
            );
            frame.fill_text(canvas::Text {
                content: "+  Create a new audio track".to_string(),
                position: Point::new(24.0, zone_y + zone_h / 2.0 - 7.0),
                color: if targeting_new {
                    theme::ACCENT_SOFT
                } else {
                    theme::TEXT_3
                },
                size: 12.0.into(),
                font: theme::UI_FONT_MEDIUM,
                ..canvas::Text::default()
            });
        }

        // --- Lit target lane ---------------------------------------------
        if let Some(i) = lane_index {
            let lane_y = header_height + i as f32 * track_h - y_off;
            if lane_y + track_h > header_height && lane_y < bounds.height {
                frame.fill_rectangle(
                    Point::new(0.0, lane_y),
                    Size::new(bounds.width, track_h),
                    theme::ACCENT_DIM,
                );
                let lane_rect = canvas::Path::rectangle(
                    Point::new(0.5, lane_y + 0.5),
                    Size::new(bounds.width - 1.0, track_h - 1.0),
                );
                frame.stroke(
                    &lane_rect,
                    canvas::Stroke::default()
                        .with_color(theme::ACCENT_LINE)
                        .with_width(1.5),
                );
            }
        }

        // --- Dashed grid-snapped ghost clip ------------------------------
        if let Some(start) = start_sample {
            // Base y of the lane (existing) or the new-track zone.
            let base_y = match lane_index {
                Some(i) => header_height + i as f32 * track_h - y_off,
                None => zone_y,
            };
            let ghost_x = self.sample_to_x(start);
            let dur_seconds = drag.asset.duration_samples as f32 / self.sample_rate as f32;
            let ghost_w = (dur_seconds * self.zoom).max(24.0);
            let ghost_y = base_y + inset;
            let ghost_h = track_h - 2.0 * inset;
            let ghost = canvas::Path::rounded_rectangle(
                Point::new(ghost_x, ghost_y),
                Size::new(ghost_w, ghost_h),
                theme::RADIUS_LG.into(),
            );
            frame.fill(
                &ghost,
                Color {
                    a: 0.16,
                    ..theme::WARM
                },
            );
            frame.stroke(
                &ghost,
                canvas::Stroke {
                    line_dash: DASH,
                    ..canvas::Stroke::default()
                        .with_color(theme::WARM)
                        .with_width(1.4)
                },
            );
            // Clip name inside the ghost when there's room.
            if ghost_w > 40.0 {
                frame.fill_text(canvas::Text {
                    content: drag.asset.name.clone(),
                    position: Point::new(ghost_x + 8.0, ghost_y + 6.0),
                    color: theme::WARM,
                    size: 11.0.into(),
                    font: theme::UI_FONT_MEDIUM,
                    ..canvas::Text::default()
                });
            }
        }

        // --- Drag pill + drop tooltip trailing the cursor ----------------
        let cursor = drag.cursor;
        // Pill: the file name in a small card offset from the cursor.
        let pill_x = cursor.x + 12.0;
        let pill_y = cursor.y + 12.0;
        let pill_w = (drag.asset.name.chars().count() as f32 * 6.6 + 30.0).max(60.0);
        let pill_h = 22.0;
        let pill = canvas::Path::rounded_rectangle(
            Point::new(pill_x, pill_y),
            Size::new(pill_w, pill_h),
            theme::RADIUS_MD.into(),
        );
        frame.fill(&pill, theme::BG_2);
        frame.stroke(
            &pill,
            canvas::Stroke::default()
                .with_color(theme::ACCENT_LINE)
                .with_width(1.0),
        );
        // WARM dot marks the audio domain.
        frame.fill(
            &canvas::Path::rounded_rectangle(
                Point::new(pill_x + 9.0, pill_y + pill_h / 2.0 - 3.0),
                Size::new(6.0, 6.0),
                2.0.into(),
            ),
            theme::WARM,
        );
        frame.fill_text(canvas::Text {
            content: drag.asset.name.clone(),
            position: Point::new(pill_x + 22.0, pill_y + 5.0),
            color: theme::TEXT_1,
            size: 11.0.into(),
            font: theme::UI_FONT_MEDIUM,
            ..canvas::Text::default()
        });

        // Tooltip: target track + bar + any conversion, below the pill.
        if let Some(res) = drag.resolved.as_ref() {
            let track_name = match lane_index {
                Some(i) => sorted_tracks
                    .get(i)
                    .map(|t| t.name.clone())
                    .unwrap_or_else(|| "Track".to_string()),
                None => "New audio track".to_string(),
            };
            let mut parts = vec![track_name, res.bar_label.clone()];
            if let Some(conv) = drag.asset.conversion.as_ref() {
                parts.push(conv.clone());
            }
            let tip = parts.join("  ·  ");
            let tip_x = pill_x;
            let tip_y = pill_y + pill_h + 6.0;
            let tip_w = (tip.chars().count() as f32 * 6.0 + 20.0).max(80.0);
            let tip_h = 20.0;
            let tip_rect = canvas::Path::rounded_rectangle(
                Point::new(tip_x, tip_y),
                Size::new(tip_w, tip_h),
                theme::RADIUS_MD.into(),
            );
            frame.fill(&tip_rect, theme::BG_1);
            frame.stroke(
                &tip_rect,
                canvas::Stroke::default()
                    .with_color(theme::LINE)
                    .with_width(1.0),
            );
            frame.fill_text(canvas::Text {
                content: tip,
                position: Point::new(tip_x + 10.0, tip_y + 4.0),
                color: theme::TEXT_2,
                size: 10.0.into(),
                font: theme::UI_FONT_MEDIUM,
                ..canvas::Text::default()
            });
        }
    }
}
