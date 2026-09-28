//! Harmonic-signature analysis for the insert-chain probe
//! (warmth-width-depth.md §2.1, §7.3).
//!
//! The probe drives a chain with a synthetic signal and reads its
//! harmonics off one FFT. Both halves live here so the engine only moves
//! samples:
//!
//! * [`probe_sine`] / [`smpte_pair`] generate the stimulus, at frequencies
//!   snapped to FFT bins of a [`PROBE_LEN`]-point frame
//!   ([`bin_exact_hz`]). A bin-exact, steady-state tone has no leakage
//!   under a rectangular window, so every harmonic — and every aliased
//!   harmonic, which folds onto an integer bin too — lands on exactly one
//!   bin. That is what lets [`aliasing_floor_dbc`](HarmonicReport::aliasing_floor_dbc)
//!   reach −150 dBc without window sidelobes masking it.
//! * [`analyze_harmonics`] / [`imd_pct`] read the output frame.
//!
//! Levels are dBc: relative to the fundamental at the output, floored at
//! [`FLOOR_DBC`].

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// Analysis frame, samples (1.37 s at 48 kHz, 0.73 Hz bins).
pub const PROBE_LEN: usize = 65_536;
/// Lowest level a harmonic or the aliasing floor reports, dBc.
pub const FLOOR_DBC: f64 = -160.0;
/// Highest harmonic order reported (`h2` .. `h9`).
pub const MAX_ORDER: usize = 9;
/// Harmonics at or below this are treated as absent when fitting the
/// per-order decay: a symmetric shaper has no even orders at all, and a
/// fit through its numerical noise would be meaningless.
pub const DECAY_FIT_FLOOR_DBC: f64 = -140.0;
/// Bins either side of the fundamental and each harmonic excluded from
/// the aliasing search, so a chain with slight modulation does not read
/// its own skirts as aliasing.
pub const GUARD_BINS: usize = 2;
/// SMPTE RP120 IMD pair: low tone, Hz.
pub const SMPTE_LOW_HZ: f64 = 60.0;
/// SMPTE RP120 IMD pair: high tone, Hz.
pub const SMPTE_HIGH_HZ: f64 = 7_000.0;
/// Sidebands `high ± n·low` summed into [`imd_pct`], n = 1..=this.
pub const IMD_SIDEBANDS: usize = 4;

/// `freq` moved to the nearest centre of a [`PROBE_LEN`]-point FFT bin
/// (never DC, never past Nyquist).
pub fn bin_exact_hz(sample_rate: f64, freq: f64) -> f64 {
    let bin = sample_rate / PROBE_LEN as f64;
    let k = (freq / bin).round().clamp(1.0, (PROBE_LEN / 2 - 1) as f64);
    k * bin
}

/// A sine of `frames` samples at `level_dbfs` peak. `freq` should come
/// from [`bin_exact_hz`]. Phase is computed in f64, so the tone stays
/// clean for as long as a probe runs.
pub fn probe_sine(sample_rate: f64, freq: f64, level_dbfs: f64, frames: usize) -> Vec<f32> {
    let amp = 10f64.powf(level_dbfs / 20.0);
    (0..frames)
        .map(|n| (amp * (std::f64::consts::TAU * freq * n as f64 / sample_rate).sin()) as f32)
        .collect()
}

/// The SMPTE IMD stimulus: [`SMPTE_LOW_HZ`] and [`SMPTE_HIGH_HZ`] (both
/// bin-exact) at 4:1 amplitude, their summed peak at `level_dbfs`.
/// Returns the samples and the two exact frequencies.
pub fn smpte_pair(sample_rate: f64, level_dbfs: f64, frames: usize) -> (Vec<f32>, f64, f64) {
    let low = bin_exact_hz(sample_rate, SMPTE_LOW_HZ);
    let high = bin_exact_hz(sample_rate, SMPTE_HIGH_HZ);
    let peak = 10f64.powf(level_dbfs / 20.0);
    let (a_low, a_high) = (peak * 0.8, peak * 0.2);
    let tau = std::f64::consts::TAU;
    let samples = (0..frames)
        .map(|n| {
            let t = n as f64 / sample_rate;
            (a_low * (tau * low * t).sin() + a_high * (tau * high * t).sin()) as f32
        })
        .collect();
    (samples, low, high)
}

/// What [`analyze_harmonics`] reads off one output frame.
#[derive(Debug, Clone, PartialEq)]
pub struct HarmonicReport {
    /// The (bin-exact) fundamental, Hz.
    pub freq_hz: f64,
    /// Fundamental level at the output, dBFS (peak).
    pub fundamental_dbfs: f64,
    /// Total harmonic distortion over the in-band harmonics 2..=9, %:
    /// `100 · √Σ A_k² / A_1`.
    pub thd_pct: f64,
    /// `h[0]` is H2 … `h[7]` is H9, dBc, floored at [`FLOOR_DBC`]. `None`
    /// for a harmonic above Nyquist (its energy, folded back, is what
    /// [`aliasing_floor_dbc`](Self::aliasing_floor_dbc) measures).
    pub h: Vec<Option<f64>>,
    /// H2 − H3, dB. Positive is even-dominant ("warm"), negative
    /// odd-dominant. `None` if either is above Nyquist.
    pub h2_h3_db: Option<f64>,
    /// How fast the harmonic series falls, dB per order (positive =
    /// falling): minus the least-squares slope of the in-band harmonics
    /// that stand above [`DECAY_FIT_FLOOR_DBC`]. `None` with fewer than
    /// two such harmonics.
    pub decay_db_per_order: Option<f64>,
    /// The strongest bin that is neither DC, the fundamental nor an
    /// in-band harmonic (± [`GUARD_BINS`]), dBc: aliasing, plus whatever
    /// noise or inharmonic products the chain adds.
    pub aliasing_floor_dbc: f64,
}

