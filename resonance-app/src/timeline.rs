/// Timeline canvas rendering for the DAW arrangement view.
use std::time::Instant;

use iced::widget::canvas;
use iced::{keyboard, mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use crate::theme;
use crate::state::{self, ClipEdge, ClipState, LoopDragTarget, MidiClipState, TrackState};
use crate::message::*;

use resonance_audio::types::{ClipId, TempoMap, TrackId, bpm_at_bar};

pub mod hit_test;
pub mod scrollbar;

use hit_test::{HitKind, track_index};
use scrollbar::{ScrollbarRects, scroll_from_thumb_pos};

/// Maximum interval between two clicks to count as a double-click.
const DOUBLE_CLICK_MS: u128 = 400;

/// Snap a sample position to the nearest bar or beat boundary,
/// accounting for the tempo map. At high zoom (bar wider than 40 px)
/// snaps to beats; lower zoom snaps to bars.
pub fn snap_sample_to_grid(
    sample: u64,
    bpm: f32,
    time_sig_num: u8,
    sample_rate: u32,
    zoom: f32,
) -> u64 {
    let tm = TempoMap::default();
    snap_sample_to_grid_tempo(sample, bpm, time_sig_num, sample_rate, zoom, &tm)
}

/// Tempo-map-aware version of `snap_sample_to_grid`. Uses the shared
/// `TempoMap` for bar boundary computation.
pub fn snap_sample_to_grid_tempo(
    sample: u64,
    bpm: f32,
    time_sig_num: u8,
    sample_rate: u32,
    zoom: f32,
    tempo_map: &TempoMap,
) -> u64 {
    if bpm <= 0.0 || time_sig_num == 0 || zoom <= 0.0 {
        return sample;
    }
    // When there's no meaningful tempo map, use the flat-BPM path.
    if tempo_map.tempo_points.len() <= 1 {
        let samples_per_beat = sample_rate as f64 * 60.0 / bpm as f64;
        let samples_per_bar = samples_per_beat * time_sig_num as f64;
        let bar_pixel_width = (samples_per_bar / sample_rate as f64) as f32 * zoom;
        let step = if bar_pixel_width >= 40.0 {
            samples_per_beat
        } else if bar_pixel_width >= 20.0 {
            samples_per_bar
        } else {
            samples_per_bar * (20.0 / bar_pixel_width).ceil() as f64
        };
        if step <= 0.0 {
            return sample;
        }
        return ((sample as f64 / step).round() * step).round() as u64;
    }

    // Tempo map is active: find which bar we're in and snap to the
    // nearest bar or beat boundary.
    let (bar, frac) = tempo_map.sample_to_bar(sample, sample_rate);

    // Determine snap resolution from the local bar pixel width.
    let local_bpm = bpm_at_bar(bar as f64, &tempo_map.tempo_points);
    let cur_num = tempo_map.numerator_at_bar(bar);
    let spb = sample_rate as f64 * 60.0 / local_bpm;
    let bar_samples = spb * cur_num as f64;
    let bar_px = (bar_samples / sample_rate as f64) as f32 * zoom;

    let snap_to_beats = bar_px >= 40.0;

    if snap_to_beats {
        // Snap to the nearest beat within the bar.
        let beat_frac = frac * cur_num as f64;
        let nearest_beat = beat_frac.round() as u32;
        if nearest_beat >= cur_num as u32 {
            // Snaps to start of next bar
            tempo_map.bar_to_sample(bar + 1)
        } else {
            // Snaps to beat within this bar
            let bar_start = tempo_map.bar_to_sample(bar);
            let bar_end = tempo_map.bar_to_sample(bar + 1);
            let total = (bar_end - bar_start) as f64;
            let beat_frac_pos = nearest_beat as f64 / cur_num as f64;
            bar_start + (beat_frac_pos * total) as u64
        }
    } else {
        // Snap to the nearest bar.
        let nearest_bar = if frac >= 0.5 { bar + 1 } else { bar };
        tempo_map.bar_to_sample(nearest_bar)
    }
}

/// Data passed to the timeline canvas for rendering.
#[derive(Debug)]
pub struct TimelineCanvas<'a> {
    pub tracks: &'a [TrackState],
    pub clips: &'a [ClipState],
    pub playhead: u64,
    pub sample_rate: u32,
    pub zoom: f32,
    pub scroll_offset: f32,
    pub recording_tracks: Vec<TrackId>,
    pub recording_start_sample: u64,
    pub bpm: f32,
    pub time_sig_num: u8,
    pub scroll_offset_y: f32,
    pub loop_enabled: bool,
    pub loop_in: u64,
    pub loop_out: u64,
    pub selected_clip: Option<ClipId>,
    pub midi_clips: &'a [MidiClipState],
    pub selected_midi_clip: Option<ClipId>,
    pub selected_track: Option<TrackId>,
    pub global_tracks_expanded: bool,
    pub tempo_map: &'a TempoMap,
    pub selected_global_event: Option<crate::state::SelectedGlobalEvent>,
}

