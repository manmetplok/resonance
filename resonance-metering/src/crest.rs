//! Crest factor meter (peak ÷ RMS) in dB.
//!
//! Sliding 100 ms window, updated per sample. Used as a quick transient-
//! density readout for the mastering assistant and the meter UI.

/// Window length in seconds.
const WINDOW_SECS: f32 = 0.1;

pub struct CrestMeter {
    ring: Box<[f32]>,
    sq_ring: Box<[f64]>,
    pos: usize,
    samples_pushed: u64,

    running_sq_sum: f64,
}

impl CrestMeter {
    pub fn new(sample_rate: f32) -> Self {
        let len = ((WINDOW_SECS * sample_rate) as usize).max(4);
        Self {
            ring: vec![0.0; len].into_boxed_slice(),
            sq_ring: vec![0.0; len].into_boxed_slice(),
            pos: 0,
            samples_pushed: 0,
            running_sq_sum: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.sq_ring.fill(0.0);
        self.pos = 0;
        self.samples_pushed = 0;
        self.running_sq_sum = 0.0;
    }

    /// Feed a stereo block — we operate on max(|L|, |R|) so the meter
    /// reflects the loudest channel's crest factor.
    pub fn push_stereo(&mut self, left: &[f32], right: &[f32]) {
        let n = left.len().min(right.len());
        for i in 0..n {
            let s = left[i].abs().max(right[i].abs());
            let sq = (s as f64) * (s as f64);

            let old_sq = self.sq_ring[self.pos];
            self.running_sq_sum += sq - old_sq;
            if self.running_sq_sum < 0.0 {
                self.running_sq_sum = 0.0;
            }

            self.ring[self.pos] = s;
            self.sq_ring[self.pos] = sq;
            self.pos = (self.pos + 1) % self.ring.len();
            self.samples_pushed += 1;
        }
    }

    /// Crest factor in dB, `20*log10(peak/rms)`. Returns 0 for silence.
    pub fn crest_db(&self) -> f32 {
        let n = (self.samples_pushed as usize).min(self.ring.len());
        if n == 0 || self.running_sq_sum <= 1e-20 {
            return 0.0;
        }
        let rms = (self.running_sq_sum / n as f64).sqrt() as f32;
        let peak = self.ring.iter().copied().fold(0.0_f32, f32::max);
        if peak <= 0.0 || rms <= 1e-20 {
            return 0.0;
        }
        20.0 * (peak / rms).log10()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_wave_crest_is_about_three_db() {
        // A unit sine has peak = 1, RMS = 1/sqrt(2) → crest = 3.01 dB.
        let sr = 48_000.0;
        let mut m = CrestMeter::new(sr);
        let mut l = vec![0.0_f32; (sr * 0.2) as usize];
        for (i, s) in l.iter_mut().enumerate() {
            *s = (i as f32 / sr * 1000.0 * std::f32::consts::TAU).sin();
        }
        let r = l.clone();
        m.push_stereo(&l, &r);
        let crest = m.crest_db();
        assert!((crest - 3.01).abs() < 0.5, "crest = {crest}");
    }

    #[test]
    fn silence_is_zero_crest() {
        let m = CrestMeter::new(48_000.0);
        assert_eq!(m.crest_db(), 0.0);
    }
}
