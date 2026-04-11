/// Timeline canvas rendering for the DAW arrangement view.
use std::time::Instant;

use iced::widget::canvas;
use iced::{keyboard, mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use crate::theme;
use crate::state::{ClipEdge, ClipState, LoopDragTarget, MidiClipState, TrackState};
use crate::message::Message;

use resonance_audio::types::{ClipId, TrackId, TICKS_PER_QUARTER_NOTE};

/// Maximum interval between two clicks to count as a double-click.
const DOUBLE_CLICK_MS: u128 = 400;

/// Thickness of the scrollbar strips drawn inside the timeline canvas.
const SCROLLBAR_THICKNESS: f32 = 10.0;
/// Minimum thumb size in pixels so the thumb stays clickable at any zoom.
const SCROLLBAR_MIN_THUMB: f32 = 24.0;

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
}

impl TimelineCanvas<'_> {
    /// Seconds per beat at current BPM.
    pub(crate) fn seconds_per_beat(&self) -> f32 {
        60.0 / self.bpm
    }

    /// Seconds per bar.
    pub(crate) fn seconds_per_bar(&self) -> f32 {
        self.seconds_per_beat() * self.time_sig_num as f32
    }

    /// Convert a sample position to pixel x coordinate.
    fn sample_to_x(&self, sample: u64) -> f32 {
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
        let samples_per_tick =
            (self.sample_rate as f64 * 60.0 / self.bpm as f64) / TICKS_PER_QUARTER_NOTE as f64;
        for c in self.midi_clips {
            let dur = (c.duration_ticks as f64 * samples_per_tick) as u64;
            let end = c.start_sample + dur;
            if end > max_sample {
                max_sample = end;
            }
        }
        let content = (max_sample as f64 / self.sample_rate as f64) as f32 * self.zoom;
        content.max(viewport_width * 1.5).max(viewport_width)
    }

    /// Total vertical content height (tracks + ruler). Excludes
    /// sub-tracks since the arrange view hides them entirely.
    pub(crate) fn content_height_px(&self) -> f32 {
        let visible = self
            .tracks
            .iter()
            .filter(|t| t.sub_track.is_none())
            .count();
        30.0 + visible as f32 * theme::TRACK_HEIGHT
    }

    /// Tracks visible in the arrange view, sorted by `order`. Excludes
    /// sub-tracks (rendered only in the mixer view).
    fn visible_tracks_sorted(&self) -> Vec<&TrackState> {
        let mut v: Vec<&TrackState> = self
            .tracks
            .iter()
            .filter(|t| t.sub_track.is_none())
            .collect();
        v.sort_by_key(|t| t.order);
        v
    }
}

/// Hit test for the horizontal scrollbar. Returns `(track_rect, thumb_rect)`
/// when the scrollbar is visible (content wider than the viewport).
fn h_scrollbar_rects(
    bounds_width: f32,
    bounds_height: f32,
    content_width: f32,
    scroll_offset: f32,
    show_v_bar: bool,
) -> Option<(Rectangle, Rectangle)> {
    if content_width <= bounds_width + 0.5 {
        return None;
    }
    let track_width = if show_v_bar {
        (bounds_width - SCROLLBAR_THICKNESS).max(0.0)
    } else {
        bounds_width
    };
    if track_width <= 0.0 {
        return None;
    }
    let track = Rectangle {
        x: 0.0,
        y: bounds_height - SCROLLBAR_THICKNESS,
        width: track_width,
        height: SCROLLBAR_THICKNESS,
    };
    let ratio_visible = (bounds_width / content_width).clamp(0.0, 1.0);
    let thumb_w = (track_width * ratio_visible).max(SCROLLBAR_MIN_THUMB);
    let max_scroll = (content_width - bounds_width).max(1.0);
    let travel = (track_width - thumb_w).max(0.0);
    let thumb_x = (scroll_offset / max_scroll).clamp(0.0, 1.0) * travel;
    let thumb = Rectangle {
        x: thumb_x,
        y: track.y,
        width: thumb_w,
        height: SCROLLBAR_THICKNESS,
    };
    Some((track, thumb))
}

