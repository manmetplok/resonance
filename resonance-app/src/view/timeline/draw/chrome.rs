//! Shared clip-drawing primitives: chrome struct, lane-rect helper, fade
//! utilities, and standalone draw helpers reused by `clip`, `midi_notes`, etc.
use iced::widget::canvas;
use iced::widget::text::Alignment as TextAlignment;
use iced::{Color, Point, Size};

use crate::state::ClipState;
use crate::theme;
use crate::view::arrange_layout::ArrangeRowLayout;
use super::TimelineCanvas;
use resonance_audio::types::TrackId;

/// Lane-relative rect for a clip on `track_id`: returns `(y, height,
/// indent)`, or `None` when the track has no visible lane (unknown, a
/// member of a collapsed group, or scrolled out of view). The lane Y /
/// height come from the shared [`ArrangeRowLayout`] (doc #203) rather
/// than `track_index * TRACK_HEIGHT`, so clips sit correctly under the
/// variable row pitch and clips on hidden (collapsed-member) tracks are
/// not drawn. The `CLIP_LANE_INSET` top/bottom inset matches the design.
///
/// `ruler_height` is the fixed canvas-header height the lane area starts
/// below; the layout's `y_top` is measured from the top of that lane
/// area, so the two are simply added.
///
/// Shared with the `input` modules so the pointer hit rect is the *same*
/// body the user sees drawn — there is no longer a separate uniform-pitch
/// hit rect (epic #36, todo #732).
pub(in crate::view::timeline) fn clip_lane_rect(
    canvas: &TimelineCanvas,
    track_id: TrackId,
    layout: &ArrangeRowLayout,
    ruler_height: f32,
    y_off: f32,
    visible_height: f32,
) -> Option<(f32, f32, f32)> {
    let (row_y_top, row_height) = layout.track_row_rect(track_id)?;

    // Calculate indent for group members using the registry method
    let indent_level = canvas.track_groups.indent_depth(track_id);
    let indent = indent_level as f32 * theme::GROUP_MEMBER_INDENT;
    let lane_y =
        crate::view::timeline::hit_test::lane_canvas_y(row_y_top, ruler_height, y_off);
    let y = lane_y + theme::CLIP_LANE_INSET;
    let clip_height = row_height - 2.0 * theme::CLIP_LANE_INSET;

    if y + clip_height < ruler_height || y > visible_height {
        return None;
    }

    Some((y, clip_height, indent))
}

/// Shared chrome for audio and MIDI clips on the timeline: rounded
/// body wash, truncated name label, and selection-aware border. The
/// kind-specific interior (waveform or note preview) is drawn by the
/// `content` closure between the body fill and the name/border, so
/// layering matches the per-kind draw order.
pub(super) struct ClipChrome<'a> {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) w: f32,
    pub(super) h: f32,
    pub(super) body_color: Color,
    pub(super) border_color: Color,
    pub(super) is_selected: bool,
    pub(super) name: &'a str,
    pub(super) name_color: Color,
    /// Audio clips gate the label on clip width; MIDI clips always
    /// draw it.
    pub(super) show_name: bool,
}

impl ClipChrome<'_> {
    pub(super) fn draw(self, frame: &mut canvas::Frame, content: impl FnOnce(&mut canvas::Frame)) {
        let body = canvas::Path::rounded_rectangle(
            Point::new(self.x, self.y),
            Size::new(self.w, self.h),
            8.0.into(),
        );
        frame.fill(&body, self.body_color);

        content(frame);

        // Clip name in the header row, truncated for long names.
        // ASCII "..." suffix (not '…') — keeps the canvas text metrics
        // identical to the pre-refactor rendering.
        let display_name = crate::util::short_with(self.name, 20, "...");
        if self.show_name {
            frame.fill_text(canvas::Text {
                content: display_name,
                position: Point::new(self.x + 9.0, self.y + 4.0),
                color: self.name_color,
                size: 10.5.into(),
                ..canvas::Text::default()
            });
        }

        // Border. Selection wins over normal hairline.
        let border_w = if self.is_selected { 1.5 } else { 1.0 };
        let stroke_path = canvas::Path::rounded_rectangle(
            Point::new(self.x, self.y),
            Size::new(self.w, self.h),
            8.0.into(),
        );
        frame.stroke(
            &stroke_path,
            canvas::Stroke::default()
                .with_color(self.border_color)
                .with_width(border_w),
        );
    }
}