impl TimelineCanvas<'_> {
    /// Height of the global tracks area (tempo + time signature rows).
    /// Returns 0.0 when collapsed.
    pub(crate) fn global_tracks_height(&self) -> f32 {
        if self.global_tracks_expanded {
            2.0 * theme::GLOBAL_TRACK_ROW_HEIGHT
        } else {
            0.0
        }
    }

    /// Total fixed header height: ruler + global tracks area.
    /// This is the Y offset where regular track rows begin.
    pub(crate) fn fixed_header_height(&self) -> f32 {
        theme::RULER_HEIGHT + self.global_tracks_height()
    }

    /// Convert a sample position to pixel x coordinate.
    pub(crate) fn sample_to_x(&self, sample: u64) -> f32 {
        (sample as f64 / self.sample_rate as f64) as f32 * self.zoom - self.scroll_offset
    }

    /// Rightmost pixel needed to show all content (clips + MIDI clips).
    /// Always returns at least `viewport_width * 1.5` so users can scroll a
    /// bit past the last clip, and never less than `viewport_width` itself.
    pub(crate) fn content_width_px(&self, viewport_width: f32) -> f32 {
        let mut max_sample: u64 = 0;
        for c in self.clips {
            let end = c.start_sample + c.duration_samples;
            if end > max_sample {
                max_sample = end;
            }
        }
        for c in self.midi_clips {
            let end = self.tempo_map.tick_to_abs_sample(
                c.start_sample, c.duration_ticks, self.sample_rate,
            );
            if end > max_sample {
                max_sample = end;
            }
        }
        let content = (max_sample as f64 / self.sample_rate as f64) as f32 * self.zoom;
        content.max(viewport_width * 1.5).max(viewport_width)
    }

    /// Total vertical content height (tracks + ruler + global tracks).
    /// Excludes sub-tracks since the arrange view hides them entirely.
    pub(crate) fn content_height_px(&self) -> f32 {
        let visible = self
            .tracks
            .iter()
            .filter(|t| t.sub_track.is_none())
            .count();
        self.fixed_header_height() + visible as f32 * theme::TRACK_HEIGHT
    }

    /// Tracks visible in the arrange view, sorted by `order`. Excludes
    /// sub-tracks (rendered only in the mixer view).
    fn visible_tracks_sorted(&self) -> Vec<&TrackState> {
        hit_test::sorted_arrange_tracks(self.tracks)
    }
}

/// Is `pos` inside `rect`?
fn rect_contains(rect: &Rectangle, pos: Point) -> bool {
    pos.x >= rect.x
        && pos.x <= rect.x + rect.width
        && pos.y >= rect.y
        && pos.y <= rect.y + rect.height
}

/// Which part of a clip is being dragged.
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum ClipInteraction {
    Move { clip_id: ClipId, grab_offset_x: f32 },
    Trim { clip_id: ClipId, edge: ClipEdge },
    MidiMove { clip_id: ClipId, grab_offset_x: f32 },
    MidiTrim { clip_id: ClipId, edge: ClipEdge },
}

/// Active drag on a tempo event point.
#[derive(Debug)]
struct TempoDrag {
    /// Index into `tempo_events` at drag start.
    index: usize,
    /// Original BPM of the dragged event.
    original_bpm: f32,
    /// Mouse y at drag start.
    anchor_y: f32,
}

