//! Cursor-query handlers for the timeline canvas: wheel-scroll routing
//! (`handle_wheel`), pointer cursor shape (`hover_interaction`), right-click
//! context menu (`handle_right_press`), and viewport-size reporting
//! (`report_viewport`).

use iced::{mouse, Rectangle};

use crate::message::*;
use super::super::hit_test::{HitKind, MarkerHit};
use super::super::{TimelineCanvas, TimelineState};
use super::{captured, ClipInteraction, UpdateResult};

impl TimelineCanvas<'_> {
    pub(in crate::view::timeline) fn handle_wheel(
        &self,
        delta: mouse::ScrollDelta,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> UpdateResult {
        // Only handle wheel events when the cursor is actually over the
        // timeline — otherwise scrolling the piano roll would also scroll
        // the arrangement behind it.
        cursor.position_in(bounds)?;
        // Horizontal scroll is owned by the outer `Scrollable` that wraps
        // the timeline canvas — returning `Ignored` for any wheel-X delta
        // lets the event bubble up so the scrollable can handle it natively.
        // Vertical scroll stays inside the canvas because the track lanes
        // scroll in lockstep with the ruler / section band / global-track
        // header (vertical scrollbar drawing + drag handling live here too).
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

    /// Pointer cursor for the current hover / drag state. `ew-resize` for
    /// trim and fade handles, `ns-resize` for the gain bead, `grab` over a
    /// clip body. During an active drag the matching resize/grab cursor is
    /// held regardless of pointer position.
    pub(in crate::view::timeline) fn hover_interaction(
        &self,
        state: &TimelineState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        use mouse::Interaction;
        // An active breakpoint drag holds the grabbing cursor regardless of
        // where the pointer wandered.
        if state.breakpoint_drag.is_some() {
            return Interaction::Grabbing;
        }
        // An active marker drag holds its cursor regardless of pointer
        // position: resize for an end-edge drag, grabbing for a start move.
        if let Some(drag) = state.marker_drag {
            return match drag.hit {
                MarkerHit::EndEdge => Interaction::ResizingHorizontally,
                MarkerHit::Flag => Interaction::Grabbing,
            };
        }
        match &state.clip_interaction {
            Some(ClipInteraction::Trim)
            | Some(ClipInteraction::MidiTrim)
            | Some(ClipInteraction::Fade) => return Interaction::ResizingHorizontally,
            Some(ClipInteraction::Gain) => return Interaction::ResizingVertically,
            Some(ClipInteraction::Move) | Some(ClipInteraction::MidiMove) => {
                return Interaction::Grabbing
            }
            None => {}
        }
        let Some(pos) = cursor.position_in(bounds) else {
            return Interaction::default();
        };
        // Marker hover in the ruler band: resize over a region end edge,
        // grab over a flag. Checked before the header guard below because
        // markers live in the ruler (above the lane area).
        if let Some((_, hit)) = self.marker_at(pos) {
            return match hit {
                MarkerHit::EndEdge => Interaction::ResizingHorizontally,
                MarkerHit::Flag => Interaction::Grab,
            };
        }
        if pos.y < self.fixed_header_height() {
            return Interaction::default();
        }
        // Breakpoint dots win over clips (matching the press order).
        if self.breakpoint_hit(pos).is_some() {
            return Interaction::Grab;
        }
        let sorted_tracks = self.visible_tracks_sorted();
        // MIDI clips on top, then audio — matching the press hit order.
        for clip in self.midi_clips.iter().rev() {
            let clip_end = self.tempo_map.tick_to_abs_sample(
                clip.start_sample,
                clip.duration_ticks,
                self.sample_rate,
            );
            let duration_samples = clip_end.saturating_sub(clip.start_sample);
            if let Some(hit) = self.hit_test_lane(
                pos,
                &sorted_tracks,
                clip.track_id,
                clip.start_sample,
                duration_samples,
            ) {
                return match hit {
                    HitKind::Trim(_) => Interaction::ResizingHorizontally,
                    HitKind::Move { .. } => Interaction::Grab,
                    _ => Interaction::default(),
                };
            }
        }
        for clip in self.clips.iter().rev() {
            if let Some(hit) = self.hit_test_audio_lane(pos, &sorted_tracks, clip) {
                return match hit {
                    HitKind::Trim(_) | HitKind::FadeIn | HitKind::FadeOut => {
                        Interaction::ResizingHorizontally
                    }
                    HitKind::Gain => Interaction::ResizingVertically,
                    HitKind::Move { .. } => Interaction::Grab,
                    HitKind::Miss => Interaction::default(),
                };
            }
        }
        // Empty band space (not over a clip) hints click-to-add a breakpoint.
        if self.band_add_at(pos).is_some() {
            return Interaction::Crosshair;
        }
        Interaction::default()
    }

    /// Right-click press. Two behaviours share the gesture, keyed by where
    /// the pointer lands:
    ///
    /// * over a marker in the ruler band → open the marker context menu,
    ///   anchored at the cursor's window-space position so the floating
    ///   overlay lands under the pointer regardless of horizontal scroll;
    /// * over an automation breakpoint in a track lane → delete it (the
    ///   codebase convention is right-click = delete, as in the chord lane /
    ///   MIDI editor / vocal roll).
    ///
    /// The two regions don't overlap (markers live in the ruler, breakpoints
    /// below the fixed header), so the marker check runs first. Misses fall
    /// through (`None`) so the event keeps propagating.
    pub(in crate::view::timeline) fn handle_right_press(
        &self,
        state: &mut TimelineState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> UpdateResult {
        let pos = cursor.position_in(bounds)?;
        if let Some((id, _hit)) = self.marker_at(pos) {
            let anchor = cursor.position().unwrap_or(pos);
            return captured(Message::MarkerUi(MarkerUiMessage::OpenMenu {
                id,
                x: anchor.x,
                y: anchor.y,
            }));
        }
        if pos.y < self.fixed_header_height() {
            return None;
        }
        let hit = self.breakpoint_hit(pos)?;
        state.selected_breakpoint = None;
        state.breakpoint_drag = None;
        captured(Message::Automation(AutomationMessage::DeleteBreakpoint {
            target: hit.target,
            index: hit.index,
        }))
    }

    /// Emit `ViewportWidth` / `ViewportHeight` / `TimelineContentSize`
    /// messages when any value has moved enough to be worth pushing
    /// upstream. Called at the tail of every event the canvas sees.
    /// Width takes priority over height takes priority over content
    /// size; `report_viewport` returns at most one message per call,
    /// and the canvas calls it repeatedly until quiescent.
    pub(in crate::view::timeline) fn report_viewport(
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
        if (bounds.height - state.last_reported_height).abs() > 1.0 {
            state.last_reported_height = bounds.height;
            return Some(Message::Viewport(ViewportMessage::ViewportHeight(
                bounds.height,
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