/// Hit test for the vertical scrollbar (track area only, excludes ruler).
fn v_scrollbar_rects(
    bounds_width: f32,
    bounds_height: f32,
    content_height: f32,
    scroll_offset_y: f32,
    ruler_height: f32,
    show_h_bar: bool,
) -> Option<(Rectangle, Rectangle)> {
    let viewport_height = bounds_height - ruler_height
        - if show_h_bar { SCROLLBAR_THICKNESS } else { 0.0 };
    let track_content_h = content_height - ruler_height;
    if viewport_height <= 0.0 || track_content_h <= viewport_height + 0.5 {
        return None;
    }
    let track = Rectangle {
        x: bounds_width - SCROLLBAR_THICKNESS,
        y: ruler_height,
        width: SCROLLBAR_THICKNESS,
        height: viewport_height,
    };
    let ratio_visible = (viewport_height / track_content_h).clamp(0.0, 1.0);
    let thumb_h = (viewport_height * ratio_visible).max(SCROLLBAR_MIN_THUMB);
    let max_scroll = (track_content_h - viewport_height).max(1.0);
    let travel = (viewport_height - thumb_h).max(0.0);
    let thumb_y = ruler_height + (scroll_offset_y / max_scroll).clamp(0.0, 1.0) * travel;
    let thumb = Rectangle {
        x: track.x,
        y: thumb_y,
        width: SCROLLBAR_THICKNESS,
        height: thumb_h,
    };
    Some((track, thumb))
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
}

