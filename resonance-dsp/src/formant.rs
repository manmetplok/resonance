//! Formant-preserving pitch-shift resynthesis (doc #160, todo #353).
//!
//! Repitches a mono (or, channel-by-channel, stereo) buffer by a
//! **time-varying** pitch ratio while holding the *spectral envelope*
//! (the formant structure) fixed. Timing is left untouched — the output
//! is exactly as long as the input — so this primitive is purely a
//! pitch operation that downstream stages compose with an independent
//! time-stretch ([`crate::TimeStretch`]) when warping clips (epic #20).
//!
//! # Why a separate primitive
//!
//! A plain resampling pitch-shift (what [`TimeStretch`] does) multiplies
//! *every* frequency by the pitch ratio, dragging the formants along and
//! producing the "chipmunk" effect: a voice shifted up an octave sounds
//! like a small animal because its resonances doubled too. Vocal tuning
//! needs the opposite — move the harmonics (the perceived pitch) but keep
//! the formants (the perceived vowel / timbre) where they were.
//!
//! [`TimeStretch`]: crate::TimeStretch
//!
//! # Algorithm
//!
//! A phase-vocoder operating on a fixed STFT grid (analysis hop ==
//! synthesis hop, so unmodified frames reconstruct the input to
//! numerical precision). For each frame:
//!
//! 1. Estimate the spectral envelope `env[k]` by cepstral liftering
//!    (smooth the log-magnitude spectrum, keeping only low quefrencies so
//!    the harmonic comb is averaged out but the formant peaks survive).
//! 2. Whiten the magnitude: `white[k] = mag[k] / env[k]` — the source /
//!    excitation spectrum, carrying the pitch (harmonic comb) but not the
//!    formants.
//! 3. Shift the *whitened* spectrum up by the pitch ratio `ρ`
//!    (`white[k/ρ]`, linearly interpolated) — this moves the harmonics,
//!    i.e. the pitch.
//! 4. Re-impose the **original** envelope at each output bin:
//!    `out_mag[k] = shifted_white[k] · env[k]`. The formants stay put.
//! 5. Propagate phase per bin from the (frequency-scaled) instantaneous
//!    frequency so the moved partials stay coherent across frames.
//!
//! At `ρ = 1` the frame is passed through unmodified, so a unit ratio is
//! a near-bit-identical copy of the input.
//!
//! # Determinism
//!
//! Generation is a pure function of the input samples, the sample rate
//! and the ratio curve — no RNG, no wall-clock, no global state.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::window::hann_window;

/// STFT frame size (samples); a power of two for the FFT. 2048 gives
/// ~23 Hz bins at 48 kHz — fine enough to resolve a vocal harmonic comb
/// and trace the formant envelope over it.
const FRAME: usize = 2048;
/// Overlap factor: synthesis hop is `FRAME / OVERLAP` (75 % overlap).
const OVERLAP: usize = 4;
/// Analysis == synthesis hop, in samples.
const HOP: usize = FRAME / OVERLAP;
/// Floor on the OLA normalisation weight, avoiding 0/0 at the partially
/// covered leading/trailing edges.
const WEIGHT_EPS: f32 = 1e-6;
/// Floor added to magnitudes before the log in the cepstral envelope, so
/// silent bins give a finite (very negative) log instead of `-inf`.
const MAG_EPS: f32 = 1e-9;

/// Pitch ratios are clamped to ±two octaves: well past the DoD's ±12
/// semitones and the range over which the envelope model stays sane.
const MIN_RATIO: f32 = 0.25;
const MAX_RATIO: f32 = 4.0;

/// A frame whose ratio is within this of 1.0 is passed through unmodified
/// (exact reconstruction) rather than round-tripped through the vocoder.
const UNITY_EPS: f32 = 1e-6;

/// Reusable formant-preserving pitch shifter.
///
/// Holds the FFT plans and the analysis window; build once per sample
/// rate and reuse across clips. [`process`](Self::process) is offline and
/// re-entrant — it allocates its own scratch and per-channel phase state,
/// so the same shifter can render the two channels of a stereo clip (or
/// many clips) without cross-talk.
pub struct FormantShifter {
    sample_rate: f32,
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// Cepstral lifter cutoff (quefrency bins kept each side). Lower =
    /// smoother envelope.
    lifter: usize,
}

impl FormantShifter {
    /// Create a shifter for `sample_rate` Hz with a default cepstral
    /// lifter tuned for voice (≈ `sample_rate / 300` quefrency bins).
    ///
    /// # Panics
    /// Panics if `sample_rate` is not positive and finite.
    pub fn new(sample_rate: f32) -> Self {
        assert!(
            sample_rate.is_finite() && sample_rate > 0.0,
            "sample_rate must be positive and finite"
        );
        let lifter = default_lifter(sample_rate);
        Self::with_lifter(sample_rate, lifter)
    }

