//! Timeline take lanes (epic #15, design doc #165; todo #413).
//!
//! Cycle recording keeps every loop pass as a distinct **take**, stacked in
//! a **take lane** under the track. The lane is one lane per *slot*: all the
//! takes recorded over a given loop region live in a single
//! [`TakeGroup`](resonance_common::TakeGroup) stack, whether they came from
//! one record run or five — the same folder model Logic / Pro Tools /
//! Reaper use.
//!
//! Follows the seams the rest of the timeline module uses (draw split #835,
//! input split #945):
//!
//! * [`geometry`] — pure cover/band helpers: no canvas, no `self`,
//!   unit-testable. [`effective_cover`](geometry::effective_cover) is the
//!   important one — it resolves what the engine will actually play over
//!   the slot (active take → comp segment → latest take).
//! * [`draw`] — the two [`TimelineCanvas`](super::TimelineCanvas) passes:
//!   the always-visible comp ribbon on the track lane, and the expanded
//!   stack of take cards.
//!
//! Interaction (select the active take, split at the playhead, promote a
//! segment) is todo #414 and lands beside these as an `input` module.

mod draw;
pub mod geometry;

pub use geometry::{
    comp_ribbon_band, effective_cover, take_card_band, take_label, unlit_ranges, CoverSource,
    CoverSpan,
};
