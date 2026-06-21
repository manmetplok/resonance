//! Monophonic fundamental-frequency (f0) detection for vocal clips.
//!
//! This is the first DSP primitive behind the vocal-tuning epic (#27): a
//! windowed, hop-based pitch tracker that turns a mono vocal signal into a
//! per-frame f0 contour with a voiced/unvoiced flag and a confidence value.
//! Downstream stages segment this contour into note blobs (#354) and drive
//! formant-preserving resynthesis (#353).
//!
//! The estimator is the YIN algorithm (de Cheveigné & Kawahara, 2002): a
//! cumulative-mean-normalised difference function with an absolute threshold
//! and parabolic interpolation. YIN is robust on monophonic, mildly noisy
//! speech/song material and needs no FFT, which keeps the implementation
//! self-contained and easy to test.
//!
//! Everything here is pure and offline: [`detect_f0`] is a free function over
//! a sample slice, and [`YinDetector`] holds reusable scratch buffers so a
//! caller analysing many clips avoids per-call allocation in the hot path.

/// One analysed frame of the f0 contour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct F0Frame {
    /// Centre time of the analysis frame, in seconds from the clip start.
    pub time_secs: f32,
    /// Estimated fundamental frequency in Hz. Meaningful only when
    /// [`voiced`](Self::voiced) is `true`; `0.0` for unvoiced frames.
    pub f0_hz: f32,
    /// Periodicity confidence in `[0, 1]` (`1 − d'(τ)` at the chosen lag).
    /// Higher means a cleaner, more periodic frame.
    pub confidence: f32,
    /// Whether the frame is voiced (a reliable pitch was found).
    pub voiced: bool,
}

/// Configuration for [`YinDetector`] / [`detect_f0`].
///
/// Build with [`F0Config::new`] for vocal-tuned defaults, then override fields
/// as needed. All durations are expressed in samples so the analysis is
/// deterministic for a given sample rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct F0Config {
    /// Sample rate of the input signal, in Hz.
    pub sample_rate: f32,
    /// Analysis frame length in samples. Must be large enough to hold two
    /// periods of the lowest detectable pitch: `frame_size ≥ 2·sr/f_min`.
    pub frame_size: usize,
    /// Hop between successive frames, in samples.
    pub hop_size: usize,
    /// Lowest detectable fundamental, in Hz.
    pub f_min: f32,
    /// Highest detectable fundamental, in Hz.
    pub f_max: f32,
    /// YIN absolute threshold on the normalised difference function. Lower is
    /// stricter (fewer octave errors, more unvoiced frames). ~0.1–0.2 is
    /// typical for voice.
    pub threshold: f32,
    /// Minimum confidence (`1 − d'(τ)`) for a frame to be marked voiced.
    pub voiced_confidence: f32,
    /// Frames whose RMS is below this are treated as silence (unvoiced).
    pub silence_rms: f32,
}

impl F0Config {
    /// Vocal-tuned defaults for the given sample rate:
    /// 2048-sample frames, 256-sample hop, 65–1000 Hz range.
    ///
    /// `frame_size` is the smallest power of two that holds two periods of
    /// `f_min`, so the configuration stays valid across sample rates.
    pub fn new(sample_rate: f32) -> Self {
        let f_min = 65.0;
        let min_frame = (2.0 * sample_rate / f_min).ceil() as usize;
        let frame_size = min_frame.next_power_of_two().max(1024);
        Self {
            sample_rate,
            frame_size,
            hop_size: frame_size / 8,
            f_min,
            f_max: 1000.0,
            threshold: 0.15,
            voiced_confidence: 0.5,
            silence_rms: 1.0e-4,
        }
    }

    /// Longest lag searched, in samples (`sr / f_min`), clamped so the
    /// difference function never reads past the frame.
    fn tau_max(&self) -> usize {
        let raw = (self.sample_rate / self.f_min).ceil() as usize;
        raw.min(self.frame_size / 2).max(2)
    }

