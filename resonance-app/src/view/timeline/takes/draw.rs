//! Take-lane drawing on the [`TimelineCanvas`] (epic #15, doc #165, todo
//! #413).
//!
//! Two passes, both inside the *cached* geometry layer — nothing here moves
//! with the playhead, so a stack of eight waveforms costs one repaint per
//! comp edit rather than one per frame (view-performance rules, MEMORY
//! ui-work §11). The cache key is `TimelineFingerprint::takes_hash` /
//! `take_expanded_hash`.
//!
//! * [`draw_take_comp_ribbons`](TimelineCanvas::draw_take_comp_ribbons) —
//!   the always-visible summary strip on the track's own lane: which take
//!   is audible over which part of the slot. Drawn whether the lane is
//!   expanded or folded, so a closed take folder still answers the only
//!   question that matters at a glance.
//! * [`draw_take_rows`](TimelineCanvas::draw_take_rows) — the expanded
//!   stack: one card per take across the group's slot, carrying its
//!   waveform (audio) or note blocks (MIDI), with everything the comp does
//!   *not* use scrimmed back.
//!
//! Both are driven by [`effective_cover`], so the lane always depicts what
//! the engine will play — including the "comp still empty, so the newest
//! pass wins" case.
//!
//! **Nothing here reads `Resonance::clips`.** A take clip is not a
//! timeline clip and never enters that mirror (todo #1396), so a take's
//! *width* comes from its own `extent` and its *waveform* from
//! `TakeGroupState::peaks`, read off the recording when the app learned of
//! the take (todo #1400). The lookup that used to stand in for both was
//! true-for-every-take, which is why every recorded pass drew as hatched
//! `media missing` until #1400 removed it.

use iced::widget::canvas;
use iced::{Color, Point, Rectangle, Size};

use resonance_common::{TakeContent, TakeGroup, TimelineRange};

use crate::theme;
use crate::view::arrange_layout::ArrangeRowLayout;

use super::super::TimelineCanvas;
use super::geometry::{
    comp_ribbon_band, effective_cover, silent_ranges, take_card_band, take_label, unlit_ranges,
    CoverSource,
};

/// Minimum span width, in px, that still gets its `T{n}` label. Narrower
/// than this the glyph would collide with the seam and read as noise.
const SPAN_LABEL_MIN_W: f32 = 20.0;
/// Corner radius shared by the ribbon backing and the take cards.
const CARD_RADIUS: f32 = theme::RADIUS_XS;

