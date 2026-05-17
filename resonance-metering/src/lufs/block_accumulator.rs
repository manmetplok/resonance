//! Sliding mean-square accumulator for BS.1770-4 loudness windows.
//!
//! Keeps running sums of K-weighted squared samples across two windows:
//! - **400 ms** (momentary loudness / integrated-gating blocks)
//! - **3 s**   (short-term loudness)
//!
//! Every 100 ms (75% overlap on the 400 ms window) the accumulator emits
//! one block-mean-square value, which is consumed by the integrated-loudness
//! gating logic in `integrated.rs`.
//!
//! All running sums are kept in `f64` to avoid drift over long sessions.

/// Size of the momentary window in seconds.
pub const MOMENTARY_SECS: f32 = 0.4;
/// Size of the short-term window in seconds.
pub const SHORT_TERM_SECS: f32 = 3.0;
/// Hop between integrated-gating blocks (100 ms → 75% overlap on 400 ms).
pub const BLOCK_HOP_SECS: f32 = 0.1;

/// Streaming mean-square accumulator. Not thread-safe — meant to live on
/// the audio thread and be read from the same thread.
pub struct BlockAccumulator {
    /// Ring buffer of K-weighted squared-sum samples. Length = 3 s of
    /// samples so we can subtract either the 400 ms or the 3 s tail.
    ring: Box<[f64]>,
    /// Next write position in the ring.
    write_pos: usize,
    /// Number of samples pushed since construction / reset. Used to detect
    /// when each window has been filled for the first time.
    samples_pushed: u64,

    /// Running sum over the last `window_m_samples` samples.
    sum_m: f64,
    /// Running sum over the last `window_s_samples` samples.
    sum_s: f64,

    /// Number of samples in the momentary window.
    window_m_samples: usize,
    /// Number of samples in the short-term window.
    window_s_samples: usize,
    /// Number of samples between integrated-gating blocks (100 ms).
    block_hop_samples: usize,

    /// Samples since the last integrated block was emitted.
    samples_since_block: usize,
}

impl BlockAccumulator {
    pub fn new(sample_rate: f32) -> Self {
        let window_s = (SHORT_TERM_SECS * sample_rate).round() as usize;
        let window_m = (MOMENTARY_SECS * sample_rate).round() as usize;
        let block_hop = (BLOCK_HOP_SECS * sample_rate).round() as usize;
        Self {
            ring: vec![0.0; window_s].into_boxed_slice(),
            write_pos: 0,
            samples_pushed: 0,
            sum_m: 0.0,
            sum_s: 0.0,
            window_m_samples: window_m,
            window_s_samples: window_s,
            block_hop_samples: block_hop,
            samples_since_block: 0,
        }
    }

    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.write_pos = 0;
        self.samples_pushed = 0;
        self.sum_m = 0.0;
        self.sum_s = 0.0;
        self.samples_since_block = 0;
    }

    /// Number of samples contributing to the momentary window right now.
    /// While the ring is filling this is smaller than `window_m_samples`.
    #[inline]
    pub fn momentary_count(&self) -> usize {
        (self.samples_pushed as usize).min(self.window_m_samples)
    }

    /// Number of samples contributing to the short-term window right now.
    #[inline]
    pub fn short_term_count(&self) -> usize {
        (self.samples_pushed as usize).min(self.window_s_samples)
    }

    /// Current mean-square of the momentary window. Returns `None` until
    /// at least one sample has been observed (avoids 0/0).
    pub fn momentary_mean_square(&self) -> Option<f64> {
        let n = self.momentary_count();
        if n == 0 {
            None
        } else {
            Some(self.sum_m / n as f64)
        }
    }

    /// Current mean-square of the short-term window.
    pub fn short_term_mean_square(&self) -> Option<f64> {
        let n = self.short_term_count();
        if n == 0 {
            None
        } else {
            Some(self.sum_s / n as f64)
        }
    }

    /// Push a single K-weighted squared-sum sample. Returns a block mean-
    /// square value when an integrated-gating block boundary is crossed
    /// (roughly once every 100 ms) *and* the momentary window is fully
    /// primed — partial blocks are not emitted.
    #[inline]
    pub fn push_sample(&mut self, squared: f64) -> Option<f64> {
        // Update momentary running sum: add new, subtract the sample that
        // aged out of the 400 ms window.
        self.sum_m += squared;
        if self.samples_pushed >= self.window_m_samples as u64 {
            let tail_idx = self.write_pos + self.ring.len() - self.window_m_samples;
            let old_m = self.ring[tail_idx % self.ring.len()];
            self.sum_m -= old_m;
        }

        // Short-term running sum: the ring length equals `window_s_samples`
        // so the value being overwritten is exactly the one to subtract.
        self.sum_s += squared;
        if self.samples_pushed >= self.window_s_samples as u64 {
            let old_s = self.ring[self.write_pos];
            self.sum_s -= old_s;
        }

        // Store in the ring.
        self.ring[self.write_pos] = squared;
        self.write_pos = (self.write_pos + 1) % self.ring.len();
        self.samples_pushed += 1;

        // Guard the running sums against accumulating f64 drift going
        // negative on all-silence blocks.
        if self.sum_m < 0.0 {
            self.sum_m = 0.0;
        }
        if self.sum_s < 0.0 {
            self.sum_s = 0.0;
        }

        // Emit an integrated-gating block every 100 ms, but only once the
        // first momentary window has been fully populated.
        self.samples_since_block += 1;
        if self.samples_since_block >= self.block_hop_samples
            && self.samples_pushed >= self.window_m_samples as u64
        {
            self.samples_since_block = 0;
            return Some(self.sum_m / self.window_m_samples as f64);
        }
        None
    }
}

