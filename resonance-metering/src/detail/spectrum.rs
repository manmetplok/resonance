//! Spectral warmth proxies (warmth-width-depth.md §2.1, §7.1 `spectrum`).
//!
//! Everything here is read off the **stereo LTAS**: the mean of the left
//! and right power spectra, Welch-averaged over the whole buffer
//! ([`StereoSpectrum`], [`Channel::Both`]). That differs deliberately
//! from [`band_shares`][crate::offline::band_shares], which measures the
//! mono sum: anti-phase side content is part of how a mix sounds on
//! speakers and belongs in its tonal balance.
//!
//! ## Levels
//!
//! A band level is the band's power in dB, offset so that a full-scale
//! sine inside the band reads 0 dB (`10·log10(2·mean_square)`). Pink noise
//! therefore reads *flat* across [`third_octave`](SpectrumDetail::third_octave)
//! (equal power per band), which is the RTA convention, while its
//! [`tilt_db_per_oct`](SpectrumDetail::tilt_db_per_oct) — the slope of the
//! power *density* — reads −3.0 dB/oct. The two are tied exactly:
//! `tilt = slope(third_octave) − 10·log10(2)`, because a 1/3-octave band's
//! width doubles every octave.

use super::ratio_db;
use crate::spectrum::offline::{Channel, StereoSpectrum};

/// Number of ISO 1/3-octave bands, 20 Hz to 20 kHz.
pub const THIRD_OCTAVE_BANDS: usize = 31;

/// Nominal ISO 266 centre frequencies of the [`THIRD_OCTAVE_BANDS`], in
/// Hz. Band `i` is exactly centred at `1000 · 2^((i − 17) / 3)`; these are
/// the rounded names.
pub const THIRD_OCTAVE_NOMINAL_HZ: [f32; THIRD_OCTAVE_BANDS] = [
    20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0,
    500.0, 630.0, 800.0, 1_000.0, 1_250.0, 1_600.0, 2_000.0, 2_500.0, 3_150.0, 4_000.0, 5_000.0,
    6_300.0, 8_000.0, 10_000.0, 12_500.0, 16_000.0, 20_000.0,
];

/// Level reported for a band with no energy, dB.
pub const LEVEL_FLOOR_DB: f32 = -120.0;

/// Tilt regression range, Hz (band centres inside it are used).
pub const TILT_LO_HZ: f64 = 100.0;
/// Upper end of the tilt regression range, Hz.
pub const TILT_HI_HZ: f64 = 10_000.0;

/// How many resonances [`SpectrumDetail::peaks`] reports at most.
pub const MAX_PEAKS: usize = 5;
/// A 1/6-octave band must stand at least this far above the smoothed
/// LTAS to be reported as a resonance, dB — and also clear
/// [`PEAK_SIGMAS`] standard deviations of its own level estimate, which
/// is the stricter bar on a short range or a narrow low band.
pub const PEAK_MIN_EXCESS_DB: f32 = 1.0;
/// How many standard deviations of a band's Welch level estimate its
/// excess must clear to count as a resonance rather than estimator noise.
/// Four, not three: about sixty bands are tested per spectrum, and at 3σ
/// one of them clears the bar by chance every few measurements.
pub const PEAK_SIGMAS: f64 = 4.0;
/// Averaged bins per statistically independent estimate. Hann frames at
/// 50 % overlap are correlated in time and adjacent Hann bins in
/// frequency; dividing `bins × frames` by this is a conservative count of
/// independent averages.
const WELCH_CORRELATION: f64 = 2.5;
/// Half-width of the window that defines the reference LTAS for
/// [`SpectrumDetail::peaks`], in 1/6-octave bands (6 = one octave each
/// side).
pub const PEAK_SMOOTH_HALF_BANDS: usize = 6;
/// The widest a resonance may be, in 1/6-octave bands (2 = 1/3 octave):
/// counted as the contiguous bands around the candidate that stay within
/// half its excess of its level. A broad hump — a high-passed low end, a
/// gentle bell — is tonal balance, not a resonance, however far it stands
/// above its surroundings.
pub const PEAK_MAX_WIDTH_BANDS: usize = 2;

/// `10·log10(2)`: the dB/oct that separates band-power slope from
/// density slope for any constant-Q band set.
const DB_PER_OCT_BANDWIDTH: f64 = 3.010_299_956_639_812;