    /// Create a shifter with an explicit cepstral lifter cutoff (the
    /// number of low-quefrency cepstrum bins kept when smoothing the
    /// spectrum into the formant envelope). Larger follows the spectrum
    /// more closely (down to tracing individual harmonics); smaller gives
    /// a smoother, broader envelope. Clamped to `[4, FRAME/4]`.
    pub fn with_lifter(sample_rate: f32, lifter: usize) -> Self {
        let mut planner = FftPlanner::new();
        Self {
            sample_rate,
            fft: planner.plan_fft_forward(FRAME),
            ifft: planner.plan_fft_inverse(FRAME),
            window: hann_window(FRAME),
            lifter: lifter.clamp(4, FRAME / 4),
        }
    }

    /// The sample rate this shifter was built for.
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// The cepstral lifter cutoff in effect.
    pub fn lifter_cutoff(&self) -> usize {
        self.lifter
    }

    /// Repitch mono `samples` by `ratio_curve`, preserving the spectral
    /// envelope. The returned buffer has exactly `samples.len()` samples.
    ///
    /// `ratio_curve` holds frequency multipliers (`2^(semitones/12)`)
    /// sampled uniformly across the clip: an empty curve or all-ones means
    /// no shift, a single value is a constant ratio, and longer curves are
    /// linearly interpolated to each analysis frame's centre — so the
    /// pitch can sweep over the clip. Ratios are clamped to ±two octaves
    /// and non-finite values fall back to 1.0.
    pub fn process(&self, samples: &[f32], ratio_curve: &[f32]) -> Vec<f32> {
        if samples.is_empty() {
            return Vec::new();
        }

        // Pad so the first and last real samples sit under full window
        // coverage; trim back to the original span at the end.
        let pad = FRAME - HOP;
        let mut padded = vec![0.0f32; pad + samples.len() + FRAME];
        padded[pad..pad + samples.len()].copy_from_slice(samples);

        let total = padded.len();
        let bins = FRAME / 2 + 1;
        let mut signal = vec![0.0f32; total];
        let mut weight = vec![0.0f32; total];

        // Per-frame scratch + phase state (local → re-entrant).
        let mut spectrum = vec![Complex::new(0.0, 0.0); FRAME];
        let mut scratch = vec![
            Complex::new(0.0, 0.0);
            self.fft
                .get_inplace_scratch_len()
                .max(self.ifft.get_inplace_scratch_len())
        ];
        let mut cep = vec![Complex::new(0.0, 0.0); FRAME];
        let mut mag = vec![0.0f32; bins];
        let mut env = vec![0.0f32; bins];
        let mut white = vec![0.0f32; bins];
        let mut anal_phase = vec![0.0f32; bins];
        let mut true_freq = vec![0.0f32; bins];
        let mut last_phase = vec![0.0f32; bins];
        let mut sum_phase = vec![0.0f32; bins];

        let mut start = 0;
        while start + FRAME <= total {
            // Frame-centre position over the original clip, normalised to
            // [0, 1], then sample the ratio curve there.
            let centre = start as f32 + FRAME as f32 * 0.5 - pad as f32;
            let t = (centre / samples.len() as f32).clamp(0.0, 1.0);
            let ratio = sample_curve(ratio_curve, t);

            // Windowed analysis frame → spectrum.
            for j in 0..FRAME {
                spectrum[j] = Complex::new(padded[start + j] * self.window[j], 0.0);
            }
            self.fft.process_with_scratch(&mut spectrum, &mut scratch);

            if (ratio - 1.0).abs() <= UNITY_EPS {
                // Unity: keep the frame exactly, but still advance the
                // phase trackers so a later shifted frame stays coherent.
                for b in 0..bins {
                    let ph = spectrum[b].im.atan2(spectrum[b].re);
                    last_phase[b] = ph;
                    sum_phase[b] = ph;
                }
            } else {
                // Analysis magnitude + per-bin instantaneous frequency.
                for b in 0..bins {
                    let re = spectrum[b].re;
                    let im = spectrum[b].im;
                    mag[b] = (re * re + im * im).sqrt();
                    let ph = im.atan2(re);
                    anal_phase[b] = ph;
                    let omega = std::f32::consts::TAU * b as f32 / FRAME as f32;
                    let expected = omega * HOP as f32;
                    let delta = princ_arg(ph - last_phase[b] - expected);
                    true_freq[b] = omega + delta / HOP as f32;
                    last_phase[b] = ph;
                }

                self.spectral_envelope(&mag, &mut cep, &mut scratch, &mut env);
                for b in 0..bins {
                    white[b] = mag[b] / env[b].max(MAG_EPS);
                }

                // Build the shifted spectrum: move the whitened (formant-
                // free) magnitude up by `ratio`, re-impose the original
                // envelope, and accumulate the frequency-scaled phase.
                let last = bins - 1;
                for k in 0..bins {
                    let src = k as f32 / ratio;
                    let (w, f) = if src <= last as f32 {
                        (interp(&white, src), interp(&true_freq, src))
                    } else {
                        (0.0, 0.0)
                    };
                    let out_mag = w * env[k];
                    let out_freq = f * ratio;
                    sum_phase[k] = princ_arg(sum_phase[k] + out_freq * HOP as f32);
                    spectrum[k] = Complex::from_polar(out_mag, sum_phase[k]);
                }
                // DC and Nyquist must be real for a real inverse transform.
                spectrum[0].im = 0.0;
                spectrum[last].im = 0.0;
                // Hermitian-symmetric upper half.
                for b in 1..last {
                    spectrum[FRAME - b] = spectrum[b].conj();
                }
            }

            self.ifft.process_with_scratch(&mut spectrum, &mut scratch);
            let norm = 1.0 / FRAME as f32;
            for j in 0..FRAME {
                signal[start + j] += spectrum[j].re * norm * self.window[j];
                weight[start + j] += self.window[j] * self.window[j];
            }

            start += HOP;
        }

        // Normalise by accumulated window weight and trim the padding.
        let mut out = vec![0.0f32; samples.len()];
        for (i, o) in out.iter_mut().enumerate() {
            let w = weight[pad + i];
            *o = if w > WEIGHT_EPS {
                signal[pad + i] / w
            } else {
                0.0
            };
        }
        out
    }