/// Darkening applied over the attenuated part of a fade ramp so the
/// waveform under it reads as faded out.
pub(super) const FADE_WEDGE_COLOR: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.22,
};

/// Visual radius of the circular fade / gain handle beads. Smaller than
/// the 10px hit radius in [`super::super::hit_test`] so the bead reads as a neat
/// dot while staying easy to grab.
pub(super) const BEAD_RADIUS: f32 = 4.5;

/// Map a clip's gain (dB) to its body wash colour. Unity sits at the
/// existing warm wash; louder raises the alpha (brighter), quieter lowers
/// it (darker) so a clip's level is legible without opening anything.
/// Clamped over ±18 dB so extreme gains stay within a readable band.
pub fn gain_tinted_body(gain_db: f32) -> Color {
    let norm = (gain_db / 18.0).clamp(-1.0, 1.0);
    let a = (0.10 + norm * 0.08).clamp(0.03, 0.20);
    Color { a, ..theme::WARM }
}

/// Format a clip-gain value as a signed mono dB tag, e.g. `+3.0 dB`,
/// `-6.0 dB`. Thin wrapper over the one view-layer dB formatter
/// (`util::format_db_signed`, UX-17), which already collapses values
/// within 0.05 dB of unity to `+0.0 dB` so a `-0.0` never prints.
pub fn format_gain_db(gain_db: f32) -> String {
    crate::util::format_db_signed(gain_db, true)
}

/// Sample-overlap of two clips (in samples), or `None` if they don't
/// overlap. Used to derive the automatic crossfade region.
pub fn overlap_range(
    a_start: u64,
    a_dur: u64,
    b_start: u64,
    b_dur: u64,
) -> Option<(u64, u64)> {
    let a_end = a_start.saturating_add(a_dur);
    let b_end = b_start.saturating_add(b_dur);
    let start = a_start.max(b_start);
    let end = a_end.min(b_end);
    if end > start {
        Some((start, end))
    } else {
        None
    }
}

/// Screen-space points tracing a fade ramp envelope across `[x0, x0+rw]`.
/// `fade_in` runs silence→unity left→right; otherwise unity→silence. The
/// curve's [`FadeCurve::coefficient`] maps progress to amplitude, and
/// amplitude maps to height (unity at the clip top, silence at the
/// bottom), so the polyline is the ramp line and the area above it is the
/// attenuated wedge.
pub fn fade_envelope(
    curve: resonance_audio::types::FadeCurve,
    x0: f32,
    rw: f32,
    top_y: f32,
    h: f32,
    fade_in: bool,
) -> Vec<Point> {
    const SEGMENTS: usize = 16;
    (0..=SEGMENTS)
        .map(|i| {
            let t = i as f32 / SEGMENTS as f32;
            let amp = if fade_in {
                curve.coefficient(t)
            } else {
                curve.coefficient(1.0 - t)
            };
            Point::new(x0 + t * rw, top_y + h * (1.0 - amp))
        })
        .collect()
}

/// Polygon for the darkened part of a fade ramp: the region between the
/// clip's top edge (`[x0, x0+rw]`) and the envelope below it.
pub(super) fn fade_wedge_path(env: &[Point], x0: f32, rw: f32, top_y: f32) -> canvas::Path {
    canvas::Path::new(|b| {
        if let Some(first) = env.first() {
            b.move_to(*first);
            for p in &env[1..] {
                b.line_to(*p);
            }
            b.line_to(Point::new(x0 + rw, top_y));
            b.line_to(Point::new(x0, top_y));
            b.close();
        }
    })
}

