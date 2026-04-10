/// Timeline canvas rendering for the DAW arrangement view.
use std::time::Instant;

use iced::widget::canvas;
use iced::{keyboard, mouse, Color, Point, Rectangle, Renderer, Size, Theme};

use crate::theme;
use crate::state::{ClipEdge, ClipState, MidiClipState, PunchDragTarget, TrackState};
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
    pub punch_enabled: bool,
    pub punch_in: u64,
    pub punch_out: u64,
    pub selected_clip: Option<ClipId>,
    pub midi_clips: &'a [MidiClipState],
    pub selected_midi_clip: Option<ClipId>,
}

impl TimelineCanvas<'_> {
    /// Seconds per beat at current BPM.
    fn seconds_per_beat(&self) -> f32 {
        60.0 / self.bpm
    }

    /// Seconds per bar.
    fn seconds_per_bar(&self) -> f32 {
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

    /// Total vertical content height (tracks + ruler).
    pub(crate) fn content_height_px(&self) -> f32 {
        30.0 + self.tracks.len() as f32 * theme::TRACK_HEIGHT
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
    dragging_punch: bool,
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

                    // Punch marker dragging (ruler area only)
                    if self.punch_enabled && pos.y < ruler_height {
                        let punch_in_x = self.sample_to_x(self.punch_in);
                        let punch_out_x = self.sample_to_x(self.punch_out);
                        let dist_in = (pos.x - punch_in_x).abs();
                        let dist_out = (pos.x - punch_out_x).abs();
                        if dist_in < 8.0 || dist_out < 8.0 {
                            let target = if dist_in < 8.0 && dist_out < 8.0 {
                                if dist_in < dist_out {
                                    PunchDragTarget::In
                                } else {
                                    PunchDragTarget::Out
                                }
                            } else if dist_in < 8.0 {
                                PunchDragTarget::In
                            } else {
                                PunchDragTarget::Out
                            };
                            state.dragging_punch = true;
                            return (
                                canvas::event::Status::Captured,
                                Some(Message::StartPunchDrag(target)),
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
                        let mut sorted_tracks: Vec<&TrackState> = self.tracks.iter().collect();
                        sorted_tracks.sort_by_key(|t| t.order);

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

                                let edge_threshold = 6.0;
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
                                let edge_threshold = 6.0;
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

                    if state.dragging_punch {
                        return (
                            canvas::event::Status::Captured,
                            Some(Message::UpdatePunchDrag(pos.x)),
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
                if state.dragging_punch {
                    state.dragging_punch = false;
                    return (
                        canvas::event::Status::Captured,
                        Some(Message::EndPunchDrag),
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

        // Draw track backgrounds
        let mut sorted_tracks: Vec<&TrackState> = self.tracks.iter().collect();
        sorted_tracks.sort_by_key(|t| t.order);
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
                let (overlay_start, overlay_end) = if self.punch_enabled {
                    (self.punch_in, self.playhead.min(self.punch_out))
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

        // Draw punch in/out markers
        if self.punch_enabled {
            let punch_in_x = self.sample_to_x(self.punch_in);
            let punch_out_x = self.sample_to_x(self.punch_out);
            let total_height = (ruler_height + track_area_height - y_off).max(bounds.height);
            let punch_color = theme::PUNCH_MARKER;

            // Dim overlay outside punch range (over track area only)
            if punch_in_x > 0.0 {
                frame.fill_rectangle(
                    Point::new(0.0, ruler_height),
                    Size::new(punch_in_x.min(bounds.width), total_height - ruler_height),
                    Color::from_rgba(0.0, 0.0, 0.0, 0.15),
                );
            }
            if punch_out_x < bounds.width {
                let right_start = punch_out_x.max(0.0);
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
            let range_x = punch_in_x.max(0.0);
            let range_w = (punch_out_x - range_x).max(0.0).min(bounds.width - range_x);
            if range_w > 0.0 {
                frame.fill_rectangle(
                    Point::new(range_x, 0.0),
                    Size::new(range_w, ruler_height),
                    Color::from_rgba(0.9, 0.72, 0.1, 0.15),
                );
            }

            // Punch In line + handle
            if punch_in_x >= -1.0 && punch_in_x <= bounds.width + 1.0 {
                frame.fill_rectangle(
                    Point::new(punch_in_x - 0.5, 0.0),
                    Size::new(1.0, total_height),
                    punch_color,
                );
                let tri = canvas::Path::new(|b| {
                    b.move_to(Point::new(punch_in_x - 6.0, 0.0));
                    b.line_to(Point::new(punch_in_x + 6.0, 0.0));
                    b.line_to(Point::new(punch_in_x, 8.0));
                    b.close();
                });
                frame.fill(&tri, punch_color);
            }

            // Punch Out line + handle
            if punch_out_x >= -1.0 && punch_out_x <= bounds.width + 1.0 {
                frame.fill_rectangle(
                    Point::new(punch_out_x - 0.5, 0.0),
                    Size::new(1.0, total_height),
                    punch_color,
                );
                let tri = canvas::Path::new(|b| {
                    b.move_to(Point::new(punch_out_x - 6.0, 0.0));
                    b.line_to(Point::new(punch_out_x + 6.0, 0.0));
                    b.line_to(Point::new(punch_out_x, 8.0));
                    b.close();
                });
                frame.fill(&tri, punch_color);
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

impl TimelineCanvas<'_> {
    /// Draw vertical bar and beat grid lines in the track area.
    fn draw_grid_lines(
        &self,
        frame: &mut canvas::Frame,
        width: f32,
        ruler_height: f32,
        track_area_height: f32,
        _y_off: f32,
    ) {
        let spb_seconds = self.seconds_per_beat();
        let spbar_seconds = self.seconds_per_bar();
        let line_height = track_area_height.max(600.0);

        let start_time = self.scroll_offset / self.zoom;
        let end_time = start_time + width / self.zoom;

        // Determine bar step for readability at low zoom
        let bar_pixel_width = spbar_seconds * self.zoom;
        let bar_step = if bar_pixel_width < 20.0 {
            (20.0 / bar_pixel_width).ceil() as u32
        } else {
            1
        };

        let first_bar = (start_time / spbar_seconds).floor() as i64;
        let last_bar = (end_time / spbar_seconds).ceil() as i64;

        for bar_idx in first_bar..=last_bar {
            if bar_step > 1 && bar_idx.rem_euclid(bar_step as i64) != 0 {
                continue;
            }
            let bar_time = bar_idx as f32 * spbar_seconds;

            // Bar line
            let x = bar_time * self.zoom - self.scroll_offset;
            if x >= -1.0 && x <= width + 1.0 {
                frame.fill_rectangle(
                    Point::new(x, ruler_height),
                    Size::new(1.0, line_height),
                    theme::BAR_LINE,
                );
            }

            // Beat lines within this bar (skip beat 1, that's the bar line)
            if bar_pixel_width >= 40.0 {
                for beat in 1..self.time_sig_num {
                    let beat_time = bar_time + beat as f32 * spb_seconds;
                    let bx = beat_time * self.zoom - self.scroll_offset;
                    if bx >= 0.0 && bx <= width {
                        frame.fill_rectangle(
                            Point::new(bx, ruler_height),
                            Size::new(1.0, line_height),
                            theme::BEAT_LINE,
                        );
                    }
                }
            }
        }
    }

    /// Draw the bar/beat ruler at the top.
    fn draw_ruler(&self, frame: &mut canvas::Frame, width: f32, ruler_height: f32) {
        let spbar_seconds = self.seconds_per_bar();
        let spb_seconds = self.seconds_per_beat();

        let start_time = self.scroll_offset / self.zoom;
        let end_time = start_time + width / self.zoom;

        // Determine bar step for readability at low zoom
        let bar_pixel_width = spbar_seconds * self.zoom;
        let bar_step = if bar_pixel_width < 40.0 {
            (40.0 / bar_pixel_width).ceil() as u32
        } else {
            1
        };

        let first_bar = (start_time / spbar_seconds).floor() as i64;
        let last_bar = (end_time / spbar_seconds).ceil() as i64;

        for bar_idx in first_bar..=last_bar {
            let bar_time = bar_idx as f32 * spbar_seconds;
            let bar_number = bar_idx + 1; // 1-based

            if bar_step > 1 && bar_idx.rem_euclid(bar_step as i64) != 0 {
                continue;
            }

            let x = bar_time * self.zoom - self.scroll_offset;

            if x < -1.0 || x > width + 1.0 {
                continue;
            }

            // Major tick (bar)
            frame.fill_rectangle(
                Point::new(x, ruler_height - 12.0),
                Size::new(1.0, 12.0),
                theme::TEXT_DIM,
            );

            // Bar number label
            frame.fill_text(canvas::Text {
                content: format!("{}", bar_number),
                position: Point::new(x + 3.0, ruler_height - 24.0),
                color: theme::TEXT_DIM,
                size: 11.0.into(),
                ..canvas::Text::default()
            });

            // Beat ticks within bar (only if enough space)
            if bar_pixel_width >= 40.0 {
                for beat in 1..self.time_sig_num {
                    let beat_time = bar_time + beat as f32 * spb_seconds;
                    let bx = beat_time * self.zoom - self.scroll_offset;
                    if bx >= 0.0 && bx <= width {
                        frame.fill_rectangle(
                            Point::new(bx, ruler_height - 6.0),
                            Size::new(1.0, 6.0),
                            Color::from_rgb(0.25, 0.25, 0.25),
                        );
                    }
                }
            }
        }

        // Ruler bottom line
        frame.fill_rectangle(
            Point::new(0.0, ruler_height - 1.0),
            Size::new(width, 1.0),
            theme::SEPARATOR,
        );
    }

    fn draw_clip(
        &self,
        frame: &mut canvas::Frame,
        clip: &ClipState,
        sorted_tracks: &[&TrackState],
        ruler_height: f32,
        y_off: f32,
        visible_height: f32,
    ) {
        let track_index = sorted_tracks
            .iter()
            .position(|t| t.id == clip.track_id);

        let track_index = match track_index {
            Some(i) => i,
            None => return,
        };

        let y = ruler_height + track_index as f32 * theme::TRACK_HEIGHT + 2.0 - y_off;

        // Skip clips on tracks outside visible area
        if y + theme::TRACK_HEIGHT < ruler_height || y > visible_height {
            return;
        }
        let clip_height = theme::TRACK_HEIGHT - 4.0;

        let start_seconds = clip.start_sample as f32 / self.sample_rate as f32;
        let duration_seconds = clip.duration_samples as f32 / self.sample_rate as f32;

        let x = start_seconds * self.zoom - self.scroll_offset;
        let w = duration_seconds * self.zoom;

        // Clip body
        frame.fill_rectangle(
            Point::new(x, y),
            Size::new(w, clip_height),
            theme::CLIP_BODY,
        );

        // Waveform rendering
        let header_height = 18.0;
        if !clip.waveform_peaks.is_empty() {
            let wave_y = y + header_height;
            let wave_h = clip_height - header_height;
            let wave_center = wave_y + wave_h * 0.5;

            let peak_frames = resonance_audio::types::WAVEFORM_PEAK_FRAMES as f32;
            let seconds_per_peak = peak_frames / self.sample_rate as f32;
            let pixels_per_peak = seconds_per_peak * self.zoom;

            // Determine which peaks are visible (accounting for trim)
            let trim_start_peaks = clip.trim_start_frames as f32 / peak_frames;
            let _total_visible_peaks =
                clip.duration_samples as f32 / peak_frames;

            let waveform_color = Color::from_rgba(0.7, 0.85, 1.0, 0.5);

            let start_px = (-x).max(0.0);
            let mut px = start_px;
            while px < w {
                let peak_idx_f = trim_start_peaks + px / pixels_per_peak;
                let peak_idx = peak_idx_f as usize;
                if peak_idx >= clip.waveform_peaks.len() {
                    break;
                }
                let (min_val, max_val) = clip.waveform_peaks[peak_idx];

                let draw_x = x + px;
                // Only draw if on-screen
                if draw_x + pixels_per_peak >= 0.0 && draw_x <= w + x {
                    let top = wave_center - max_val * wave_h * 0.5;
                    let bottom = wave_center - min_val * wave_h * 0.5;
                    let bar_h = (bottom - top).max(1.0);
                    frame.fill_rectangle(
                        Point::new(draw_x, top),
                        Size::new(pixels_per_peak.max(1.0), bar_h),
                        waveform_color,
                    );
                }
                px += pixels_per_peak.max(1.0);
            }
        }

        // Clip header bar
        frame.fill_rectangle(
            Point::new(x, y),
            Size::new(w, header_height),
            theme::CLIP_HEADER,
        );

        // Clip name (truncated safely for multi-byte UTF-8)
        let display_name: String = if clip.name.chars().count() > 20 {
            let mut truncated: String = clip.name.chars().take(17).collect();
            truncated.push_str("...");
            truncated
        } else {
            clip.name.clone()
        };
        frame.fill_text(canvas::Text {
            content: display_name,
            position: Point::new(x + 4.0, y + 2.0),
            color: theme::TEXT,
            size: 11.0.into(),
            ..canvas::Text::default()
        });

        // Clip border (highlighted if selected)
        let is_selected = self.selected_clip == Some(clip.id);
        let border = canvas::Path::rectangle(Point::new(x, y), Size::new(w, clip_height));
        if is_selected {
            frame.stroke(
                &border,
                canvas::Stroke::default()
                    .with_color(theme::CLIP_SELECTED_BORDER)
                    .with_width(2.0),
            );
        } else {
            frame.stroke(
                &border,
                canvas::Stroke::default()
                    .with_color(Color::from_rgba(0.0, 0.0, 0.0, 0.3))
                    .with_width(1.0),
            );
        }
    }

    fn draw_midi_clip(
        &self,
        frame: &mut canvas::Frame,
        clip: &MidiClipState,
        sorted_tracks: &[&TrackState],
        ruler_height: f32,
        y_off: f32,
        visible_height: f32,
    ) {
        let track_index = sorted_tracks
            .iter()
            .position(|t| t.id == clip.track_id);

        let track_index = match track_index {
            Some(i) => i,
            None => return,
        };

        let y = ruler_height + track_index as f32 * theme::TRACK_HEIGHT + 2.0 - y_off;

        if y + theme::TRACK_HEIGHT < ruler_height || y > visible_height {
            return;
        }
        let clip_height = theme::TRACK_HEIGHT - 4.0;

        // Convert tick duration to samples, then to seconds for pixel width
        let samples_per_tick = (self.sample_rate as f64 * 60.0 / self.bpm as f64)
            / TICKS_PER_QUARTER_NOTE as f64;
        let duration_samples = clip.duration_ticks as f64 * samples_per_tick;
        let start_seconds = clip.start_sample as f32 / self.sample_rate as f32;
        let duration_seconds = duration_samples as f32 / self.sample_rate as f32;

        let x = start_seconds * self.zoom - self.scroll_offset;
        let w = duration_seconds * self.zoom;

        // Teal/cyan clip body
        let midi_body_color = Color::from_rgb(0.12, 0.22, 0.25);
        frame.fill_rectangle(
            Point::new(x, y),
            Size::new(w, clip_height),
            midi_body_color,
        );

        // Draw note rectangles inside the clip body
        let header_height = 18.0;
        let note_area_y = y + header_height;
        let note_area_h = clip_height - header_height;

        if !clip.notes.is_empty() && note_area_h > 2.0 && w > 2.0 {
            // Find the note range for vertical mapping
            let mut min_note: u8 = 127;
            let mut max_note: u8 = 0;
            for note in &clip.notes {
                if note.note < min_note {
                    min_note = note.note;
                }
                if note.note > max_note {
                    max_note = note.note;
                }
            }
            // Add padding so notes aren't flush with edges
            let range_min = min_note.saturating_sub(2);
            let range_max = (max_note + 2).min(127);
            let note_range = (range_max - range_min).max(1) as f32;

            let total_ticks = clip.duration_ticks as f32;
            if total_ticks > 0.0 {
                for note in &clip.notes {
                    // Horizontal position: note start relative to clip visible start
                    let note_start_in_clip = note.start_tick as f32
                        - clip.trim_start_ticks as f32;
                    if note_start_in_clip + note.duration_ticks as f32 <= 0.0 {
                        continue; // note is before visible area
                    }
                    if note_start_in_clip >= total_ticks {
                        continue; // note is after visible area
                    }
                    let visible_start = note_start_in_clip.max(0.0);
                    let visible_end = (note_start_in_clip + note.duration_ticks as f32)
                        .min(total_ticks);

                    let nx = x + (visible_start / total_ticks) * w;
                    let nw = ((visible_end - visible_start) / total_ticks) * w;

                    // Vertical position: highest note at top
                    let ny = note_area_y
                        + (1.0 - (note.note as f32 - range_min as f32) / note_range)
                            * (note_area_h - 3.0);
                    let nh = (note_area_h / note_range).max(2.0).min(6.0);

                    // Color intensity maps to velocity
                    let vel = note.velocity.clamp(0.0, 1.0);
                    let note_color = Color::from_rgba(
                        0.2 + 0.6 * vel,
                        0.7 + 0.3 * vel,
                        0.8 + 0.2 * vel,
                        0.7 + 0.3 * vel,
                    );

                    frame.fill_rectangle(
                        Point::new(nx, ny),
                        Size::new(nw.max(1.0), nh),
                        note_color,
                    );
                }
            }
        }

        // Clip header bar (teal accent)
        let midi_header_color = Color::from_rgb(0.15, 0.45, 0.50);
        frame.fill_rectangle(
            Point::new(x, y),
            Size::new(w, header_height),
            midi_header_color,
        );

        // Clip name
        let display_name: String = if clip.name.chars().count() > 20 {
            let mut truncated: String = clip.name.chars().take(17).collect();
            truncated.push_str("...");
            truncated
        } else {
            clip.name.clone()
        };
        frame.fill_text(canvas::Text {
            content: display_name,
            position: Point::new(x + 4.0, y + 2.0),
            color: theme::TEXT,
            size: 11.0.into(),
            ..canvas::Text::default()
        });

        // Clip border (highlighted if selected)
        let is_selected = self.selected_midi_clip == Some(clip.id);
        let border = canvas::Path::rectangle(Point::new(x, y), Size::new(w, clip_height));
        if is_selected {
            frame.stroke(
                &border,
                canvas::Stroke::default()
                    .with_color(theme::CLIP_SELECTED_BORDER)
                    .with_width(2.0),
            );
        } else {
            frame.stroke(
                &border,
                canvas::Stroke::default()
                    .with_color(Color::from_rgba(0.0, 0.0, 0.0, 0.3))
                    .with_width(1.0),
            );
        }
    }
}
