//! Take-lane hit-testing (epic #15, doc #165, todo #414).
//!
//! Turns a pointer position into the comping verb it names. Three regions
//! carry a gesture, and each resolves through the *same* helpers the draw
//! pass uses — [`ArrangeRowLayout`] for the row, `take_card_band` /
//! `comp_ribbon_band` for the band, `slot_rect` for the x extent — so the
//! pointer can only ever hit what the user sees drawn (the #732 rule).
//!
//! # Promote targets the SLOT, not the card
//!
//! This is the trap todo #413 left signposted. A take **card** is drawn
//! over the take's *audible extent* — a pass that punched in a second late
//! gets a shorter card — while the comp addresses the whole **slot**, and
//! promoting a take across a stretch it never recorded is a legal (if
//! disclosed) state. So [`TakeCardHit`] carries the group's `slot`, the x
//! hit region is the slot's pixel span, and a press in the "silent" part
//! of a take row is a hit, not a miss. Pinning the hit region to the drawn
//! card instead would make the lead-in of every punched-in take
//! un-comp-able.
//!
//! # This module decides nothing
//!
//! It resolves geometry and nothing else. Whether an edit *lands* is
//! `update::takes::plan`'s call, gated pre-dispatch by
//! `Resonance::take_edit_is_refused` — and a refused edit is dropped
//! silently, on purpose, so it spends no undo entry (ba doc #292). The
//! consequence for this layer is that the UI can neither pre-validate nor
//! read back "did that work?" from the history: it has to *describe the
//! gesture before the release* instead. [`split_has_a_cut_point`] is the
//! one predicate here, and it is deliberately only the positional
//! precondition the affordance needs ("is there a cut point at all"), not
//! a second copy of the refusal rule.
//!
//! [`split_has_a_cut_point`]: TimelineCanvas::split_has_a_cut_point
//! [`ArrangeRowLayout`]: crate::view::arrange_layout::ArrangeRowLayout

use iced::Point;

use resonance_audio::types::TrackId;
use resonance_common::{TakeGroupId, TakeId, TimelineRange};

use crate::view::arrange_layout::{ArrangeRowKind, ArrangeRowLayout};

use super::super::TimelineCanvas;
use super::geometry::{comp_ribbon_band, take_card_band};

/// How far the pointer must travel before a press on a take card reads as
/// a *promote drag* rather than a *click to solo*. Matches the slack the
/// rest of the canvas allows on a click — small enough that a deliberate
/// sweep is never mistaken for a click, large enough that a shaky click on
/// a 29 px card never promotes a 3 ms sliver.
pub(crate) const TAKE_DRAG_SLOP_PX: f32 = 4.0;

/// A pointer hit on one take's card inside an expanded take lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TakeCardHit {
    pub track_id: TrackId,
    pub group_id: TakeGroupId,
    pub take_id: TakeId,
    /// The group's **slot** — the region a comp edit addresses, and the x
    /// extent of this hit. Deliberately not the take's audible extent: see
    /// the module docs.
    pub slot: TimelineRange,
}

/// A pointer hit on a group's comp ribbon, the summary strip along the
/// bottom of the track's own lane. Present whether the take lane is folded
/// or open, so the split gesture never needs the stack unfolded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CompRibbonHit {
    pub group_id: TakeGroupId,
    pub slot: TimelineRange,
    pub band_top: f32,
    pub band_height: f32,
}

