//! Clip warp on the timeline: the `WARP` badge on a warped audio clip and
//! its warp markers — a hairline through the clip with a handle in the
//! marker strip along the bottom edge (`theme::WARP_MARKER_STRIP_HEIGHT`),
//! where the pointer grabs, adds and removes them.
//!
//! Marker beats project onto the timeline at the project's base tempo
//! (`state::clip_warp` module docs), the same projection the drag reducer
//! inverts.

use iced::widget::canvas;
use iced::widget::canvas::path::Builder;
use iced::{Color, Point, Rectangle, Size};

use super::TimelineCanvas;
use crate::state::ClipState;
use crate::theme;

/// What a pointer over a warped clip's marker strip is on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::view::timeline) enum WarpHit {
    /// Marker `index`'s handle.
    Marker(usize),
    /// Bare strip, `beat` beats into the clip (a double-click adds a
    /// marker there).
    Strip { beat: f64 },
}

impl TimelineCanvas<'_> {
    /// Pixels per beat at the project's base tempo.
    fn px_per_beat(&self) -> f32 {
        if self.bpm <= 0.0 {
            return 0.0;
        }
        60.0 / self.bpm * self.zoom
    }

    /// Canvas x of `beat` beats into a clip whose left edge is at `clip_x`.
    pub(in crate::view::timeline) fn warp_beat_x(&self, clip_x: f32, beat: f64) -> f32 {
        clip_x + beat as f32 * self.px_per_beat()
    }

    /// Hit-test `pos` against the marker strip of a warped clip drawn in
    /// `rect`. `None` when the clip is not warped or `pos` is outside the
    /// strip. The nearest marker within [`theme::WARP_MARKER_HIT_PX`] wins.
    pub(in crate::view::timeline) fn warp_hit_in_rect(
        &self,
        clip: &ClipState,
        rect: Rectangle,
        pos: Point,
    ) -> Option<WarpHit> {
        if !clip.warp.enabled {
            return None;
        }
        let strip_top = rect.y + rect.height - theme::WARP_MARKER_STRIP_HEIGHT;
        if pos.y < strip_top
            || pos.y > rect.y + rect.height
            || pos.x < rect.x
            || pos.x > rect.x + rect.width
        {
            return None;
        }
        let nearest = clip
            .warp
            .markers
            .iter()
            .enumerate()
            .map(|(i, m)| (i, (self.warp_beat_x(rect.x, m.timeline_beat) - pos.x).abs()))
            .filter(|&(_, d)| d <= theme::WARP_MARKER_HIT_PX)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((index, _)) = nearest {
            return Some(WarpHit::Marker(index));
        }
        let ppb = self.px_per_beat();
        if ppb <= 0.0 {
            return None;
        }
        Some(WarpHit::Strip {
            beat: ((pos.x - rect.x) / ppb) as f64,
        })
    }

    /// The badge and markers of a warped clip, drawn inside its body.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_clip_warp(
        &self,
        frame: &mut canvas::Frame,
        clip: &ClipState,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) {
        if !clip.warp.enabled {
            return;
        }
        draw_warp_badge(frame, clip, x, y, w, h);

        let strip_top = y + h - theme::WARP_MARKER_STRIP_HEIGHT;
        // The strip itself: a faint band so the grab zone is discoverable.
        frame.fill_rectangle(
            Point::new(x, strip_top),
            Size::new(w, theme::WARP_MARKER_STRIP_HEIGHT),
            Color {
                a: 0.10,
                ..theme::WARM
            },
        );
        for marker in &clip.warp.markers {
            let mx = self.warp_beat_x(x, marker.timeline_beat);
            if mx < x - 0.5 || mx > x + w + 0.5 {
                continue;
            }
            let line = canvas::Path::line(Point::new(mx, y), Point::new(mx, strip_top));
            frame.stroke(
                &line,
                canvas::Stroke::default()
                    .with_color(Color {
                        a: 0.55,
                        ..theme::WARM
                    })
                    .with_width(1.0),
            );
            // Handle: an upward wedge filling the strip.
            let half = 4.5;
            let handle = canvas::Path::new(|b: &mut Builder| {
                b.move_to(Point::new(mx, strip_top + 1.0));
                b.line_to(Point::new(mx + half, y + h - 1.0));
                b.line_to(Point::new(mx - half, y + h - 1.0));
                b.close();
            });
            frame.fill(&handle, theme::WARM);
        }
    }
}

/// `WARP` (and the source tempo, when known) in a small warm pill at the
/// clip's top-left, under the name row — the at-a-glance mark that this
/// clip follows tempo.
fn draw_warp_badge(frame: &mut canvas::Frame, clip: &ClipState, x: f32, y: f32, w: f32, h: f32) {
    let label = match clip.warp.original_bpm {
        Some(bpm) => format!("WARP {}", crate::state::format_warp_bpm(bpm)),
        None => "WARP".to_string(),
    };
    // ~6.4 px per mono glyph at 9 px (with margin), plus padding.
    let pill_w = label.chars().count() as f32 * 6.4 + 8.0;
    let pill_h = 12.0;
    let top = y + 17.0;
    if w < pill_w + 8.0 || h < top - y + pill_h + theme::WARP_MARKER_STRIP_HEIGHT {
        return;
    }
    let pill = canvas::Path::rounded_rectangle(
        Point::new(x + 5.0, top),
        Size::new(pill_w, pill_h),
        theme::RADIUS_XS.into(),
    );
    frame.fill(
        &pill,
        Color {
            a: 0.22,
            ..theme::WARM
        },
    );
    frame.fill_text(canvas::Text {
        content: label,
        position: Point::new(x + 9.0, top + 1.5),
        color: theme::WARM,
        size: 9.0.into(),
        font: theme::MONO_FONT,
        ..canvas::Text::default()
    });
}
