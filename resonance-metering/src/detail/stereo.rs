//! Width and mono-safety proxies (warmth-width-depth.md §2.2, §7.1
//! `stereo`).
//!
//! Per-band figures come from the shared Welch analysis
//! ([`StereoSpectrum`]): a band's `E[L²]`, `E[R²]` and `E[L·R]` are sums
//! of the auto- and cross-power bins inside it, which is exactly the
//! correlation of the band-limited signals. Mid and side follow from
//! those three: `M = (L+R)/2`, `S = (L−R)/2`, so
//! `E[M²] = (LL + RR + 2·LR)/4` and `E[S²] = (LL + RR − 2·LR)/4`.
//!
//! With equal L/R energy, correlation `r` and the side/mid power ratio
//! `ρ` are one number in two spellings: `r = (1 − ρ)/(1 + ρ)`.
//!
//! ## Hard-panned mono
//!
//! One silent channel makes the correlation 0/0. Meters in the field
//! disagree on whether that is 0 or +1, and either answer misleads, so
//! this module reports it explicitly: [`StereoDetail::one_sided`] is set,
//! every correlation that would be 0/0 is `None`, and
//! [`StereoDetail::balance_db`] says which side carries the signal.

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

use crate::spectrum::offline::{Channel, StereoSpectrum};

/// Edges of the eight [`StereoDetail::bands`], Hz. The 150 Hz and 1 kHz
/// edges are where the §2.2 health rules change (lows ≥ +0.9, mids ≥ +0.5,
/// highs ≥ 0).
pub const STEREO_BAND_EDGES_HZ: [f64; 9] = [
    20.0, 60.0, 150.0, 400.0, 1_000.0, 2_500.0, 5_000.0, 10_000.0, 20_000.0,
];

/// A channel (or band) whose energy is this far below the other side's is
/// treated as silent: `-40 dB`.
pub const ONE_SIDED_RATIO: f64 = 1e-4;

/// A band holding less than this fraction of the signal's total power
/// (`-70 dB`) reports `None` for all three of its figures: what is left
/// there is window leakage from other bands, not content.
pub const EMPTY_BAND_RATIO: f64 = 1e-7;

/// Largest magnitude `side_mid_db`, `balance_db` and `mono_loss_db`
/// report, dB. `-60` side/mid is "no side at all", `+60` is "no mid"
/// (anti-phase); `±60` balance is one channel silent.
pub const RATIO_LIMIT_DB: f32 = 60.0;

/// Length of one [`CorrelationWindows`] window, seconds.
pub const WINDOW_SECS: f64 = 0.4;
/// A window whose mean-square level is below this (`-100 dBFS`) is
/// silence and is not counted.
pub const WINDOW_SILENCE_MS: f64 = 1e-10;
/// The `pct_below_0_3` threshold.
pub const WINDOW_WARN_CORRELATION: f32 = 0.3;

/// Shortest lag [`StereoDetail::haas_lag_ms`] reports, ms.
pub const HAAS_MIN_MS: f64 = 1.0;
/// Longest lag searched, ms.
pub const HAAS_MAX_MS: f64 = 35.0;
/// Normalized cross-correlation the peak must exceed.
pub const HAAS_MIN_CORRELATION: f64 = 0.5;
/// How far the delayed peak must stand above the zero-lag correlation
/// (the best match within ±1 ms): a mono signal with a strong periodic
/// component correlates with itself one period later, which is not a
/// delay between the channels.
pub const HAAS_ZERO_LAG_MARGIN: f64 = 0.05;

/// Rectangular analysis frame of the cross-correlation, samples. Zero-
/// padded to twice this for a linear (not circular) correlation.
const XCORR_FRAME: usize = 8_192;