impl TimelineCanvas<'_> {
    /// The take sub-row under a canvas-space `y`, as
    /// `(track, group, take, row_y, row_height)` in screen space.
    fn take_row_at(
        &self,
        layout: &ArrangeRowLayout,
        y: f32,
    ) -> Option<(TrackId, TakeGroupId, TakeId, f32, f32)> {
        let header_height = self.fixed_header_height();
        let y_off = self.scroll_offset_y;
        let row = layout.row_at_y(y - header_height + y_off)?;
        let ArrangeRowKind::TakeRow { track, group, take } = row.kind else {
            return None;
        };
        Some((
            track,
            group,
            take,
            header_height + row.y_top - y_off,
            row.height,
        ))
    }

    /// The take card under `pos`, resolved against a layout the caller has
    /// already built. The draw pass builds one per frame; re-deriving it
    /// per hit test would allocate the whole row list again.
    pub(in crate::view::timeline) fn take_card_at_in(
        &self,
        layout: &ArrangeRowLayout,
        pos: Point,
    ) -> Option<TakeCardHit> {
        if self.take_groups.groups.is_empty() {
            return None;
        }
        let (track_id, group_id, take_id, row_y, row_height) = self.take_row_at(layout, pos.y)?;
        // The card band, not the whole 38 px row: the few pixels of row
        // chrome above and below stay inert, so a press there falls
        // through to selecting the owning track exactly as it did before
        // this todo.
        let (card_top, card_height) = take_card_band(row_y, row_height);
        if pos.y < card_top || pos.y > card_top + card_height {
            return None;
        }
        let group = self.take_groups.group(group_id)?;
        // The SLOT's pixel span — the region the comp addresses — not the
        // card's. `f32::INFINITY` because the row was already resolved as
        // visible; the horizontal viewport test belongs to the draw pass.
        let (x, w) = self.slot_rect(group.slot, f32::INFINITY)?;
        (pos.x >= x && pos.x <= x + w).then_some(TakeCardHit {
            track_id,
            group_id,
            take_id,
            slot: group.slot,
        })
    }

    /// The take card under `pos`. Convenience wrapper for the input path,
    /// which has no layout in hand.
    pub(crate) fn take_card_at(&self, pos: Point) -> Option<TakeCardHit> {
        if self.take_groups.groups.is_empty() {
            return None;
        }
        self.take_card_at_in(&self.arrange_layout(), pos)
    }

    /// The comp ribbon under `pos`, resolved against an existing layout.
    ///
    /// The ribbon rides the *track's* own lane, overlapping the bottom of
    /// the clip body (the ribbon band starts `TAKE_COMP_RIBBON_HEIGHT + 3`
    /// px above the row's bottom edge, `CLIP_LANE_INSET` is 10), and the
    /// draw pass paints it **after** the clips. The press handler must
    /// therefore consult this *before* clip hit-testing or the two would
    /// disagree about which one is on top.
    pub(in crate::view::timeline) fn comp_ribbon_at_in(
        &self,
        layout: &ArrangeRowLayout,
        pos: Point,
    ) -> Option<CompRibbonHit> {
        if self.take_groups.groups.is_empty() {
            return None;
        }
        let header_height = self.fixed_header_height();
        let y_off = self.scroll_offset_y;
        let row = layout.row_at_y(pos.y - header_height + y_off)?;
        let ArrangeRowKind::Track(track_id) = row.kind else {
            return None;
        };
        let row_y = header_height + row.y_top - y_off;
        let (band_top, band_height) = comp_ribbon_band(row_y, row.height);
        if pos.y < band_top || pos.y > band_top + band_height {
            return None;
        }
        // A track can own several groups (one per loop region it was
        // recorded over); the x picks which.
        self.take_groups
            .groups
            .iter()
            .filter(|g| g.track_id == track_id)
            .find_map(|group| {
                let (x, w) = self.slot_rect(group.slot, f32::INFINITY)?;
                (pos.x >= x && pos.x <= x + w).then_some(CompRibbonHit {
                    group_id: group.id,
                    slot: group.slot,
                    band_top,
                    band_height,
                })
            })
    }

    /// The comp ribbon under `pos`.
    pub(crate) fn comp_ribbon_at(&self, pos: Point) -> Option<CompRibbonHit> {
        if self.take_groups.groups.is_empty() {
            return None;
        }
        self.comp_ribbon_at_in(&self.arrange_layout(), pos)
    }

    /// The promote request a drag from `from_x` to `to_x` names, in raw
    /// timeline frames.
    ///
    /// **Raw on purpose.** `TakeMessage::PromoteTakeSegment`'s range is a
    /// *request*: `update::takes::plan_promote` clamps it to the slot and
    /// to the take's audible extent, and that clamp is the single place
    /// the rule lives. Clamping here as well would give two clamps that
    /// can disagree — and the one the user's edit actually obeys would be
    /// the one they cannot see.
    pub(in crate::view::timeline) fn take_drag_range(
        &self,
        from_x: f32,
        to_x: f32,
    ) -> TimelineRange {
        TimelineRange::from_bounds(
            self.x_to_frames(from_x.min(to_x)),
            self.x_to_frames(from_x.max(to_x)),
        )
    }

    /// Does a split on this slot have a cut point at all — i.e. is the
    /// playhead strictly inside it?
    ///
    /// Only the *positional* precondition, and only so the affordance can
    /// say "not here" before the press rather than after a silent refusal.
    /// It is not a mirror of the refusal rule: `plan_split` also drops a
    /// cut that lands on an existing boundary, and that one stays silent.
    pub(in crate::view::timeline) fn split_has_a_cut_point(&self, slot: TimelineRange) -> bool {
        self.playhead > slot.start && self.playhead < slot.end()
    }

    /// The active take a click on `take_id` should ask for: `None` when it
    /// is already the soloed take (a second click releases the solo),
    /// `Some(take_id)` otherwise.
    ///
    /// Read off the mirror rather than tracked in canvas state — the solo
    /// can also be cleared by any comp edit, or by an engine echo, and a
    /// remembered toggle would drift out of step with both.
    pub(in crate::view::timeline) fn take_solo_toggle(
        &self,
        group_id: TakeGroupId,
        take_id: TakeId,
    ) -> Option<TakeId> {
        let already = self
            .take_groups
            .group(group_id)
            .and_then(|g| g.active_take)
            == Some(take_id);
        (!already).then_some(take_id)
    }
}