    /// Shortest lag searched, in samples (`sr / f_max`), at least 1.
    fn tau_min(&self) -> usize {
        ((self.sample_rate / self.f_max).floor() as usize).max(1)
    }
}

/// Reusable YIN pitch tracker.
///
/// Holds the difference-function scratch buffers so repeated [`analyze`] calls
/// allocate nothing. Construct once per configuration and reuse.
///
/// [`analyze`]: YinDetector::analyze
#[derive(Debug, Clone)]
pub struct YinDetector {
    config: F0Config,
    tau_min: usize,
    tau_max: usize,
    /// Squared-difference function `d(τ)`, indices `0..=tau_max`.
    diff: Vec<f32>,
    /// Cumulative-mean-normalised difference `d'(τ)`, indices `0..=tau_max`.
    cmnd: Vec<f32>,
}

impl YinDetector {
    /// Create a detector for `config`.
    ///
    /// # Panics
    /// Panics if the configuration is degenerate (`frame_size < 4`,
    /// `hop_size == 0`, non-finite or non-positive sample rate, or
    /// `f_min`/`f_max` outside `(0, sr/2]` with `f_min < f_max`).
    pub fn new(config: F0Config) -> Self {
        assert!(
            config.sample_rate.is_finite() && config.sample_rate > 0.0,
            "sample_rate must be positive and finite"
        );
        assert!(config.frame_size >= 4, "frame_size must be >= 4");
        assert!(config.hop_size >= 1, "hop_size must be >= 1");
        assert!(
            config.f_min > 0.0 && config.f_min < config.f_max,
            "require 0 < f_min < f_max"
        );
        assert!(
            config.f_max <= config.sample_rate / 2.0,
            "f_max must be <= Nyquist"
        );
        let tau_max = config.tau_max();
        let tau_min = config.tau_min().min(tau_max - 1);
        Self {
            config,
            tau_min,
            tau_max,
            diff: vec![0.0; tau_max + 1],
            cmnd: vec![0.0; tau_max + 1],
        }
    }

    /// The configuration this detector was built with.
    pub fn config(&self) -> &F0Config {
        &self.config
    }

    /// Analyse `samples` and return one [`F0Frame`] per hop.
    ///
    /// Frames are centred on `start + frame_size/2`; the last partial frame is
    /// dropped. Returns an empty vector when `samples` is shorter than one
    /// frame.
    pub fn analyze(&mut self, samples: &[f32]) -> Vec<F0Frame> {
        let frame_size = self.config.frame_size;
        if samples.len() < frame_size {
            return Vec::new();
        }
        let hop = self.config.hop_size;
        let n_frames = (samples.len() - frame_size) / hop + 1;
        let mut out = Vec::with_capacity(n_frames);
        let mut start = 0;
        while start + frame_size <= samples.len() {
            let frame = &samples[start..start + frame_size];
            out.push(self.analyze_frame(frame, start));
            start += hop;
        }
        out
    }

    /// Analyse a single `frame` whose first sample is at `start_index`.
    fn analyze_frame(&mut self, frame: &[f32], start_index: usize) -> F0Frame {
        let sr = self.config.sample_rate;
        let time_secs = (start_index as f32 + frame.len() as f32 * 0.5) / sr;

        // Reject silence up front: a near-zero frame has no meaningful pitch.
        let rms = (frame.iter().map(|&x| x * x).sum::<f32>() / frame.len() as f32).sqrt();
        if rms < self.config.silence_rms {
            return F0Frame {
                time_secs,
                f0_hz: 0.0,
                confidence: 0.0,
                voiced: false,
            };
        }

        self.difference_function(frame);
        self.cumulative_mean_normalize();

        let (tau, periodicity) = self.absolute_threshold();
        match tau {
            Some(tau) => {
                let refined = self.parabolic_interpolation(tau);
                let f0 = sr / refined;
                let in_range = f0 >= self.config.f_min && f0 <= self.config.f_max;
                let voiced = in_range && periodicity >= self.config.voiced_confidence;
                F0Frame {
                    time_secs,
                    f0_hz: if voiced { f0 } else { 0.0 },
                    confidence: periodicity,
                    voiced,
                }
            }
            None => F0Frame {
                time_secs,
                f0_hz: 0.0,
                confidence: periodicity,
                voiced: false,
            },
        }
    }