/// One narrow resonance: a 1/6-octave band standing above the local
/// trend of the spectrum around it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectralPeak {
    /// Frequency of the strongest FFT bin inside the band, Hz.
    pub freq_hz: f32,
    /// How far the band's level stands above the local trend — a straight
    /// line (in dB over log frequency) fitted to the octave either side of
    /// it, leaving out the band's immediate neighbours — dB.
    pub excess_db: f32,
}

/// The `spectrum` detail of one measurement.
#[derive(Debug, Clone, PartialEq)]
pub struct SpectrumDetail {
    /// Level of each ISO 1/3-octave band, dB (see the module docs for the
    /// reference). Always [`THIRD_OCTAVE_BANDS`] long, lowest band first;
    /// a band without energy reads [`LEVEL_FLOOR_DB`].
    pub third_octave: Vec<f32>,
    /// Slope of the power density over 100 Hz–10 kHz, dB/octave: a
    /// least-squares line through the 1/3-octave levels, minus the 3.01
    /// dB/oct their widening bandwidth contributes. Pink noise −3.0,
    /// white noise 0; more negative is darker / warmer.
    pub tilt_db_per_oct: Option<f32>,
    /// Power-weighted mean frequency over 20 Hz–20 kHz, Hz.
    pub centroid_hz: Option<f32>,
    /// `E(150–500 Hz) / E(2–5 kHz)`, dB. Rises when a mix gets warmer
    /// (or muddier); falls when it gets harsher.
    pub lowmid_presence_db: Option<f32>,
    /// Spectral crest inside 2–5 kHz at 1/6-octave resolution: the
    /// loudest band over the mean band power, dB. `0` is perfectly even
    /// (pink); a high value means a presence resonance.
    pub presence_peakiness_db: Option<f32>,
    /// `E(8–16 kHz) / E(20 Hz–20 kHz)`, dB. Always ≤ 0.
    pub air_ratio_db: Option<f32>,
    /// Up to [`MAX_PEAKS`] narrow resonances, strongest excess first.
    /// Empty when nothing stands clear of the local trend by both
    /// [`PEAK_MIN_EXCESS_DB`] and [`PEAK_SIGMAS`] of its estimate — so a
    /// short range, whose estimate is noisier, reports only bigger peaks —
    /// while staying at most [`PEAK_MAX_WIDTH_BANDS`] wide.
    pub peaks: Vec<SpectralPeak>,
}

/// Exact centre of 1/3-octave band `i` (0 = 20 Hz), Hz.
pub fn third_octave_center_hz(i: usize) -> f64 {
    1_000.0 * 2f64.powf((i as f64 - 17.0) / 3.0)
}

/// Compute the `spectrum` detail of a stereo buffer.
///
/// Convenience wrapper that runs the shared analysis itself; a caller
/// that also wants other details should run
/// [`analyze_detail`][super::analyze_detail] once and call
/// [`spectrum_detail_from`].
pub fn spectrum_detail(sample_rate: f32, left: &[f32], right: &[f32]) -> SpectrumDetail {
    spectrum_detail_from(&super::analyze_detail(sample_rate, left, right))
}

/// Compute the `spectrum` detail from an already-run analysis.
pub fn spectrum_detail_from(spec: &StereoSpectrum) -> SpectrumDetail {
    let band = |lo: f64, hi: f64| spec.band(Channel::Both, lo, hi);
    let nyquist = spec.sample_rate as f64 / 2.0;

    let mut third_octave = Vec::with_capacity(THIRD_OCTAVE_BANDS);
    let mut third_power = Vec::with_capacity(THIRD_OCTAVE_BANDS);
    for i in 0..THIRD_OCTAVE_BANDS {
        let fc = third_octave_center_hz(i);
        let p = band(fc * 2f64.powf(-1.0 / 6.0), fc * 2f64.powf(1.0 / 6.0));
        third_power.push(p);
        third_octave.push(level_db(p));
    }

    let total = band(20.0, 20_000.0);
    if !(total > 0.0 && total.is_finite()) {
        return SpectrumDetail {
            third_octave,
            tilt_db_per_oct: None,
            centroid_hz: None,
            lowmid_presence_db: None,
            presence_peakiness_db: None,
            air_ratio_db: None,
            peaks: Vec::new(),
        };
    }

    SpectrumDetail {
        tilt_db_per_oct: tilt(&third_octave, &third_power, nyquist),
        centroid_hz: centroid(spec),
        lowmid_presence_db: ratio_db(band(150.0, 500.0), band(2_000.0, 5_000.0)),
        presence_peakiness_db: presence_peakiness(spec),
        air_ratio_db: ratio_db(band(8_000.0, 16_000.0), total),
        peaks: peaks(spec),
        third_octave,
    }
}

