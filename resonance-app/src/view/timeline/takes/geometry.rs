//! Pure take-lane geometry: no canvas, no `self`, no iced widgets — so it
//! is unit-testable on its own and shared by the draw pass (todo #413) and
//! the comping gestures that land on top of it (todo #414).
//!
//! What the lane *draws over the slot* is not decided here. A
//! [`TakeGroup`](resonance_common::TakeGroup)'s
//! [`Comp`](resonance_common::Comp) may be empty or partial, but something is
//! always audible over the slot, and the lane must show what the mixer will
//! actually play. Since todo #1395 that resolution has one implementation,
//! [`resonance_common::effective_cover`], which the mixer's
//! `take_comp::resolve_spans` reads too — so the lane and the engine cannot
//! drift apart. It is re-exported below rather than adapted: the
//! [`CoverSource`] tiering the lane needs (a deliberate promotion must not
//! draw like a fallback) is part of that shared definition.
//!
//! Everything else here — the audible extent, the bands, the unlit and silent
//! remainders — is view geometry proper and stays.

use resonance_common::{TakeId, TimelineRange};

use crate::theme;

/// The one definition of what a take group plays over its slot — **active
/// take → comp segment → latest take** (design doc #165, todo #1395) — shared
/// verbatim with the mixer. Spans carry a [`CoverSource`] so the ribbon can
/// distinguish a promotion the user made from the latest-take fallback that
/// fills the gaps around it.
pub use resonance_common::{effective_cover, CoverSource, CoverSpan};

/// The sub-ranges of `slot` that `take_id` is **not** audible over, given
/// `cover` — i.e. where the take row should be scrimmed back. The
/// complement of the take's own spans within the slot.
pub fn unlit_ranges(cover: &[CoverSpan], slot: TimelineRange, take_id: TakeId) -> Vec<TimelineRange> {
    let mut out = Vec::new();
    let mut cursor = slot.start;
    for span in cover.iter().filter(|s| s.take_id == take_id) {
        if span.range.start > cursor {
            out.push(TimelineRange::from_bounds(cursor, span.range.start));
        }
        cursor = cursor.max(span.range.end());
    }
    if cursor < slot.end() {
        out.push(TimelineRange::from_bounds(cursor, slot.end()));
    }
    out
}

/// Where a take actually carries audio inside its group's `slot`: the
/// intersection of the slot with its recorded clip's extent, or `None` when
/// the two do not overlap at all.
///
/// A take does **not** necessarily fill its lane. `finalize_loop_record_pass`
/// starts pass 0's clip at the punch-in point rather than at the loop start,
/// and any pass cut short at stop ends before the slot does — so the clip's
/// `[start, start + length)` can sit strictly inside the slot at either end.
/// Todo #409 made the engine intersect exactly this way before reading a
/// take (`take_comp::mix_track_comp`); the lane has to draw the same
/// intersection or a punched-in take renders as a full-width, stretched
/// waveform that claims audio where the engine plays none.
///
/// `clip_start` / `clip_len` are the clip's *audible* (post-trim) extent —
/// `ClipState::start_sample` and `ClipState::duration_samples`.
pub fn audible_extent(
    slot: TimelineRange,
    clip_start: u64,
    clip_len: u64,
) -> Option<TimelineRange> {
    let start = slot.start.max(clip_start);
    let end = slot.end().min(clip_start.saturating_add(clip_len));
    (end > start).then(|| TimelineRange::from_bounds(start, end))
}

/// The sub-ranges of `slot` that `audible` leaves uncovered — where a take
/// row shows the "no audio here" flat line instead of a waveform. `None`
/// means the take carries nothing anywhere in the slot, so the whole slot is
/// the remainder.
pub fn silent_ranges(slot: TimelineRange, audible: Option<TimelineRange>) -> Vec<TimelineRange> {
    let Some(audible) = audible else {
        return if slot.is_empty() { Vec::new() } else { vec![slot] };
    };
    let mut out = Vec::new();
    if audible.start > slot.start {
        out.push(TimelineRange::from_bounds(slot.start, audible.start));
    }
    if audible.end() < slot.end() {
        out.push(TimelineRange::from_bounds(audible.end(), slot.end()));
    }
    out
}

/// The comp ribbon's `(top, height)` inside a track lane whose row runs
/// from `row_y` for `row_height` px. The ribbon hugs the bottom edge, one
/// hairline clear of the row separator, so it never collides with the clip
/// cards above it (which are inset by `CLIP_LANE_INSET`).
pub fn comp_ribbon_band(row_y: f32, row_height: f32) -> (f32, f32) {
    let height = theme::TAKE_COMP_RIBBON_HEIGHT.min((row_height - 4.0).max(1.0));
    (row_y + row_height - height - 3.0, height)
}

/// The take card's `(top, height)` inside a `TAKE_ROW_HEIGHT` take sub-row
/// running from `row_y`. Symmetric inset, minus the row's bottom hairline.
pub fn take_card_band(row_y: f32, row_height: f32) -> (f32, f32) {
    let height = (row_height - 2.0 * theme::TAKE_ROW_INSET - 1.0).max(1.0);
    (row_y + theme::TAKE_ROW_INSET, height)
}

/// Display label for a take: `T1`, `T2`, … from its zero-based
/// `pass_index`. One-based because musicians count takes from one.
pub fn take_label(pass_index: u32) -> String {
    format!("T{}", pass_index.saturating_add(1))
}
