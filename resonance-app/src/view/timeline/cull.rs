//! Horizontal draw-culling for the timeline's cached pass.
//!
//! The arrange canvas is sized to the whole song (`view_timeline` wraps
//! it in a horizontal `Scrollable`), so before this module every cache
//! invalidation — vertical scroll, zoom, any edit — re-tessellated every
//! waveform and note minimap in the project. The cached pass now draws
//! only the clips whose pixel span may intersect a *quantized* window
//! around the visible viewport, and that window is part of the cache
//! fingerprint.
//!
//! Correctness argument (why nothing blank can ever scroll into view):
//! the visible viewport is captured by
//! [`viewport_probe::ViewportProbe`](super::viewport_probe::ViewportProbe)
//! immediately before every `Widget::draw`, and the fingerprint —
//! including the quantized window derived from it — is re-checked inside
//! the very same frame's `Program::draw`. A frame whose viewport left
//! the previously drawn window therefore invalidates and redraws with
//! the new window *before* it is presented. The quantization and margin
//! never carry correctness; they only add hysteresis so that small
//! scrolls keep hitting the cache instead of re-tessellating per pixel.

/// Symmetric slack, in pixels, added around a clip's body span when
/// testing it against the cull window. Absorbs everything drawn slightly
/// outside the raw span: the per-track group indent
/// (`indent_depth * GROUP_MEMBER_INDENT`, 14 px per level), the fade /
/// gain beads that overhang the corners by a few pixels, and f32
/// rounding differences between the span helper and the draw routines.
pub const SPAN_PAD: f32 = 128.0;

/// Minimum quantization step, so a tiny viewport (early layout passes
/// report near-zero sizes) can't degenerate into per-pixel invalidation.
const MIN_STEP: f32 = 256.0;

/// Quantize the visible rectangle of the canvas (canvas-local
/// coordinates) into the horizontal window the cached pass draws:
/// half-viewport steps with a margin of one step on both sides. The
/// result always covers the visible range — `x0 ≤ visible.x` and
/// `x1 ≥ visible.x + visible.width` — and is identical for any two
/// viewports whose edges fall in the same quantization cells, which is
/// what lets a scroll travel up to half a viewport before the cache is
/// invalidated at all.
pub fn quantized_window(visible: iced::Rectangle) -> (f32, f32) {
    let step = (visible.width * 0.5).max(MIN_STEP);
    let x0 = (((visible.x - step) / step).floor() * step).max(0.0);
    let x1 = ((visible.x + visible.width + step) / step).ceil() * step;
    (x0, x1)
}

/// Whether a clip whose body occupies `span` (`(x_start, x_end)` in
/// canvas pixels, [`SPAN_PAD`] slack applied here) may intersect the
/// cull window. `None` — no window known — draws everything, which is
/// always correct. This exact predicate gates both the draw pass and
/// nothing else: hashing stays unconditional, so an edit to a culled
/// clip still flips the fingerprint and repaints (cheaply, without it).
pub fn span_may_be_visible(span: (f32, f32), window: Option<(f32, f32)>) -> bool {
    let Some((w0, w1)) = window else {
        return true;
    };
    span.1 + SPAN_PAD >= w0 && span.0 - SPAN_PAD <= w1
}

/// Pixel span of a `[start_sample, end_sample)` range at the given zoom
/// in canvas content coordinates (x=0 is sample 0) — the same mapping `sample_to_x` uses, as
/// a free function so the cull tests can drive it without a canvas.
pub fn sample_span_x(
    start_sample: u64,
    end_sample: u64,
    sample_rate: u32,
    zoom: f32,
) -> (f32, f32) {
    let to_x = |sample: u64| {
        (sample as f64 / sample_rate as f64) as f32 * zoom
    };
    (to_x(start_sample), to_x(end_sample))
}

/// The clip-local pixel range `[lo, hi]` of a clip body at canvas x `x`,
/// width `w`, that may intersect the cull window (with [`SPAN_PAD`]
/// slack). Per-column work — waveform bars, the frozen silhouette, note
/// rects — outside it is off screen and is skipped, so a long clip only
/// a sliver of which is visible no longer tessellates its full width on
/// every cache miss. `None` — no window — is the whole clip.
pub fn clip_px_range(x: f32, w: f32, window: Option<(f32, f32)>) -> (f32, f32) {
    match window {
        None => (0.0, w),
        Some((w0, w1)) => ((w0 - SPAN_PAD - x).max(0.0), (w1 + SPAN_PAD - x).min(w)),
    }
}

/// Answers "does any note sound at this tick?" for non-decreasing ticks
/// in O(1) amortised, replacing a per-column scan over every note. Built
/// from `(start, end)` half-open tick intervals in any order: a tick is
/// covered iff the largest `end` among the intervals with `start <= tick`
/// exceeds it — exactly `any(|(s, e)| s <= tick && tick < e)`.
pub struct CoverageSweep {
    intervals: Vec<(f32, f32)>,
    next: usize,
    max_end: f32,
}

impl CoverageSweep {
    pub fn new(mut intervals: Vec<(f32, f32)>) -> Self {
        intervals.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self {
            intervals,
            next: 0,
            max_end: f32::NEG_INFINITY,
        }
    }

    /// Whether `tick` is covered. Ticks must be passed in non-decreasing
    /// order.
    pub fn covers(&mut self, tick: f32) -> bool {
        while let Some(&(start, end)) = self.intervals.get(self.next) {
            if start > tick {
                break;
            }
            self.max_end = self.max_end.max(end);
            self.next += 1;
        }
        tick < self.max_end
    }
}

/// Index pairs `(i, j)`, `i < j`, of same-track clips whose
/// `[start, end)` sample ranges may overlap, in the `(i, j)` order a
/// nested `for i { for j > i }` walk visits them. `clips` is `(track,
/// start, end)` per clip. Grouped per track and swept in start order, so
/// the cost is O(n log n + pairs) instead of O(n²). Callers still apply
/// the exact overlap test to each pair.
pub fn overlapping_pairs(clips: &[(u64, u64, u64)]) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..clips.len()).collect();
    order.sort_by_key(|&i| (clips[i].0, clips[i].1, i));
    let mut pairs = Vec::new();
    for (p, &i) in order.iter().enumerate() {
        let (track, _, end) = clips[i];
        for &j in &order[p + 1..] {
            let (other_track, other_start, _) = clips[j];
            if other_track != track || other_start >= end {
                break;
            }
            pairs.push((i.min(j), i.max(j)));
        }
    }
    pairs.sort_unstable();
    pairs
}