impl TimelineCanvas<'_> {
    /// Draw the comp ribbon on every visible track lane that owns take
    /// groups. Called from the cached `draw_into` pass, after the clips, so
    /// the ribbon sits on top of a clip that happens to overlap the slot.
    pub(in crate::view::timeline) fn draw_take_comp_ribbons(
        &self,
        frame: &mut canvas::Frame,
        layout: &ArrangeRowLayout,
        header_height: f32,
        y_off: f32,
        bounds: Rectangle,
    ) {
        if self.take_groups.groups.is_empty() {
            return;
        }
        for group in &self.take_groups.groups {
            // A track hidden inside a collapsed group has no visible lane,
            // so its ribbon vanishes with it — matching clip behaviour.
            let Some((row_y_top, row_height)) = layout.track_row_rect(group.track_id) else {
                continue;
            };
            let row_y = header_height + row_y_top - y_off;
            if row_y + row_height < header_height || row_y > bounds.height {
                continue;
            }
            let Some((x, w)) = self.slot_rect(group.slot, bounds.width) else {
                continue;
            };
            let (top, height) = comp_ribbon_band(row_y, row_height);
            self.draw_comp_ribbon(frame, group, x, top, w, height);
        }
    }

    /// One group's ribbon: a recessed backing plus one block per span of
    /// the effective cover, seams marked by a 1 px gutter.
    fn draw_comp_ribbon(
        &self,
        frame: &mut canvas::Frame,
        group: &TakeGroup,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) {
        let backing = canvas::Path::rounded_rectangle(
            Point::new(x, y),
            Size::new(w, h),
            CARD_RADIUS.into(),
        );
        frame.fill(&backing, theme::BG_1);
        frame.stroke(
            &backing,
            canvas::Stroke::default()
                .with_width(1.0)
                .with_color(theme::LINE),
        );

        let cover = effective_cover(group);
        if cover.is_empty() {
            // Degenerate: a group with no takes (or a zero-length slot).
            // Say so rather than leaving an unexplained empty strip.
            frame.fill_text(canvas::Text {
                content: "no takes".to_string(),
                position: Point::new(x + 5.0, y + 2.0),
                color: theme::TEXT_3,
                size: 9.0.into(),
                font: theme::MONO_FONT,
                ..canvas::Text::default()
            });
            return;
        }

        let backing_rect = Rectangle {
            x,
            y,
            width: w,
            height: h,
        };
        for span in &cover {
            // Clamped to the backing so a segment overhanging the slot (a
            // stale mirror) can never paint outside the ribbon.
            let Some((x0, x1)) = self.sub_span(span.range, backing_rect) else {
                continue;
            };
            let (fill, rule) = span_colors(span.source);
            frame.fill_rectangle(Point::new(x0, y + 1.0), Size::new(x1 - x0, h - 2.0), fill);
            // Rule along the top edge: the strongest cue that this block is
            // a deliberate promotion (a solid 2 px accent) rather than the
            // latest-take fallback (a hairline).
            let rule_h = if span.source == CoverSource::LatestFallback {
                1.0
            } else {
                2.0
            };
            frame.fill_rectangle(Point::new(x0, y + 1.0), Size::new(x1 - x0, rule_h), rule);
            if x1 - x0 >= SPAN_LABEL_MIN_W {
                if let Some(take) = group.take(span.take_id) {
                    let mut label = take_label(take.pass_index);
                    if span.source == CoverSource::ActiveTake {
                        label.push_str(" solo");
                    }
                    frame.fill_text(canvas::Text {
                        content: label,
                        position: Point::new(x0 + 4.0, y + 2.0),
                        // Not `rule`: the fallback tier's hairline is a
                        // 34 %-alpha line, which is right for a rule and
                        // unreadable for a glyph. The tier is already
                        // carried by the rule's weight, so the label only
                        // has to stay legible (principle 1). Mono at 9 px
                        // to match every other micro-label on the canvas.
                        color: span_label_color(span.source),
                        size: 9.0.into(),
                        font: theme::MONO_FONT,
                        ..canvas::Text::default()
                    });
                }
            }
            // Seam gutter on the block's right edge, unless it is the
            // ribbon's own right edge.
            if x1 < x + w - 0.5 {
                frame.fill_rectangle(
                    Point::new(x1 - 0.5, y + 1.0),
                    Size::new(1.0, h - 2.0),
                    theme::BG_1,
                );
            }
        }
    }

    /// Draw the expanded take stacks: one card per
    /// [`ArrangeRowKind::TakeRow`](crate::view::arrange_layout::ArrangeRowKind::TakeRow)
    /// in the shared layout. Called from the cached `draw_into` pass inside
    /// the lane clip.
    pub(in crate::view::timeline) fn draw_take_rows(
        &self,
        frame: &mut canvas::Frame,
        layout: &ArrangeRowLayout,
        header_height: f32,
        y_off: f32,
        bounds: Rectangle,
    ) {
        if self.take_groups.groups.is_empty() {
            return;
        }
        for group in &self.take_groups.groups {
            if !self.take_lane_expanded_tracks.contains(&group.track_id) {
                continue;
            }
            // The whole stack shares the group's slot, so its x extent is
            // resolved once — an off-screen slot skips the group entirely.
            let Some((x, w)) = self.slot_rect(group.slot, bounds.width) else {
                continue;
            };
            let cover = effective_cover(group);
            for take in &group.takes {
                let Some((row_y_top, row_height)) =
                    layout.take_row_rect(group.track_id, group.id, take.id)
                else {
                    continue;
                };
                let row_y = header_height + row_y_top - y_off;
                if row_y + row_height < header_height || row_y > bounds.height {
                    continue;
                }
                let (card_y, card_h) = take_card_band(row_y, row_height);
                let card = Rectangle {
                    x,
                    y: card_y,
                    width: w,
                    height: card_h,
                };
                self.draw_take_card(frame, group, take, &cover, card);
            }
        }
    }

    /// One take's card: content silhouette, comp lighting, edge and tag.
    ///
    /// `slot_rect` is the lane the take occupies. The card itself is drawn
    /// over the take's **audible extent** inside that lane — the stretch
    /// the pass really recorded, resolved by
    /// [`take_audible_extent`](Self::take_audible_extent) — so a pass that
    /// punched in late, or was cut short at stop, gets a shorter card and a
    /// "no audio" flat line over the remainder, rather than a full-width
    /// waveform stretched to fit.
    fn draw_take_card(
        &self,
        frame: &mut canvas::Frame,
        group: &TakeGroup,
        take: &resonance_common::Take,
        cover: &[super::geometry::CoverSpan],
        slot_rect: Rectangle,
    ) {
        let (y, h) = (slot_rect.y, slot_rect.height);
        let is_active = group.active_take == Some(take.id);
        let missing = self.take_is_missing(group, take);
        // Where this take really carries audio. Resolved through the shared
        // helper so the comping gestures of todo #414 target exactly what
        // this card draws (the #732 rule).
        let audible = self.take_audible_extent(group, take);

        // ---- "No audio here" ----
        // The parts of the lane the take does not reach. Drawn before the
        // card so the card's rounded edge sits on top of the recess.
        for range in silent_ranges(group.slot, audible) {
            if let Some((x0, x1)) = self.sub_span(range, slot_rect) {
                frame.fill_rectangle(
                    Point::new(x0, y),
                    Size::new(x1 - x0, h),
                    Color {
                        a: 0.35,
                        ..theme::BG_0
                    },
                );
            }
        }

        // A take whose clip does not overlap its own slot at all (or whose
        // overlap is sub-pixel) has no card: the flat line is the whole
        // story.
        let card_span = audible.and_then(|a| self.sub_span(a, slot_rect));
        let Some((x0, x1)) = card_span else {
            self.draw_silence_rule(frame, group.slot, slot_rect, None);
            self.draw_take_tag(frame, take, is_active, slot_rect.x, y);
            return;
        };
        let audible = audible.expect("card_span is Some only when audible is");
        let (x, w) = (x0, x1 - x0);

        // ---- Body: audio warm / MIDI lavender / missing-media pink ----
        let path = canvas::Path::rounded_rectangle(
            Point::new(x, y),
            Size::new(w, h),
            CARD_RADIUS.into(),
        );
        frame.fill(
            &path,
            match (&take.content, missing) {
                (_, true) => Color {
                    a: 0.10,
                    ..theme::BAD
                },
                (TakeContent::Audio { .. }, _) => Color {
                    a: 0.07,
                    ..theme::WARM
                },
                (TakeContent::Midi { .. }, _) => Color {
                    a: 0.08,
                    ..theme::ACCENT
                },
            },
        );

        match (&take.content, missing) {
            // The degenerate case the design calls out: the take's
            // recorded clip is gone (relink pending, project moved, or
            // #412 flagged the WAV absent at load). Hatch + a `BAD` label,
            // never a silently empty card that would read as "nothing was
            // recorded here".
            (TakeContent::Audio { .. }, true) => {
                draw_hatch(frame, x, y, w, h);
                frame.fill_text(canvas::Text {
                    content: "media missing".to_string(),
                    position: Point::new(x + 26.0, y + h * 0.5 - 5.0),
                    color: theme::BAD,
                    size: 9.0.into(),
                    ..canvas::Text::default()
                });
            }
            (TakeContent::Audio { clip_ref }, false) => self.draw_take_waveform(
                frame,
                self.take_groups.peaks(group.id, take.id, *clip_ref),
                take.extent.start,
                audible,
                x,
                y,
                w,
                h,
            ),
            (TakeContent::Midi { notes }, _) => {
                self.draw_take_notes(frame, notes, group.slot, x, y, w, h)
            }
        }

        // ---- Comp lighting ----
        // The comp addresses the whole **slot**, not just the part of it
        // this take can fill, so the lighting spans `slot_rect`: a segment
        // promoted over a stretch where the take has no audio is a real
        // state (the engine renders silence there) and the lane has to show
        // it — lit lane, flat line under it.
        //
        // Scrim everything this take is not audible over...
        for range in unlit_ranges(cover, group.slot, take.id) {
            if let Some((sx0, sx1)) = self.sub_span(range, slot_rect) {
                frame.fill_rectangle(
                    Point::new(sx0, y),
                    Size::new(sx1 - sx0, h),
                    Color {
                        a: 0.55,
                        ..theme::BG_0
                    },
                );
            }
        }
        // ...and mark what it *is* audible over, in the same language the
        // ribbon uses, so the two read as one statement.
        for span in cover.iter().filter(|s| s.take_id == take.id) {
            let Some((sx0, sx1)) = self.sub_span(span.range, slot_rect) else {
                continue;
            };
            let (wash, rule) = lit_colors(span.source);
            frame.fill_rectangle(Point::new(sx0, y), Size::new(sx1 - sx0, h), wash);
            frame.fill_rectangle(Point::new(sx0, y), Size::new(sx1 - sx0, 2.0), rule);
        }

        // The flat line rides on top of the lighting so a lit-but-silent
        // stretch still reads as silent.
        self.draw_silence_rule(frame, group.slot, slot_rect, Some(audible));

        // ---- Edge ----
        // An active (soloed) take is ringed in warm amber — the palette's
        // solo token — so "this whole take is playing" never has to be
        // inferred from the wash alone.
        frame.stroke(
            &path,
            canvas::Stroke::default()
                .with_width(if is_active { 1.5 } else { 1.0 })
                .with_color(if is_active { theme::WARM } else { theme::LINE }),
        );

        // Tag at the take's own start, not the lane's, so a punch-in reads
        // as "this take begins here".
        self.draw_take_tag(frame, take, is_active, x, y);
    }

    /// The `T{n}` tag in a take card's top-left corner.
    fn draw_take_tag(
        &self,
        frame: &mut canvas::Frame,
        take: &resonance_common::Take,
        is_active: bool,
        x: f32,
        y: f32,
    ) {
        let mut tag = take_label(take.pass_index);
        if is_active {
            tag.push_str(" · SOLO");
        }
        frame.fill_text(canvas::Text {
            content: tag,
            position: Point::new(x + 5.0, y + 2.0),
            color: if is_active { theme::WARM } else { theme::TEXT_2 },
            size: 9.0.into(),
            ..canvas::Text::default()
        });
    }

    /// The flat "no audio here" rule across the parts of the lane a take
    /// does not reach — the zero-signal line a waveform lane draws when
    /// there is nothing to draw. Needs no label: next to a waveform it is
    /// unambiguous at any width.
    fn draw_silence_rule(
        &self,
        frame: &mut canvas::Frame,
        slot: TimelineRange,
        slot_rect: Rectangle,
        audible: Option<TimelineRange>,
    ) {
        for range in silent_ranges(slot, audible) {
            let Some((x0, x1)) = self.sub_span(range, slot_rect) else {
                continue;
            };
            // `TEXT_3` rather than a border token: this line has to stay
            // legible over the *lit* wash too — a comp can promote a take
            // across a stretch it never recorded, and that is exactly the
            // case the line exists to disclose.
            frame.fill_rectangle(
                Point::new(x0, slot_rect.y + slot_rect.height * 0.5 - 0.5),
                Size::new(x1 - x0, 1.0),
                Color {
                    a: 0.8,
                    ..theme::TEXT_3
                },
            );
        }
    }

    /// Is this take's recording absent from this machine?
    ///
    /// One way in, and it is the one the words mean: the app tried to read
    /// the take's WAV and could not (todo #412's `missing_takes`, set by
    /// the project load, which keeps the take in the comp rather than
    /// silently punching a hole in the cover). Only audio takes can be
    /// missing — a MIDI take carries its notes inline.
    ///
    /// **This used to OR in a clip lookup, and that arm was a standing
    /// false positive** (todo #1400). A take clip is never mirrored into
    /// `Resonance::clips` at all — no `RecordingFinished` follows one, and
    /// a project load restores take groups without their clips (todo
    /// #1396) — so `self.take_clip(clip_ref).is_none()` was true for
    /// *every* audio take and every freshly recorded pass drew as hatched
    /// `media missing`. What kept the arm alive was that it also fed the
    /// waveform: dropping it alone would have left a fresh take with a
    /// blank card, which design #153 forbids. Take peaks now come from the
    /// recording itself
    /// ([`load_take_peaks`](crate::project::load_take_peaks)), so the lane
    /// consults no clip at any point and this is a plain flag lookup.
    ///
    /// `pub(crate)` for the test hook
    /// [`test_take_draws_missing_media`](crate::Resonance::test_take_draws_missing_media):
    /// the false positive this replaced was invisible to every headless
    /// assertion in the suite and showed up only in pixels, which the
    /// verify gate is allowed to skip.
    pub(crate) fn take_is_missing(
        &self,
        group: &TakeGroup,
        take: &resonance_common::Take,
    ) -> bool {
        match &take.content {
            TakeContent::Audio { .. } => self.take_groups.is_missing(group.id, take.id),
            TakeContent::Midi { .. } => false,
        }
    }

    /// The stretch of `group`'s slot this take actually carries material
    /// over, or `None` when the two do not meet at all.
    ///
    /// Resolved from the take's own
    /// [`extent`](resonance_common::Take::extent) — the engine's account of
    /// what the pass recorded, carried on `TakeCaptured` and persisted with
    /// the project (todo #1396) — and **not** from a clip lookup. A take
    /// clip never enters `Resonance::clips` (no `RecordingFinished` is
    /// emitted for one, and a project load restores take groups without
    /// materialising their clips), so the old lookup fell back to the slot
    /// for every take in the normal recording flow and drew a punched-in
    /// pass full width.
    ///
    /// A missing recording is *not* widened back out to the slot: the
    /// extent is a fact about what was recorded, independent of whether
    /// the WAV is on this machine, so the hatch marks exactly the stretch
    /// whose audio is gone and the flat line covers the rest.
    ///
    /// Shared with the interaction pass so a promote preview marks silence
    /// exactly where the card draws its flat line.
    pub(in crate::view::timeline::takes) fn take_audible_extent(
        &self,
        group: &TakeGroup,
        take: &resonance_common::Take,
    ) -> Option<TimelineRange> {
        let audible = take.audible_extent(group.slot);
        (!audible.is_empty()).then_some(audible)
    }

    /// A sub-range of the slot as `(x0, x1)` clamped to `card`, or `None`
    /// when it collapses to nothing. Clamping matters: a stale mirror can
    /// echo a segment that overhangs the slot, and it must never paint
    /// outside the card it belongs to.
    pub(in crate::view::timeline::takes) fn sub_span(
        &self,
        range: TimelineRange,
        card: Rectangle,
    ) -> Option<(f32, f32)> {
        let (sx, sw) = self.slot_rect(range, f32::INFINITY)?;
        let x0 = sx.max(card.x);
        let x1 = (sx + sw).min(card.x + card.width);
        (x1 - x0 > 0.5).then_some((x0, x1))
    }

    /// The `(x, width)` of a timeline range in canvas space, or `None` when
    /// it is degenerate or entirely off-screen. `viewport_width` bounds the
    /// visibility test; pass `f32::INFINITY` for sub-ranges already known to
    /// sit inside a visible slot.
    pub(in crate::view::timeline::takes) fn slot_rect(
        &self,
        range: TimelineRange,
        viewport_width: f32,
    ) -> Option<(f32, f32)> {
        if range.is_empty() {
            return None;
        }
        let x = self.sample_to_x(range.start);
        let w = self.sample_to_x(range.end()) - x;
        if w <= 0.5 || x > viewport_width || x + w < 0.0 {
            return None;
        }
        Some((x, w))
    }

    /// Take waveform, **anchored to the timeline** rather than stretched to
    /// the card.
    ///
    /// Each pixel column resolves the frame it sits over, converts that to
    /// a position inside the recording (`frame - recording_start`), and
    /// reads the peak there — the same indexing `draw_clip_waveform` uses
    /// for a placed clip, over the same `WAVEFORM_PEAK_FRAMES` buckets.
    /// Mapping card-fraction → peak-fraction instead would time-stretch
    /// every take that does not exactly fill its slot: a pass that punched
    /// in a second late would draw its audio a second early, and slower
    /// than it plays.
    ///
    /// `peaks` is the take's own table, read off its WAV when the app
    /// learned of the take (todo #1400) — **not** borrowed from a mirrored
    /// clip, because a take clip never enters `Resonance::clips`.
    /// `recording_start` is the timeline frame the table's bucket 0 covers,
    /// i.e. the take's `extent.start`: the engine writes a pass's WAV from
    /// the punch-in point, so bucket 0 is the first frame recorded and not
    /// the start of the slot. `audible` is the timeline range the card
    /// covers, so column 0 of the card is frame `audible.start`.
    ///
    /// An empty table draws nothing. That is the honest state for a take
    /// whose recording the app could not read but which is *not* flagged
    /// missing — the card, its edge and its tag still say a pass was
    /// recorded here.
    #[allow(clippy::too_many_arguments)]
    fn draw_take_waveform(
        &self,
        frame: &mut canvas::Frame,
        peaks: &[(f32, f32)],
        recording_start: u64,
        audible: TimelineRange,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) {
        if peaks.is_empty() || w <= 2.0 || h <= 4.0 {
            return;
        }
        let center = y + h * 0.5;
        let half = (h - 4.0) * 0.5;
        let color = Color {
            a: 0.7,
            ..theme::WARM
        };
        let peak_frames = resonance_audio::types::WAVEFORM_PEAK_FRAMES as f64;
        // Frames per pixel column at the current zoom.
        let frames_per_px = self.sample_rate as f64 / self.zoom.max(f32::EPSILON) as f64;
        // Where the card's first column sits inside the recording's own
        // frames.
        let head_frames = audible.start.saturating_sub(recording_start);

        // One bar per pixel column, clamped to the visible part of the card
        // so a long take off the left edge costs nothing.
        let start_px = (-x).max(0.0);
        let mut px = start_px;
        while px < w {
            let clip_frame = head_frames as f64 + px as f64 * frames_per_px;
            let idx = (clip_frame / peak_frames) as usize;
            let Some((min_val, max_val)) = peaks.get(idx).copied() else {
                // Ran past the end of the peak table: the rest of the card
                // has no audio to show.
                break;
            };
            let top = center - max_val.clamp(-1.0, 1.0) * half;
            let bottom = center - min_val.clamp(-1.0, 1.0) * half;
            frame.fill_rectangle(
                Point::new(x + px, top),
                Size::new(1.0, (bottom - top).max(1.0)),
                color,
            );
            px += 1.0;
        }
    }

    /// Take note blocks: the MIDI counterpart of the waveform. Note
    /// positions are ticks from the take's start, so they map against the
    /// slot's tick length — which comes from the tempo map, not from the
    /// notes themselves, so a take whose notes stop early stays short
    /// instead of stretching to fill the card.
    #[allow(clippy::too_many_arguments)]
    fn draw_take_notes(
        &self,
        frame: &mut canvas::Frame,
        notes: &[resonance_common::TakeNote],
        slot: TimelineRange,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) {
        if notes.is_empty() || w <= 2.0 || h <= 4.0 {
            return;
        }
        let slot_ticks = self
            .tempo_map
            .sample_to_abs_tick(slot.end(), self.sample_rate)
            .saturating_sub(
                self.tempo_map
                    .sample_to_abs_tick(slot.start, self.sample_rate),
            );
        // Degenerate tempo map (or a zero-length slot): fall back to the
        // notes' own extent so the card still says something.
        let total_ticks = if slot_ticks > 0 {
            slot_ticks as f32
        } else {
            notes
                .iter()
                .map(|n| n.start_tick + n.duration_ticks)
                .max()
                .unwrap_or(1) as f32
        };
        if total_ticks <= 0.0 {
            return;
        }

        let min_note = notes.iter().map(|n| n.note).min().unwrap_or(0);
        let max_note = notes.iter().map(|n| n.note).max().unwrap_or(127);
        let range_min = min_note.saturating_sub(2);
        let range_max = max_note.saturating_add(2).min(127);
        let note_range = (range_max.saturating_sub(range_min)).max(1) as f32;

        let area_y = y + 2.0;
        let area_h = (h - 4.0).max(1.0);
        let color = Color {
            a: 0.85,
            ..theme::ACCENT_SOFT
        };
        for note in notes {
            let nx = x + (note.start_tick as f32 / total_ticks) * w;
            let nw = ((note.duration_ticks as f32 / total_ticks) * w).max(1.0);
            if nx > x + w || nx + nw < x {
                continue;
            }
            let ny = area_y
                + (1.0 - (note.note.saturating_sub(range_min)) as f32 / note_range)
                    * (area_h - 2.0);
            frame.fill_rectangle(
                Point::new(nx, ny),
                Size::new(nw.min(x + w - nx), 2.0),
                color,
            );
        }
    }
}