/// A band power in dB re a full-scale sine, floored.
fn level_db(power: f64) -> f32 {
    if power > 0.0 && power.is_finite() {
        ((10.0 * (2.0 * power).log10()) as f32).max(LEVEL_FLOOR_DB)
    } else {
        LEVEL_FLOOR_DB
    }
}

/// Least-squares slope of the 1/3-octave levels against octaves, over the
/// bands centred in [`TILT_LO_HZ`, `TILT_HI_HZ`] that lie below Nyquist
/// and carry energy, converted to a density slope.
fn tilt(levels: &[f32], powers: &[f64], nyquist: f64) -> Option<f32> {
    let points: Vec<(f64, f64)> = (0..THIRD_OCTAVE_BANDS)
        .filter_map(|i| {
            let fc = third_octave_center_hz(i);
            // Nominal-edge tolerance: the exact 100 Hz band centre is
            // 99.2 Hz and the 10 kHz one 10.08 kHz.
            let inside = fc >= TILT_LO_HZ * 0.98 && fc <= TILT_HI_HZ * 1.02;
            let usable = fc * 2f64.powf(1.0 / 6.0) < nyquist && powers[i] > 0.0;
            (inside && usable).then(|| (fc.log2(), levels[i] as f64))
        })
        .collect();
    if points.len() < 3 {
        return None;
    }
    let n = points.len() as f64;
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let sxy: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let sxx: f64 = points.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum();
    (sxx > 0.0).then(|| (sxy / sxx - DB_PER_OCT_BANDWIDTH) as f32)
}

/// Power-weighted mean bin frequency over 20 Hz–20 kHz.
fn centroid(spec: &StereoSpectrum) -> Option<f32> {
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for k in 1..spec.ll.len() {
        let f = spec.center_hz(k);
        if !(20.0..20_000.0).contains(&f) {
            continue;
        }
        let p = spec.value(Channel::Both, k);
        num += f * p;
        den += p;
    }
    (den > 0.0).then(|| (num / den) as f32)
}

/// Exact centre of 1/6-octave band `n` on the grid anchored at 1 kHz.
fn sixth_center_hz(n: i32) -> f64 {
    1_000.0 * 2f64.powf(n as f64 / 6.0)
}

/// Power of the 1/6-octave band centred at `fc`.
fn sixth_band(spec: &StereoSpectrum, fc: f64) -> f64 {
    spec.band(Channel::Both, fc * 2f64.powf(-1.0 / 12.0), fc * 2f64.powf(1.0 / 12.0))
}

/// Loudest 1/6-octave band centred in 2–5 kHz over their mean, dB.
fn presence_peakiness(spec: &StereoSpectrum) -> Option<f32> {
    // n = 6 is 2 kHz, n = 13 is 4.49 kHz; n = 14 (5.04 kHz) is outside.
    let powers: Vec<f64> = (6..=13).map(|n| sixth_band(spec, sixth_center_hz(n))).collect();
    let max = powers.iter().copied().fold(0.0f64, f64::max);
    let mean = powers.iter().sum::<f64>() / powers.len() as f64;
    ratio_db(max, mean)
}

