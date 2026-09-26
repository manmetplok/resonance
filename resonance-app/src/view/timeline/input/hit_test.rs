//! Geometry helpers and canvas hit-testing methods for the timeline canvas.
//! These pure-query functions map pixel positions to logical timeline
//! elements (clips, markers, scrollbars) without mutating any state.

use iced::{Point, Rectangle};

use resonance_audio::types::TrackId;

use crate::state;
use crate::theme;
use crate::view::arrange_layout::ArrangeRowLayout;
use super::super::hit_test::{self, ClipHandles, HitKind, MarkerHit};
use super::super::scrollbar::ScrollbarRects;
use super::super::snap::snap_sample_to_grid_tempo;
use super::super::TimelineCanvas;

impl TimelineCanvas<'_> {
    /// The vertical scrollbar's rects, `None` when the lanes fit. There
    /// is no in-canvas horizontal bar: horizontal scroll is owned by the
    /// outer `Scrollable` that wraps the canvas (see `view_timeline`).
    /// The vertical bar stays — tracks scroll inside the canvas so the
    /// ruler / section band / global-tracks header line up with their
    /// lanes. It hugs the right edge of the part of the canvas that is on
    /// screen (the [`ViewportProbe`](crate::view::timeline::viewport_probe)
    /// window), not the song's end; with no probe write yet it falls back
    /// to the canvas edge (review VIEW-33).
    pub(in crate::view::timeline) fn scrollbar_rects(
        &self,
        bounds: Rectangle,
    ) -> Option<ScrollbarRects> {
        use crate::view::timeline::scrollbar;
        let content_h = self.content_height_px();
        let header_h = self.fixed_header_height();
        let right_edge = self
            .visible_viewport
            .get()
            .map_or(bounds.width, |v| v.x + v.width);
        scrollbar::v_rects(bounds, right_edge, content_h, self.scroll_offset_y, header_h)
    }

    /// Test-only: [`scrollbar_rects`](Self::scrollbar_rects) for
    /// `test_support` (VIEW-33).
    #[doc(hidden)]
    pub(crate) fn test_scrollbar_rects(&self, bounds: Rectangle) -> Option<ScrollbarRects> {
        self.scrollbar_rects(bounds)
    }

    /// Hit-test a pointer press against a clip lane (MIDI or audio).
    /// `duration_samples` is already tick-converted for MIDI clips.
    ///
    /// The lane Y / height come from the shared [`ArrangeRowLayout`]
    /// (`layout`), so the hit honours the variable 60/96 px row pitch and
    /// returns `None` for a clip whose track is hidden (a collapsed
    /// group's member has no visible lane → `track_row_rect` is `None`).
    pub(in crate::view::timeline::input) fn hit_test_lane(
        &self,
        pos: Point,
        layout: &ArrangeRowLayout,
        clip_track_id: TrackId,
        clip_start_sample: u64,
        duration_samples: u64,
    ) -> Option<HitKind> {
        let header_height = self.fixed_header_height();
        let (body_y, body_height, indent) = super::super::draw::clip_lane_rect(
            self,
            clip_track_id,
            layout,
            header_height,
            self.scroll_offset_y,
            // Hit-testing should not cull on viewport height: the pointer
            // that produced `pos` is by definition on-screen. Pass an
            // unbounded visible height so an off-by-a-pixel partial row at
            // the bottom edge still resolves.
            f32::INFINITY,
        )?;
        // `clip_lane_rect` returns the clip *body* rect (already inset +
        // indented); `clip_pixel_rect` just adds the horizontal extent so
        // the hit rect matches the drawn body exactly.
        let rect = hit_test::clip_pixel_rect(
            hit_test::ClipLaneBody {
                y: body_y,
                height: body_height,
                indent,
            },
            clip_start_sample,
            duration_samples,
            self.zoom,
            self.sample_rate,
            self.scroll_offset,
        );
        match hit_test::hit_test(pos, rect, theme::CLIP_EDGE_THRESHOLD) {
            HitKind::Miss => None,
            hit => Some(hit),
        }
    }

    /// Hit-test a pointer against an audio clip, including its fade/gain
    /// handle beads. Returns `None` on a miss so callers can fall through to
    /// the next clip. Lane geometry comes from the shared
    /// [`ArrangeRowLayout`] (see [`hit_test_lane`](Self::hit_test_lane)).
    pub(in crate::view::timeline::input) fn hit_test_audio_lane(
        &self,
        pos: Point,
        layout: &ArrangeRowLayout,
        clip: &state::ClipState,
    ) -> Option<HitKind> {
        let header_height = self.fixed_header_height();
        let (body_y, body_height, indent) = super::super::draw::clip_lane_rect(
            self,
            clip.track_id,
            layout,
            header_height,
            self.scroll_offset_y,
            f32::INFINITY,
        )?;
        let rect = hit_test::clip_pixel_rect(
            hit_test::ClipLaneBody {
                y: body_y,
                height: body_height,
                indent,
            },
            clip.start_sample,
            clip.duration_samples,
            self.zoom,
            self.sample_rate,
            self.scroll_offset,
        );
        let handles = self.clip_handles(clip, rect);
        match hit_test::hit_test_audio(pos, rect, theme::CLIP_EDGE_THRESHOLD, &handles) {
            HitKind::Miss => None,
            hit => Some(hit),
        }
    }

    /// Build the fade/gain handle geometry for an audio clip.
    ///
    /// The per-clip fade lengths and the `fadeable` (frozen / no-source)
    /// flag live on `ClipState` once todo #316 mirrors them in from the
    /// engine. Until then fades read as 0 (handles sit at the top corners,
    /// discoverable on hover) and every audio clip is treated as fadeable;
    /// the gain bead is always present. Swap these reads for the real
    /// `ClipState` fields when #316 lands — the geometry math is unchanged.
    fn clip_handles(&self, _clip: &state::ClipState, rect: Rectangle) -> ClipHandles {
        let fade_in_seconds = 0.0;
        let fade_out_seconds = 0.0;
        let fadeable = true;
        hit_test::audio_clip_handles(rect, fade_in_seconds, fade_out_seconds, self.zoom, fadeable)
    }

    /// Map a pixel x-position to an absolute sample position (clamped at 0).
    /// The inverse of [`Self::sample_to_x`], used by the marker drag handlers.
    pub(in crate::view::timeline::input) fn x_to_sample(&self, x: f32) -> u64 {
        let seconds = ((x + self.scroll_offset) / self.zoom).max(0.0);
        (seconds as f64 * self.sample_rate as f64) as u64
    }

    /// Snap a raw sample position to the grid using the live tempo / zoom —
    /// the same helper the transport loop-drag, clip, and marker reducers
    /// use, so a dragged region edge lands on the same lines as everything
    /// else.
    pub(in crate::view::timeline::input) fn snap_sample(&self, sample: u64) -> u64 {
        snap_sample_to_grid_tempo(
            sample,
            self.bpm,
            self.time_sig_num,
            self.sample_rate,
            self.zoom,
            self.tempo_map,
        )
    }

    /// Map a pixel x-position to a bar number using the tempo map.
    pub(in crate::view::timeline::input) fn x_to_bar(&self, x: f32) -> u32 {
        let seconds = ((x + self.scroll_offset) / self.zoom).max(0.0);
        let sample = (seconds as f64 * self.sample_rate as f64) as u64;
        let (bar, frac) = self.tempo_map.sample_to_bar(sample, self.sample_rate);
        if frac >= 0.5 {
            bar + 1
        } else {
            bar
        }
    }

    /// Hit-test a pointer against the arrangement markers in the ruler band.
    /// Returns the topmost (last-drawn) marker under the pointer and which
    /// handle was grabbed, or `None` when the pointer misses every marker or
    /// sits below the ruler. Iterates in reverse so a later marker painted on
    /// top of an earlier one wins, matching the clip hit-test convention.
    pub(in crate::view::timeline::input) fn marker_at(
        &self,
        pos: Point,
    ) -> Option<(u64, MarkerHit)> {
        if pos.y >= theme::RULER_HEIGHT {
            return None;
        }
        for marker in self.markers.iter().rev() {
            let start_x = self.sample_to_x(marker.start_sample);
            let end_x = marker.end_sample.map(|e| self.sample_to_x(e));
            if let Some(hit) = hit_test::marker_hit(pos.x, start_x, end_x) {
                return Some((marker.id, hit));
            }
        }
        None
    }
}
