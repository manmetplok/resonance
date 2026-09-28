//! Offline mix-analysis primitives.
//!
//! Pure functions over an already-rendered stereo buffer: slice in, value
//! out, no state kept between calls. They are meant to run off the audio
//! thread (the engine's offline measure path, export normalisation, the
//! control API's `meter.measure`), so they allocate scratch buffers freely —
//! never call them from a realtime callback.
//!
//! Everything else the mix report needs already exists as a streaming meter
//! elsewhere in this crate and is *not* duplicated here:
//! [`LufsMeter`][crate::LufsMeter] (integrated / short-term / momentary LUFS,
//! plus [`LufsMeter::analyze_offline`][crate::LufsMeter::analyze_offline]),
//! [`LraMeter`][crate::LraMeter], [`TruePeakMeter`][crate::TruePeakMeter],
//! [`CrestMeter`][crate::CrestMeter], [`CorrelationMeter`][crate::CorrelationMeter]
//! and [`PlrMeter`][crate::PlrMeter].
//!
//! ## Channel convention
//!
//! Every function here takes `left` and `right` separately and analyses
//! `min(left.len(), right.len())` samples, matching `push_stereo` across the
//! rest of the crate. Mono material should be passed as the same buffer
//! twice.

use crate::lufs::LufsMeter;
use crate::spectrum::offline::analyze_mono;

/// dBFS reported by [`sample_peak_db`] for a silent buffer, instead of
/// `-inf`. Matches [`FLOOR_DBTP`][crate::true_peak::FLOOR_DBTP] so the sample
/// peak and the true peak share a floor and stay comparable.
pub const FLOOR_DBFS: f32 = -120.0;

/// Most negative value [`mono_penalty_db`] reports, for the case where the
/// mono sum cancels to (near) silence. Without a floor a perfectly
/// anti-phase signal would measure `-inf`, which no wire format can carry.
pub const MONO_PENALTY_FLOOR_DB: f32 = -60.0;

/// Upper edge of the `low` band and lower edge of `mid`, in Hz.
pub const BAND_LOW_MID_HZ: f32 = 250.0;
/// Upper edge of the `mid` band and lower edge of `high`, in Hz.
pub const BAND_MID_HIGH_HZ: f32 = 2_000.0;
/// Upper edge of the `high` band and lower edge of `air`, in Hz.
pub const BAND_HIGH_AIR_HZ: f32 = 8_000.0;
/// Lower edge of the `low` band, in Hz. Everything below is ignored.
pub const BAND_BOTTOM_HZ: f32 = 20.0;
/// Upper edge of the `air` band, in Hz. Everything above is ignored.
pub const BAND_TOP_HZ: f32 = 20_000.0;

/// Fraction of a mix's energy in each of the four AES tonal-balance bands.
///
/// The four fields sum to `1.0` (they are normalised against each other, so
/// energy below [`BAND_BOTTOM_HZ`] or above [`BAND_TOP_HZ`] is excluded from
/// both the numerator and the denominator) — except for silence, where all
/// four are `0.0` rather than `NaN`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandShares {
    /// 20 Hz – 250 Hz.
    pub low: f32,
    /// 250 Hz – 2 kHz.
    pub mid: f32,
    /// 2 kHz – 8 kHz.
    pub high: f32,
    /// 8 kHz – 20 kHz.
    pub air: f32,
}

impl BandShares {
    /// All-zero shares, as reported for silence.
    pub const SILENT: Self = Self {
        low: 0.0,
        mid: 0.0,
        high: 0.0,
        air: 0.0,
    };
}

/// Fractions of total energy in the four AES bands (20–250 / 250–2000 /
/// 2000–8000 / 8000–20000 Hz) of the mono sum `(L + R) * 0.5`.
///
/// Unweighted (no K-weighting, no equal-loudness curve) linear energy, from
/// a Welch average of Hann-windowed FFTs over the whole buffer — see
/// [`spectrum::offline`][crate::spectrum::offline]. Returns
/// [`BandShares::SILENT`] for an empty or silent buffer.
///
/// Note that this measures the *mono sum*, so anti-phase side content is
/// invisible here by construction; [`mono_penalty_db`] is what reports it.
pub fn band_shares(sample_rate: f32, left: &[f32], right: &[f32]) -> BandShares {
    let n = left.len().min(right.len());
    if n == 0 {
        return BandShares::SILENT;
    }
    let mut mono = vec![0.0_f32; n];
    for (i, slot) in mono.iter_mut().enumerate() {
        *slot = 0.5 * (left[i] + right[i]);
    }

    let spectrum = analyze_mono(sample_rate, &mono);
    let low = spectrum.band_power(BAND_BOTTOM_HZ, BAND_LOW_MID_HZ);
    let mid = spectrum.band_power(BAND_LOW_MID_HZ, BAND_MID_HIGH_HZ);
    let high = spectrum.band_power(BAND_MID_HIGH_HZ, BAND_HIGH_AIR_HZ);
    let air = spectrum.band_power(BAND_HIGH_AIR_HZ, BAND_TOP_HZ);

    let total = low + mid + high + air;
    if !(total.is_finite() && total > 0.0) {
        return BandShares::SILENT;
    }
    BandShares {
        low: (low / total) as f32,
        mid: (mid / total) as f32,
        high: (high / total) as f32,
        air: (air / total) as f32,
    }
}