/// The strongest narrow resonances relative to the local trend.
///
/// Levels are taken per 1/6-octave band (grid anchored at 1 kHz,
/// 20 Hz–20 kHz, below Nyquist). A band is a candidate when it is a
/// strict local maximum. Its reference is the least-squares line through
/// the levels of the [`PEAK_SMOOTH_HALF_BANDS`] bands on each side, left
/// out: the band itself and its two neighbours (a resonance's own skirts)
/// and every band without energy. A line follows a tilt without bias,
/// and — unlike the plain mean it replaced — a window cut short at either
/// end of the grid. Its excess over that line must reach
/// [`PEAK_MIN_EXCESS_DB`] and [`PEAK_SIGMAS`] standard deviations of its
/// own estimate, and the peak must be narrow: at most
/// [`PEAK_MAX_WIDTH_BANDS`] contiguous bands within half the excess of its
/// level. A curved but broad shape (the hump a high-pass leaves at the
/// bottom of a tilted mix) can stand above any straight line; the width
/// test is what tells it from a resonance. The frequency reported is the
/// strongest bin inside the band, so a pure tone reads at its own
/// frequency.
fn peaks(spec: &StereoSpectrum) -> Vec<SpectralPeak> {
    let nyquist = spec.sample_rate as f64 / 2.0;
    let centres: Vec<f64> = (-34..=26)
        .map(sixth_center_hz)
        .filter(|&fc| fc >= 19.0 && fc <= 20_500.0 && fc * 2f64.powf(1.0 / 12.0) < nyquist)
        .collect();
    let levels: Vec<f32> = centres.iter().map(|&fc| level_db(sixth_band(spec, fc))).collect();

    let mut found: Vec<SpectralPeak> = Vec::new();
    for i in 1..levels.len().saturating_sub(1) {
        let here = levels[i];
        if here <= LEVEL_FLOOR_DB || here <= levels[i - 1] || here <= levels[i + 1] {
            continue;
        }
        let Some(trend) = local_trend(&levels, i) else {
            continue;
        };
        let excess = here - trend;
        let noise_floor = PEAK_SIGMAS * level_sigma_db(spec, centres[i]);
        if excess < PEAK_MIN_EXCESS_DB || (excess as f64) < noise_floor {
            continue;
        }
        if peak_width_bands(&levels, i, here - excess / 2.0) > PEAK_MAX_WIDTH_BANDS {
            continue;
        }
        found.push(SpectralPeak {
            freq_hz: strongest_bin_hz(spec, centres[i]) as f32,
            excess_db: excess,
        });
    }
    found.sort_by(|a, b| b.excess_db.total_cmp(&a.excess_db));
    found.truncate(MAX_PEAKS);
    found
}

/// The least-squares line through the levels around band `i`, evaluated
/// at `i`: bands within [`PEAK_SMOOTH_HALF_BANDS`] of it, except `i − 1`
/// to `i + 1` and floor bands. `None` with fewer than three such bands.
fn local_trend(levels: &[f32], i: usize) -> Option<f32> {
    let lo = i.saturating_sub(PEAK_SMOOTH_HALF_BANDS);
    let hi = (i + PEAK_SMOOTH_HALF_BANDS).min(levels.len() - 1);
    let points: Vec<(f64, f64)> = (lo..=hi)
        .filter(|&j| j.abs_diff(i) > 1 && levels[j] > LEVEL_FLOOR_DB)
        .map(|j| (j as f64 - i as f64, f64::from(levels[j])))
        .collect();
    if points.len() < 3 {
        return None;
    }
    let n = points.len() as f64;
    let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let sxy: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let sxx: f64 = points.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum();
    let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    // The line at offset 0, i.e. at band `i`.
    Some((my - slope * mx) as f32)
}

/// How many contiguous bands around `i` (itself included) stay at or
/// above `threshold` dB.
fn peak_width_bands(levels: &[f32], i: usize, threshold: f32) -> usize {
    let below = levels[..i].iter().rev().take_while(|&&l| l >= threshold).count();
    let above = levels[i + 1..].iter().take_while(|&&l| l >= threshold).count();
    1 + below + above
}

/// Standard deviation of a 1/6-octave band's Welch level estimate, dB.
///
/// A band level averages `bins × frames` periodogram values, each with a
/// relative spread of about 1; with [`WELCH_CORRELATION`] of them per
/// independent estimate the relative spread of the mean is `1/√K`, i.e.
/// `4.34/√K` dB.
fn level_sigma_db(spec: &StereoSpectrum, fc: f64) -> f64 {
    let width = fc * (2f64.powf(1.0 / 12.0) - 2f64.powf(-1.0 / 12.0));
    let averages = (width / spec.bin_hz()).max(1.0) * spec.frames.max(1) as f64;
    let independent = (averages / WELCH_CORRELATION).max(1.0);
    10.0 / std::f64::consts::LN_10 / independent.sqrt()
}

/// Centre frequency of the strongest bin inside the 1/6-octave band at `fc`.
fn strongest_bin_hz(spec: &StereoSpectrum, fc: f64) -> f64 {
    let bin = spec.bin_hz();
    let lo = ((fc * 2f64.powf(-1.0 / 12.0) / bin).round().max(1.0)) as usize;
    let hi = (((fc * 2f64.powf(1.0 / 12.0) / bin).round()) as usize).min(spec.ll.len() - 1);
    (lo..=hi.max(lo))
        .max_by(|&a, &b| {
            spec.value(Channel::Both, a)
                .total_cmp(&spec.value(Channel::Both, b))
        })
        .map_or(fc, |k| spec.center_hz(k))
}
