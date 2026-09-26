//! Pointer event handlers for the timeline canvas: press, move, and release.
//! Wheel-scroll, hover cursor, right-press, and viewport reporting live in
//! the sibling `hover` module.
//!
//! `handle_press` delegates to three focused helpers (`press_scrollbars`,
//! `press_ruler`, `press_clips`) so that no single function exceeds ~120 lines.

use std::time::Instant;

use iced::widget::canvas;
use iced::{mouse, Point, Rectangle};

use crate::message::*;
use crate::theme;
use crate::view::arrange_layout::ArrangeRowKind;
use super::super::hit_test::{self, HitKind, MarkerHit};
use super::super::scrollbar::scroll_from_thumb_pos;
use super::super::{TimelineCanvas, TimelineState};
use super::{
    captured, BreakpointDrag, ClipInteraction, MarkerDrag, TakePromoteDrag, UpdateResult,
    DOUBLE_CLICK_MS,
};
use resonance_common::CurveKind;

/// Is `pos` inside `rect`?
fn rect_contains(rect: &Rectangle, pos: Point) -> bool {
    pos.x >= rect.x
        && pos.x <= rect.x + rect.width
        && pos.y >= rect.y
        && pos.y <= rect.y + rect.height
}

impl TimelineCanvas<'_> {
    /// Try to handle the press on a scrollbar (horizontal or vertical).
    /// Returns `Some` if the event was consumed; `None` to fall through.
    fn press_scrollbars(
        &self,
        state: &mut TimelineState,
        pos: Point,
        bounds: Rectangle,
    ) -> UpdateResult {
        let header_height = self.fixed_header_height();
        let (h_rects, v_rects) = self.scrollbar_rects(bounds);

        if let Some(sb) = h_rects {
            if rect_contains(&sb.track, pos) {
                if rect_contains(&sb.thumb, pos) {
                    state.h_scrollbar_grab = Some(pos.x - sb.thumb.x);
                } else {
                    let new_scroll = scroll_from_thumb_pos(
                        pos.x - sb.thumb.width / 2.0,
                        sb.travel,
                        sb.max_scroll,
                    );
                    state.h_scrollbar_grab = Some(sb.thumb.width / 2.0);
                    return captured(Message::Viewport(ViewportMessage::ScrollToX(new_scroll)));
                }
                return Some(canvas::Action::capture());
            }
        }

        if let Some(sb) = v_rects {
            if rect_contains(&sb.track, pos) {
                if rect_contains(&sb.thumb, pos) {
                    state.v_scrollbar_grab = Some(pos.y - sb.thumb.y);
                } else {
                    let new_scroll = scroll_from_thumb_pos(
                        pos.y - header_height - sb.thumb.height / 2.0,
                        sb.travel,
                        sb.max_scroll,
                    );
                    state.v_scrollbar_grab = Some(sb.thumb.height / 2.0);
                    return captured(Message::Viewport(ViewportMessage::ScrollToY(new_scroll)));
                }
                return Some(canvas::Action::capture());
            }
        }

        None
    }

    /// Try to handle the press inside the ruler band (loop-marker drag,
    /// arrangement-marker hit, or ruler-seek). Returns `Some` if consumed.
    fn press_ruler(
        &self,
        state: &mut TimelineState,
        pos: Point,
        cursor: mouse::Cursor,
    ) -> UpdateResult {
        let ruler_height = theme::RULER_HEIGHT;
        if pos.y >= ruler_height {
            return None;
        }

        // Loop marker dragging.
        if self.loop_enabled {
            let loop_in_x = self.sample_to_x(self.loop_in);
            let loop_out_x = self.sample_to_x(self.loop_out);
            let dist_in = (pos.x - loop_in_x).abs();
            let dist_out = (pos.x - loop_out_x).abs();
            if dist_in < 8.0 || dist_out < 8.0 {
                let target = if dist_in < 8.0 && dist_out < 8.0 {
                    if dist_in < dist_out {
                        crate::state::LoopDragTarget::In
                    } else {
                        crate::state::LoopDragTarget::Out
                    }
                } else if dist_in < 8.0 {
                    crate::state::LoopDragTarget::In
                } else {
                    crate::state::LoopDragTarget::Out
                };
                state.dragging_loop = true;
                return captured(Message::Transport(TransportMessage::StartLoopDrag(target)));
            }
        }

        // Arrangement-marker hit-testing in the ruler band. A flag click
        // selects and begins a start-move drag; a region end-edge click
        // begins a resize drag; a double-click on a flag opens the inline
        // rename. Sits ahead of the generic ruler-seek below so clicking a
        // marker never also moves the playhead.
        if let Some((id, hit)) = self.marker_at(pos) {
            let now = Instant::now();
            let is_double = state
                .last_marker_click
                .map(|(t, mid)| {
                    mid == id && now.duration_since(t).as_millis() <= DOUBLE_CLICK_MS
                })
                .unwrap_or(false);
            state.last_marker_click = Some((now, id));
            if is_double && hit == MarkerHit::Flag {
                state.last_marker_click = None;
                let anchor = cursor.position().unwrap_or(pos);
                return captured(Message::MarkerUi(MarkerUiMessage::BeginRename {
                    id,
                    x: anchor.x,
                    y: anchor.y,
                }));
            }
            state.marker_drag = Some(MarkerDrag { id, hit });
            return captured(Message::MarkerUi(MarkerUiMessage::Select(Some(id))));
        }

        // Any other click in the ruler → seek the playhead, snapped to the
        // nearest grid line.
        let seconds = ((pos.x + self.scroll_offset) / self.zoom).max(0.0);
        let sample = (seconds as f64 * self.sample_rate as f64) as u64;
        let snapped = self.snap_sample(sample);
        captured(Message::Transport(TransportMessage::SeekToSample(snapped)))
    }

    /// Try to handle the press on a MIDI or audio clip in the track lanes.
    /// Returns `Some` if a clip was hit; `None` to fall through to empty-area
    /// handling. Checks MIDI clips first (they're on top), then audio clips.
    fn press_clips(
        &self,
        state: &mut TimelineState,
        pos: Point,
        layout: &crate::view::arrange_layout::ArrangeRowLayout,
    ) -> UpdateResult {
        // MIDI clips (reverse order so topmost wins).
        for clip in self.midi_clips.iter().rev() {
            let clip_end = self.tempo_map.tick_to_abs_sample(
                clip.start_sample,
                clip.duration_ticks,
                self.sample_rate,
            );
            let duration_samples = clip_end.saturating_sub(clip.start_sample);
            let Some(hit) = self.hit_test_lane(
                pos,
                layout,
                clip.track_id,
                clip.start_sample,
                duration_samples,
            ) else {
                continue;
            };

            // Double-click on a MIDI clip body opens the piano roll editor.
            let now = Instant::now();
            let is_double_click = state
                .last_midi_click
                .map(|(t, id)| {
                    id == clip.id && now.duration_since(t).as_millis() <= DOUBLE_CLICK_MS
                })
                .unwrap_or(false);
            state.last_midi_click = Some((now, clip.id));
            if is_double_click {
                state.last_midi_click = None;
                return captured(Message::MidiEditor(MidiEditorMessage::OpenMidiEditor(
                    clip.id,
                )));
            }

            return match hit {
                HitKind::Trim(edge) => {
                    state.clip_interaction = Some(ClipInteraction::MidiTrim);
                    captured(Message::MidiClip(MidiClipMessage::StartMidiClipTrim {
                        clip_id: clip.id,
                        edge,
                        anchor_x: pos.x,
                    }))
                }
                HitKind::Move { grab_offset_x } => {
                    state.clip_interaction = Some(ClipInteraction::MidiMove);
                    captured(Message::MidiClip(MidiClipMessage::StartMidiClipDrag {
                        clip_id: clip.id,
                        grab_offset_x,
                        start_x: pos.x,
                        start_y: pos.y,
                    }))
                }
                HitKind::FadeIn | HitKind::FadeOut | HitKind::Gain => {
                    unreachable!("MIDI clips have no fade/gain handles")
                }
                HitKind::Miss => unreachable!("None path taken above"),
            };
        }

        // Audio clips (reverse order so topmost wins).
        for clip in self.clips.iter().rev() {
            let Some(hit) = self.hit_test_audio_lane(pos, layout, clip) else {
                continue;
            };

            return match hit {
                HitKind::Trim(edge) => {
                    state.clip_interaction = Some(ClipInteraction::Trim);
                    captured(Message::Clip(ClipMessage::StartClipTrim {
                        clip_id: clip.id,
                        edge,
                        anchor_x: pos.x,
                    }))
                }
                HitKind::FadeIn => {
                    state.clip_interaction = Some(ClipInteraction::Fade);
                    captured(Message::Clip(ClipMessage::StartClipFadeDrag {
                        clip_id: clip.id,
                        edge: crate::state::ClipEdge::Left,
                        anchor_x: pos.x,
                    }))
                }
                HitKind::FadeOut => {
                    state.clip_interaction = Some(ClipInteraction::Fade);
                    captured(Message::Clip(ClipMessage::StartClipFadeDrag {
                        clip_id: clip.id,
                        edge: crate::state::ClipEdge::Right,
                        anchor_x: pos.x,
                    }))
                }
                HitKind::Gain => {
                    state.clip_interaction = Some(ClipInteraction::Gain);
                    captured(Message::Clip(ClipMessage::StartClipGainDrag {
                        clip_id: clip.id,
                        anchor_y: pos.y,
                    }))
                }
                HitKind::Move { grab_offset_x } => {
                    state.clip_interaction = Some(ClipInteraction::Move);
                    captured(Message::Clip(ClipMessage::StartClipDrag {
                        clip_id: clip.id,
                        grab_offset_x,
                        start_x: pos.x,
                        start_y: pos.y,
                    }))
                }
                HitKind::Miss => unreachable!("None path taken above"),
            };
        }

        None
    }

    /// Try to handle the press on a take lane (epic #15, todo #414):
    /// the comp ribbon on a track's own lane, or a take card in an
    /// expanded stack. Returns `Some` if consumed.
    ///
    /// The ribbon publishes immediately — a split names no range, so
    /// there is nothing to wait for. A take card does **not**: click and
    /// drag are the same press, and only the release tells them apart, so
    /// the press just opens a [`TakePromoteDrag`] and captures.
    fn press_take_lane(
        &self,
        state: &mut TimelineState,
        pos: Point,
        layout: &crate::view::arrange_layout::ArrangeRowLayout,
    ) -> UpdateResult {
        if let Some(hit) = self.comp_ribbon_at_in(layout, pos) {
            // Emitted even when the playhead sits outside the slot: the
            // gate refuses it silently and spends no undo entry, and the
            // hover affordance has already said "nothing to split" — so a
            // pre-check here would only be a second copy of the rule.
            return captured(Message::Take(TakeMessage::SplitCompAtPlayhead {
                group_id: hit.group_id,
            }));
        }
        let hit = self.take_card_at_in(layout, pos)?;
        state.take_promote_drag = Some(TakePromoteDrag {
            track_id: hit.track_id,
            group_id: hit.group_id,
            take_id: hit.take_id,
            anchor_x: pos.x,
            cursor_x: pos.x,
            cursor_y: pos.y,
        });
        // Redraw so the affordance caption switches from "hover" to
        // "gesture in flight" on the press itself.
        Some(canvas::Action::request_redraw().and_capture())
    }

    pub(in crate::view::timeline) fn handle_press(
        &self,
        state: &mut TimelineState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> UpdateResult {
        let pos = cursor.position_in(bounds)?;
        let ruler_height = theme::RULER_HEIGHT;
        let header_height = self.fixed_header_height();

        // Scrollbar hit tests.
        if let Some(result) = self.press_scrollbars(state, pos, bounds) {
            return Some(result);
        }

        // Ruler area: loop markers, arrangement markers, ruler seek.
        if let Some(result) = self.press_ruler(state, pos, cursor) {
            return Some(result);
        }

        // Clicks in the global-shelf area (between the section band and the
        // track lanes). The shelf is split into:
        //   - section band (`SECTION_BAND_HEIGHT` when sections exist)
        //   - shelf header strip (`GLOBAL_SHELF_HEADER_HEIGHT`, always)
        //   - lanes: chord / tempo / signature (only when expanded)
        // A click in the header strip always toggles the shelf;
        // a click in the lanes routes to the per-lane handler.
        let band_h = self.section_band_height();
        let shelf_top = ruler_height + band_h;
        let shelf_header_bottom = shelf_top + theme::GLOBAL_SHELF_HEADER_HEIGHT;

        if pos.y >= shelf_top && pos.y < shelf_header_bottom {
            return captured(Message::Ui(UiMessage::ToggleGlobalTracks));
        }
        if pos.y >= shelf_header_bottom
            && pos.y < header_height
            && self.global_tracks_expanded
        {
            return self.handle_global_track_click(state, pos, bounds);
        }

        // Parameter-label chip on a multi-lane automation band: a click
        // cycles which lane the band shows (todo #1095). Checked ahead of
        // the breakpoint dots — the chip is a discrete control drawn on top
        // of the band, and a breakpoint pinned at value 1.0 can reach into
        // the chip zone with its pick radius. Single-lane tracks have no
        // clickable chip (`lane_chip_hit` returns `None`), so their clicks
        // fall through unchanged.
        if let Some(track_id) = self.lane_chip_hit(pos) {
            state.selected_breakpoint = None;
            return captured(Message::Automation(AutomationMessage::CycleTrackLane(
                track_id,
            )));
        }

        // Take-lane comping (epic #15, doc #165, todo #414). Build the
        // shared arrange-row layout once here: every hit below consults it,
        // so the variable 60/96 px pitch, the automation / take sub-rows and
        // collapsed-member hiding are honoured (epic #36, doc #203).
        //
        // Ahead of *both* the automation dots and the clips, because the
        // draw pass paints the comp ribbon after both of them: the ribbon
        // band overlaps the bottom of the clip body (`CLIP_LANE_INSET` is
        // 10 px, the ribbon starts 16 px above the row's bottom edge) and
        // reaches into the 7 px pick radius of a breakpoint pinned at value
        // 0. Whatever is drawn on top has to be what the pointer hits.
        let layout = self.arrange_layout();
        if let Some(result) = self.press_take_lane(state, pos, &layout) {
            // A take-lane press is not a breakpoint press, so it drops the
            // keyboard-delete selection like every other non-dot press.
            state.selected_breakpoint = None;
            return Some(result);
        }

        // Automation breakpoint dots win over clips (they're small targets
        // drawn on top of the lane band). A dot hit selects + starts a drag,
        // or — on a double-click — toggles its curve kind.
        if let Some(hit) = self.breakpoint_hit(pos) {
            let now = Instant::now();
            let is_double = state
                .last_breakpoint_click
                .as_ref()
                .map(|(t, target, idx)| {
                    *target == hit.target
                        && *idx == hit.index
                        && now.duration_since(*t).as_millis() <= DOUBLE_CLICK_MS
                })
                .unwrap_or(false);
            state.last_breakpoint_click = Some((now, hit.target.clone(), hit.index));
            state.selected_breakpoint = Some((hit.target.clone(), hit.index));
            if is_double {
                state.last_breakpoint_click = None;
                let curve = self.toggled_breakpoint_curve(hit.target.clone(), hit.index);
                return captured(Message::Automation(AutomationMessage::SetCurveKind {
                    target: hit.target,
                    index: hit.index,
                    curve,
                }));
            }
            state.breakpoint_drag = Some(BreakpointDrag {
                target: hit.target.clone(),
                index: hit.index,
            });
            return captured(Message::Automation(AutomationMessage::StartBreakpointDrag {
                target: hit.target,
                index: hit.index,
            }));
        }
        // Any press that isn't on a breakpoint clears the keyboard-delete
        // selection (clip / track / band-add presses all fall through here).
        state.selected_breakpoint = None;

        // Clip hit-testing in the track area, against the layout built
        // above.
        if let Some(result) = self.press_clips(state, pos, &layout) {
            return Some(result);
        }

        // Empty space inside an automated track's value band → add a
        // breakpoint there. Checked after clips so clicking a clip still
        // moves the clip; only the bare band adds a point.
        if let Some((target, time_frames, value)) = self.band_add_at(pos) {
            return captured(Message::Automation(AutomationMessage::AddBreakpoint {
                target,
                time_frames,
                value,
                curve: CurveKind::default(),
            }));
        }

        // Clicked on empty lane area (no clip hit). Resolve the row under
        // the cursor through the shared layout so the variable 60/96 px
        // pitch and collapsed-member hiding are honoured:
        //   - a press on a GROUP-HEADER lane routes to the group (fold /
        //     unfold via `ToggleCollapse`) — it is *not* a track row, so it
        //     must never select a phantom track underneath it;
        //   - a press on a TRACK lane selects that track (and drops any
        //     clip selection, as before);
        //   - a press below the last row clears the selection.
        match hit_test::row_at_canvas_y(&layout, pos.y, header_height, self.scroll_offset_y) {
            Some(ArrangeRowKind::GroupHeader(group_id)) => {
                captured(Message::Group(GroupMessage::ToggleCollapse(group_id)))
            }
            Some(ArrangeRowKind::Track(track_id)) => {
                captured(Message::Ui(UiMessage::SelectTrack(Some(track_id))))
            }
            // A press on an automation sub-row that missed every band
            // gesture (the chip is suppressed on lane rows; the breakpoint
            // dots and band-add were checked above via `automation_row_at`,
            // which resolves the lane directly from this row — todo #1097)
            // falls back to selecting the owning track, so the label strip
            // above the band still behaves like track chrome.
            Some(ArrangeRowKind::AutomationLane { track, .. }) => {
                captured(Message::Ui(UiMessage::SelectTrack(Some(track))))
            }
            // Same fallback for a take sub-row: selecting the owning track
            // is the safe, non-destructive reading of a press on the take
            // stack. The comping gestures that will claim these rows first
            // (select active take / split / promote) are todo #414's —
            // rendering, todo #413, deliberately adds no new pointer verb.
            Some(ArrangeRowKind::TakeRow { track, .. }) => {
                captured(Message::Ui(UiMessage::SelectTrack(Some(track))))
            }
            None => captured(Message::Ui(UiMessage::SelectTrack(None))),
        }
    }

    pub(in crate::view::timeline) fn handle_move(
        &self,
        state: &mut TimelineState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> UpdateResult {
        let pos = cursor.position_in(bounds)?;

        // Drag-to-timeline placement in flight (doc #175, todo #605). The
        // gesture starts in the media browser, so there is no local drag
        // state to consult — the presence of `self.drag` is the signal.
        // Resolve the drop target for the current cursor and publish a
        // Hover so the pill / lit lane / ghost / tooltip follow the pointer.
        // Capture so the release lands here too.
        if self.drag.is_some() {
            let geo = self.placement_geometry();
            let resolved = super::super::placement::resolve_drop(
                &geo,
                self.tempo_map,
                self.arrange_layout(),
                pos,
            );
            return captured(Message::Drag(DragMessage::Hover {
                cursor: pos,
                resolved,
            }));
        }

        // Horizontal scrollbar drag.
        if let Some(grab) = state.h_scrollbar_grab {
            let (h_rects, _) = self.scrollbar_rects(bounds);
            if let Some(sb) = h_rects {
                let new_scroll = scroll_from_thumb_pos(pos.x - grab, sb.travel, sb.max_scroll);
                return captured(Message::Viewport(ViewportMessage::ScrollToX(new_scroll)));
            }
        }
        // Vertical scrollbar drag.
        if let Some(grab) = state.v_scrollbar_grab {
            let (_, v_rects) = self.scrollbar_rects(bounds);
            if let Some(sb) = v_rects {
                let new_scroll =
                    scroll_from_thumb_pos(pos.y - sb.track.y - grab, sb.travel, sb.max_scroll);
                return captured(Message::Viewport(ViewportMessage::ScrollToY(new_scroll)));
            }
        }

        if state.dragging_loop {
            return captured(Message::Transport(TransportMessage::UpdateLoopDrag(pos.x)));
        }
        // Marker drag: move the start pole, or resize a region's end edge.
        // `MoveStart` snaps in its reducer; the end edge is snapped here so
        // both handles land on the same grid lines.
        if let Some(drag) = state.marker_drag {
            let sample = self.x_to_sample(pos.x);
            return match drag.hit {
                MarkerHit::Flag => {
                    captured(Message::Marker(MarkerMessage::MoveStart(drag.id, sample)))
                }
                MarkerHit::EndEdge => {
                    let snapped = self.snap_sample(sample);
                    captured(Message::Marker(MarkerMessage::SetRegionEnd(
                        drag.id,
                        Some(snapped),
                    )))
                }
            };
        }
        // Tempo event drag: vertical = BPM (1 px = 1 BPM), horizontal = bar.
        if let Some(drag) = &state.tempo_drag {
            let bar = self.x_to_bar(pos.x);
            let delta_y = drag.anchor_y - pos.y; // up = positive = increase BPM
            let bpm = (drag.original_bpm + delta_y).clamp(20.0, 300.0);
            let index = drag.index;
            return captured(Message::GlobalTrack(GlobalTrackMessage::UpdateTempoEvent {
                index,
                bar,
                bpm,
            }));
        }
        // Automation breakpoint drag: x → time (clamped between neighbors),
        // y → value.
        if let Some(drag) = &state.breakpoint_drag {
            let (time_frames, value) =
                self.breakpoint_drag_to(drag.target.clone(), drag.index, pos)?;
            return captured(Message::Automation(AutomationMessage::DragBreakpoint {
                target: drag.target.clone(),
                index: drag.index,
                time_frames,
                value,
            }));
        }
        // Take-lane comping drag (todo #414). Publishes nothing: the
        // gesture is only resolved on release, so the move just carries
        // the pointer into the drag state and asks for a repaint of the
        // preview band. Keeping it silent also keeps a promote to exactly
        // one undo entry — the reducer never sees an intermediate range.
        if let Some(drag) = &mut state.take_promote_drag {
            drag.cursor_x = pos.x;
            drag.cursor_y = pos.y;
            return Some(canvas::Action::request_redraw().and_capture());
        }

        match &state.clip_interaction {
            Some(ClipInteraction::Move) => {
                captured(Message::Clip(ClipMessage::UpdateClipDrag(pos.x, pos.y)))
            }
            Some(ClipInteraction::Trim) => {
                captured(Message::Clip(ClipMessage::UpdateClipTrim(pos.x)))
            }
            Some(ClipInteraction::Fade) => {
                captured(Message::Clip(ClipMessage::UpdateClipFadeDrag(pos.x)))
            }
            Some(ClipInteraction::Gain) => {
                captured(Message::Clip(ClipMessage::UpdateClipGainDrag(pos.y)))
            }
            Some(ClipInteraction::MidiMove) => captured(Message::MidiClip(
                MidiClipMessage::UpdateMidiClipDrag(pos.x, pos.y),
            )),
            Some(ClipInteraction::MidiTrim) => captured(Message::MidiClip(
                MidiClipMessage::UpdateMidiClipTrim(pos.x),
            )),
            None => None,
        }
    }

    pub(in crate::view::timeline) fn handle_release(
        &self,
        state: &mut TimelineState,
    ) -> UpdateResult {
        // Releasing over the timeline while a browser drag is in flight
        // commits the placement (doc #175, todo #605). The resolved target
        // was stashed by the last Hover; the update handler reads it.
        if self.drag.is_some() {
            return captured(Message::Drag(DragMessage::Drop));
        }
        if state.h_scrollbar_grab.take().is_some() {
            return Some(canvas::Action::capture());
        }
        if state.v_scrollbar_grab.take().is_some() {
            return Some(canvas::Action::capture());
        }
        if state.dragging_loop {
            state.dragging_loop = false;
            return captured(Message::Transport(TransportMessage::EndLoopDrag));
        }
        if state.tempo_drag.take().is_some() {
            return captured(Message::GlobalTrack(GlobalTrackMessage::EndTempoDrag));
        }
        if state.breakpoint_drag.take().is_some() {
            return captured(Message::Automation(AutomationMessage::EndBreakpointDrag));
        }
        // Marker drag end: nothing to commit (each move already coalesces
        // into a single undo entry via the reducer), just drop the drag and
        // swallow the release so it doesn't fall through to other handlers.
        if state.marker_drag.take().is_some() {
            return Some(canvas::Action::capture());
        }
        // Take-lane comping (todo #414): the release is where the press
        // becomes a verb. Under the slop threshold it was a click, which
        // solos the take (or releases the solo if it was already the
        // active one); anything wider promotes the dragged range.
        //
        // The range emitted is the **raw** drag —
        // `update::takes::plan_promote` clamps it to the slot and to the
        // take's audible extent, and that clamp is the single place the
        // rule lives. Pre-clamping here would give two clamps free to
        // disagree.
        if let Some(drag) = state.take_promote_drag.take() {
            return if drag.is_promote() {
                captured(Message::Take(TakeMessage::PromoteTakeSegment {
                    group_id: drag.group_id,
                    take_id: drag.take_id,
                    range: self.take_drag_range(drag.anchor_x, drag.cursor_x),
                }))
            } else {
                captured(Message::Take(TakeMessage::SetActiveTake {
                    group_id: drag.group_id,
                    take_id: self.take_solo_toggle(drag.group_id, drag.take_id),
                }))
            };
        }

        if let Some(interaction) = state.clip_interaction.take() {
            return match interaction {
                ClipInteraction::Move => captured(Message::Clip(ClipMessage::EndClipDrag)),
                ClipInteraction::Trim => captured(Message::Clip(ClipMessage::EndClipTrim)),
                ClipInteraction::Fade => captured(Message::Clip(ClipMessage::EndClipFadeDrag)),
                ClipInteraction::Gain => captured(Message::Clip(ClipMessage::EndClipGainDrag)),
                ClipInteraction::MidiMove => {
                    captured(Message::MidiClip(MidiClipMessage::EndMidiClipDrag))
                }
                ClipInteraction::MidiTrim => {
                    captured(Message::MidiClip(MidiClipMessage::EndMidiClipTrim))
                }
            };
        }
        None
    }

}
