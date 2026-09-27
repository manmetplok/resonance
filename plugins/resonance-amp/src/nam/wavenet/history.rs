//! Dilated-conv input history for the block forward pass.
//!
//! A linear (not circular) buffer of past channel-vectors: the newest
//! block is written after at least `max_delay` frames of history, so every
//! tap's input for a whole block — frames `t - delay` for the block's `t`
//! — is ONE contiguous frame-major window, fed straight to the block GEMM.
//! When the next block would run off the end, the last `max_delay` frames
//! are copied back to the front (the reference NeuralAmpModelerCore
//! `_rewind_buffers` scheme). The slack past the history makes that copy
//! rare: its amortised cost is under one block's worth of writes.

pub(super) struct History {
    data: Vec<f32>,
    channels: usize,
    /// Longest lookback any tap takes, in frames.
    max_delay: usize,
    /// Capacity in frames.
    capacity: usize,
    /// Frame index where the current block starts (always >= max_delay).
    write: usize,
}

impl History {
    /// `max_delay` frames of lookback for blocks of up to `max_block`
    /// frames of `channels` values each.
    pub(super) fn new(max_delay: usize, channels: usize, max_block: usize) -> Self {
        let capacity = max_delay + max_delay.max(4 * max_block);
        Self {
            data: vec![0.0; capacity * channels],
            channels,
            max_delay,
            capacity,
            write: max_delay,
        }
    }

    /// Start a block of `n` frames: rewinds if needed and returns the
    /// frame-major slots to fill with the block's inputs.
    #[inline]
    pub(super) fn begin_block(&mut self, n: usize) -> &mut [f32] {
        let ch = self.channels;
        if self.write + n > self.capacity {
            let from = (self.write - self.max_delay) * ch;
            self.data.copy_within(from..self.write * ch, 0);
            self.write = self.max_delay;
        }
        &mut self.data[self.write * ch..(self.write + n) * ch]
    }

    /// The current block's inputs delayed by `delay` frames: `n` frames
    /// starting at `write - delay`, frame-major. Valid between
    /// [`Self::begin_block`] and [`Self::end_block`].
    #[inline]
    pub(super) fn window(&self, delay: usize, n: usize) -> &[f32] {
        debug_assert!(delay <= self.max_delay);
        let ch = self.channels;
        let start = (self.write - delay) * ch;
        &self.data[start..start + n * ch]
    }

    /// Commit the block written by [`Self::begin_block`].
    #[inline]
    pub(super) fn end_block(&mut self, n: usize) {
        self.write += n;
    }

    pub(super) fn reset(&mut self) {
        self.data.fill(0.0);
        self.write = self.max_delay;
    }
}
