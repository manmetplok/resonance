//! The take lane's *interaction* pass (epic #15, doc #165, todo #414):
//! what a comping gesture is about to do, drawn before it is committed.
//!
//! Everything todo #413 draws lives in the **cached** geometry layer,
//! because nothing in a take lane follows the playhead. Everything here is
//! the opposite — it follows the pointer — so it rides the *uncached*
//! overlay frame beside the playhead and the drag-placement affordances,
//! and never touches `TimelineFingerprint`. Both halves resolve their
//! geometry through the same helpers, so the preview band a drag draws
//! lands exactly on the card the promote will edit (the #732 rule).
//!
//! # Why this exists at all
//!
//! Three of the constraints this lane inherits are invisible by
//! construction, and this pass is where they are made visible:
//!
//! * **A refused edit is silent.** `update::takes` gates an impossible
//!   edit *before* dispatch so it spends no undo entry (ba doc #292) —
//!   which also means the user gets no "that didn't work" and the UI
//!   cannot use the undo history as a receipt. The answer is to describe
//!   the gesture *before* the release: the caption names the take, the
//!   range and anything notable about them, so the outcome is predictable
//!   rather than reported.
//! * **A comp edit ends the take solo.** Deliberate, and it matches Logic
//!   and Pro Tools — but the undo label is only `"split comp"`, so
//!   nothing says so. The caption does.
//! * **A split taken while a take is soloed can change what you hear.**
//!   `state::takes::effective_segments` mirrors the engine's comp tiers 2
//!   and 3 but not tier 1, so a split materializes the *fallback* take's
//!   cover and then clears the solo: the user was hearing take 0, used a
//!   gesture that names no take at all, and now hears the latest take.
//!   Todo **#1395** fixes it by seeding the cover from the active take.
//!   Until then the split affordance says out loud that the audible take
//!   may change, so the gesture never *looks* safe while a solo is up.
//!
//! Plus the one #413 could not reach: a **MIDI take soloed over audio
//! takes** silences the audio path by design
//! (`TakeGroupState::active_take_silences_audio`), which is
//! indistinguishable from a bug unless something says so.

use iced::widget::canvas;
use iced::{mouse, Color, Point, Rectangle, Size};

use resonance_common::{TakeGroup, TimelineRange};

use crate::theme;
use crate::view::arrange_layout::ArrangeRowLayout;

use super::super::input::TakePromoteDrag;
use super::super::TimelineCanvas;
use super::geometry::{silent_ranges, take_card_band, take_label};

/// Caption glyph metrics. Mono at 9 px is the take lane's own micro-label
/// size (`T1`, `media missing`), so the hint reads as part of the lane
/// rather than as generic chrome.
const HINT_TEXT_SIZE: f32 = 9.0;
const HINT_CHAR_W: f32 = 5.4;
const HINT_LINE_H: f32 = 13.0;
const HINT_PAD: f32 = 7.0;