    /// Repitch a stereo pair with one shared `ratio_curve`, returning the
    /// shifted `(left, right)`. Channels are processed independently so
    /// each keeps its own phase coherence.
    pub fn process_stereo(
        &self,
        left: &[f32],
        right: &[f32],
        ratio_curve: &[f32],
    ) -> (Vec<f32>, Vec<f32>) {
        (
            self.process(left, ratio_curve),
            self.process(right, ratio_curve),
        )
    }

    /// Cepstral spectral envelope of the magnitude spectrum `mag`
    /// (`FRAME/2 + 1` bins). Smooths the log-magnitude by keeping only the
    /// low-quefrency cepstrum, writing `exp(smoothed log-mag)` into `env`.
    fn spectral_envelope(
        &self,
        mag: &[f32],
        cep: &mut [Complex<f32>],
        scratch: &mut [Complex<f32>],
        env: &mut [f32],
    ) {
        let bins = mag.len();
        // Full (mirror-symmetric) log-magnitude spectrum.
        for (b, c) in cep.iter_mut().enumerate() {
            let src = if b < bins { b } else { FRAME - b };
            *c = Complex::new((mag[src] + MAG_EPS).ln(), 0.0);
        }
        // Real cepstrum (inverse FFT, normalised by FRAME).
        self.ifft.process_with_scratch(cep, scratch);
        let norm = 1.0 / FRAME as f32;
        for c in cep.iter_mut() {
            *c *= norm;
        }
        // Lifter: zero the high quefrencies, keeping `±lifter` bins around
        // DC so only the slow spectral trend (the formants) survives.
        cep[self.lifter..=FRAME - self.lifter].fill(Complex::new(0.0, 0.0));
        // Back to a smoothed log-magnitude, then exponentiate.
        self.fft.process_with_scratch(cep, scratch);
        for (e, c) in env.iter_mut().zip(cep.iter()) {
            *e = c.re.exp();
        }
    }
}

/// Repitch mono `samples` at `sample_rate` Hz by `ratio_curve`, preserving
/// the spectral envelope. Convenience wrapper around [`FormantShifter`] for
/// one-shot use; build a [`FormantShifter`] directly to reuse FFT plans
/// across many clips.
pub fn formant_pitch_shift(samples: &[f32], sample_rate: f32, ratio_curve: &[f32]) -> Vec<f32> {
    FormantShifter::new(sample_rate).process(samples, ratio_curve)
}

/// Default cepstral lifter cutoff for `sample_rate`: ≈ `sr / 300` bins,
/// smoothing over a voice's harmonic comb (f0 ≳ 80 Hz) while keeping the
/// formant peaks.
fn default_lifter(sample_rate: f32) -> usize {
    (sample_rate / 300.0).round() as usize
}

/// Sample a uniformly spaced ratio curve at normalised position `t ∈
/// [0, 1]`, linearly interpolating between points. Empty → 1.0; a single
/// point → that constant. Each sampled value is clamped to the valid
/// ratio range, with non-finite values falling back to 1.0.
fn sample_curve(curve: &[f32], t: f32) -> f32 {
    let raw = match curve {
        [] => 1.0,
        [only] => *only,
        _ => {
            let pos = t.clamp(0.0, 1.0) * (curve.len() - 1) as f32;
            let i = pos.floor() as usize;
            if i >= curve.len() - 1 {
                curve[curve.len() - 1]
            } else {
                let frac = pos - i as f32;
                curve[i] + (curve[i + 1] - curve[i]) * frac
            }
        }
    };
    if raw.is_finite() {
        raw.clamp(MIN_RATIO, MAX_RATIO)
    } else {
        1.0
    }
}

/// Linear interpolation of `data` at fractional index `x` (`x ≥ 0`),
/// clamped to the last sample at the top end.
fn interp(data: &[f32], x: f32) -> f32 {
    let i = x.floor() as usize;
    if i + 1 >= data.len() {
        data[data.len() - 1]
    } else {
        let frac = x - i as f32;
        data[i] + (data[i + 1] - data[i]) * frac
    }
}

/// Wrap a phase to the principal range (−π, π].
fn princ_arg(phase: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let mut p = phase;
    while p > PI {
        p -= TAU;
    }
    while p < -PI {
        p += TAU;
    }
    p
}
