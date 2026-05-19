//! Canvas event handling for the timeline. These are the impl methods
//! that take a `&mut TimelineState`, hit-test against the canvas
//! geometry, and translate pointer / wheel / keyboard events into
//! `Message`s. Drawing lives in `timeline_draw.rs`; this file is the
//! input counterpart.

use std::time::Instant;

use iced::widget::canvas;
use iced::{keyboard, mouse, Point, Rectangle};

use resonance_audio::types::{bpm_at_bar, TrackId};

use crate::message::*;
use crate::state::{self, TrackState};
use crate::theme;
use crate::timeline::{TimelineCanvas, TimelineState};
use crate::timeline::hit_test::{self, track_index, HitKind};
use crate::timeline::scrollbar::{scroll_from_thumb_pos, ScrollbarRects};
use crate::timeline_snap::snap_sample_to_grid_tempo;

/// Maximum interval between two clicks to count as a double-click.
pub(super) const DOUBLE_CLICK_MS: u128 = 400;

/// Which part of a clip is being dragged. The drag state itself lives in
/// the per-clip `*DragState` / `*TrimState` structs on `Resonance`; this
/// enum exists only to remember which of those is currently active so the
/// pointer-move and pointer-release handlers dispatch to the right end-drag
/// message.
#[derive(Debug, Clone)]
pub(super) enum ClipInteraction {
    Move,
    Trim,
    MidiMove,
    MidiTrim,
}

/// Active drag on a tempo event point.
#[derive(Debug)]
pub(super) struct TempoDrag {
    /// Index into `tempo_events` at drag start.
    pub index: usize,
    /// Original BPM of the dragged event.
    pub original_bpm: f32,
    /// Mouse y at drag start.
    pub anchor_y: f32,
}

pub(super) type UpdateResult = Option<canvas::Action<Message>>;

pub(super) fn captured(msg: Message) -> UpdateResult {
    Some(canvas::Action::publish(msg).and_capture())
}

/// Is `pos` inside `rect`?
fn rect_contains(rect: &Rectangle, pos: Point) -> bool {
    pos.x >= rect.x
        && pos.x <= rect.x + rect.width
        && pos.y >= rect.y
        && pos.y <= rect.y + rect.height
}

