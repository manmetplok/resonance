//! Streaming integrated-loudness accumulator.
//!
//! Holds the growing list of per-block mean-square energies produced by the
//! [`BlockAccumulator`][crate::lufs::block_accumulator::BlockAccumulator]
//! and runs the BS.1770-4 two-pass gate on demand.
//!
//! The Vec is pre-grown to the capacity implied by a maximum-session
//! length so the audio thread never reallocates during normal operation.
//! Pushing past the cap drops the block (counted in `dropped_blocks()`)
//! and raises `cap_reached()` — sessions longer than the 60-minute cap
//! are unusual but not bugs.

use std::sync::atomic::{AtomicBool, Ordering};

use super::block_accumulator::BLOCK_HOP_SECS;
use super::gating::gated_integrated_lufs;
use super::incremental::IncrementalGate;

/// Maximum number of seconds of audio the integrated meter can hold before
/// it starts dropping new blocks. Pick something generous enough to cover
/// any realistic mastering session.
pub const MAX_SESSION_SECONDS: f32 = 60.0 * 60.0;

/// Accumulator for per-block mean-squares used by the integrated LUFS
/// calculation. Pure data container + one reader function.
pub struct IntegratedAccumulator {
    blocks: Vec<f64>,
    /// Soft cap on the number of blocks we accept before dropping new ones.
    cap: usize,
    /// How many blocks were dropped after the cap was reached. Exposed so
    /// callers can report overflow in the UI / test harness.
    dropped: u64,
    /// Set (relaxed) when the first block is dropped, so a UI thread can
    /// poll the condition lock-free instead of the audio thread logging.
    cap_reached: AtomicBool,
    /// The same blocks as a loudness histogram, for the realtime readout
    /// ([`Self::integrated_lufs_live`]). Uncapped: it is constant memory.
    live: IncrementalGate,
}

impl IntegratedAccumulator {
    pub fn new() -> Self {
        let cap = (MAX_SESSION_SECONDS / BLOCK_HOP_SECS).ceil() as usize;
        Self {
            blocks: Vec::with_capacity(cap),
            cap,
            dropped: 0,
            cap_reached: AtomicBool::new(false),
            live: IncrementalGate::new(),
        }
    }

    pub fn reset(&mut self) {
        self.blocks.clear();
        self.dropped = 0;
        self.cap_reached.store(false, Ordering::Relaxed);
        self.live.reset();
    }

    /// Add one block mean-square. If the cap has been reached, the value
    /// is dropped, `dropped_blocks()` is incremented and `cap_reached()`
    /// flips to `true` (per session / [`Self::reset`]). Long sessions are
    /// not bugs, and the audio thread never performs I/O — UI code polls
    /// the flag instead.
    #[inline]
    pub fn push_block(&mut self, mean_square: f64) {
        self.live.push_block(mean_square);
        if self.blocks.len() < self.cap {
            self.blocks.push(mean_square);
        } else {
            self.dropped += 1;
            if self.dropped == 1 {
                self.cap_reached.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Number of blocks currently held.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Whether the accumulator has any blocks yet.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Number of blocks dropped after hitting the cap.
    pub fn dropped_blocks(&self) -> u64 {
        self.dropped
    }

    /// Whether the 60-minute block cap has been hit this session. Lock-free
    /// (relaxed), safe to poll from a UI thread.
    pub fn cap_reached(&self) -> bool {
        self.cap_reached.load(Ordering::Relaxed)
    }

    /// Run the two-pass gate and return the integrated LUFS value. Returns
    /// `f64::NEG_INFINITY` if there's nothing to report yet.
    ///
    /// Exact, but `O(blocks)`: offline analysis only. A realtime caller
    /// uses [`Self::integrated_lufs_live`].
    pub fn integrated_lufs(&self) -> f64 {
        gated_integrated_lufs(&self.blocks)
    }

    /// The integrated LUFS from the incremental histogram gate: cost
    /// independent of session length, allocation-free, and not subject to
    /// the 60-minute cap. Matches [`Self::integrated_lufs`] to well under
    /// a hundredth of an LU (see `IncrementalGate`).
    pub fn integrated_lufs_live(&self) -> f64 {
        self.live.integrated_lufs()
    }

    /// Blocks that passed the absolute gate since the last reset — the
    /// only blocks [`Self::integrated_lufs_live`] depends on, so a caller
    /// can cache the readout on it. Uncapped.
    pub fn live_gated_blocks(&self) -> u64 {
        self.live.gated_block_count()
    }
}

impl Default for IntegratedAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

