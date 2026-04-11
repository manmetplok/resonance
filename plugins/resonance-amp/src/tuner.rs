//! Monophonic pitch tracker for the amp's built-in tuner.
//!
//! Implementation follows the YIN algorithm (de Cheveigné & Kawahara,
//! 2002). It's the standard choice for monophonic instruments — simple,
//! allocation-free after construction, and robust enough for guitar on
//! a live signal. Tuned for the 65–1200 Hz range (low-E flat to a high
//! D6, comfortably covering all six strings with headroom).
//!
//! ## Pipeline
//!
//! 1. Incoming samples are appended to a ring buffer sized for one full
//!    analysis frame.
//! 2. When the ring has filled since the last analysis, `analyze()`
//!    runs a single YIN pass and reports `(hz, confidence)`.
//! 3. The audio thread calls `feed()` + `analyze()` once per block, so
//!    the O(N²) difference function runs at block rate (not per-sample).

const FRAME_LEN: usize = 2048;

/// Lowest pitch the tracker will report (Hz). A low-E on a 6-string guitar
/// sits at 82.4 Hz; we drop a little below that so detuned/dropped strings
/// still track.
const PITCH_MIN_HZ: f32 = 65.0;

/// Highest pitch the tracker will report (Hz). 1200 Hz is comfortably
/// above the high-E string at the 24th fret (~1320 Hz — close enough).
const PITCH_MAX_HZ: f32 = 1200.0;

/// YIN absolute threshold. Below this, the tap is considered confidently
/// periodic; above it, we fall back to the global minimum.
const YIN_THRESHOLD: f32 = 0.15;

pub struct Tuner {
    sample_rate: f32,
    /// Ring buffer of the most recent `FRAME_LEN` samples.
    ring: [f32; FRAME_LEN],
    write_pos: usize,
    /// Samples written since the last analysis (capped at FRAME_LEN).
    fill_since_analyze: usize,
    /// Working buffers, sized to the max lag we search.
    diff: Vec<f32>,
    cmnd: Vec<f32>,
    /// Inclusive lag bounds derived from `PITCH_MIN/MAX_HZ`.
    tau_min: usize,
    tau_max: usize,
}

impl Tuner {
    pub fn new(sample_rate: f32) -> Self {
        let tau_min = ((sample_rate / PITCH_MAX_HZ).floor() as usize).max(2);
        let tau_max = ((sample_rate / PITCH_MIN_HZ).ceil() as usize)
            .min(FRAME_LEN / 2 - 1);
        let buf_len = tau_max + 1;
        Self {
            sample_rate,
            ring: [0.0; FRAME_LEN],
            write_pos: 0,
            fill_since_analyze: 0,
            diff: vec![0.0; buf_len],
            cmnd: vec![0.0; buf_len],
            tau_min,
            tau_max,
        }
    }

    /// Append a block of samples to the ring. Cheap — plain copy.
    pub fn feed(&mut self, samples: &[f32]) {
        for &s in samples {
            self.ring[self.write_pos] = s;
            self.write_pos = (self.write_pos + 1) % FRAME_LEN;
        }
        self.fill_since_analyze = (self.fill_since_analyze + samples.len()).min(FRAME_LEN);
    }