/// One of the eight [`StereoDetail::bands`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StereoBand {
    /// Lower edge, Hz.
    pub lo_hz: f32,
    /// Upper edge, Hz.
    pub hi_hz: f32,
    /// Correlation of L and R inside the band, `[-1, 1]`. `None` for an
    /// empty band or a one-sided one (one channel ≥ 40 dB below the
    /// other), where it would be 0/0.
    pub correlation: Option<f32>,
    /// `10·log10(E[S²]/E[M²])` inside the band, dB, clamped to
    /// ±[`RATIO_LIMIT_DB`]. `-60` is mono, `0` is hard-panned or equal
    /// and uncorrelated, `+60` is anti-phase; see the module docs for its
    /// tie to `correlation`. `None` for an empty band.
    pub side_mid_db: Option<f32>,
    /// Level lost in the band when folded to mono, dB: `(L+R)/2` played on
    /// both speakers against the stereo original. `0` for mono, about
    /// `-3` for uncorrelated or hard-panned content, `-60` (the floor) for
    /// anti-phase. `None` for an empty band.
    pub mono_loss_db: Option<f32>,
}

/// 400 ms windowed correlation over the range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CorrelationWindows {
    /// Windows counted: silent and one-sided windows are skipped.
    pub windows: u32,
    /// Percentage of counted windows with correlation below +0.3.
    pub pct_below_0_3: f32,
    /// Lowest window correlation.
    pub worst: f32,
    /// Start of that window, seconds from the start of the buffer.
    pub worst_at_seconds: f64,
}

/// The `stereo` detail of one measurement.
#[derive(Debug, Clone, PartialEq)]
pub struct StereoDetail {
    /// Eight bands at [`STEREO_BAND_EDGES_HZ`].
    pub bands: Vec<StereoBand>,
    /// Windowed correlation summary; `None` when no window was countable
    /// (silence, or a one-sided signal).
    pub correlation_windows: Option<CorrelationWindows>,
    /// `10·log10(E[L²]/E[R²])`, dB, clamped to ±[`RATIO_LIMIT_DB`].
    /// Positive leans left. `None` for silence.
    pub balance_db: Option<f32>,
    /// One channel carries (almost) nothing: the other is at least 40 dB
    /// louder. Correlations are `None` then, never 0 or +1.
    pub one_sided: bool,
    /// Lag of the strongest normalized L/R cross-correlation peak between
    /// 1 and 35 ms, when it exceeds 0.5 and beats the zero-lag
    /// correlation — a static inter-channel delay (Haas), which combs in
    /// mono with nulls at `(2k+1)/(2·lag)`. Positive means the RIGHT
    /// channel is late. `None` when there is no such delay.
    pub haas_lag_ms: Option<f32>,
}

/// Compute the `stereo` detail of a stereo buffer, running the shared
/// analysis itself. See [`stereo_detail_from`].
pub fn stereo_detail(sample_rate: f32, left: &[f32], right: &[f32]) -> StereoDetail {
    stereo_detail_from(&super::analyze_detail(sample_rate, left, right), left, right)
}

/// Compute the `stereo` detail from an already-run analysis of the same
/// `left` / `right` (the time-domain figures need the samples too).
pub fn stereo_detail_from(spec: &StereoSpectrum, left: &[f32], right: &[f32]) -> StereoDetail {
    let n = left.len().min(right.len());
    let (left, right) = (&left[..n], &right[..n]);
    let (el, er, _) = energies(left, right);
    let one_sided = is_one_sided(el, er);

    let nyquist = spec.sample_rate as f64 / 2.0;
    let total = spec.band(Channel::Left, 0.0, nyquist) + spec.band(Channel::Right, 0.0, nyquist);
    let bands = STEREO_BAND_EDGES_HZ
        .windows(2)
        .map(|edge| stereo_band(spec, edge[0], edge[1], total))
        .collect();

    let balance_db = (el + er > 0.0).then(|| clamp_db(10.0 * (el / er).log10()));
    let silent = el + er <= 0.0;
    StereoDetail {
        bands,
        correlation_windows: correlation_windows(spec.sample_rate as f64, left, right),
        balance_db,
        one_sided,
        haas_lag_ms: (!one_sided && !silent)
            .then(|| haas_lag_ms(spec.sample_rate as f64, left, right, el, er))
            .flatten(),
    }
}