/// `(fill, rule)` for a ribbon block. Lavender is the comp domain
/// (selection / deliberate choice); warm amber is solo, matching every
/// other solo affordance in the app.
fn span_colors(source: CoverSource) -> (Color, Color) {
    match source {
        CoverSource::ActiveTake => (
            Color {
                a: 0.30,
                ..theme::WARM
            },
            theme::WARM,
        ),
        CoverSource::CompSegment => (
            Color {
                a: 0.34,
                ..theme::ACCENT
            },
            theme::ACCENT_SOFT,
        ),
        CoverSource::LatestFallback => (
            Color {
                a: 0.12,
                ..theme::ACCENT
            },
            theme::ACCENT_LINE,
        ),
    }
}

/// Colour of a ribbon block's `T{n}` label. Follows the block's tier but
/// stays at readable contrast — the tier itself is signalled by the rule's
/// weight, not by dimming the text into illegibility.
fn span_label_color(source: CoverSource) -> Color {
    match source {
        CoverSource::ActiveTake => theme::WARM,
        CoverSource::CompSegment => theme::ACCENT_SOFT,
        CoverSource::LatestFallback => Color {
            a: 0.7,
            ..theme::ACCENT_SOFT
        },
    }
}

/// `(wash, rule)` for the lit region of a take card. Same language as
/// [`span_colors`], one step quieter so the waveform underneath still
/// reads through it.
fn lit_colors(source: CoverSource) -> (Color, Color) {
    match source {
        CoverSource::ActiveTake => (
            Color {
                a: 0.14,
                ..theme::WARM
            },
            theme::WARM,
        ),
        CoverSource::CompSegment => (
            Color {
                a: 0.16,
                ..theme::ACCENT
            },
            theme::ACCENT,
        ),
        CoverSource::LatestFallback => (
            Color {
                a: 0.06,
                ..theme::ACCENT
            },
            theme::ACCENT_LINE,
        ),
    }
}

/// Diagonal hatch marking an unusable surface — the same language the
/// frozen / unsupported clip body uses (design doc #153), so "you cannot
/// work with this" looks identical wherever it appears.
fn draw_hatch(frame: &mut canvas::Frame, x: f32, y: f32, w: f32, h: f32) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let color = Color {
        a: 0.16,
        ..theme::TEXT_1
    };
    const SPACING: f32 = 7.0;
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
