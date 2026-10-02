//! The loop (cycle) range, published to the audio callback as one value
//! (code review RT-13).
//!
//! It used to be three independent relaxed atomics (`loop_enabled`,
//! `loop_in`, `loop_out`). A block that read them while the engine thread
//! was moving the loop could see the new `loop_in` with the old
//! `loop_out` and wrap to the wrong place, or skip the wrap. The range now
//! lives in one `ArcSwap<LoopRange>`, so every reader sees a whole range
//! that the engine thread actually published. The replaced snapshot is
//! freed by the engine loop's retire sweep, never by the audio thread.

use std::sync::Arc;

use super::SharedState;

/// The transport's loop region, in engine sample frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoopRange {
    /// Cycle playback on: the callback wraps the playhead from
    /// `loop_out` back to `loop_in`.
    pub enabled: bool,
    pub loop_in: u64,
    pub loop_out: u64,
}

impl LoopRange {
    pub fn new(enabled: bool, loop_in: u64, loop_out: u64) -> Self {
        Self {
            enabled,
            loop_in,
            loop_out,
        }
    }
}

impl SharedState {
    /// The loop range as one consistent snapshot. Wait-free (one
    /// `ArcSwap` load), safe on the audio thread.
    #[inline]
    pub fn loop_range(&self) -> LoopRange {
        **self.loop_range.load()
    }

    /// Publish a new loop range. Called by the engine thread (and tests);
    /// the replaced snapshot goes to the retire queue.
    pub fn set_loop_range(&self, range: LoopRange) {
        super::retire::publish(&self.loop_range, Arc::new(range), &self.retired);
    }
}
