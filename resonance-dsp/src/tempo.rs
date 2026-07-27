//! Tempo / BPM detection over a decoded mono buffer (clip warp, doc #166).
//!
//! [`detect_tempo`] estimates the global tempo of a clip from its raw
//! samples and reports a confidence. It is the analysis half of the warp
//! feature: the engine uses the estimate to seed a clip's BPM (and hence
//! the time-stretch ratio needed to lock it to the project tempo). This
//! module is pure DSP with no engine types — input is `&[f32]` plus a
//! sample rate, output is a [`TempoEstimate`].
//!
//! # How it works
//!
//! 1. **Onset envelope.** A short-time Fourier transform (Hann-windowed,
//!    [`TempoConfig::window_size`] with [`TempoConfig::hop_size`] hop)
//!    yields a per-frame magnitude spectrum; the half-wave-rectified
//!    **spectral flux** (sum of positive bin-to-bin magnitude increases)
//!    gives an onset-detection function (ODF) that spikes at note/beat
//!    onsets and is sampled at `sample_rate / hop_size` Hz.
//! 2. **Periodicity.** The mean-removed ODF is autocorrelated. Beats are
//!    periodic, so the ODF's autocorrelation peaks at the beat period and
//!    its multiples.
//! 3. **Comb / enhanced autocorrelation.** Each candidate lag is scored by
//!    summing the autocorrelation at the lag and its first few harmonics
//!    (`lag, 2·lag, 3·lag, …`). The true beat period lines up with peaks
//!    at *all* its multiples, so this reinforces the fundamental tempo over
//!    spurious half-tempo candidates. The search is restricted to lags
//!    inside `[min_bpm, max_bpm]`, and the winning lag is refined with
//!    parabolic interpolation for sub-frame resolution.
//!
//! The estimate the detector returns already lies inside the configured
//! `[min_bpm, max_bpm]` band. To fold a tempo known from elsewhere (a tag,
//! a user entry, a detection with a different band) into a target range by
//! octaves, use [`fold_bpm`].
//!
//! Everything here is offline and deterministic: no RNG, no global state.

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

use crate::window::hann_window;

/// Configuration for [`detect_tempo`].
///
/// Build with [`TempoConfig::new`] for sensible defaults, then override
/// fields as needed. Sizes are in samples so the analysis is deterministic
/// for a given sample rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoConfig {
    /// Sample rate of the input signal, in Hz.
    pub sample_rate: f32,
    /// STFT analysis frame length, in samples. A power of two for the FFT.
    pub window_size: usize,
    /// Hop between successive STFT frames, in samples. Sets the onset
    /// envelope's sample rate (`sample_rate / hop_size`).
    pub hop_size: usize,
    /// Slowest tempo to consider, in BPM. Bounds the autocorrelation lag
    /// search (longest lag) and the returned estimate.
    pub min_bpm: f32,
    /// Fastest tempo to consider, in BPM. Bounds the lag search (shortest
    /// lag) and the returned estimate.
    pub max_bpm: f32,
}

impl TempoConfig {
    /// Defaults for the given sample rate: 1024-sample frames, 256-sample
    /// hop, searching 60–200 BPM.
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            window_size: 1024,
            hop_size: 256,
            min_bpm: 60.0,
            max_bpm: 200.0,
        }
    }
}

/// Result of [`detect_tempo`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoEstimate {
    /// Estimated tempo in beats per minute, inside the configured
    /// `[min_bpm, max_bpm]` band. `0.0` when no tempo could be estimated
    /// (too little input).
    pub bpm: f32,
    /// Periodicity confidence in `[0, 1]`: the normalised autocorrelation
    /// at the chosen beat period. Higher means a steadier, more clearly
    /// periodic pulse. `0.0` accompanies a `0.0` BPM.
    pub confidence: f32,
}

/// Number of harmonics summed by the enhanced-autocorrelation comb.
const COMB_HARMONICS: usize = 4;

/// Centre of the tempo-preference weighting, in BPM. Comb scores are
/// nudged toward this tempo to resolve octave ambiguity: a pure pulse has
/// near-equal autocorrelation peaks at the beat period *and* its multiples,
/// so the comb alone cannot tell 70 from 140 BPM — the weighting tips the
/// choice toward the perceptually more likely tempo.
const WEIGHT_CENTER_BPM: f32 = 120.0;
/// Standard deviation of the (log2-domain) tempo preference, in octaves. A
/// gentle curve: it only decides between otherwise near-tied octaves, not
/// against a clearly stronger candidate.
const WEIGHT_OCTAVES: f32 = 1.0;