impl TimelineCanvas<'_> {
    /// Draw the take-lane interaction affordances onto the uncached
    /// overlay frame: an in-flight promote's preview band, or — when
    /// nothing is being dragged — the hover affordance for whatever
    /// comping verb the pointer is over.
    pub(in crate::view::timeline) fn draw_take_interaction(
        &self,
        frame: &mut canvas::Frame,
        layout: &ArrangeRowLayout,
        bounds: Rectangle,
        drag: Option<&TakePromoteDrag>,
        cursor: mouse::Cursor,
    ) {
        if self.take_groups.groups.is_empty() {
            return;
        }
        if let Some(drag) = drag {
            self.draw_promote_preview(frame, layout, bounds, drag);
            return;
        }
        let Some(pos) = cursor.position_in(bounds) else {
            return;
        };
        // Above the lanes there is no take geometry at all, and the ruler
        // owns its own affordances.
        if pos.y < self.fixed_header_height() {
            return;
        }
        // Same order as the press handler: the ribbon rides the track's
        // own lane and is drawn last, so it wins over the take rows it
        // summarises and over anything sharing that lane.
        if let Some(hit) = self.comp_ribbon_at_in(layout, pos) {
            self.draw_split_affordance(frame, bounds, pos, hit);
        } else if let Some(hit) = self.take_card_at_in(layout, pos) {
            self.draw_take_card_hint(frame, bounds, pos, hit.group_id, hit.take_id);
        }
    }

    /// The in-flight promote: a lavender band across the dragged range on
    /// the take's own row, plus a caption naming the take and the range.
    ///
    /// The band is clamped to the slot for drawing only — the *message*
    /// carries the raw drag range, because `update::takes` owns the clamp.
    fn draw_promote_preview(
        &self,
        frame: &mut canvas::Frame,
        layout: &ArrangeRowLayout,
        bounds: Rectangle,
        drag: &TakePromoteDrag,
    ) {
        let Some(group) = self.take_groups.group(drag.group_id) else {
            return;
        };
        let Some(take) = group.take(drag.take_id) else {
            return;
        };
        let label = take_label(take.pass_index);
        let anchor = Point::new(drag.cursor_x, drag.cursor_y);

        // Below the slop threshold the gesture is still a click, and a
        // click solos rather than promotes. Say that instead of drawing a
        // 2 px promote band the release will not perform.
        if !drag.is_promote() {
            let releasing = group.active_take == Some(take.id);
            self.draw_hint(
                frame,
                bounds,
                anchor,
                &[(
                    if releasing {
                        format!("release the {label} solo")
                    } else {
                        format!("solo {label}")
                    },
                    theme::WARM,
                )],
            );
            return;
        }

        let Some((slot_x, slot_w)) = self.slot_rect(group.slot, f32::INFINITY) else {
            return;
        };
        let Some((row_y_top, row_height)) =
            layout.take_row_rect(drag.track_id, drag.group_id, drag.take_id)
        else {
            return;
        };
        let row_y = self.fixed_header_height() + row_y_top - self.scroll_offset_y;
        let (card_y, card_h) = take_card_band(row_y, row_height);
        let card = Rectangle {
            x: slot_x,
            y: card_y,
            width: slot_w,
            height: card_h,
        };

        let raw = self.take_drag_range(drag.anchor_x, drag.cursor_x);
        // Display-only intersection: a band cannot be painted outside the
        // lane it belongs to. The emitted range stays raw.
        let target = intersect(raw, group.slot);
        let sounding = self
            .take_audible_extent(group, take)
            .map(|a| intersect(a, target))
            .filter(|r| !r.is_empty());
        let silent = silent_ranges(target, sounding);

        if let Some((x0, x1)) = self.sub_span(target, card) {
            frame.fill_rectangle(
                Point::new(x0, card_y),
                Size::new(x1 - x0, card_h),
                theme::ACCENT_DIM,
            );
            // Top rule + both edges: the same 2 px accent a promoted comp
            // span already wears, so the preview reads as "this is about
            // to become a promotion" rather than as a selection brush.
            frame.fill_rectangle(
                Point::new(x0, card_y),
                Size::new(x1 - x0, 2.0),
                theme::ACCENT,
            );
            for edge_x in [x0, x1 - 1.0] {
                frame.fill_rectangle(
                    Point::new(edge_x, card_y),
                    Size::new(1.0, card_h),
                    theme::ACCENT,
                );
            }
        }
        // Where the take has nothing to give: a `BAD` rule on the card's
        // centre line, right on top of the `TEXT_3` flat line the card
        // already draws there. Promoting across it is legal and the engine
        // renders it as silence — the point is that it is a choice, not a
        // surprise.
        for range in &silent {
            if let Some((x0, x1)) = self.sub_span(*range, card) {
                frame.fill_rectangle(
                    Point::new(x0, card_y + card_h * 0.5 - 1.0),
                    Size::new(x1 - x0, 2.0),
                    theme::BAD,
                );
            }
        }

        let mut lines = vec![(
            format!(
                "promote {label} · {} – {}",
                self.seconds_label(target.start),
                self.seconds_label(target.end())
            ),
            theme::ACCENT_SOFT,
        )];
        if !silent.is_empty() {
            lines.push((
                format!("{label} has no audio over part of this — it plays silent"),
                theme::BAD,
            ));
        }
        lines.extend(self.solo_ends_notes(group));
        self.draw_hint(frame, bounds, anchor, &lines);
    }

    /// The split affordance on a hovered comp ribbon: a warm tick where
    /// the cut lands, and a caption for everything the cut carries with it.
    fn draw_split_affordance(
        &self,
        frame: &mut canvas::Frame,
        bounds: Rectangle,
        pos: Point,
        hit: super::input::CompRibbonHit,
    ) {
        let Some(group) = self.take_groups.group(hit.group_id) else {
            return;
        };
        let has_cut = self.split_has_a_cut_point(hit.slot);

        let mut lines = Vec::new();
        if has_cut {
            // The tick sits at the playhead, not at the pointer: the cut
            // point is the transport's, and a marker under the cursor
            // would imply a click-to-cut-here the message cannot honour.
            let x = self.sample_to_x(self.playhead);
            frame.fill_rectangle(
                Point::new(x - 0.5, hit.band_top - 3.0),
                Size::new(1.0, hit.band_height + 6.0),
                theme::WARM,
            );
            for y in [hit.band_top - 3.0, hit.band_top + hit.band_height - 1.0] {
                frame.fill_rectangle(Point::new(x - 3.0, y), Size::new(6.0, 4.0), theme::WARM);
            }
            lines.push((
                format!("split the comp at {}", self.seconds_label(self.playhead)),
                theme::ACCENT_SOFT,
            ));
        } else {
            lines.push((
                "playhead is outside this lane — nothing to split".to_string(),
                theme::BAD,
            ));
        }
        lines.extend(self.solo_ends_notes(group));
        // Pending todo **#1395** a split materializes the *fallback*
        // take's cover rather than the soloed one's, so the take you are
        // listening to can change under a gesture that names no take.
        // Never let the gesture look safe while a solo is up. The ticket
        // number stays in this comment: the user needs the consequence,
        // not the bookkeeping. When #1395 lands, delete this note.
        if has_cut && group.active_take.is_some() {
            lines.push((
                "the take you hear may change with it".to_string(),
                theme::BAD,
            ));
        }
        self.draw_hint(frame, bounds, pos, &lines);
    }

    /// The hover affordance on a take card: which verbs the card answers
    /// to, and whether soloing it would silence the group's audio.
    fn draw_take_card_hint(
        &self,
        frame: &mut canvas::Frame,
        bounds: Rectangle,
        pos: Point,
        group_id: resonance_common::TakeGroupId,
        take_id: resonance_common::TakeId,
    ) {
        let Some(group) = self.take_groups.group(group_id) else {
            return;
        };
        let Some(take) = group.take(take_id) else {
            return;
        };
        let label = take_label(take.pass_index);
        let soloed = group.active_take == Some(take_id);
        let mut lines = vec![(
            format!(
                "{label} · {} · drag to promote · right-click deletes",
                if soloed {
                    "click to release the solo"
                } else {
                    "click to solo"
                }
            ),
            theme::TEXT_2,
        )];
        if soloed {
            lines.extend(self.midi_solo_note(group));
        }
        self.draw_hint(frame, bounds, pos, &lines);
    }

    /// The "this comp edit ends the solo" note, plus the MIDI-solo
    /// consequence when it applies. Empty when no take is soloed.
    ///
    /// Split and promote both send `SetActiveTake(None)` — an active take
    /// overrides the comp entirely, so an edit made under a solo would be
    /// inaudible. That is the right rule and matches Logic / Pro Tools; it
    /// is just not *discoverable*, because the undo entry is labelled only
    /// `"split comp"`.
    fn solo_ends_notes(&self, group: &TakeGroup) -> Vec<(String, Color)> {
        let Some(active) = group.active_take.and_then(|id| group.take(id)) else {
            return Vec::new();
        };
        let mut notes = vec![(
            format!(
                "ends the {} solo — the comp plays again",
                take_label(active.pass_index)
            ),
            theme::WARM,
        )];
        notes.extend(self.midi_solo_note(group));
        notes
    }

    /// A MIDI take soloed on a group that also holds audio takes resolves
    /// to zero audio spans while those takes stay governed, so the lane
    /// goes silent on the audio path. By design (ba doc #292) — and
    /// indistinguishable from a bug unless the UI says so.
    fn midi_solo_note(&self, group: &TakeGroup) -> Vec<(String, Color)> {
        if !self.take_groups.active_take_silences_audio(group.id) {
            return Vec::new();
        }
        let label = group
            .active_take
            .and_then(|id| group.take(id))
            .map(|t| take_label(t.pass_index))
            .unwrap_or_default();
        vec![(
            format!("{label} is a MIDI take — the audio takes are silent"),
            theme::BAD,
        )]
    }

    /// A sample position as `12.34 s`. Seconds rather than bars: a take
    /// slot is a sample range the engine handed over, and the comp edits
    /// this caption describes are not grid-quantized.
    fn seconds_label(&self, frames: u64) -> String {
        format!("{:.2} s", frames as f64 / self.sample_rate.max(1) as f64)
    }

    /// A small caption card trailing the cursor, one row per line. Follows
    /// the drop-tooltip idiom (`BG_1` on a `LINE` hairline) and flips back
    /// across the cursor when it would otherwise leave the canvas.
    fn draw_hint(
        &self,
        frame: &mut canvas::Frame,
        bounds: Rectangle,
        anchor: Point,
        lines: &[(String, Color)],
    ) {
        if lines.is_empty() {
            return;
        }
        let widest = lines
            .iter()
            .map(|(text, _)| text.chars().count())
            .max()
            .unwrap_or(0) as f32;
        let w = widest * HINT_CHAR_W + 2.0 * HINT_PAD;
        let h = lines.len() as f32 * HINT_LINE_H + 2.0 * HINT_PAD - 3.0;

        let mut x = anchor.x + 14.0;
        if x + w > bounds.width {
            x = (anchor.x - 14.0 - w).max(0.0);
        }
        let mut y = anchor.y + 16.0;
        if y + h > bounds.height {
            y = (anchor.y - 16.0 - h).max(0.0);
        }

        let card = canvas::Path::rounded_rectangle(
            Point::new(x, y),
            Size::new(w, h),
            theme::RADIUS_XS.into(),
        );
        frame.fill(&card, theme::BG_1);
        frame.stroke(
            &card,
            canvas::Stroke::default()
                .with_color(theme::LINE)
                .with_width(1.0),
        );
        for (i, (text, color)) in lines.iter().enumerate() {
            frame.fill_text(canvas::Text {
                content: text.clone(),
                position: Point::new(x + HINT_PAD, y + HINT_PAD - 3.0 + i as f32 * HINT_LINE_H),
                color: *color,
                size: HINT_TEXT_SIZE.into(),
                font: theme::MONO_FONT,
                ..canvas::Text::default()
            });
        }
    }
}

/// The overlap of two ranges, empty when they do not meet. Mirrors
/// `update::takes::intersect`; display-only here.
fn intersect(a: TimelineRange, b: TimelineRange) -> TimelineRange {
    TimelineRange::from_bounds(a.start.max(b.start), a.end().min(b.end()))
}
