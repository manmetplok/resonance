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
/// Resolution order (design doc #165): **active take → comp segment →
/// latest take**. The last tier matters: a group whose comp is still empty
/// is not silent, it plays its newest pass, and a lane that drew that as
/// "nothing promoted" would be lying about what the user hears. Spans carry
/// their [`CoverSource`] so the ribbon can distinguish a deliberate
/// promotion from that fallback.
///
/// # PROVISIONAL — superseded by todo #1395
///
/// This is **not** currently a mirror of the engine, and the difference is
/// visible the moment a comp has any segment at all. Todo #409's
/// `take_comp::resolve_spans` reaches its latest-take tier only when
/// `comp.segments` is *completely empty*; with one segment present it emits
/// that segment alone and plays **silence** either side of it. This
/// function fills those gaps unconditionally. "Latest" also differs: the
/// engine takes the last *audio* take in vector order, this takes
/// `max_by_key((captured_at, pass_index, id))` across all takes.
///
/// The user has ruled on the semantics, and it is the behaviour here that
/// ships: **the latest take fills the gaps** — promote one phrase and you
/// hear the most recent pass everywhere else, so a comp is a complete part
/// from its first gesture rather than a hole with one island in it. The
/// engine is what changes.
///
/// Todo **#1395** lands that as a single shared definition in
/// `resonance-common` (a `Comp::promote` that seeds a full cover, one
/// `latest_take`, one cover resolution) which both this lane and the mixer
/// call. Until then this function is the app's own reading — do not "fix"
/// it towards `resolve_spans`, and do not build a second copy of the
/// tiering anywhere else.
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
    // recent pass. `captured_at` is the primary key (it is what "latest"
    // means), with `pass_index` and id as deterministic tie-breaks for
    // takes captured inside the same millisecond. See the PROVISIONAL note
    // above — the engine picks its latest differently, and #1395 collapses
    // the two into one shared definition.
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