    /// Run one YIN pass against the current ring contents and return
    /// `(hz, confidence)` on success. Returns `None` if the ring hasn't
    /// filled at least once yet or the signal is too quiet to analyse.
    pub fn analyze(&mut self) -> Option<(f32, f32)> {
        if self.fill_since_analyze < FRAME_LEN {
            return None;
        }

        // Linearised snapshot of the ring in chronological order.
        let mut frame = [0.0f32; FRAME_LEN];
        let start = self.write_pos;
        for i in 0..FRAME_LEN {
            frame[i] = self.ring[(start + i) % FRAME_LEN];
        }

        // Reject blocks that are mostly silent — avoids locking the tuner
        // onto low-level noise / hum.
        let rms_sq: f32 = frame.iter().map(|x| x * x).sum::<f32>() / FRAME_LEN as f32;
        if rms_sq < 1e-6 {
            return None;
        }

        // Step 1: difference function d(τ) = Σ (x[i] - x[i+τ])² for i in
        // [0, W-τ), W = FRAME_LEN/2. Using half the frame keeps the
        // summation balanced across lags.
        let window = FRAME_LEN / 2;
        self.diff[0] = 0.0;
        for tau in 1..=self.tau_max {
            let mut sum = 0.0f32;
            for i in 0..window {
                let d = frame[i] - frame[i + tau];
                sum += d * d;
            }
            self.diff[tau] = sum;
        }

        // Step 2: cumulative mean normalized difference function d'(τ).
        self.cmnd[0] = 1.0;
        let mut running = 0.0f32;
        for tau in 1..=self.tau_max {
            running += self.diff[tau];
            if running > 0.0 {
                self.cmnd[tau] = self.diff[tau] * tau as f32 / running;
            } else {
                self.cmnd[tau] = 1.0;
            }
        }

        // Step 3: absolute-threshold pick. Walk up from tau_min and stop
        // at the first local minimum below the threshold. Fall back to
        // the global argmin if nothing dips below.
        let mut tau_star: Option<usize> = None;
        let mut t = self.tau_min;
        while t < self.tau_max {
            if self.cmnd[t] < YIN_THRESHOLD {
                while t + 1 < self.tau_max && self.cmnd[t + 1] < self.cmnd[t] {
                    t += 1;
                }
                tau_star = Some(t);
                break;
            }
            t += 1;
        }
        let tau_star = tau_star.unwrap_or_else(|| {
            let mut best = self.tau_min;
            for i in self.tau_min..=self.tau_max {
                if self.cmnd[i] < self.cmnd[best] {
                    best = i;
                }
            }
            best
        });

        // Step 4: parabolic interpolation around tau_star for sub-sample
        // precision. Guards against the endpoints.
        let refined = if tau_star > self.tau_min && tau_star + 1 <= self.tau_max {
            let s0 = self.cmnd[tau_star - 1];
            let s1 = self.cmnd[tau_star];
            let s2 = self.cmnd[tau_star + 1];
            let denom = 2.0 * (2.0 * s1 - s2 - s0);
            if denom.abs() > 1e-12 {
                tau_star as f32 + (s2 - s0) / denom
            } else {
                tau_star as f32
            }
        } else {
            tau_star as f32
        };

        let hz = self.sample_rate / refined;
        if !(PITCH_MIN_HZ..=PITCH_MAX_HZ).contains(&hz) {
            return None;
        }

        // Confidence: 1.0 - d'(τ*), clamped. Lower cmnd means a stronger
        // periodic peak, so higher confidence.
        let confidence = (1.0 - self.cmnd[tau_star]).clamp(0.0, 1.0);

        // Reset the fill counter so we only analyse after receiving a
        // fresh frame's worth of audio.
        self.fill_since_analyze = 0;

        Some((hz, confidence))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    /// Drive the tuner with a pure sine and verify it locks on.
    fn detect_sine(sample_rate: f32, hz: f32, total_samples: usize) -> (f32, f32) {
        let mut tuner = Tuner::new(sample_rate);
        let mut buf = vec![0.0f32; 256];
        let mut result = (0.0, 0.0);
        let mut n = 0usize;
        while n < total_samples {
            for (i, s) in buf.iter_mut().enumerate() {
                let t = (n + i) as f32 / sample_rate;
                *s = (TAU * hz * t).sin();
            }
            tuner.feed(&buf);
            if let Some(r) = tuner.analyze() {
                result = r;
            }
            n += buf.len();
        }
        result
    }

    #[test]
    fn detects_a4() {
        let (hz, conf) = detect_sine(48_000.0, 440.0, 8192);
        assert!((hz - 440.0).abs() < 1.0, "A4: got {hz} Hz");
        assert!(conf > 0.8, "A4 confidence too low: {conf}");
    }

    #[test]
    fn detects_low_e() {
        // Low-E guitar string = 82.407 Hz.
        let (hz, conf) = detect_sine(48_000.0, 82.407, 8192);
        assert!((hz - 82.407).abs() < 1.0, "low E: got {hz} Hz");
        assert!(conf > 0.8, "low E confidence too low: {conf}");
    }

    #[test]
    fn detects_high_e() {
        // High-E guitar string = 329.628 Hz.
        let (hz, conf) = detect_sine(48_000.0, 329.628, 8192);
        assert!((hz - 329.628).abs() < 1.5, "high E: got {hz} Hz");
        assert!(conf > 0.8, "high E confidence too low: {conf}");
    }

    #[test]
    fn silence_reports_nothing() {
        let mut tuner = Tuner::new(48_000.0);
        let silence = vec![0.0f32; FRAME_LEN * 2];
        tuner.feed(&silence);
        assert!(tuner.analyze().is_none());
    }
}
