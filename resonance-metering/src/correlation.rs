//! Stereo correlation over a ~100 ms sliding window.
//!
//! Uses per-sample running sums of `L*L`, `R*R`, `L*R` maintained over a
//! fixed-size ring buffer. The ~100 ms sliding window provides sufficient
//! visual smoothing for meter displays.

/// Window length in seconds.
const WINDOW_SECS: f32 = 0.1;

pub struct CorrelationMeter {
    ring_ll: Box<[f64]>,
    ring_rr: Box<[f64]>,
    ring_lr: Box<[f64]>,
    pos: usize,
    samples_pushed: u64,

    sum_ll: f64,
    sum_rr: f64,
    sum_lr: f64,
}

impl CorrelationMeter {
    pub fn new(sample_rate: f32) -> Self {
        let len = ((WINDOW_SECS * sample_rate) as usize).max(4);
        Self {
            ring_ll: vec![0.0; len].into_boxed_slice(),
            ring_rr: vec![0.0; len].into_boxed_slice(),
            ring_lr: vec![0.0; len].into_boxed_slice(),
            pos: 0,
            samples_pushed: 0,
            sum_ll: 0.0,
            sum_rr: 0.0,
            sum_lr: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.ring_ll.fill(0.0);
        self.ring_rr.fill(0.0);
        self.ring_lr.fill(0.0);
        self.pos = 0;
        self.samples_pushed = 0;
        self.sum_ll = 0.0;
        self.sum_rr = 0.0;
        self.sum_lr = 0.0;
    }

    /// Feed a stereo block.
    pub fn push_stereo(&mut self, left: &[f32], right: &[f32]) {
        let n = left.len().min(right.len());
        for i in 0..n {
            let l = left[i] as f64;
            let r = right[i] as f64;

            let old_ll = self.ring_ll[self.pos];
            let old_rr = self.ring_rr[self.pos];
            let old_lr = self.ring_lr[self.pos];

            let new_ll = l * l;
            let new_rr = r * r;
            let new_lr = l * r;

            self.sum_ll += new_ll - old_ll;
            self.sum_rr += new_rr - old_rr;
            self.sum_lr += new_lr - old_lr;

            self.ring_ll[self.pos] = new_ll;
            self.ring_rr[self.pos] = new_rr;
            self.ring_lr[self.pos] = new_lr;

            self.pos = (self.pos + 1) % self.ring_ll.len();
            self.samples_pushed += 1;
        }

        // Guard against long-running drift.
        if self.sum_ll < 0.0 {
            self.sum_ll = 0.0;
        }
        if self.sum_rr < 0.0 {
            self.sum_rr = 0.0;
        }

    }

    /// Latest stereo correlation in `[-1, 1]`.
    ///
    /// Computed from the sliding-window sums; the ~100 ms window already
    /// provides visual smoothing so an additional one-pole filter is
    /// unnecessary.
    ///
    /// Until a full window of samples has been pushed the readout is
    /// gated to `0.0` — the neutral centre of the UI's -1…+1 bar and
    /// the same value a fresh meter reports. A partial window would
    /// otherwise show an unsmoothed, jumpy estimate for the first
    /// ~100 ms after construction or [`reset`](Self::reset).
    pub fn correlation(&self) -> f32 {
        if self.samples_pushed < self.ring_ll.len() as u64 {
            return 0.0;
        }
        compute_correlation(self.sum_ll, self.sum_rr, self.sum_lr)
    }
}

#[inline]
fn compute_correlation(sum_ll: f64, sum_rr: f64, sum_lr: f64) -> f32 {
    let denom_sq = sum_ll * sum_rr;
    if denom_sq <= 1e-20 {
        return 0.0;
    }
    (sum_lr / denom_sq.sqrt()).clamp(-1.0, 1.0) as f32
}

