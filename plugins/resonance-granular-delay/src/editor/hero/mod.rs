//! Hero buffer view — the circular-buffer visualization at the heart
//! of the editor (ba todo #1139, design doc #264 req-1; prototype
//! states 'idle', 'default', 'synced').
//!
//! Everything is painter-drawn (no GL) from params + `GranularViz`
//! reads at the editor's 16 ms repaint cadence. The band is split by
//! the reason each part changes (ba todo #1265):
//!
//! - [`layout`] — pure geometry: the band rect, the time/pitch axis
//!   mappings and the delay-tap hit zone. No painter, no params, so it
//!   is unit-tested directly (tests/hero_layout.rs).
//! - [`draw`] — the static frame: backdrop silhouette, division ticks,
//!   pitch ruler, write head, delay tap, empty state, gesture legend.
//! - [`cloud`] — the live pitch layer painted into the seam the frame
//!   leaves: scale lanes and the grain cloud itself.
//! - [`interact`] — gestures only: tap drag ⇄ time, cloud drag ⇅ pitch,
//!   scroll = density, every write through `Param::set_plain`.
//!
//! The tempo grid the ticks and the drag-snapping share is not derived
//! here at all: it lives in [`crate::sync`] as plain arithmetic on the
//! host BPM. This module used to fabricate a `TempoInfo` in two places
//! to reach it.

mod cloud;
mod draw;
mod interact;
mod layout;

pub use draw::draw;
pub use interact::{interact, HeroDrag};
pub use layout::{HeroLayout, PITCH_RANGE_ST, TAP_HIT_HALF_W};