impl canvas::Program<Message> for TimelineCanvas<'_> {
    type State = TimelineState;

    fn update(
        &self,
        state: &mut Self::State,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        match event {
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                // Only handle wheel events when the cursor is actually over the
                // timeline — otherwise scrolling the piano roll would also
                // scroll the arrangement behind it.
                if cursor.position_in(bounds).is_none() {
                    return (canvas::event::Status::Ignored, None);
                }
                match delta {
                    mouse::ScrollDelta::Lines { x, y } => {
                        if x.abs() > f32::EPSILON {
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::ScrollX(-x * 30.0)),
                            );
                        }
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::ScrollY(-y * 30.0)),
                        );
                    }
                    mouse::ScrollDelta::Pixels { x, y } => {
                        if x.abs() > f32::EPSILON {
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::ScrollX(-x)),
                            );
                        }
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::ScrollY(-y)),
                        );
                    }
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    let ruler_height = 30.0;
                    let content_w = self.content_width_px(bounds.width);
                    let content_h = self.content_height_px();
                    let h_rects = h_scrollbar_rects(
                        bounds.width,
                        bounds.height,
                        content_w,
                        self.scroll_offset,
                        v_scrollbar_rects(
                            bounds.width,
                            bounds.height,
                            content_h,
                            self.scroll_offset_y,
                            ruler_height,
                            true,
                        )
                        .is_some(),
                    );
                    let v_rects = v_scrollbar_rects(
                        bounds.width,
                        bounds.height,
                        content_h,
                        self.scroll_offset_y,
                        ruler_height,
                        h_rects.is_some(),
                    );

                    // Horizontal scrollbar hit test.
                    if let Some((track, thumb)) = h_rects {
                        if rect_contains(&track, pos) {
                            if rect_contains(&thumb, pos) {
                                state.h_scrollbar_grab = Some(pos.x - thumb.x);
                            } else {
                                // Page-jump: center thumb on click position.
                                let travel = (track.width - thumb.width).max(0.0);
                                let max_scroll =
                                    (content_w - bounds.width).max(1.0);
                                let new_thumb_x =
                                    (pos.x - thumb.width / 2.0).clamp(0.0, travel);
                                let new_scroll = if travel > 0.0 {
                                    new_thumb_x / travel * max_scroll
                                } else {
                                    0.0
                                };
                                state.h_scrollbar_grab = Some(thumb.width / 2.0);
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::ScrollToX(new_scroll)),
                                );
                            }
                            return (canvas::event::Status::Captured, None);
                        }
                    }

                    // Vertical scrollbar hit test.
                    if let Some((track, thumb)) = v_rects {
                        if rect_contains(&track, pos) {
                            if rect_contains(&thumb, pos) {
                                state.v_scrollbar_grab = Some(pos.y - thumb.y);
                            } else {
                                let travel = (track.height - thumb.height).max(0.0);
                                let max_scroll =
                                    (content_h - ruler_height
                                        - (track.height))
                                        .max(1.0);
                                let new_thumb_y =
                                    (pos.y - ruler_height - thumb.height / 2.0)
                                        .clamp(0.0, travel);
                                let new_scroll = if travel > 0.0 {
                                    new_thumb_y / travel * max_scroll
                                } else {
                                    0.0
                                };
                                state.v_scrollbar_grab = Some(thumb.height / 2.0);
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::ScrollToY(new_scroll)),
                                );
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
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::StartLoopDrag(target)),
                            );
                        }
                    }

                    // Any other click in the ruler → seek the playhead.
                    // Punch handle drags above have already captured their
                    // own clicks, so we only get here on empty ruler space.
                    if pos.y < ruler_height {
                        let seconds =
                            ((pos.x + self.scroll_offset) / self.zoom).max(0.0);
                        let sample =
                            (seconds as f64 * self.sample_rate as f64) as u64;
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::SeekToSample(sample)),
                        );
                    }

                    // Clip hit-testing (track area)
                    if pos.y >= ruler_height {
                        let sorted_tracks = self.visible_tracks_sorted();

                        // Check MIDI clips (reverse order so topmost wins)
                        for clip in self.midi_clips.iter().rev() {
                            let track_idx = sorted_tracks.iter().position(|t| t.id == clip.track_id);
                            let track_idx = match track_idx {
                                Some(i) => i,
                                None => continue,
                            };
                            let cy = ruler_height + track_idx as f32 * theme::TRACK_HEIGHT + 2.0
                                - self.scroll_offset_y;
                            let clip_height = theme::TRACK_HEIGHT - 4.0;
                            let samples_per_tick = (self.sample_rate as f64 * 60.0 / self.bpm as f64)
                                / TICKS_PER_QUARTER_NOTE as f64;
                            let duration_samples = clip.duration_ticks as f64 * samples_per_tick;
                            let start_seconds = clip.start_sample as f32 / self.sample_rate as f32;
                            let duration_seconds = duration_samples as f32 / self.sample_rate as f32;
                            let cx = start_seconds * self.zoom - self.scroll_offset;
                            let cw = duration_seconds * self.zoom;

                            if pos.x >= cx && pos.x <= cx + cw && pos.y >= cy && pos.y <= cy + clip_height {
                                // Double-click on a MIDI clip body opens the piano roll editor.
                                let now = Instant::now();
                                let is_double_click = state
                                    .last_midi_click
                                    .map(|(t, id)| {
                                        id == clip.id
                                            && now.duration_since(t).as_millis() <= DOUBLE_CLICK_MS
                                    })
                                    .unwrap_or(false);
                                state.last_midi_click = Some((now, clip.id));
                                if is_double_click {
                                    state.last_midi_click = None;
                                    return (
                                        canvas::event::Status::Captured,
                                        Some(Message::OpenMidiEditor(clip.id)),
                                    );
                                }

                                let edge_threshold = theme::CLIP_EDGE_THRESHOLD;
                                if pos.x - cx < edge_threshold {
                                    state.clip_interaction = Some(ClipInteraction::MidiTrim {
                                        clip_id: clip.id,
                                        edge: ClipEdge::Left,
                                    });
                                    return (
                                        canvas::event::Status::Captured,
                                        Some(Message::StartMidiClipTrim {
                                            clip_id: clip.id,
                                            edge: ClipEdge::Left,
                                            anchor_x: pos.x,
                                        }),
                                    );
                                }
                                if (cx + cw) - pos.x < edge_threshold {
                                    state.clip_interaction = Some(ClipInteraction::MidiTrim {
                                        clip_id: clip.id,
                                        edge: ClipEdge::Right,
                                    });
                                    return (
                                        canvas::event::Status::Captured,
                                        Some(Message::StartMidiClipTrim {
                                            clip_id: clip.id,
                                            edge: ClipEdge::Right,
                                            anchor_x: pos.x,
                                        }),
                                    );
                                }
                                let grab_offset = pos.x - cx;
                                state.clip_interaction = Some(ClipInteraction::MidiMove {
                                    clip_id: clip.id,
                                    grab_offset_x: grab_offset,
                                });
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::StartMidiClipDrag {
                                        clip_id: clip.id,
                                        grab_offset_x: grab_offset,
                                        start_x: pos.x,
                                        start_y: pos.y,
                                    }),
                                );
                            }
                        }

                        // Check audio clips in reverse order so topmost clip wins
                        for clip in self.clips.iter().rev() {
                            let track_idx = sorted_tracks.iter().position(|t| t.id == clip.track_id);
                            let track_idx = match track_idx {
                                Some(i) => i,
                                None => continue,
                            };
                            let cy = ruler_height + track_idx as f32 * theme::TRACK_HEIGHT + 2.0
                                - self.scroll_offset_y;
                            let clip_height = theme::TRACK_HEIGHT - 4.0;
                            let start_seconds = clip.start_sample as f32 / self.sample_rate as f32;
                            let duration_seconds =
                                clip.duration_samples as f32 / self.sample_rate as f32;
                            let cx = start_seconds * self.zoom - self.scroll_offset;
                            let cw = duration_seconds * self.zoom;

                            if pos.x >= cx && pos.x <= cx + cw && pos.y >= cy && pos.y <= cy + clip_height {
                                let edge_threshold = theme::CLIP_EDGE_THRESHOLD;
                                // Left edge trim
                                if pos.x - cx < edge_threshold {
                                    state.clip_interaction = Some(ClipInteraction::Trim {
                                        clip_id: clip.id,
                                        edge: ClipEdge::Left,
                                    });
                                    return (
                                        canvas::event::Status::Captured,
                                        Some(Message::StartClipTrim {
                                            clip_id: clip.id,
                                            edge: ClipEdge::Left,
                                            anchor_x: pos.x,
                                        }),
                                    );
                                }
                                // Right edge trim
                                if (cx + cw) - pos.x < edge_threshold {
                                    state.clip_interaction = Some(ClipInteraction::Trim {
                                        clip_id: clip.id,
                                        edge: ClipEdge::Right,
                                    });
                                    return (
                                        canvas::event::Status::Captured,
                                        Some(Message::StartClipTrim {
                                            clip_id: clip.id,
                                            edge: ClipEdge::Right,
                                            anchor_x: pos.x,
                                        }),
                                    );
                                }
                                // Body click → start move drag
                                let grab_offset = pos.x - cx;
                                state.clip_interaction = Some(ClipInteraction::Move {
                                    clip_id: clip.id,
                                    grab_offset_x: grab_offset,
                                });
                                return (
                                    canvas::event::Status::Captured,
                                    Some(Message::StartClipDrag {
                                        clip_id: clip.id,
                                        grab_offset_x: grab_offset,
                                        start_x: pos.x,
                                        start_y: pos.y,
                                    }),
                                );
                            }
                        }
                        // Clicked on empty space → deselect
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::SelectClip(None)),
                        );
                    }
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some(pos) = cursor.position_in(bounds) {
                    // Scrollbar drag updates.
                    if let Some(grab) = state.h_scrollbar_grab {
                        let content_w = self.content_width_px(bounds.width);
                        let show_v = v_scrollbar_rects(
                            bounds.width,
                            bounds.height,
                            self.content_height_px(),
                            self.scroll_offset_y,
                            30.0,
                            true,
                        )
                        .is_some();
                        if let Some((track, thumb)) = h_scrollbar_rects(
                            bounds.width,
                            bounds.height,
                            content_w,
                            self.scroll_offset,
                            show_v,
                        ) {
                            let new_thumb_x =
                                (pos.x - grab).clamp(0.0, track.width - thumb.width);
                            let travel = (track.width - thumb.width).max(0.0);
                            let max_scroll = (content_w - bounds.width).max(1.0);
                            let new_scroll = if travel > 0.0 {
                                new_thumb_x / travel * max_scroll
                            } else {
                                0.0
                            };
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::ScrollToX(new_scroll)),
                            );
                        }
                    }
                    if let Some(grab) = state.v_scrollbar_grab {
                        let content_h = self.content_height_px();
                        let show_h = h_scrollbar_rects(
                            bounds.width,
                            bounds.height,
                            self.content_width_px(bounds.width),
                            self.scroll_offset,
                            true,
                        )
                        .is_some();
                        if let Some((track, thumb)) = v_scrollbar_rects(
                            bounds.width,
                            bounds.height,
                            content_h,
                            self.scroll_offset_y,
                            30.0,
                            show_h,
                        ) {
                            let rel_y = pos.y - track.y - grab;
                            let travel = (track.height - thumb.height).max(0.0);
                            let new_thumb_rel =
                                rel_y.clamp(0.0, travel);
                            let max_scroll =
                                (content_h - 30.0 - track.height).max(1.0);
                            let new_scroll = if travel > 0.0 {
                                new_thumb_rel / travel * max_scroll
                            } else {
                                0.0
                            };
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::ScrollToY(new_scroll)),
                            );
                        }
                    }

                    if state.dragging_loop {
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::UpdateLoopDrag(pos.x)),
                        );
                    }
                    match &state.clip_interaction {
                        Some(ClipInteraction::Move { .. }) => {
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::UpdateClipDrag(pos.x, pos.y)),
                            );
                        }
                        Some(ClipInteraction::Trim { .. }) => {
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::UpdateClipTrim(pos.x)),
                            );
                        }
                        Some(ClipInteraction::MidiMove { .. }) => {
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::UpdateMidiClipDrag(pos.x, pos.y)),
                            );
                        }
                        Some(ClipInteraction::MidiTrim { .. }) => {
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::UpdateMidiClipTrim(pos.x)),
                            );
                        }
                        None => {}
                    }
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.h_scrollbar_grab.is_some() {
                    state.h_scrollbar_grab = None;
                    return (canvas::event::Status::Captured, None);
                }
                if state.v_scrollbar_grab.is_some() {
                    state.v_scrollbar_grab = None;
                    return (canvas::event::Status::Captured, None);
                }
                if state.dragging_loop {
                    state.dragging_loop = false;
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::EndLoopDrag),
                    );
                }
                if let Some(interaction) = state.clip_interaction.take() {
                    return match interaction {
                        ClipInteraction::Move { .. } => (
                            canvas::event::Status::Captured,
                            Some(Message::EndClipDrag),
                        ),
                        ClipInteraction::Trim { .. } => (
                            canvas::event::Status::Captured,
                            Some(Message::EndClipTrim),
                        ),
                        ClipInteraction::MidiMove { .. } => (
                            canvas::event::Status::Captured,
                            Some(Message::EndMidiClipDrag),
                        ),
                        ClipInteraction::MidiTrim { .. } => (
                            canvas::event::Status::Captured,
                            Some(Message::EndMidiClipTrim),
                        ),
                    };
                }
            }
            canvas::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Delete),
                ..
            })
            | canvas::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Backspace),
                ..
            }) => {
                if let Some(clip_id) = self.selected_midi_clip {
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::DeleteMidiClip(clip_id)),
                    );
                }
                if let Some(clip_id) = self.selected_clip {
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::DeleteClip(clip_id)),
                    );
                }
            }
            _ => {}
        }
        // Report viewport width changes so the app can use it for auto-scroll
        if (bounds.width - state.last_reported_width).abs() > 1.0 {
            state.last_reported_width = bounds.width;
            return (
                canvas::event::Status::Ignored,
                Some(Message::ViewportWidth(bounds.width)),
            );
        }
        // Report content size so scroll clamping and scrollbar sizing stays
        // in sync with the clips on the timeline.
        let cw = self.content_width_px(bounds.width);
        let ch = self.content_height_px();
        if (cw - state.last_reported_content_width).abs() > 1.0
            || (ch - state.last_reported_content_height).abs() > 1.0
        {
            state.last_reported_content_width = cw;
            state.last_reported_content_height = ch;
            return (
                canvas::event::Status::Ignored,
                Some(Message::TimelineContentSize(cw, ch)),
            );
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
        let ruler_height = 30.0;
        let y_off = self.scroll_offset_y;

        // Draw ruler background
        frame.fill_rectangle(
            Point::new(0.0, 0.0),
            Size::new(bounds.width, ruler_height),
            theme::RULER_BG,
        );

        // Draw track backgrounds. Only non-sub-tracks are rendered; the
        // mixer view is where sub-track lanes live.
        let sorted_tracks = self.visible_tracks_sorted();
        let track_area_height = sorted_tracks.len() as f32 * theme::TRACK_HEIGHT;

        for (i, track) in sorted_tracks.iter().enumerate() {
            let y = ruler_height + i as f32 * theme::TRACK_HEIGHT - y_off;

            // Skip tracks entirely above or below the visible area
            if y + theme::TRACK_HEIGHT < ruler_height || y > bounds.height {
                continue;
            }

            let bg = if i % 2 == 0 {
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
        self.draw_grid_lines(&mut frame, bounds.width, ruler_height, track_area_height, y_off);

        // Draw bar/beat ruler
        self.draw_ruler(&mut frame, bounds.width, ruler_height);

        // Draw audio clips
        for clip in self.clips {
            self.draw_clip(&mut frame, clip, &sorted_tracks, ruler_height, y_off, bounds.height);
        }

        // Draw MIDI clips
        for clip in self.midi_clips {
            self.draw_midi_clip(&mut frame, clip, &sorted_tracks, ruler_height, y_off, bounds.height);
        }

        // Draw loop in/out markers
        if self.loop_enabled {
            let loop_in_x = self.sample_to_x(self.loop_in);
            let loop_out_x = self.sample_to_x(self.loop_out);
            let total_height = (ruler_height + track_area_height - y_off).max(bounds.height);
            let loop_color = theme::LOOP_MARKER;

            // Dim overlay outside loop range (over track area only)
            if loop_in_x > 0.0 {
                frame.fill_rectangle(
                    Point::new(0.0, ruler_height),
                    Size::new(loop_in_x.min(bounds.width), total_height - ruler_height),
                    Color::from_rgba(0.0, 0.0, 0.0, 0.15),
                );
            }
            if loop_out_x < bounds.width {
                let right_start = loop_out_x.max(0.0);
                frame.fill_rectangle(
                    Point::new(right_start, ruler_height),
                    Size::new(
                        (bounds.width - right_start).max(0.0),
                        total_height - ruler_height,
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
            let total_height = (ruler_height + track_area_height - y_off).max(bounds.height);

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
        let content_w = self.content_width_px(bounds.width);
        let content_h = self.content_height_px();
        let h_pre = h_scrollbar_rects(
            bounds.width,
            bounds.height,
            content_w,
            self.scroll_offset,
            false,
        );
        let v_rects = v_scrollbar_rects(
            bounds.width,
            bounds.height,
            content_h,
            self.scroll_offset_y,
            ruler_height,
            h_pre.is_some(),
        );
        let h_rects = h_scrollbar_rects(
            bounds.width,
            bounds.height,
            content_w,
            self.scroll_offset,
            v_rects.is_some(),
        );

        let track_color = Color::from_rgba(0.08, 0.08, 0.08, 0.8);
        let thumb_color = Color::from_rgba(0.45, 0.45, 0.45, 0.85);

        if let Some((track, thumb)) = h_rects {
            frame.fill_rectangle(
                Point::new(track.x, track.y),
                Size::new(track.width, track.height),
                track_color,
            );
            frame.fill_rectangle(
                Point::new(thumb.x, thumb.y),
                Size::new(thumb.width, thumb.height),
                thumb_color,
            );
        }
        if let Some((track, thumb)) = v_rects {
            frame.fill_rectangle(
                Point::new(track.x, track.y),
                Size::new(track.width, track.height),
                track_color,
            );
            frame.fill_rectangle(
                Point::new(thumb.x, thumb.y),
                Size::new(thumb.width, thumb.height),
                thumb_color,
            );
        }

        vec![frame.into_geometry()]
    }
}