/// `(Σl², Σr², Σl·r)` in f64.
fn energies(left: &[f32], right: &[f32]) -> (f64, f64, f64) {
    let (mut ll, mut rr, mut lr) = (0.0f64, 0.0f64, 0.0f64);
    for (&l, &r) in left.iter().zip(right) {
        let (l, r) = (l as f64, r as f64);
        ll += l * l;
        rr += r * r;
        lr += l * r;
    }
    (ll, rr, lr)
}

fn is_one_sided(ll: f64, rr: f64) -> bool {
    let (lo, hi) = (ll.min(rr), ll.max(rr));
    hi > 0.0 && lo < ONE_SIDED_RATIO * hi
}

/// A dB value clamped to ±[`RATIO_LIMIT_DB`]; ±inf clamps too.
fn clamp_db(db: f64) -> f32 {
    let limit = RATIO_LIMIT_DB as f64;
    if db.is_nan() {
        return 0.0;
    }
    db.clamp(-limit, limit) as f32
}

fn stereo_band(spec: &StereoSpectrum, lo: f64, hi: f64, total: f64) -> StereoBand {
    let ll = spec.band(Channel::Left, lo, hi);
    let rr = spec.band(Channel::Right, lo, hi);
    let lr = spec.band(Channel::Cross, lo, hi);
    let mut band = StereoBand {
        lo_hz: lo as f32,
        hi_hz: hi as f32,
        correlation: None,
        side_mid_db: None,
        mono_loss_db: None,
    };
    if !(total > 0.0) || ll + rr <= EMPTY_BAND_RATIO * total {
        return band;
    }
    if !is_one_sided(ll, rr) {
        band.correlation = Some((lr / (ll * rr).sqrt()).clamp(-1.0, 1.0) as f32);
    }
    let mid = ((ll + rr + 2.0 * lr) / 4.0).max(0.0);
    let side = ((ll + rr - 2.0 * lr) / 4.0).max(0.0);
    band.side_mid_db = Some(clamp_db(10.0 * (side / mid).log10()));
    band.mono_loss_db = Some(clamp_db(10.0 * (2.0 * mid / (ll + rr)).log10()));
    band
}

/// Correlation of non-overlapping [`WINDOW_SECS`] windows. A buffer
/// shorter than one window is one window.
fn correlation_windows(sample_rate: f64, left: &[f32], right: &[f32]) -> Option<CorrelationWindows> {
    let n = left.len();
    let len = ((WINDOW_SECS * sample_rate) as usize).max(1);
    let mut counted = 0u32;
    let mut below = 0u32;
    let mut worst = (f32::INFINITY, 0usize);
    let mut start = 0usize;
    while start < n {
        let stop = if n - start < 2 * len { n } else { start + len };
        let (ll, rr, lr) = energies(&left[start..stop], &right[start..stop]);
        let frames = (stop - start) as f64;
        let quiet = (ll + rr) / (2.0 * frames) < WINDOW_SILENCE_MS;
        if !quiet && !is_one_sided(ll, rr) {
            let r = (lr / (ll * rr).sqrt()).clamp(-1.0, 1.0) as f32;
            counted += 1;
            if r < WINDOW_WARN_CORRELATION {
                below += 1;
            }
            if r < worst.0 {
                worst = (r, start);
            }
        }
        start = stop;
    }
    (counted > 0).then(|| CorrelationWindows {
        windows: counted,
        pct_below_0_3: 100.0 * below as f32 / counted as f32,
        worst: worst.0,
        worst_at_seconds: worst.1 as f64 / sample_rate,
    })
}

