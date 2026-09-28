//! `meter.compare` deltas (warmth-width-depth.md §7.2): `B − A` for every
//! proxy, with B taken at the loudness-match gain.
//!
//! Matching is done on the stored measurements, not by re-rendering B at
//! a new gain. That is exact, because a linear gain `g` changes each
//! figure in one of two known ways:
//!
//! * **level figures** move by exactly `g` dB: the three LUFS figures,
//!   true peak, sample peak and every 1/3-octave band level;
//! * **shape figures** do not move at all: crest, LRA, correlation, mono
//!   penalty, the `bands` shares, tilt, centroid, every ratio and the
//!   peakiness, PLR / PSR (peak and loudness move together), and the
//!   whole stereo block.
//!
//! The one figure neither rule covers is the clip count — a gain changes
//! which samples reach full scale, and a measurement cannot say which —
//! so its delta is reported as measured, and documented as such.
//!
//! Floors are respected: a level sitting on its silence floor (-120) is
//! not a level, so shifting it would invent one; such a delta is `None`.

use resonance_audio::types::MixMeasurement;
use resonance_control::methods::meter::{
    BandsDelta, CompareDeltas, DynamicsDelta, SpectrumDelta, StereoBandDelta, StereoDelta,
};

/// Silence floor of true peak, sample peak and band levels, dB.
const LEVEL_FLOOR_DB: f32 = -120.0;

/// The match gain for B, dB: `A − B` integrated loudness, or `None` when
/// either side has none (silence), in which case nothing is matched.
pub(crate) fn match_gain_db(a: &MixMeasurement, b: &MixMeasurement) -> Option<f64> {
    (a.lufs_integrated.is_finite() && b.lufs_integrated.is_finite())
        .then(|| f64::from(a.lufs_integrated) - f64::from(b.lufs_integrated))
}

/// Round a delta for the wire, as the detail blocks are rounded.
fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// A shape delta: both sides finite, no gain.
fn shape(a: f32, b: f32) -> Option<f64> {
    (a.is_finite() && b.is_finite()).then(|| round(f64::from(b) - f64::from(a)))
}

fn shape_opt(a: Option<f32>, b: Option<f32>) -> Option<f64> {
    shape(a?, b?)
}

/// A level delta: both sides finite and off the floor, B moved by `gain`.
fn level(a: f32, b: f32, gain: f64) -> Option<f64> {
    let real = |v: f32| v.is_finite() && v > LEVEL_FLOOR_DB;
    (real(a) && real(b)).then(|| round(f64::from(b) + gain - f64::from(a)))
}

/// `B − A`, B at `gain` dB.
pub(crate) fn deltas(a: &MixMeasurement, b: &MixMeasurement, gain: f64) -> CompareDeltas {
    // `bands` / crest / correlation / mono penalty are only real on the
    // render path, which is the only path snapshot and compare use.
    CompareDeltas {
        lufs_integrated: level(a.lufs_integrated, b.lufs_integrated, gain),
        lufs_short_max: level(a.lufs_short_term_max, b.lufs_short_term_max, gain),
        lufs_momentary_max: level(a.lufs_momentary_max, b.lufs_momentary_max, gain),
        lra: shape(a.lra_lu, b.lra_lu),
        true_peak_db: level(a.true_peak_dbtp, b.true_peak_dbtp, gain),
        sample_peak_db: level(a.sample_peak_db, b.sample_peak_db, gain),
        crest_db: shape(a.crest_db, b.crest_db),
        clipped_samples: Some(b.clipped_samples as i64 - a.clipped_samples as i64),
        correlation: shape(a.correlation, b.correlation),
        mono_penalty_db: shape(a.mono_penalty_db, b.mono_penalty_db),
        bands: Some(BandsDelta {
            low: round4(b.bands.low - a.bands.low),
            mid: round4(b.bands.mid - a.bands.mid),
            high: round4(b.bands.high - a.bands.high),
            air: round4(b.bands.air - a.bands.air),
        }),
        spectrum: spectrum(a, b, gain),
        stereo: stereo(a, b),
        dynamics: match (a.detail.dynamics, b.detail.dynamics) {
            (Some(x), Some(y)) => Some(DynamicsDelta {
                plr_db: shape_opt(x.plr_db, y.plr_db),
                psr_db: shape_opt(x.psr_db, y.psr_db),
            }),
            _ => None,
        },
    }
}

/// Energy shares carry more significant digits than dB figures.
fn round4(value: f32) -> f64 {
    (f64::from(value) * 10_000.0).round() / 10_000.0
}

fn spectrum(a: &MixMeasurement, b: &MixMeasurement, gain: f64) -> Option<SpectrumDelta> {
    let (x, y) = (a.detail.spectrum.as_ref()?, b.detail.spectrum.as_ref()?);
    let centroid_pct = match (x.centroid_hz, y.centroid_hz) {
        (Some(ca), Some(cb)) if ca > 0.0 => {
            Some(round(100.0 * (f64::from(cb) - f64::from(ca)) / f64::from(ca)))
        }
        _ => None,
    };
    Some(SpectrumDelta {
        third_octave: x
            .third_octave
            .iter()
            .zip(&y.third_octave)
            .map(|(&la, &lb)| level(la, lb, gain))
            .collect(),
        tilt_db_per_oct: shape_opt(x.tilt_db_per_oct, y.tilt_db_per_oct),
        centroid_hz: shape_opt(x.centroid_hz, y.centroid_hz),
        centroid_pct,
        lowmid_presence_db: shape_opt(x.lowmid_presence_db, y.lowmid_presence_db),
        presence_peakiness_db: shape_opt(x.presence_peakiness_db, y.presence_peakiness_db),
        air_ratio_db: shape_opt(x.air_ratio_db, y.air_ratio_db),
    })
}

fn stereo(a: &MixMeasurement, b: &MixMeasurement) -> Option<StereoDelta> {
    let (x, y) = (a.detail.stereo.as_ref()?, b.detail.stereo.as_ref()?);
    let (wa, wb) = (x.correlation_windows, y.correlation_windows);
    Some(StereoDelta {
        bands: x
            .bands
            .iter()
            .zip(&y.bands)
            .map(|(ba, bb)| StereoBandDelta {
                lo_hz: f64::from(ba.lo_hz),
                hi_hz: f64::from(ba.hi_hz),
                correlation: shape_opt(ba.correlation, bb.correlation),
                side_mid_db: shape_opt(ba.side_mid_db, bb.side_mid_db),
                mono_loss_db: shape_opt(ba.mono_loss_db, bb.mono_loss_db),
            })
            .collect(),
        balance_db: shape_opt(x.balance_db, y.balance_db),
        pct_below_0_3: shape_opt(wa.map(|w| w.pct_below_0_3), wb.map(|w| w.pct_below_0_3)),
        worst_window_correlation: shape_opt(wa.map(|w| w.worst), wb.map(|w| w.worst)),
    })
}