    /// YIN step 1: squared-difference function `d(τ)` over an integration
    /// window of `frame_size − tau_max` samples, for `τ ∈ [0, tau_max]`.
    fn difference_function(&mut self, frame: &[f32]) {
        let window = frame.len() - self.tau_max;
        self.diff[0] = 0.0;
        for tau in 1..=self.tau_max {
            let mut sum = 0.0;
            for j in 0..window {
                let delta = frame[j] - frame[j + tau];
                sum += delta * delta;
            }
            self.diff[tau] = sum;
        }
    }

    /// YIN step 2: cumulative mean normalised difference `d'(τ)`.
    /// `d'(0) = 1`; `d'(τ) = d(τ) · τ / Σ_{k=1..τ} d(k)`.
    fn cumulative_mean_normalize(&mut self) {
        self.cmnd[0] = 1.0;
        let mut running = 0.0;
        for tau in 1..=self.tau_max {
            running += self.diff[tau];
            self.cmnd[tau] = if running > 0.0 {
                self.diff[tau] * tau as f32 / running
            } else {
                1.0
            };
        }
    }

    /// YIN step 3: absolute threshold. Returns the first lag dipping below the
    /// threshold (walked to its local minimum), plus the periodicity
    /// confidence `1 − d'(τ)` at the selected lag.
    ///
    /// When nothing crosses the threshold, falls back to the global minimum of
    /// `d'` within the search range and reports its (low) confidence, so the
    /// caller still gets a `confidence` value while the frame stays unvoiced.
    fn absolute_threshold(&self) -> (Option<usize>, f32) {
        let mut tau = self.tau_min.max(1);
        while tau <= self.tau_max {
            if self.cmnd[tau] < self.config.threshold {
                // Descend to the local minimum of the dip.
                while tau < self.tau_max && self.cmnd[tau + 1] < self.cmnd[tau] {
                    tau += 1;
                }
                let confidence = (1.0 - self.cmnd[tau]).clamp(0.0, 1.0);
                return (Some(tau), confidence);
            }
            tau += 1;
        }

        // No crossing: report the global minimum's confidence, unvoiced.
        let mut best = self.tau_min.max(1);
        for t in (self.tau_min.max(1))..=self.tau_max {
            if self.cmnd[t] < self.cmnd[best] {
                best = t;
            }
        }
        let confidence = (1.0 - self.cmnd[best]).clamp(0.0, 1.0);
        (None, confidence)
    }

    /// Refine `tau` to sub-sample precision by fitting a parabola to the
    /// normalised difference at `tau − 1, tau, tau + 1`.
    fn parabolic_interpolation(&self, tau: usize) -> f32 {
        if tau == 0 || tau >= self.tau_max {
            return tau as f32;
        }
        let s0 = self.cmnd[tau - 1];
        let s1 = self.cmnd[tau];
        let s2 = self.cmnd[tau + 1];
        let denom = s0 + s2 - 2.0 * s1;
        if denom.abs() < f32::EPSILON {
            return tau as f32;
        }
        let adjustment = 0.5 * (s0 - s2) / denom;
        // Keep the correction within one sample of the integer estimate.
        tau as f32 + adjustment.clamp(-1.0, 1.0)
    }
}

/// Detect the f0 contour of `samples` using YIN with the given `config`.
///
/// Convenience wrapper around [`YinDetector`] for one-shot analysis. For
/// repeated analysis with the same configuration, build a [`YinDetector`] once
/// and call [`YinDetector::analyze`] to avoid re-allocating scratch buffers.
pub fn detect_f0(samples: &[f32], config: F0Config) -> Vec<F0Frame> {
    YinDetector::new(config).analyze(samples)
}
