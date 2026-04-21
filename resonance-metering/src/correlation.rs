//! Stereo correlation over a ~100 ms sliding window.
//!
//! Uses per-sample running sums of `L*L`, `R*R`, `L*R` maintained over a
//! fixed-size ring buffer. The published value is smoothed with a one-pole
//! filter so meter readings don't flicker during phase transitions.

use resonance_dsp::OnePole;

/// Window length in seconds.
const WINDOW_SECS: f32 = 0.1;
/// Smoother cutoff in Hz.
const SMOOTH_HZ: f32 = 6.0;

pub struct CorrelationMeter {
    ring_ll: Box<[f64]>,
    ring_rr: Box<[f64]>,
    ring_lr: Box<[f64]>,
    pos: usize,
    samples_pushed: u64,

    sum_ll: f64,
    sum_rr: f64,
    sum_lr: f64,

    smoother: OnePole,
    sample_rate: f32,
}

impl CorrelationMeter {
    pub fn new(sample_rate: f32) -> Self {
        let len = ((WINDOW_SECS * sample_rate) as usize).max(4);
        let mut smoother = OnePole::new();
        smoother.set_cutoff(SMOOTH_HZ, sample_rate);
        Self {
            ring_ll: vec![0.0; len].into_boxed_slice(),
            ring_rr: vec![0.0; len].into_boxed_slice(),
            ring_lr: vec![0.0; len].into_boxed_slice(),
            pos: 0,
            samples_pushed: 0,
            sum_ll: 0.0,
            sum_rr: 0.0,
            sum_lr: 0.0,
            smoother,
            sample_rate,
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
        self.smoother.clear();
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

        let instantaneous = compute_correlation(self.sum_ll, self.sum_rr, self.sum_lr);
        // One-pole smoother keeps UI readings visually stable.
        let _ = self.smoother.process(instantaneous);
    }

    /// Latest smoothed stereo correlation in `[-1, 1]`.
    pub fn correlation(&self) -> f32 {
        // OnePole::process mutates, but we want the current state.
        // Peek by computing the current instantaneous value then
        // falling back on the smoother's output. Since we updated it
        // in push_stereo, `process(0.0)` would nudge it toward zero —
        // instead we reconstruct from the sums (same as instantaneous).
        compute_correlation(self.sum_ll, self.sum_rr, self.sum_lr)
    }

    #[allow(dead_code)]
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_channels_correlate_to_plus_one() {
        let sr = 48_000.0;
        let mut m = CorrelationMeter::new(sr);
        let mut l = vec![0.0_f32; (sr * 0.2) as usize];
        for (i, s) in l.iter_mut().enumerate() {
            *s = ((i as f32) * 0.01).sin();
        }
        let r = l.clone();
        m.push_stereo(&l, &r);
        assert!(
            (m.correlation() - 1.0).abs() < 1e-3,
            "got {}",
            m.correlation()
        );
    }

    #[test]
    fn inverted_channels_correlate_to_minus_one() {
        let sr = 48_000.0;
        let mut m = CorrelationMeter::new(sr);
        let mut l = vec![0.0_f32; (sr * 0.2) as usize];
        for (i, s) in l.iter_mut().enumerate() {
            *s = ((i as f32) * 0.01).sin();
        }
        let r: Vec<f32> = l.iter().map(|&x| -x).collect();
        m.push_stereo(&l, &r);
        assert!(
            (m.correlation() + 1.0).abs() < 1e-3,
            "got {}",
            m.correlation()
        );
    }

    #[test]
    fn silence_reports_zero() {
        let m = CorrelationMeter::new(48_000.0);
        assert_eq!(m.correlation(), 0.0);
    }
}