/// Estimate the tempo of `samples` (a mono buffer at `config.sample_rate`).
///
/// Returns the BPM (within `[min_bpm, max_bpm]`) and a `[0, 1]` confidence.
/// Buffers too short to span a beat period at `min_bpm` yield
/// `{ bpm: 0.0, confidence: 0.0 }`.
pub fn detect_tempo(samples: &[f32], config: TempoConfig) -> TempoEstimate {
    let odf = onset_envelope(samples, &config);
    estimate_from_odf(&odf, &config)
}

/// Convenience wrapper: [`detect_tempo`] with [`TempoConfig::new`] defaults.
pub fn detect_tempo_default(samples: &[f32], sample_rate: f32) -> TempoEstimate {
    detect_tempo(samples, TempoConfig::new(sample_rate))
}

/// Fold `bpm` into `[min_bpm, max_bpm)` by repeatedly doubling or halving
/// (octave moves), so an out-of-band estimate maps to the equivalent tempo
/// in the target range — e.g. `fold_bpm(60, 70, 180) == 120`.
///
/// Returns `bpm` unchanged if it is not finite, is non-positive, or the
/// range is degenerate (`min_bpm <= 0` or `max_bpm <= min_bpm`).
pub fn fold_bpm(bpm: f32, min_bpm: f32, max_bpm: f32) -> f32 {
    if !bpm.is_finite() || bpm <= 0.0 || min_bpm <= 0.0 || max_bpm <= min_bpm {
        return bpm;
    }
    let mut b = bpm;
    while b < min_bpm {
        b *= 2.0;
    }
    while b >= max_bpm {
        b *= 0.5;
    }
    // A range narrower than an octave can leave `b` just under `min_bpm`
    // after halving; nudge it back up rather than loop forever.
    if b < min_bpm {
        b = min_bpm;
    }
    b
}

// ---------------------------------------------------------------------------
// Onset envelope (spectral flux)
// ---------------------------------------------------------------------------

/// Compute the half-wave-rectified spectral-flux onset envelope of
/// `samples`: one value per STFT hop, spiking at onsets. Empty if the
/// buffer is shorter than a single analysis window.
fn onset_envelope(samples: &[f32], config: &TempoConfig) -> Vec<f32> {
    let n = config.window_size;
    let hop = config.hop_size.max(1);
    if n < 2 || samples.len() < n {
        return Vec::new();
    }
    let window = hann_window(n);
    let bins = n / 2 + 1;

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(n);
    let mut scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
    let mut spectrum = vec![Complex::new(0.0, 0.0); n];

    let mut prev_mag = vec![0.0f32; bins];
    let mut cur_mag = vec![0.0f32; bins];
    let mut flux = Vec::new();

    let mut pos = 0;
    let mut have_prev = false;
    while pos + n <= samples.len() {
        for j in 0..n {
            spectrum[j] = Complex::new(samples[pos + j] * window[j], 0.0);
        }
        fft.process_with_scratch(&mut spectrum, &mut scratch);
        for b in 0..bins {
            cur_mag[b] = spectrum[b].norm();
        }
        if have_prev {
            let mut f = 0.0;
            for b in 0..bins {
                let d = cur_mag[b] - prev_mag[b];
                if d > 0.0 {
                    f += d;
                }
            }
            flux.push(f);
        }
        std::mem::swap(&mut prev_mag, &mut cur_mag);
        have_prev = true;
        pos += hop;
    }
    flux
}

// ---------------------------------------------------------------------------
// Periodicity estimate (enhanced autocorrelation)
// ---------------------------------------------------------------------------