impl TimelineCanvas<'_> {
    /// Returns both scrollbar rects with each bar's visibility informed by
    /// the other (the vertical bar's track shrinks when the horizontal bar
    /// is shown, and vice versa).
    pub(super) fn scrollbar_rects(
        &self,
        bounds: Rectangle,
    ) -> (Option<ScrollbarRects>, Option<ScrollbarRects>) {
        use crate::timeline::scrollbar;
        // Horizontal scroll is now owned by the outer `Scrollable` that
        // wraps the timeline canvas (see `view_timeline`), so we no
        // longer draw an in-canvas horizontal scrollbar. The vertical
        // bar stays — tracks scroll inside the canvas so the ruler
        // / section band / global-tracks header line up with their lanes.
        let content_h = self.content_height_px();
        let header_h = self.fixed_header_height();
        let v = scrollbar::v_rects(
            bounds,
            content_h,
            self.scroll_offset_y,
            header_h,
            false,
        );
        (None, v)
    }

    pub(super) fn handle_wheel(
        &self,
        delta: mouse::ScrollDelta,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> UpdateResult {
        // Only handle wheel events when the cursor is actually over the
        // timeline — otherwise scrolling the piano roll would also scroll
        // the arrangement behind it.
        if cursor.position_in(bounds).is_none() {
            return None;
        }
        // Horizontal scroll is owned by the outer `Scrollable` that
        // wraps the timeline canvas — returning `Ignored` for any
        // wheel-X delta lets the event bubble up so the scrollable can
        // handle it natively. Vertical scroll stays inside the canvas
        // because the track lanes scroll in lockstep with the ruler /
        // section band / global-track header (vertical scrollbar
        // drawing + drag handling live in the canvas too).
        match delta {
            mouse::ScrollDelta::Lines { x, y } => {
                if x.abs() > f32::EPSILON {
                    return None;
                }
                captured(Message::Viewport(ViewportMessage::ScrollY(-y * 30.0)))
            }
            mouse::ScrollDelta::Pixels { x, y } => {
                if x.abs() > f32::EPSILON {
                    return None;
                }
                captured(Message::Viewport(ViewportMessage::ScrollY(-y)))
            }
        }
    }

    /// Hit-test a pointer press against a clip lane (MIDI or audio).
    /// `duration_samples` is already tick-converted for MIDI clips.
    fn hit_test_lane(
        &self,
        pos: Point,
        sorted_tracks: &[&TrackState],
        clip_track_id: TrackId,
        clip_start_sample: u64,
        duration_samples: u64,
    ) -> Option<HitKind> {
        let header_height = self.fixed_header_height();
        let track_idx = track_index(sorted_tracks, clip_track_id)?;
        let row_y = hit_test::track_row_y(
            track_idx,
            header_height,
            self.scroll_offset_y,
            theme::TRACK_HEIGHT,
        );
        let rect = hit_test::clip_rect(
            row_y,
            theme::TRACK_HEIGHT,
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

    pub(super) fn handle_press(
        &self,
        state: &mut TimelineState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> UpdateResult {
        let Some(pos) = cursor.position_in(bounds) else {
            return None;
        };
        let ruler_height = theme::RULER_HEIGHT;
        let header_height = self.fixed_header_height();
        let (h_rects, v_rects) = self.scrollbar_rects(bounds);

        // Horizontal scrollbar hit test.
        if let Some(sb) = h_rects {
            if rect_contains(&sb.track, pos) {
                if rect_contains(&sb.thumb, pos) {
                    state.h_scrollbar_grab = Some(pos.x - sb.thumb.x);
                } else {
                    // Page-jump: center thumb on click position.
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

        // Vertical scrollbar hit test.
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

        // Loop marker dragging (ruler area only)
        if self.loop_enabled && pos.y < ruler_height {
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

        // Any other click in the ruler → seek the playhead, snapped
        // to the nearest grid line.
        if pos.y < ruler_height {
            let seconds = ((pos.x + self.scroll_offset) / self.zoom).max(0.0);
            let sample = (seconds as f64 * self.sample_rate as f64) as u64;
            let snapped = snap_sample_to_grid_tempo(
                sample,
                self.bpm,
                self.time_sig_num,
                self.sample_rate,
                self.zoom,
                self.tempo_map,
            );
            return captured(Message::Transport(TransportMessage::SeekToSample(snapped)));
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
        let shelf_header_bottom =
            shelf_top + theme::GLOBAL_SHELF_HEADER_HEIGHT;

        if pos.y >= shelf_top && pos.y < shelf_header_bottom {
            return captured(Message::Ui(UiMessage::ToggleGlobalTracks));
        }
        if pos.y >= shelf_header_bottom
            && pos.y < header_height
            && self.global_tracks_expanded
        {
            return self.handle_global_track_click(state, pos, bounds);
        }

        // Clip hit-testing (track area)
        let sorted_tracks = self.visible_tracks_sorted();

        // Check MIDI clips (reverse order so topmost wins)
        for clip in self.midi_clips.iter().rev() {
            let clip_end = self.tempo_map.tick_to_abs_sample(
                clip.start_sample,
                clip.duration_ticks,
                self.sample_rate,
            );
            let duration_samples = clip_end.saturating_sub(clip.start_sample);
            let Some(hit) = self.hit_test_lane(
                pos,
                &sorted_tracks,
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
                HitKind::Miss => unreachable!("None path taken above"),
            };
        }

        // Check audio clips in reverse order so topmost clip wins
        for clip in self.clips.iter().rev() {
            let Some(hit) = self.hit_test_lane(
                pos,
                &sorted_tracks,
                clip.track_id,
                clip.start_sample,
                clip.duration_samples,
            ) else {
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

        // Clicked on empty track area → select the track under the cursor
        // and deselect any active clip selection.
        let clicked_track = {
            let track_idx = ((pos.y - header_height + self.scroll_offset_y) / theme::TRACK_HEIGHT)
                .floor()
                .max(0.0) as usize;
            sorted_tracks.get(track_idx).map(|t| t.id)
        };
        captured(Message::Ui(UiMessage::SelectTrack(clicked_track)))
    }

    pub(super) fn handle_move(
        &self,
        state: &mut TimelineState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> UpdateResult {
        let Some(pos) = cursor.position_in(bounds) else {
            return None;
        };

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
        match &state.clip_interaction {
            Some(ClipInteraction::Move) => {
                captured(Message::Clip(ClipMessage::UpdateClipDrag(pos.x, pos.y)))
            }
            Some(ClipInteraction::Trim) => {
                captured(Message::Clip(ClipMessage::UpdateClipTrim(pos.x)))
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

    pub(super) fn handle_release(&self, state: &mut TimelineState) -> UpdateResult {
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
        if let Some(interaction) = state.clip_interaction.take() {
            return match interaction {
                ClipInteraction::Move => captured(Message::Clip(ClipMessage::EndClipDrag)),
                ClipInteraction::Trim => captured(Message::Clip(ClipMessage::EndClipTrim)),
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

    /// Compute BPM range for the tempo row graph (matches draw code).
    fn tempo_bpm_range(&self) -> (f32, f32) {
        let mut min_bpm = f32::MAX;
        let mut max_bpm = f32::MIN;
        for e in &self.tempo_map.tempo_points {
            min_bpm = min_bpm.min(e.bpm);
            max_bpm = max_bpm.max(e.bpm);
        }
        let range = (max_bpm - min_bpm).max(10.0);
        let pad = range * 0.15;
        (min_bpm - pad, max_bpm + pad)
    }

    /// Map a pixel x-position to a bar number using the tempo map.
    fn x_to_bar(&self, x: f32) -> u32 {
        let seconds = ((x + self.scroll_offset) / self.zoom).max(0.0);
        let sample = (seconds as f64 * self.sample_rate as f64) as u64;
        let (bar, frac) = self.tempo_map.sample_to_bar(sample, self.sample_rate);
        if frac >= 0.5 {
            bar + 1
        } else {
            bar
        }
    }

    /// Handle a click in the global-tracks lane area (below the shelf
    /// header). Single click on a tempo point starts a drag; double-click
    /// on empty space in the tempo lane adds a new event. Clicks in the
    /// chord lane are passive for now (the chord lane is read-only,
    /// driven by the compose sections).
    fn handle_global_track_click(
        &self,
        state: &mut TimelineState,
        pos: Point,
        _bounds: Rectangle,
    ) -> UpdateResult {
        let ruler_height = theme::RULER_HEIGHT;
        let band_h = self.section_band_height();
        let shelf_header_h = theme::GLOBAL_SHELF_HEADER_HEIGHT;
        let chord_h = theme::GLOBAL_TRACK_CHORD_HEIGHT;
        let tempo_h = theme::GLOBAL_TRACK_TEMPO_HEIGHT;
        let sig_h = theme::GLOBAL_TRACK_SIG_HEIGHT;

        let chord_top = ruler_height + band_h + shelf_header_h;
        let tempo_top = chord_top + chord_h;
        let sig_top = tempo_top + tempo_h;

        // Maintained for the rest of the function: `row_h` is the tempo
        // row height (BPM mapping math assumes that's the live lane).
        let row_h = tempo_h;

        let in_tempo = pos.y >= tempo_top && pos.y < tempo_top + tempo_h;
        let in_sig = pos.y >= sig_top && pos.y < sig_top + sig_h;
        let _in_chord = pos.y >= chord_top && pos.y < chord_top + chord_h;

        let bar = self.x_to_bar(pos.x);

        // Check if click is near an existing tempo event point.
        // For step changes (two events at same bar), pick the closest by
        // y-distance so both points are individually draggable.
        if in_tempo {
            let (lo, hi) = self.tempo_bpm_range();
            let graph_top = tempo_top + 3.0;
            let graph_bot = tempo_top + row_h - 3.0;
            let graph_h = graph_bot - graph_top;

            let mut best: Option<(usize, f32)> = None; // (index, distance²)
            for (i, event) in self.tempo_map.tempo_points.iter().enumerate() {
                let sample = self.tempo_map.bar_to_sample(event.bar);
                let ex = self.sample_to_x(sample);
                let ey = graph_bot - ((event.bpm - lo) / (hi - lo)) * graph_h;
                let dx = pos.x - ex;
                let dy = pos.y - ey;
                let dist2 = dx * dx + dy * dy;
                if dx.abs() < 10.0
                    && dy.abs() < 12.0
                    && best.is_none_or(|(_, d)| dist2 < d)
                {
                    best = Some((i, dist2));
                }
            }
            if let Some((i, _)) = best {
                state.tempo_drag = Some(TempoDrag {
                    index: i,
                    original_bpm: self.tempo_map.tempo_points[i].bpm,
                    anchor_y: pos.y,
                });
                return captured(Message::GlobalTrack(GlobalTrackMessage::StartTempoDrag(i)));
            }
            // Double-click detection for adding new tempo events.
            let now = std::time::Instant::now();
            let is_double = state
                .last_global_click
                .map(|(t, k)| {
                    k == state::GlobalTrackKind::Tempo
                        && now.duration_since(t).as_millis() <= DOUBLE_CLICK_MS
                })
                .unwrap_or(false);
            state.last_global_click = Some((now, state::GlobalTrackKind::Tempo));
            if is_double {
                state.last_global_click = None;
                // Add at the interpolated BPM for this bar so the point
                // appears on the current line.
                let bpm = bpm_at_bar(bar as f64, &self.tempo_map.tempo_points) as f32;
                return captured(Message::GlobalTrack(GlobalTrackMessage::AddTempoEvent {
                    bar,
                    bpm,
                }));
            }
            // Single click on empty space → deselect.
            return captured(Message::GlobalTrack(GlobalTrackMessage::SelectEvent(None)));
        }

        if in_sig {
            for (i, event) in self.tempo_map.signature_points.iter().enumerate() {
                let sample = self.tempo_map.bar_to_sample(event.bar);
                let ex = self.sample_to_x(sample);
                if (pos.x - ex).abs() < 8.0 {
                    return captured(Message::GlobalTrack(GlobalTrackMessage::SelectEvent(Some(
                        state::SelectedGlobalEvent {
                            kind: state::GlobalTrackKind::Signature,
                            index: i,
                        },
                    ))));
                }
            }
            let now = std::time::Instant::now();
            let is_double = state
                .last_global_click
                .map(|(t, k)| {
                    k == state::GlobalTrackKind::Signature
                        && now.duration_since(t).as_millis() <= DOUBLE_CLICK_MS
                })
                .unwrap_or(false);
            state.last_global_click = Some((now, state::GlobalTrackKind::Signature));
            if is_double {
                state.last_global_click = None;
                return captured(Message::GlobalTrack(
                    GlobalTrackMessage::AddSignatureEvent {
                        bar,
                        numerator: self.time_sig_num,
                        denominator: 4,
                    },
                ));
            }
            return captured(Message::GlobalTrack(GlobalTrackMessage::SelectEvent(None)));
        }

        Some(canvas::Action::capture())
    }

    pub(super) fn handle_key(&self, key: &keyboard::Key) -> UpdateResult {
        use keyboard::key::Named;
        let is_delete = matches!(
            key,
            keyboard::Key::Named(Named::Delete) | keyboard::Key::Named(Named::Backspace)
        );
        if !is_delete {
            return None;
        }
        // Delete selected global track event.
        if self.selected_global_event.is_some() {
            return captured(Message::GlobalTrack(
                GlobalTrackMessage::DeleteSelectedEvent,
            ));
        }
        if let Some(clip_id) = self.selected_midi_clip {
            return captured(Message::MidiClip(MidiClipMessage::DeleteMidiClip(clip_id)));
        }
        if let Some(clip_id) = self.selected_clip {
            return captured(Message::Clip(ClipMessage::DeleteClip(clip_id)));
        }
        None
    }

    /// Emit `ViewportWidth` / `TimelineContentSize` messages when either
    /// value has moved enough to be worth pushing upstream. Called at the
    /// tail of every event the canvas sees.
    pub(super) fn report_viewport(
        &self,
        state: &mut TimelineState,
        bounds: Rectangle,
    ) -> Option<Message> {
        if (bounds.width - state.last_reported_width).abs() > 1.0 {
            state.last_reported_width = bounds.width;
            return Some(Message::Viewport(ViewportMessage::ViewportWidth(
                bounds.width,
            )));
        }
        let cw = self.content_width_px(bounds.width);
        let ch = self.content_height_px();
        if (cw - state.last_reported_content_width).abs() > 1.0
            || (ch - state.last_reported_content_height).abs() > 1.0
        {
            state.last_reported_content_width = cw;
            state.last_reported_content_height = ch;
            return Some(Message::Viewport(ViewportMessage::TimelineContentSize(
                cw, ch,
            )));
        }
        None
    }
}
