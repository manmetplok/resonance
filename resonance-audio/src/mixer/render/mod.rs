//! The phases of one render block, split out of [`super::render_core`]
//! (ba todo #1252).
//!
//! `render_core::render_block` is the entry point and owns the order the
//! phases run in; everything a phase actually *does* lives here, one
//! concern per module:
//!
//! - [`context`]: the borrowed [`BlockInputs`](context::BlockInputs) /
//!   [`BlockScratch`](context::BlockScratch) parameter structs, the
//!   automation evaluation positions, and the insert-FX chain runner
//!   shared by tracks, sub-tracks and busses.
//! - [`strategy`]: `RenderStrategy` — everything that differs between the
//!   live callback and the offline bounce (locking, gating, gain ramps,
//!   meters, monitor input).
//! - [`clips`]: the audio-clip mix, its fades, the automatic same-track
//!   crossfade and the edge declick.
//! - [`frozen`]: frozen-source playback substitution.
//! - [`ports`]: the multi-output instrument's per-port CLAP call.
//! - [`routing`]: the post-fader route to a bus or to master, and the
//!   aux-send taps from a track and from a bus.
//! - [`track_pass`]: the per-track phase — source, key capture, PDC,
//!   meters, routing and sends.
//! - [`sub_track`]: the multi-output fan-out into sub-tracks.
//! - [`bus_pass`]: the per-bus phase.
//!
//! Nothing in here allocates, locks (beyond the plugin locks the strategy
//! already owns) or blocks: it is the realtime audio path.

pub(crate) mod bus_pass;
pub(crate) mod clips;
pub(crate) mod context;
pub(crate) mod frozen;
pub(crate) mod ports;
pub(crate) mod routing;
pub(crate) mod strategy;
pub(crate) mod sub_track;
pub(crate) mod track_pass;