/// Estimate tempo from a precomputed onset envelope. Separated from
/// [`onset_envelope`] so the periodicity logic is unit-testable on a
/// synthetic ODF without an FFT round-trip.
fn estimate_from_odf(odf: &[f32], config: &TempoConfig) -> TempoEstimate {
    let none = TempoEstimate {
        bpm: 0.0,
        confidence: 0.0,
    };
    let len = odf.len();
    if len < 4 {
        return none;
    }
    let odf_rate = config.sample_rate / config.hop_size.max(1) as f32; // ODF frames/sec

    // Lag (in ODF frames) ↔ BPM: bpm = 60 · odf_rate / lag.
    let lag_min = (60.0 * odf_rate / config.max_bpm).floor().max(1.0) as usize;
    let lag_max = ((60.0 * odf_rate / config.min_bpm).ceil() as usize).min(len - 1);
    if lag_max <= lag_min {
        return none;
    }

    // Mean-removed ODF so the autocorrelation measures pulse periodicity
    // rather than the (large, lag-independent) DC energy.
    let mean = odf.iter().sum::<f32>() / len as f32;
    let centered: Vec<f32> = odf.iter().map(|x| x - mean).collect();

    // Unbiased, zero-lag-normalised autocorrelation up to the highest lag
    // the comb needs. Normalising by `acf[0]` puts coefficients in
    // `[-1, 1]`, so they double as a confidence measure.
    let max_lag = (lag_max * COMB_HARMONICS).min(len - 1);
    let acf = autocorrelation(&centered, max_lag);
    if acf[0] <= 0.0 {
        return none; // flat / silent envelope: no periodicity
    }

    // Score each candidate lag by its comb sum (lag + harmonics that fall
    // within the computed range). The true period peaks at all multiples,
    // so it outscores partial candidates that miss some multiples. A gentle
    // log-normal tempo preference then breaks the residual octave ambiguity
    // (e.g. 70 vs 140 BPM) toward the more likely tempo. Refinement uses the
    // unweighted comb so the preference biases the *choice* of octave, not
    // the interpolated BPM within it.
    let mut comb = vec![f32::NEG_INFINITY; lag_max + 1];
    let mut best_lag = lag_min;
    let mut best_weighted = f32::NEG_INFINITY;
    for lag in lag_min..=lag_max {
        let mut s = 0.0;
        for h in 1..=COMB_HARMONICS {
            let l = h * lag;
            if l <= max_lag {
                s += acf[l];
            }
        }
        comb[lag] = s;
        let bpm = 60.0 * odf_rate / lag as f32;
        let weighted = s * tempo_weight(bpm);
        if weighted > best_weighted {
            best_weighted = weighted;
            best_lag = lag;
        }
    }

    // Sub-frame refinement: parabolic interpolation of the comb score
    // around the winning integer lag.
    let refined_lag = parabolic_peak(&comb, best_lag);
    let bpm = 60.0 * odf_rate / refined_lag;
    let confidence = acf[best_lag].clamp(0.0, 1.0);

    TempoEstimate { bpm, confidence }
}

/// Log-normal tempo preference in `(0, 1]`, peaking at
/// [`WEIGHT_CENTER_BPM`] and falling off with [`WEIGHT_OCTAVES`] spread in
/// the log2 (octave) domain. Used only to resolve octave ambiguity.
fn tempo_weight(bpm: f32) -> f32 {
    let octaves = (bpm / WEIGHT_CENTER_BPM).log2() / WEIGHT_OCTAVES;
    (-0.5 * octaves * octaves).exp()
}

/// Unbiased autocorrelation of `x` for lags `0..=max_lag`, normalised so
/// that `acf[0] == 1` (when the signal has any energy). Each lag is divided
/// by its overlap count, so long lags are not penalised for the shorter sum.
fn autocorrelation(x: &[f32], max_lag: usize) -> Vec<f32> {
    let n = x.len();
    let mut acf = vec![0.0f32; max_lag + 1];
    for (lag, slot) in acf.iter_mut().enumerate() {
        let count = n - lag;
        let mut sum = 0.0f32;
        for i in 0..count {
            sum += x[i] * x[i + lag];
        }
        *slot = sum / count as f32;
    }
    let zero = acf[0];
    if zero > 0.0 {
        for v in acf.iter_mut() {
            *v /= zero;
        }
    }
    acf
}

/// Refine an integer peak at `peak` in `y` to sub-sample position via
/// parabolic interpolation over its immediate neighbours. Falls back to the
/// integer index at the array edges or when the three points are not
/// concave.
fn parabolic_peak(y: &[f32], peak: usize) -> f32 {
    if peak == 0 || peak + 1 >= y.len() {
        return peak as f32;
    }
    let ym = y[peak - 1];
    let y0 = y[peak];
    let yp = y[peak + 1];
    let denom = ym - 2.0 * y0 + yp;
    if denom >= 0.0 {
        return peak as f32; // not a (strict) maximum: keep the integer lag
    }
    let offset = 0.5 * (ym - yp) / denom;
    peak as f32 + offset.clamp(-0.5, 0.5)
}