/// Loudness lost when the mix is folded to mono, in dB.
///
/// Integrated LUFS of the mono sum `(L + R) * 0.5` (fed to both channels of
/// a fresh meter) minus integrated LUFS of the stereo signal. **Negative
/// means level is lost when summed to mono**; `0.0` exactly for a signal
/// whose channels are identical, and progressively more negative the more
/// out-of-phase the sides are.
///
/// Edge cases:
/// - Clamped at [`MONO_PENALTY_FLOOR_DB`], which is what a fully anti-phase
///   signal (mono sum = digital silence) reports.
/// - `0.0` when the *stereo* signal itself is silent, shorter than one
///   400 ms gating block, or below the BS.1770 absolute gate — there is no
///   loudness to lose, so "no penalty" is the honest answer.
pub fn mono_penalty_db(sample_rate: f32, left: &[f32], right: &[f32]) -> f32 {
    let n = left.len().min(right.len());
    if n == 0 {
        return 0.0;
    }
    let stereo = LufsMeter::analyze_offline(sample_rate, &left[..n], &right[..n]).integrated;
    if !stereo.is_finite() {
        return 0.0;
    }

    let mut mono = vec![0.0_f32; n];
    for (i, slot) in mono.iter_mut().enumerate() {
        *slot = 0.5 * (left[i] + right[i]);
    }
    let summed = LufsMeter::analyze_offline(sample_rate, &mono, &mono).integrated;
    if !summed.is_finite() {
        return MONO_PENALTY_FLOOR_DB;
    }
    (summed - stereo).max(MONO_PENALTY_FLOOR_DB)
}

/// Largest absolute sample value across both channels, linear.
///
/// This is the plain per-sample peak — the inter-sample peaks a D/A
/// converter would reconstruct are [`TruePeakMeter`][crate::TruePeakMeter]'s
/// job and read higher. NaN samples are ignored.
pub fn sample_peak_linear(left: &[f32], right: &[f32]) -> f32 {
    let n = left.len().min(right.len());
    let mut peak = 0.0_f32;
    for i in 0..n {
        // `f32::max` returns the non-NaN operand, so NaN never wins.
        peak = peak.max(left[i].abs()).max(right[i].abs());
    }
    peak
}

/// Sample peak across both channels in dBFS.
///
/// `20*log10` of [`sample_peak_linear`], floored at [`FLOOR_DBFS`] for
/// silence rather than returning `-inf`. Full scale reads `0.0`; anything
/// above full scale reads positive.
pub fn sample_peak_db(left: &[f32], right: &[f32]) -> f32 {
    let peak = sample_peak_linear(left, right);
    if peak > 0.0 {
        (20.0 * peak.log10()).max(FLOOR_DBFS)
    } else {
        FLOOR_DBFS
    }
}

/// Number of samples at or beyond digital full scale (`|x| >= 1.0`).
///
/// Counted **per channel sample**, not per stereo frame: a frame where both
/// channels are clipped counts as two. NaN samples are not counted (no
/// comparison against NaN is true), so a broken buffer reads as zero clips
/// rather than as everything clipped.
pub fn clipped_samples(left: &[f32], right: &[f32]) -> u64 {
    let n = left.len().min(right.len());
    let mut count = 0_u64;
    for i in 0..n {
        if left[i].abs() >= 1.0 {
            count += 1;
        }
        if right[i].abs() >= 1.0 {
            count += 1;
        }
    }
    count
}

/// Peak-to-RMS ratio over the WHOLE buffer, dB — the crest factor of the
/// measured range, not of a sliding window. `0.0` for silence.
///
/// Peak is `max(|L|, |R|)`, RMS is over both channels, matching
/// [`CrestMeter`][crate::CrestMeter]'s definition but with the range as the
/// window: that meter is a 100 ms sliding readout built for a live display,
/// whose terminal value describes only the last 100 ms of what was pushed.
pub fn range_crest_db(left: &[f32], right: &[f32]) -> f32 {
    let n = left.len().min(right.len());
    if n == 0 {
        return 0.0;
    }
    let peak = sample_peak_linear(left, right);
    let mut sum_sq = 0.0f64;
    for i in 0..n {
        let s = left[i].abs().max(right[i].abs()) as f64;
        sum_sq += s * s;
    }
    let rms = (sum_sq / n as f64).sqrt();
    if peak <= 0.0 || rms <= 1e-20 {
        return 0.0;
    }
    20.0 * (peak as f64 / rms).log10() as f32
}

/// Pearson correlation of L against R over the WHOLE buffer, clamped to
/// `[-1, 1]`. `0.0` for a silent or single-sided buffer — the same neutral
/// value [`CorrelationMeter`][crate::CorrelationMeter] reports when it has
/// nothing to say (and, like [`range_crest_db`], the range's own figure
/// rather than that sliding meter's last window).
pub fn range_correlation(left: &[f32], right: &[f32]) -> f32 {
    let n = left.len().min(right.len());
    let (mut ll, mut rr, mut lr) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..n {
        let l = left[i] as f64;
        let r = right[i] as f64;
        ll += l * l;
        rr += r * r;
        lr += l * r;
    }
    let denom_sq = ll * rr;
    if denom_sq <= 1e-20 {
        return 0.0;
    }
    (lr / denom_sq.sqrt()).clamp(-1.0, 1.0) as f32
}