/// Single-sided amplitude spectrum of a [`PROBE_LEN`] frame, rectangular
/// window: a bin-exact sine of peak amplitude `A` reads `A` in its bin.
fn amplitudes(frame: &[f32]) -> Vec<f64> {
    assert_eq!(frame.len(), PROBE_LEN, "a probe frame is PROBE_LEN samples");
    let mut buf: Vec<Complex<f64>> = frame.iter().map(|&s| Complex::new(s as f64, 0.0)).collect();
    FftPlanner::<f64>::new().plan_fft_forward(PROBE_LEN).process(&mut buf);
    buf[..PROBE_LEN / 2]
        .iter()
        .map(|c| 2.0 * c.norm() / PROBE_LEN as f64)
        .collect()
}

fn dbc(amp: f64, fundamental: f64) -> f64 {
    if amp > 0.0 && fundamental > 0.0 {
        (20.0 * (amp / fundamental).log10()).max(FLOOR_DBC)
    } else {
        FLOOR_DBC
    }
}

/// Analyze one steady-state output `frame` ([`PROBE_LEN`] samples) of a
/// chain driven by a bin-exact sine at `freq_hz`.
pub fn analyze_harmonics(sample_rate: f64, freq_hz: f64, frame: &[f32]) -> HarmonicReport {
    let amp = amplitudes(frame);
    let bin_hz = sample_rate / PROBE_LEN as f64;
    let fundamental_bin = (freq_hz / bin_hz).round() as usize;
    let half = PROBE_LEN / 2;
    let a1 = amp[fundamental_bin];

    let mut h = Vec::with_capacity(MAX_ORDER - 1);
    let mut excluded = vec![false; half];
    let guard = |bin: usize, excluded: &mut [bool]| {
        let top = (bin + GUARD_BINS).min(excluded.len() - 1);
        excluded[bin.saturating_sub(GUARD_BINS)..=top].fill(true);
    };
    guard(fundamental_bin, &mut excluded);
    excluded[..=GUARD_BINS].fill(true); // DC and its skirt
    let mut thd_sq = 0.0f64;
    for order in 2..=MAX_ORDER {
        let bin = fundamental_bin * order;
        if bin >= half {
            h.push(None);
            continue;
        }
        guard(bin, &mut excluded);
        thd_sq += amp[bin] * amp[bin];
        h.push(Some(dbc(amp[bin], a1)));
    }
    let aliasing = (0..half)
        .filter(|&b| !excluded[b])
        .map(|b| amp[b])
        .fold(0.0f64, f64::max);

    let h2_h3_db = match (h[0], h[1]) {
        (Some(h2), Some(h3)) => Some(h2 - h3),
        _ => None,
    };
    let fit: Vec<(f64, f64)> = h
        .iter()
        .enumerate()
        .filter_map(|(i, level)| {
            level
                .filter(|&l| l > DECAY_FIT_FLOOR_DBC)
                .map(|l| ((i + 2) as f64, l))
        })
        .collect();
    let decay_db_per_order = (fit.len() >= 2).then(|| {
        let n = fit.len() as f64;
        let mx = fit.iter().map(|p| p.0).sum::<f64>() / n;
        let my = fit.iter().map(|p| p.1).sum::<f64>() / n;
        let sxy: f64 = fit.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
        let sxx: f64 = fit.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum();
        -sxy / sxx
    });

    HarmonicReport {
        freq_hz: fundamental_bin as f64 * bin_hz,
        fundamental_dbfs: if a1 > 0.0 { 20.0 * a1.log10() } else { -200.0 },
        thd_pct: if a1 > 0.0 { 100.0 * thd_sq.sqrt() / a1 } else { 0.0 },
        h,
        h2_h3_db,
        decay_db_per_order,
        aliasing_floor_dbc: dbc(aliasing, a1),
    }
}

/// SMPTE intermodulation of one steady-state output `frame` driven by
/// [`smpte_pair`]: `100 · √Σ (A(high ± n·low))² / A(high)` over n =
/// 1..=[`IMD_SIDEBANDS`], %. `None` when the high tone did not come
/// through.
pub fn imd_pct(sample_rate: f64, low_hz: f64, high_hz: f64, frame: &[f32]) -> Option<f64> {
    let amp = amplitudes(frame);
    let bin_hz = sample_rate / PROBE_LEN as f64;
    let (lo, hi) = (
        (low_hz / bin_hz).round() as usize,
        (high_hz / bin_hz).round() as usize,
    );
    let carrier = amp[hi];
    if !(carrier > 0.0) {
        return None;
    }
    let mut sum = 0.0;
    for n in 1..=IMD_SIDEBANDS {
        for bin in [hi + n * lo, hi.saturating_sub(n * lo)] {
            if bin > 0 && bin < amp.len() {
                sum += amp[bin] * amp[bin];
            }
        }
    }
    Some(100.0 * sum.sqrt() / carrier)
}
