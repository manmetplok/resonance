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
//! * [`input`] — hit-testing (todo #414): which comping verb a pointer
//!   position names. Resolves through the same layout and band helpers the
//!   draw pass uses, so a gesture can only ever hit what is drawn.
//! * [`overlay`] — the interaction pass (todo #414): the promote preview
//!   and the affordance captions, on the *uncached* overlay frame because
//!   they follow the pointer.
//!
//! The gestures the lane answers to, all resolved on the canvas:
//!
//! | gesture | message |
//! |---|---|
//! | click a take card | `SetActiveTake` (a second click releases the solo) |
//! | drag across a take card | `PromoteTakeSegment` over the raw drag range |
//! | click the comp ribbon | `SplitCompAtPlayhead` |
//! | right-click a take card | `DeleteTake` |

mod draw;
pub mod geometry;
pub(super) mod input;
mod overlay;

pub use geometry::{
    comp_ribbon_band, effective_cover, silent_ranges, take_card_band, take_label, unlit_ranges,
    CoverSource, CoverSpan,
};