/// Local state for the timeline canvas, tracking active drag operations.
#[derive(Debug, Default)]
pub struct TimelineState {
    dragging_loop: bool,
    clip_interaction: Option<ClipInteraction>,
    last_reported_width: f32,
    last_reported_content_width: f32,
    last_reported_content_height: f32,
    /// Tracks the most recent click on a MIDI clip for double-click detection.
    last_midi_click: Option<(Instant, ClipId)>,
    /// Horizontal scrollbar drag in progress. Stores the x-offset of the
    /// grab point relative to the left edge of the thumb (in track pixels).
    h_scrollbar_grab: Option<f32>,
    /// Vertical scrollbar drag in progress (y-offset within the thumb).
    v_scrollbar_grab: Option<f32>,
    /// Tracks the most recent click on a global track for double-click detection.
    last_global_click: Option<(Instant, state::GlobalTrackKind)>,
    /// Active tempo-event drag.
    tempo_drag: Option<TempoDrag>,
}

type UpdateResult = (canvas::event::Status, Option<Message>);

fn captured(msg: Message) -> UpdateResult {
    (canvas::event::Status::Captured, Some(msg))
}

impl TimelineCanvas<'_> {
    /// Returns both scrollbar rects with each bar's visibility informed by
    /// the other (the vertical bar's track shrinks when the horizontal bar
    /// is shown, and vice versa).
    fn scrollbar_rects(
        &self,
        bounds: Rectangle,
    ) -> (Option<ScrollbarRects>, Option<ScrollbarRects>) {
        let content_w = self.content_width_px(bounds.width);
        let content_h = self.content_height_px();
        let header_h = self.fixed_header_height();
        let h = scrollbar::h_rects(
            bounds,
            content_w,
            self.scroll_offset,
            scrollbar::v_rects(
                bounds,
                content_h,
                self.scroll_offset_y,
                header_h,
                true,
            )
            .is_some(),
        );
        let v = scrollbar::v_rects(
            bounds,
            content_h,
            self.scroll_offset_y,
            header_h,
            h.is_some(),
        );
        (h, v)
    }

    fn handle_wheel(
        &self,
        delta: mouse::ScrollDelta,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> UpdateResult {
        // Only handle wheel events when the cursor is actually over the
        // timeline — otherwise scrolling the piano roll would also scroll
        // the arrangement behind it.
        if cursor.position_in(bounds).is_none() {
            return (canvas::event::Status::Ignored, None);
        }
        match delta {
            mouse::ScrollDelta::Lines { x, y } => {
                if x.abs() > f32::EPSILON {
                    return captured(Message::Viewport(ViewportMessage::ScrollX(-x * 30.0)));
                }
                captured(Message::Viewport(ViewportMessage::ScrollY(-y * 30.0)))
            }
            mouse::ScrollDelta::Pixels { x, y } => {
                if x.abs() > f32::EPSILON {
                    return captured(Message::Viewport(ViewportMessage::ScrollX(-x)));
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

    fn handle_press(
        &self,
        state: &mut TimelineState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> UpdateResult {
        let Some(pos) = cursor.position_in(bounds) else {
            return (canvas::event::Status::Ignored, None);
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
                return (canvas::event::Status::Captured, None);
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
                return (canvas::event::Status::Captured, None);
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
                        LoopDragTarget::In
                    } else {
                        LoopDragTarget::Out
                    }
                } else if dist_in < 8.0 {
                    LoopDragTarget::In
                } else {
                    LoopDragTarget::Out
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

        // Clicks in the global tracks area (between ruler and track lanes).
        if pos.y >= ruler_height && pos.y < header_height && self.global_tracks_expanded {
            return self.handle_global_track_click(state, pos, bounds);
        }

        // Clip hit-testing (track area)
        let sorted_tracks = self.visible_tracks_sorted();

        // Check MIDI clips (reverse order so topmost wins)
        for clip in self.midi_clips.iter().rev() {
            let clip_end = self.tempo_map.tick_to_abs_sample(
                clip.start_sample, clip.duration_ticks, self.sample_rate,
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
                return captured(Message::MidiEditor(MidiEditorMessage::OpenMidiEditor(clip.id)));
            }

            return match hit {
                HitKind::Trim(edge) => {
                    state.clip_interaction =
                        Some(ClipInteraction::MidiTrim { clip_id: clip.id, edge });
                    captured(Message::MidiClip(MidiClipMessage::StartMidiClipTrim {
                        clip_id: clip.id,
                        edge,
                        anchor_x: pos.x,
                    }))
                }
                HitKind::Move { grab_offset_x } => {
                    state.clip_interaction = Some(ClipInteraction::MidiMove {
                        clip_id: clip.id,
                        grab_offset_x,
                    });
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
                    state.clip_interaction =
                        Some(ClipInteraction::Trim { clip_id: clip.id, edge });
                    captured(Message::Clip(ClipMessage::StartClipTrim {
                        clip_id: clip.id,
                        edge,
                        anchor_x: pos.x,
                    }))
                }
                HitKind::Move { grab_offset_x } => {
                    state.clip_interaction = Some(ClipInteraction::Move {
                        clip_id: clip.id,
                        grab_offset_x,
                    });
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

    fn handle_move(
        &self,
        state: &mut TimelineState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> UpdateResult {
        let Some(pos) = cursor.position_in(bounds) else {
            return (canvas::event::Status::Ignored, None);
        };

        // Horizontal scrollbar drag.
        if let Some(grab) = state.h_scrollbar_grab {
            let (h_rects, _) = self.scrollbar_rects(bounds);
            if let Some(sb) = h_rects {
                let new_scroll =
                    scroll_from_thumb_pos(pos.x - grab, sb.travel, sb.max_scroll);
                return captured(Message::Viewport(ViewportMessage::ScrollToX(new_scroll)));
            }
        }
        // Vertical scrollbar drag.
        if let Some(grab) = state.v_scrollbar_grab {
            let (_, v_rects) = self.scrollbar_rects(bounds);
            if let Some(sb) = v_rects {
                let new_scroll = scroll_from_thumb_pos(
                    pos.y - sb.track.y - grab,
                    sb.travel,
                    sb.max_scroll,
                );
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
            Some(ClipInteraction::Move { .. }) => captured(Message::Clip(ClipMessage::UpdateClipDrag(pos.x, pos.y))),
            Some(ClipInteraction::Trim { .. }) => captured(Message::Clip(ClipMessage::UpdateClipTrim(pos.x))),
            Some(ClipInteraction::MidiMove { .. }) => {
                captured(Message::MidiClip(MidiClipMessage::UpdateMidiClipDrag(pos.x, pos.y)))
            }
            Some(ClipInteraction::MidiTrim { .. }) => captured(Message::MidiClip(MidiClipMessage::UpdateMidiClipTrim(pos.x))),
            None => {
                (canvas::event::Status::Ignored, None)
            }
        }
    }

    fn handle_release(&self, state: &mut TimelineState) -> UpdateResult {
        if state.h_scrollbar_grab.take().is_some() {
            return (canvas::event::Status::Captured, None);
        }
        if state.v_scrollbar_grab.take().is_some() {
            return (canvas::event::Status::Captured, None);
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
                ClipInteraction::Move { .. } => captured(Message::Clip(ClipMessage::EndClipDrag)),
                ClipInteraction::Trim { .. } => captured(Message::Clip(ClipMessage::EndClipTrim)),
                ClipInteraction::MidiMove { .. } => captured(Message::MidiClip(MidiClipMessage::EndMidiClipDrag)),
                ClipInteraction::MidiTrim { .. } => captured(Message::MidiClip(MidiClipMessage::EndMidiClipTrim)),
            };
        }
        (canvas::event::Status::Ignored, None)
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
        if frac >= 0.5 { bar + 1 } else { bar }
    }

    /// Handle a click in the global tracks area. Single click on a tempo
    /// point starts a drag; double-click on empty space adds a new event.
    fn handle_global_track_click(
        &self,
        state: &mut TimelineState,
        pos: Point,
        _bounds: Rectangle,
    ) -> UpdateResult {
        let ruler_height = theme::RULER_HEIGHT;
        let row_h = theme::GLOBAL_TRACK_ROW_HEIGHT;
        let in_tempo = pos.y >= ruler_height && pos.y < ruler_height + row_h;
        let in_sig = pos.y >= ruler_height + row_h && pos.y < ruler_height + 2.0 * row_h;

        let bar = self.x_to_bar(pos.x);

        // Check if click is near an existing tempo event point.
        // For step changes (two events at same bar), pick the closest by
        // y-distance so both points are individually draggable.
        if in_tempo {
            let (lo, hi) = self.tempo_bpm_range();
            let graph_top = ruler_height + 3.0;
            let graph_bot = ruler_height + row_h - 3.0;
            let graph_h = graph_bot - graph_top;

            let mut best: Option<(usize, f32)> = None; // (index, distance²)
            for (i, event) in self.tempo_map.tempo_points.iter().enumerate() {
                let sample = self.tempo_map.bar_to_sample(event.bar);
                let ex = self.sample_to_x(sample);
                let ey = graph_bot - ((event.bpm - lo) / (hi - lo)) * graph_h;
                let dx = pos.x - ex;
                let dy = pos.y - ey;
                let dist2 = dx * dx + dy * dy;
                if dx.abs() < 10.0 && dy.abs() < 12.0 {
                    if best.map_or(true, |(_, d)| dist2 < d) {
                        best = Some((i, dist2));
                    }
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
            let is_double = state.last_global_click
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
                    return captured(Message::GlobalTrack(GlobalTrackMessage::SelectEvent(
                        Some(state::SelectedGlobalEvent {
                            kind: state::GlobalTrackKind::Signature,
                            index: i,
                        }),
                    )));
                }
            }
            let now = std::time::Instant::now();
            let is_double = state.last_global_click
                .map(|(t, k)| {
                    k == state::GlobalTrackKind::Signature
                        && now.duration_since(t).as_millis() <= DOUBLE_CLICK_MS
                })
                .unwrap_or(false);
            state.last_global_click = Some((now, state::GlobalTrackKind::Signature));
            if is_double {
                state.last_global_click = None;
                return captured(Message::GlobalTrack(GlobalTrackMessage::AddSignatureEvent {
                    bar,
                    numerator: self.time_sig_num,
                    denominator: 4,
                }));
            }
            return captured(Message::GlobalTrack(GlobalTrackMessage::SelectEvent(None)));
        }

        (canvas::event::Status::Captured, None)
    }

    fn handle_key(&self, key: &keyboard::Key) -> UpdateResult {
        use keyboard::key::Named;
        let is_delete = matches!(
            key,
            keyboard::Key::Named(Named::Delete) | keyboard::Key::Named(Named::Backspace)
        );
        if !is_delete {
            return (canvas::event::Status::Ignored, None);
        }
        // Delete selected global track event.
        if self.selected_global_event.is_some() {
            return captured(Message::GlobalTrack(GlobalTrackMessage::DeleteSelectedEvent));
        }
        if let Some(clip_id) = self.selected_midi_clip {
            return captured(Message::MidiClip(MidiClipMessage::DeleteMidiClip(clip_id)));
        }
        if let Some(clip_id) = self.selected_clip {
            return captured(Message::Clip(ClipMessage::DeleteClip(clip_id)));
        }
        (canvas::event::Status::Ignored, None)
    }

    /// Emit `ViewportWidth` / `TimelineContentSize` messages when either
    /// value has moved enough to be worth pushing upstream. Called at the
    /// tail of every event the canvas sees.
    fn report_viewport(&self, state: &mut TimelineState, bounds: Rectangle) -> Option<Message> {
        if (bounds.width - state.last_reported_width).abs() > 1.0 {
            state.last_reported_width = bounds.width;
            return Some(Message::Viewport(ViewportMessage::ViewportWidth(bounds.width)));
        }
        let cw = self.content_width_px(bounds.width);
        let ch = self.content_height_px();
        if (cw - state.last_reported_content_width).abs() > 1.0
            || (ch - state.last_reported_content_height).abs() > 1.0
        {
            state.last_reported_content_width = cw;
            state.last_reported_content_height = ch;
            return Some(Message::Viewport(ViewportMessage::TimelineContentSize(cw, ch)));
        }
        None
    }
}

impl canvas::Program<Message> for TimelineCanvas<'_> {
    type State = TimelineState;

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> UpdateResult {
        let result = match event {
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                self.handle_wheel(delta, bounds, cursor)
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                self.handle_press(state, bounds, cursor)
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                self.handle_move(state, bounds, cursor)
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                self.handle_release(state)
            }
            canvas::Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => {
                self.handle_key(&key)
            }
            _ => (canvas::event::Status::Ignored, None),
        };
        if let (_, Some(_)) = result {
            return result;
        }
        if result.0 == canvas::event::Status::Captured {
            return result;
        }
        if let Some(msg) = self.report_viewport(state, bounds) {
            return (canvas::event::Status::Ignored, Some(msg));
        }
        (canvas::event::Status::Ignored, None)
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let ruler_height = theme::RULER_HEIGHT;
        let header_height = self.fixed_header_height();
        let y_off = self.scroll_offset_y;

        // Draw ruler background
        frame.fill_rectangle(
            Point::new(0.0, 0.0),
            Size::new(bounds.width, ruler_height),
            theme::RULER_BG,
        );

        // Draw global tracks area (tempo + time signature) between ruler and tracks
        self.draw_global_tracks(&mut frame, bounds.width, ruler_height);

        // Draw track backgrounds. Only non-sub-tracks are rendered; the
        // mixer view is where sub-track lanes live.
        let sorted_tracks = self.visible_tracks_sorted();
        let track_area_height = sorted_tracks.len() as f32 * theme::TRACK_HEIGHT;

        for (i, track) in sorted_tracks.iter().enumerate() {
            let y = header_height + i as f32 * theme::TRACK_HEIGHT - y_off;

            // Skip tracks entirely above or below the visible area
            if y + theme::TRACK_HEIGHT < header_height || y > bounds.height {
                continue;
            }

            let is_selected = self.selected_track == Some(track.id);
            let bg = if is_selected {
                theme::PANEL_SELECTED
            } else if i % 2 == 0 {
                theme::BG
            } else {
                theme::PANEL_DARK
            };
            frame.fill_rectangle(
                Point::new(0.0, y),
                Size::new(bounds.width, theme::TRACK_HEIGHT),
                bg,
            );

            // Recording overlay on armed tracks
            if self.recording_tracks.contains(&track.id) {
                let (overlay_start, overlay_end) = if self.loop_enabled {
                    (self.loop_in, self.playhead.min(self.loop_out))
                } else {
                    (self.recording_start_sample, self.playhead)
                };
                let start_x = self.sample_to_x(overlay_start);
                let end_x = self.sample_to_x(overlay_end);
                let overlay_x = start_x.max(0.0);
                let overlay_w = (end_x - overlay_x).max(0.0).min(bounds.width - overlay_x);
                if overlay_w > 0.0 {
                    frame.fill_rectangle(
                        Point::new(overlay_x, y),
                        Size::new(overlay_w, theme::TRACK_HEIGHT),
                        Color::from_rgba(0.8, 0.2, 0.2, 0.08),
                    );
                }
            }

            // Track separator line
            frame.fill_rectangle(
                Point::new(0.0, y + theme::TRACK_HEIGHT - 1.0),
                Size::new(bounds.width, 1.0),
                theme::TRACK_LINE,
            );
        }

        // Draw bar/beat grid lines through track area
        self.draw_grid_lines(&mut frame, bounds.width, header_height, track_area_height, y_off);

        // Draw bar/beat ruler
        self.draw_ruler(&mut frame, bounds.width, ruler_height);

        // Draw audio clips
        for clip in self.clips {
            self.draw_clip(&mut frame, clip, &sorted_tracks, header_height, y_off, bounds.height);
        }

        // Draw MIDI clips
        for clip in self.midi_clips {
            self.draw_midi_clip(&mut frame, clip, &sorted_tracks, header_height, y_off, bounds.height);
        }

        // Draw loop in/out markers
        if self.loop_enabled {
            let loop_in_x = self.sample_to_x(self.loop_in);
            let loop_out_x = self.sample_to_x(self.loop_out);
            let total_height = (header_height + track_area_height - y_off).max(bounds.height);
            let loop_color = theme::LOOP_MARKER;

            // Dim overlay outside loop range (over track area only)
            if loop_in_x > 0.0 {
                frame.fill_rectangle(
                    Point::new(0.0, header_height),
                    Size::new(loop_in_x.min(bounds.width), total_height - header_height),
                    Color::from_rgba(0.0, 0.0, 0.0, 0.15),
                );
            }
            if loop_out_x < bounds.width {
                let right_start = loop_out_x.max(0.0);
                frame.fill_rectangle(
                    Point::new(right_start, header_height),
                    Size::new(
                        (bounds.width - right_start).max(0.0),
                        total_height - header_height,
                    ),
                    Color::from_rgba(0.0, 0.0, 0.0, 0.15),
                );
            }

            // Amber range fill in ruler area
            let range_x = loop_in_x.max(0.0);
            let range_w = (loop_out_x - range_x).max(0.0).min(bounds.width - range_x);
            if range_w > 0.0 {
                frame.fill_rectangle(
                    Point::new(range_x, 0.0),
                    Size::new(range_w, ruler_height),
                    Color::from_rgba(0.9, 0.72, 0.1, 0.15),
                );
            }

            // Loop In line + handle
            if loop_in_x >= -1.0 && loop_in_x <= bounds.width + 1.0 {
                frame.fill_rectangle(
                    Point::new(loop_in_x - 0.5, 0.0),
                    Size::new(1.0, total_height),
                    loop_color,
                );
                let tri = canvas::Path::new(|b| {
                    b.move_to(Point::new(loop_in_x - 6.0, 0.0));
                    b.line_to(Point::new(loop_in_x + 6.0, 0.0));
                    b.line_to(Point::new(loop_in_x, 8.0));
                    b.close();
                });
                frame.fill(&tri, loop_color);
            }

            // Loop Out line + handle
            if loop_out_x >= -1.0 && loop_out_x <= bounds.width + 1.0 {
                frame.fill_rectangle(
                    Point::new(loop_out_x - 0.5, 0.0),
                    Size::new(1.0, total_height),
                    loop_color,
                );
                let tri = canvas::Path::new(|b| {
                    b.move_to(Point::new(loop_out_x - 6.0, 0.0));
                    b.line_to(Point::new(loop_out_x + 6.0, 0.0));
                    b.line_to(Point::new(loop_out_x, 8.0));
                    b.close();
                });
                frame.fill(&tri, loop_color);
            }
        }

        // Draw playhead
        let playhead_seconds = (self.playhead as f64 / self.sample_rate as f64) as f32;
        let playhead_x = playhead_seconds * self.zoom - self.scroll_offset;
        if playhead_x >= 0.0 && playhead_x <= bounds.width {
            let total_height = (header_height + track_area_height - y_off).max(bounds.height);

            // Playhead triangle at top
            let triangle = canvas::Path::new(|builder| {
                builder.move_to(Point::new(playhead_x - 6.0, 0.0));
                builder.line_to(Point::new(playhead_x + 6.0, 0.0));
                builder.line_to(Point::new(playhead_x, 8.0));
                builder.close();
            });
            frame.fill(&triangle, theme::ACCENT);

            // Playhead line
            frame.fill_rectangle(
                Point::new(playhead_x - 0.5, 0.0),
                Size::new(1.0, total_height),
                theme::ACCENT,
            );
        }

        // Smart scrollbars — drawn last so they sit above clips + playhead.
        let (h_rects, v_rects) = self.scrollbar_rects(bounds);

        let track_color = Color::from_rgba(0.08, 0.08, 0.08, 0.8);
        let thumb_color = Color::from_rgba(0.45, 0.45, 0.45, 0.85);

        if let Some(sb) = h_rects {
            frame.fill_rectangle(
                Point::new(sb.track.x, sb.track.y),
                Size::new(sb.track.width, sb.track.height),
                track_color,
            );
            frame.fill_rectangle(
                Point::new(sb.thumb.x, sb.thumb.y),
                Size::new(sb.thumb.width, sb.thumb.height),
                thumb_color,
            );
        }
        if let Some(sb) = v_rects {
            frame.fill_rectangle(
                Point::new(sb.track.x, sb.track.y),
                Size::new(sb.track.width, sb.track.height),
                track_color,
            );
            frame.fill_rectangle(
                Point::new(sb.thumb.x, sb.thumb.y),
                Size::new(sb.thumb.width, sb.thumb.height),
                thumb_color,
            );
        }

        vec![frame.into_geometry()]
    }
}

