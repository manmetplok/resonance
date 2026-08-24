//! Pure take-lane geometry: no canvas, no `self`, no iced widgets — so it
//! is unit-testable on its own and shared by the draw pass (todo #413) and
//! the comping gestures that land on top of it (todo #414).
//!
//! The one non-obvious piece here is [`effective_cover`]. A [`TakeGroup`]'s
//! [`Comp`] is allowed to be empty or partial, but *something* is always
//! audible over the slot, so the lane must draw what the engine will
//! actually play rather than only what the user explicitly promoted.

use resonance_common::{Comp, TakeGroup, TakeId, TimelineRange};

use crate::theme;

/// How a span of the slot came to be covered by its take. The lane draws
/// the two differently so "I chose this" never reads the same as "this is
/// what you get by default".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverSource {
    /// The take is soloed for the whole slot (`TakeGroup::active_take`),
    /// overriding the comp entirely.
    ActiveTake,
    /// An explicit [`CompSegment`](resonance_common::CompSegment) the user
    /// promoted.
    CompSegment,
    /// Nothing covers this span, so the engine falls back to the group's
    /// most recent take.
    LatestFallback,
}

/// One span of the slot plus the take audible over it and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverSpan {
    pub range: TimelineRange,
    pub take_id: TakeId,
    pub source: CoverSource,
}

/// The take that plays over each part of `group`'s slot — an ordered,
/// gap-free cover, so every point of the slot maps to exactly one take.
///
/// Mirrors the engine's resolution order (design doc #165 / todo #409's
/// `build_comp_table`): **active take → comp segment → latest take**. The
/// fallback matters: a group whose comp is still empty is not silent, it
/// plays its newest pass, and a lane that drew it as "nothing promoted"
/// would be lying about what the user hears. Spans carry their
/// [`CoverSource`] so the ribbon can distinguish a deliberate promotion
/// from that fallback.
///
/// Returns an empty vector for an empty slot or a group with no takes.
/// Segments outside the slot are ignored, and a segment overhanging an edge
/// is clamped to it.
pub fn effective_cover(group: &TakeGroup) -> Vec<CoverSpan> {
    if group.slot.is_empty() || group.takes.is_empty() {
        return Vec::new();
    }

    // The engine's whole-slot solo override wins outright.
    if let Some(active) = group.active_take {
        if group.take(active).is_some() {
            return vec![CoverSpan {
                range: group.slot,
                take_id: active,
                source: CoverSource::ActiveTake,
            }];
        }
    }

    // Fallback take for anything the comp leaves uncovered: the most
    // recent pass, matching the engine's "latest take" tier. `captured_at`
    // is the primary key (it is what "latest" means), with `pass_index`
    // and id as deterministic tie-breaks for takes captured inside the same
    // millisecond.
    let Some(latest) = group
        .takes
        .iter()
        .max_by_key(|t| (t.captured_at, t.pass_index, t.id))
        .map(|t| t.id)
    else {
        return Vec::new();
    };

    let mut spans: Vec<CoverSpan> = Vec::new();
    let mut cursor = group.slot.start;
    for seg in sorted_segments(&group.comp) {
        // Skip segments that name a take this group doesn't hold, and
        // anything left of the cursor (the model keeps segments
        // non-overlapping, but a stale mirror must not produce a
        // backwards span).
        if group.take(seg.take_id).is_none() {
            continue;
        }
        let start = seg.range.start.max(cursor);
        let end = seg.range.end().min(group.slot.end());
        if end <= start {
            continue;
        }
        if start > cursor {
            spans.push(CoverSpan {
                range: TimelineRange::from_bounds(cursor, start),
                take_id: latest,
                source: CoverSource::LatestFallback,
            });
        }
        spans.push(CoverSpan {
            range: TimelineRange::from_bounds(start, end),
            take_id: seg.take_id,
            source: CoverSource::CompSegment,
        });
        cursor = end;
    }
    if cursor < group.slot.end() {
        spans.push(CoverSpan {
            range: TimelineRange::from_bounds(cursor, group.slot.end()),
            take_id: latest,
            source: CoverSource::LatestFallback,
        });
    }
    spans
}

/// `comp.segments` sorted ascending by start. The model's helpers already
/// maintain that order, but the mirror adopts whatever the engine echoes
/// wholesale (`TakeGroupState::comp_changed`), so the draw path sorts
/// defensively rather than trusting it.
fn sorted_segments(comp: &Comp) -> Vec<resonance_common::CompSegment> {
    let mut segments = comp.segments.clone();
    segments.sort_by_key(|s| s.range.start);
    segments
}

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
