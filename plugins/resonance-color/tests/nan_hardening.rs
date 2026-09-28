//! No NaN, no infinity and no runaway level at the extremes: maximum
//! drive on full-scale noise in every mode at every oversampling factor,
//! with every other control at the end of its range too; plus hostile
//! input (NaN, ±inf, huge values) that must not poison the state.

mod common;

use common::*;
use resonance_color::dsp::Settings;
use resonance_color::params::Mode;

fn extreme(mode: Mode, factor: resonance_dsp::OversampleFactor, sign: f32) -> Settings {
    Settings {
        mode,
        drive: 1.0,
        bias: 1.0,
        response_db: 12.0 * sign,
        tone_db: 6.0 * sign,
        mix: 1.0,
        auto_gain: true,
        output_db: 12.0,
        oversample: factor,
        speed_ips: if sign > 0.0 { 30.0 } else { 7.5 },
        flutter: 1.0,
    }
}

#[test]
fn max_drive_on_full_scale_noise_stays_finite_and_bounded() {
    let (l, r) = white_noise(48_000, 1.0, 3);
    for mode in Mode::ALL {
        for factor in factors() {
            for sign in [1.0, -1.0] {
                let s = extreme(mode, factor, sign);
                let (ol, or) = render(&s, &l, &r);
                let peak = ol
                    .iter()
                    .chain(&or)
                    .map(|v| {
                        assert!(v.is_finite(), "{} {factor:?}: non-finite output", mode.label());
                        v.abs()
                    })
                    .fold(0.0f32, f32::max);
                // +12 dB output trim and +24 dB of auto-gain headroom on a
                // full-scale input: the ceiling is generous, but finite.
                assert!(peak < 64.0, "{} {factor:?}: output peaked at {peak}", mode.label());
            }
        }
    }
}

#[test]
fn auto_gain_off_at_max_drive_stays_finite() {
    let (l, r) = white_noise(24_000, 1.0, 4);
    for mode in Mode::ALL {
        let s = Settings {
            auto_gain: false,
            ..extreme(mode, resonance_dsp::OversampleFactor::X4, 1.0)
        };
        let (ol, or) = render(&s, &l, &r);
        assert!(ol.iter().chain(&or).all(|v| v.is_finite()), "{}", mode.label());
    }
}

/// A burst of NaN / inf / 1e30 in the input must not leave the plugin
/// producing NaN once the input is clean again.
#[test]
fn hostile_input_does_not_poison_the_state() {
    let n = 48_000;
    let (mut l, mut r) = white_noise(n, 0.5, 6);
    let (clean_l, clean_r) = (l.clone(), r.clone());
    // Non-finite values only: the plugin reads them as silence, so the
    // tail must match a clean render. (A finite 1e30 is a real, if absurd,
    // level — clamped, then legitimately remembered by the auto-gain
    // followers for a few seconds; `huge_input_stays_finite` covers it.)
    for (i, v) in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::NAN]
        .iter()
        .enumerate()
    {
        l[1_000 + i] = *v;
        r[2_000 + i] = *v;
    }
    for mode in Mode::ALL {
        for factor in factors() {
            let s = Settings {
                mode,
                drive: 0.8,
                oversample: factor,
                ..Settings::default()
            };
            let (ol, or) = render(&s, &l, &r);
            assert!(
                ol.iter().chain(&or).all(|v| v.is_finite()),
                "{} {factor:?}: a hostile burst came out non-finite",
                mode.label()
            );
            // …and well after it the plugin sounds as it would have
            // without it: finite-but-silent (a NaN parked in a filter
            // state and sanitised away downstream) would pass the check
            // above and fail this one.
            let (cl, _) = render(&s, &clean_l, &clean_r);
            let tail = n - 4_800;
            let rms = |x: &[f32]| {
                (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
            };
            let db = 20.0 * (rms(&ol[tail..]) / rms(&cl[tail..])).log10();
            assert!(db.abs() < 0.5, "{} {factor:?}: tail {db:+.2} dB off the clean render", mode.label());
        }
    }
}

#[test]
fn huge_input_stays_finite() {
    let (mut l, mut r) = white_noise(24_000, 0.5, 8);
    for i in 0..8 {
        l[500 + i] = if i % 2 == 0 { 1.0e30 } else { -1.0e30 };
        r[900 + i] = f32::MAX;
    }
    for mode in Mode::ALL {
        for factor in factors() {
            let s = Settings {
                mode,
                drive: 1.0,
                oversample: factor,
                ..Settings::default()
            };
            let (ol, or) = render(&s, &l, &r);
            assert!(
                ol.iter().chain(&or).all(|v| v.is_finite()),
                "{} {factor:?}: huge input came out non-finite",
                mode.label()
            );
        }
    }
}
