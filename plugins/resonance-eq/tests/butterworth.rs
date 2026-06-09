//! True-Butterworth checks for the cut-band cascades: every slope must sit
//! at -3 dB at the cutoff frequency and be monotonic (maximally flat — no
//! sag, no ripple) across the spectrum. The old uniform Q=0.707 cascades
//! sagged to -6 dB (24 dB/oct) and -12 dB (48 dB/oct) at cutoff.

use resonance_dsp::Biquad;
use resonance_eq::band::{configure_stages, BandKind, BandSlope, MAX_STAGES_PER_BAND};
use resonance_eq::params::BandSnapshot;

const SR: f32 = 48_000.0;
const CUTOFF: f32 = 1_000.0;

fn cut_snapshot(kind: BandKind, slope: BandSlope) -> BandSnapshot {
    BandSnapshot {
        enabled: true,
        freq: CUTOFF,
        gain_db: 0.0,
        q: 0.707,
        kind,
        slope,
    }
}

fn cascade_db(stages: &[Biquad; MAX_STAGES_PER_BAND], n: usize, freq: f32) -> f32 {
    let mut lin = 1.0f32;
    for stage in stages.iter().take(n) {
        lin *= stage.magnitude(freq, SR);
    }
    20.0 * lin.max(1e-12).log10()
}

fn configure(kind: BandKind, slope: BandSlope) -> ([Biquad; MAX_STAGES_PER_BAND], usize) {
    let mut stages = [Biquad::identity(); MAX_STAGES_PER_BAND];
    let n = configure_stages(&cut_snapshot(kind, slope), SR, &mut stages);
    (stages, n)
}

const ALL_SLOPES: [(BandSlope, usize); 3] = [
    (BandSlope::Db12, 1),
    (BandSlope::Db24, 2),
    (BandSlope::Db48, 4),
];

/// Every cut slope crosses exactly -3 dB at the cutoff frequency.
#[test]
fn cut_cascades_hit_minus_3_db_at_cutoff() {
    for kind in [BandKind::LowCut, BandKind::HighCut] {
        for (slope, expected_stages) in ALL_SLOPES {
            let (stages, n) = configure(kind, slope);
            assert_eq!(n, expected_stages, "{kind:?} {slope:?} stage count");
            let db = cascade_db(&stages, n, CUTOFF);
            assert!(
                (db - (-3.01)).abs() < 0.1,
                "{kind:?} {slope:?}: {db:.3} dB at cutoff, expected -3 dB"
            );
        }
    }
}

/// Butterworth is maximally flat: the magnitude must be monotonic across
/// the whole sweep (decreasing towards the stopband, no passband ripple
/// and no resonant bump near cutoff).
#[test]
fn cut_cascades_are_monotonic() {
    let points = 400;
    for kind in [BandKind::LowCut, BandKind::HighCut] {
        for (slope, _) in ALL_SLOPES {
            let (stages, n) = configure(kind, slope);
            let mut prev = f32::NAN;
            for i in 0..points {
                let t = i as f32 / (points - 1) as f32;
                let freq = 20.0 * (20_000.0f32 / 20.0).powf(t);
                let db = cascade_db(&stages, n, freq);
                if !prev.is_nan() {
                    // LowCut (high-pass) rises with frequency; HighCut falls.
                    let delta = if kind == BandKind::LowCut {
                        db - prev
                    } else {
                        prev - db
                    };
                    assert!(
                        delta > -0.01,
                        "{kind:?} {slope:?}: non-monotonic at {freq:.1} Hz ({prev:.3} -> {db:.3})"
                    );
                }
                prev = db;
            }
        }
    }
}

/// Deep in the passband (3+ octaves from cutoff) the cascade is flat at
/// 0 dB, and an octave into the stopband each slope attenuates by roughly
/// its nominal dB/oct figure.
#[test]
fn cut_cascades_passband_flat_and_slopes_nominal() {
    for (slope, _) in ALL_SLOPES {
        let (stages, n) = configure(BandKind::HighCut, slope);
        let passband = cascade_db(&stages, n, CUTOFF / 16.0);
        assert!(
            passband.abs() < 0.05,
            "{slope:?}: passband {passband:.3} dB, expected ~0"
        );

        let nominal = match slope {
            BandSlope::Db12 => 12.0,
            BandSlope::Db24 => 24.0,
            BandSlope::Db48 => 48.0,
        };
        // Two octaves above cutoff the asymptotic slope dominates; the
        // attenuation gained between +1 and +2 octaves approaches nominal.
        let oct1 = cascade_db(&stages, n, CUTOFF * 2.0);
        let oct2 = cascade_db(&stages, n, CUTOFF * 4.0);
        let per_octave = oct1 - oct2;
        assert!(
            (per_octave - nominal).abs() < nominal * 0.15,
            "{slope:?}: {per_octave:.2} dB/oct between +1 and +2 octaves, expected ~{nominal}"
        );
    }
}