/// The Haas detector: see [`StereoDetail::haas_lag_ms`].
///
/// `c(τ) = Σ l[n]·r[n+τ]` over rectangular [`XCORR_FRAME`] frames,
/// zero-padded to twice their length so the FFT correlation is linear,
/// divided by the number of sample pairs each lag actually saw (so the
/// estimate is unbiased at every lag) and normalized by the channels'
/// mean squares.
fn haas_lag_ms(sample_rate: f64, left: &[f32], right: &[f32], el: f64, er: f64) -> Option<f32> {
    let n = left.len();
    let max_lag = ((HAAS_MAX_MS * 1e-3 * sample_rate).ceil() as usize).min(n.saturating_sub(1));
    let min_lag = (HAAS_MIN_MS * 1e-3 * sample_rate).round() as usize;
    if max_lag <= min_lag || el <= 0.0 || er <= 0.0 {
        return None;
    }
    let frame = XCORR_FRAME.max(4 * max_lag).min(n.next_power_of_two());
    let fft_len = 2 * frame.next_power_of_two();
    let mut planner = FftPlanner::<f64>::new();
    let forward = planner.plan_fft_forward(fft_len);
    let inverse = planner.plan_fft_inverse(fft_len);
    let zero = Complex::new(0.0f64, 0.0);
    let mut xl = vec![zero; fft_len];
    let mut xr = vec![zero; fft_len];

    // Index `max_lag + τ` holds lag τ in -max_lag..=max_lag.
    let mut sum = vec![0.0f64; 2 * max_lag + 1];
    let mut pairs = vec![0.0f64; 2 * max_lag + 1];
    let mut start = 0usize;
    while start < n {
        let m = frame.min(n - start);
        for i in 0..fft_len {
            let (l, r) = if i < m {
                (left[start + i] as f64, right[start + i] as f64)
            } else {
                (0.0, 0.0)
            };
            xl[i] = Complex::new(l, 0.0);
            xr[i] = Complex::new(r, 0.0);
        }
        forward.process(&mut xl);
        forward.process(&mut xr);
        for i in 0..fft_len {
            xl[i] = xl[i].conj() * xr[i];
        }
        inverse.process(&mut xl);
        for (slot, lag) in (-(max_lag as isize)..=max_lag as isize).enumerate() {
            if lag.unsigned_abs() >= m {
                continue;
            }
            let at = lag.rem_euclid(fft_len as isize) as usize;
            sum[slot] += xl[at].re / fft_len as f64;
            pairs[slot] += (m - lag.unsigned_abs()) as f64;
        }
        start += m;
    }

    let norm = ((el / n as f64) * (er / n as f64)).sqrt();
    let r: Vec<f64> = sum
        .iter()
        .zip(&pairs)
        .map(|(&s, &p)| if p > 0.0 { s / p / norm } else { 0.0 })
        .collect();
    let at_lag = |lag: isize| r[(lag + max_lag as isize) as usize];

    let zero_lag = (-(min_lag as isize) + 1..min_lag as isize)
        .map(at_lag)
        .fold(f64::NEG_INFINITY, f64::max);
    let (best_lag, best) = (-(max_lag as isize)..=max_lag as isize)
        .filter(|lag| lag.unsigned_abs() >= min_lag)
        .map(|lag| (lag, at_lag(lag)))
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    if best <= HAAS_MIN_CORRELATION || best < zero_lag + HAAS_ZERO_LAG_MARGIN {
        return None;
    }

    // Parabolic interpolation around the peak for sub-sample precision.
    let mut lag = best_lag as f64;
    if best_lag.unsigned_abs() < max_lag {
        let (y0, y1, y2) = (at_lag(best_lag - 1), best, at_lag(best_lag + 1));
        let denom = y0 - 2.0 * y1 + y2;
        if denom < 0.0 {
            lag += (0.5 * (y0 - y2) / denom).clamp(-0.5, 0.5);
        }
    }
    Some((lag / sample_rate * 1e3) as f32)
}