/// Stroke a polyline through `pts`.
pub(super) fn stroke_polyline(frame: &mut canvas::Frame, pts: &[Point], color: Color, width: f32) {
    if pts.len() < 2 {
        return;
    }
    let path = canvas::Path::new(|b| {
        b.move_to(pts[0]);
        for p in &pts[1..] {
            b.line_to(*p);
        }
    });
    frame.stroke(
        &path,
        canvas::Stroke::default().with_color(color).with_width(width),
    );
}

/// A circular handle bead centred at `(cx, cy)`: a filled disc in `color`
/// with a thin dark ring for contrast against the clip wash.
pub(super) fn draw_bead(frame: &mut canvas::Frame, cx: f32, cy: f32, color: Color) {
    let disc = canvas::Path::circle(Point::new(cx, cy), BEAD_RADIUS);
    frame.fill(&disc, color);
    frame.stroke(
        &disc,
        canvas::Stroke::default()
            .with_color(Color {
                a: 0.9,
                ..theme::BG_1
            })
            .with_width(1.0),
    );
}

/// Diagonal hatch over an "unsupported" (frozen / rendered) clip — the
/// degradation surface for clips with no editable sample source. Each
/// 45° line is clamped to the clip rect by hand (a nested `with_clip`
/// renders nothing inside the cached timeline frame), so the strokes
/// never bleed past the body.
pub(super) fn draw_clip_hatch(frame: &mut canvas::Frame, x: f32, y: f32, w: f32, h: f32) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let color = Color {
        a: 0.16,
        ..theme::TEXT_1
    };
    const SPACING: f32 = 7.0;
    // Each line runs from the bottom edge at `sx` up to the top edge at
    // `sx + h` (slope -1): point(t) = (sx + t·h, (y+h) − t·h), t ∈ [0, 1].
    // Clamp t so x stays within [x, x+w]; y then stays within [y, y+h].
    let mut sx = x - h;
    while sx < x + w {
        let t_lo = ((x - sx) / h).clamp(0.0, 1.0);
        let t_hi = ((x + w - sx) / h).clamp(0.0, 1.0);
        if t_hi > t_lo {
            let p = |t: f32| Point::new(sx + t * h, (y + h) - t * h);
            let line = canvas::Path::new(|b| {
                b.move_to(p(t_lo));
                b.line_to(p(t_hi));
            });
            frame.stroke(
                &line,
                canvas::Stroke::default().with_color(color).with_width(1.0),
            );
        }
        sx += SPACING;
    }
}

/// The mono `±N.N dB` gain tag in the clip header, right-aligned. Hidden
/// at unity and on clips too narrow to fit it, so untouched clips stay
/// clean (design #153).
pub(super) fn draw_clip_gain_tag(
    frame: &mut canvas::Frame,
    clip: &ClipState,
    x: f32,
    y: f32,
    w: f32,
) {
    if clip.gain_db.abs() <= 0.05 || w < 64.0 {
        return;
    }
    frame.fill_text(canvas::Text {
        content: format_gain_db(clip.gain_db),
        position: Point::new(x + w - 7.0, y + 4.0),
        color: theme::TEXT_2,
        size: 9.5.into(),
        font: theme::MONO_FONT,
        align_x: TextAlignment::Right,
        ..canvas::Text::default()
    });
}

/// The `⤬` crossfade badge: a small lavender disc with a white cross,
/// centred at `(cx, cy)`. Drawn with strokes rather than a glyph so it
/// renders identically regardless of font coverage.
pub(super) fn draw_crossfade_badge(frame: &mut canvas::Frame, cx: f32, cy: f32) {
    const R: f32 = 7.0;
    let disc = canvas::Path::circle(Point::new(cx, cy), R);
    frame.fill(&disc, theme::ACCENT);
    let arm = R * 0.5;
    let cross = canvas::Path::new(|b| {
        b.move_to(Point::new(cx - arm, cy - arm));
        b.line_to(Point::new(cx + arm, cy + arm));
        b.move_to(Point::new(cx + arm, cy - arm));
        b.line_to(Point::new(cx - arm, cy + arm));
    });
    frame.stroke(
        &cross,
        canvas::Stroke::default()
            .with_color(theme::TEXT_1)
            .with_width(1.5),
    );
}
