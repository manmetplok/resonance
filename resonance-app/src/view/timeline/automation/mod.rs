//! Timeline automation lanes (architecture doc #162 §3, epic #14,
//! todo #381 / A3; lane rows doc #256).
//!
//! Follows the seams the rest of the timeline module uses (draw split
//! #835, input split #945; ba todo #1133):
//!
//! * [`geometry`] — pure band/label/lane-selection helpers: no canvas,
//!   no `self`, unit-testable (band rects, value↔y mapping, polyline
//!   building, target labels/priorities, the shown-lane/cycle logic).
//! * [`draw`] — the [`super::TimelineCanvas`] drawing passes: the cached
//!   static lane layer (axis, segments, dots, label chip) and the
//!   uncached live playhead-value overlay.
//! * [`input`] — pointer hit-testing and edit geometry: band/breakpoint
//!   resolution through the shared [`crate::view::arrange_layout::ArrangeRowLayout`]
//!   (the #732 draw/hit-test rule), drag clamping and the curve toggle.
//!
//! The lane "shows" whenever the track has a lane in
//! [`crate::state::AutomationState`] and "hides" when it has none — no
//! extra UI state, so a lane added via the picker simply appears and a
//! cleared lane disappears.
//!
//! View-performance rules (MEMORY ui-work §11): the static lane layer is
//! drawn inside the cached geometry pass and only repaints when
//! [`super::TimelineFingerprint::automation_hash`] changes (an edit). The
//! live playhead value rides the uncached overlay frame so it follows the
//! playhead without invalidating the rest of the timeline.

mod draw;
mod geometry;
mod input;

pub use geometry::*;
pub use input::BREAKPOINT_HIT_RADIUS;
