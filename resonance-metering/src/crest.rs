//! Crest factor meter (peak ÷ RMS) in dB.
//!
//! Sliding 100 ms window, updated per sample. Used as a quick transient-
//! density readout for the mastering assistant and the meter UI.
//!
//! The window peak is tracked with a monotonic deque (classic sliding-
//! window maximum): `push_stereo` does amortized O(1) work per sample
//! and `crest_db()` reads the max in O(1), instead of re-scanning the
//! whole 100 ms ring. That matters because the mastering plugin calls
//! `crest_db()` once per audio block when publishing its meter snapshot.

/// Window length in seconds.
const WINDOW_SECS: f32 = 0.1;

pub struct CrestMeter {
    sq_ring: Box<[f64]>,
    pos: usize,
    samples_pushed: u64,

    running_sq_sum: f64,

    // Monotonic deque over the last `sq_ring.len()` samples: entries are
    // (sample index, |sample|) with strictly decreasing values from front
    // to back, stored in a fixed ring so pushes never allocate. The front
    // is always the window maximum.
    max_idx: Box<[u64]>,
    max_val: Box<[f32]>,
    max_head: usize,
    max_len: usize,
}

impl CrestMeter {
    pub fn new(sample_rate: f32) -> Self {
        let len = ((WINDOW_SECS * sample_rate) as usize).max(4);
        Self {
            sq_ring: vec![0.0; len].into_boxed_slice(),
            pos: 0,
            samples_pushed: 0,
            running_sq_sum: 0.0,
            // Values in the deque are strictly decreasing, so it can
            // never hold more than one entry per window slot.
            max_idx: vec![0; len].into_boxed_slice(),
            max_val: vec![0.0; len].into_boxed_slice(),
            max_head: 0,
            max_len: 0,
        }
    }

    pub fn reset(&mut self) {
        self.sq_ring.fill(0.0);
        self.pos = 0;
        self.samples_pushed = 0;
        self.running_sq_sum = 0.0;
        self.max_head = 0;
        self.max_len = 0;
    }

    /// Feed a stereo block — we operate on max(|L|, |R|) so the meter
    /// reflects the loudest channel's crest factor.
    pub fn push_stereo(&mut self, left: &[f32], right: &[f32]) {
        let n = left.len().min(right.len());
        let window = self.sq_ring.len();
        let cap = self.max_idx.len();
        for i in 0..n {
            let s = left[i].abs().max(right[i].abs());
            let sq = (s as f64) * (s as f64);

            let old_sq = self.sq_ring[self.pos];
            self.running_sq_sum += sq - old_sq;
            if self.running_sq_sum < 0.0 {
                self.running_sq_sum = 0.0;
            }

            self.sq_ring[self.pos] = sq;
            self.pos = (self.pos + 1) % window;

            // Evict the front if it just fell out of the window. At most
            // one entry can expire per pushed sample.
            if self.max_len > 0 && self.max_idx[self.max_head] + window as u64 <= self.samples_pushed
            {
                self.max_head += 1;
                if self.max_head == cap {
                    self.max_head = 0;
                }
                self.max_len -= 1;
            }
            // Drop back entries dominated by the new sample, then append
            // it; this keeps front-to-back values strictly decreasing.
            while self.max_len > 0 {
                let mut back = self.max_head + self.max_len - 1;
                if back >= cap {
                    back -= cap;
                }
                if self.max_val[back] <= s {
                    self.max_len -= 1;
                } else {
                    break;
                }
            }
            let mut back = self.max_head + self.max_len;
            if back >= cap {
                back -= cap;
            }
            self.max_idx[back] = self.samples_pushed;
            self.max_val[back] = s;
            self.max_len += 1;

            self.samples_pushed += 1;
        }
    }

    /// Crest factor in dB, `20*log10(peak/rms)`. Returns 0 for silence.
    pub fn crest_db(&self) -> f32 {
        let n = (self.samples_pushed as usize).min(self.sq_ring.len());
        if n == 0 || self.running_sq_sum <= 1e-20 {
            return 0.0;
        }
        let rms = (self.running_sq_sum / n as f64).sqrt() as f32;
        let peak = if self.max_len > 0 {
            self.max_val[self.max_head]
        } else {
            0.0
        };
        if peak <= 0.0 || rms <= 1e-20 {
            return 0.0;
        }
        20.0 * (peak / rms).log10()
    }
}
