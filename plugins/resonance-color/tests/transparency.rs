//! The exact paths (`dsp/mod.rs`, "Exact paths"):
//!
//! - `mix = 0` is the dry path: the input itself, bit for bit, with
//!   oversampling Off; at 2× / 4× the dry path is the IIR oversampler's
//!   round trip (which the wet path shares, so the blend cannot comb),
//!   and the output equals that round trip bit for bit. The round trip
//!   itself is magnitude-flat across the band; its only cost is a few
//!   samples of frequency-dependent group delay (W5's oversampler docs).
//! - Console at `drive = 0` is transparent the same way, with auto-gain
//!   on (it computes a gain of exactly 1).
//! - `flutter = 0` is a true bypass in Tape mode, and switching it on
//!   crossfades rather than stepping in the ≈ 1.1 ms centre delay.
//!
//! "Bit for bit" is compared with `==` on the samples, i.e. up to the
//! sign of a zero (the blend can turn a −0.0 into +0.0).

mod common;

use common::*;
use resonance_color::dsp::Settings;
use resonance_color::params::Mode;
use resonance_dsp::OversampleFactor;

fn program() -> (Vec<f32>, Vec<f32>) {
    // Pink noise at a hot level, plus a loud low sine: enough to drive
    // every curve well into its nonlinear region if the wet path leaked.
    let n = 24_000;
    let (mut l, mut r) = pink_noise(n, -12.0, 5);
    let (sl, sr) = sine(n, 70.0, 0.4);
    for i in 0..n {
        l[i] += sl[i];
        r[i] += sr[i];
    }
    (l, r)
}

fn assert_equal(got: &[f32], want: &[f32], what: &str) {
    assert_eq!(got.len(), want.len());
    let bad = got.iter().zip(want).position(|(a, b)| a != b);
    if let Some(i) = bad {
        panic!("{what}: sample {i} differs ({} vs {})", got[i], want[i]);
    }
}

#[test]
fn mix_zero_is_the_input_bit_for_bit_with_oversampling_off() {
    let (l, r) = program();
    for mode in Mode::ALL {
        let s = Settings {
            mode,
            drive: 1.0,
            bias: 1.0,
            response_db: 12.0,
            tone_db: 6.0,
            mix: 0.0,
            oversample: OversampleFactor::Off,
            ..Settings::default()
        };
        let (ol, or) = render(&s, &l, &r);
        assert_equal(&ol, &l, &format!("{} left", mode.label()));
        assert_equal(&or, &r, &format!("{} right", mode.label()));
    }
}

#[test]
fn mix_zero_is_the_oversampler_round_trip_at_2x_and_4x() {
    let (l, r) = program();
    for factor in [OversampleFactor::X2, OversampleFactor::X4] {
        for mode in Mode::ALL {
            let s = Settings {
                mode,
                drive: 1.0,
                bias: 1.0,
                mix: 0.0,
                oversample: factor,
                ..Settings::default()
            };
            let (ol, or) = render(&s, &l, &r);
            assert_equal(&ol, &round_trip(&l, factor), &format!("{} {factor:?} left", mode.label()));
            assert_equal(&or, &round_trip(&r, factor), &format!("{} {factor:?} right", mode.label()));
        }
    }
}

/// The documented tolerance of that round trip: flat magnitude in the
/// band (a 10 kHz sine keeps its level within 0.01 dB). What it adds is
/// phase, not colour.
#[test]
fn the_round_trip_is_magnitude_flat_in_band() {
    for factor in [OversampleFactor::X2, OversampleFactor::X4] {
        for freq in [100.0f32, 1_000.0, 10_000.0, 18_000.0] {
            let (l, _) = sine(48_000, freq, 0.5);
            let rt = round_trip(&l, factor);
            let rms = |x: &[f32]| {
                (x[4_800..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>()
                    / (x.len() - 4_800) as f64)
                    .sqrt()
            };
            let db = 20.0 * (rms(&rt) / rms(&l)).log10();
            assert!(db.abs() < 0.01, "{factor:?} round trip at {freq} Hz: {db:+.4} dB");
        }
    }
}

#[test]
fn console_at_drive_zero_is_transparent_with_auto_gain_on() {
    let (l, r) = program();
    for factor in factors() {
        let s = Settings {
            mode: Mode::Console,
            drive: 0.0,
            auto_gain: true,
            mix: 1.0,
            oversample: factor,
            ..Settings::default()
        };
        let (ol, or) = render(&s, &l, &r);
        let (want_l, want_r) = match factor {
            OversampleFactor::Off => (l.clone(), r.clone()),
            f => (round_trip(&l, f), round_trip(&r, f)),
        };
        assert_equal(&ol, &want_l, &format!("Console drive 0 {factor:?} left"));
        assert_equal(&or, &want_r, &format!("Console drive 0 {factor:?} right"));
    }
}

/// Console at drive 0 is transparent at any mix too (the wet path *is*
/// the dry path), up to the rounding of the blend itself.
#[test]
fn console_at_drive_zero_is_transparent_at_partial_mix() {
    let (l, r) = program();
    let s = Settings {
        mode: Mode::Console,
        drive: 0.0,
        mix: 0.37,
        oversample: OversampleFactor::Off,
        ..Settings::default()
    };
    let (ol, _) = render(&s, &l, &r);
    let worst = ol.iter().zip(&l).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(worst < 1e-6, "Console drive 0 at 37 % mix deviates by {worst}");
}

/// `flutter = 0` is a true bypass: in Tape mode at `mix = 0` the output
/// is still the input bit for bit, so no delay and no interpolation is
/// in the path. Above 0 the flutter moves the whole output (dry
/// included, like a transport), centred on its ≈ 1.1 ms delay.
#[test]
fn flutter_zero_in_tape_mode_is_a_true_bypass() {
    let n = 24_000;
    let (l, r) = white_noise(n, 0.3, 9);
    let base = Settings {
        mode: Mode::Tape,
        mix: 0.0,
        oversample: OversampleFactor::Off,
        flutter: 0.0,
        ..Settings::default()
    };
    let (ol, _) = render(&base, &l, &r);
    assert_equal(&ol, &l, "Tape, flutter 0, mix 0");

    let (fl, _) = render(&Settings { flutter: 0.5, ..base }, &l, &r);
    let corr = |lag: usize| -> f64 {
        fl[lag..]
            .iter()
            .zip(&l)
            .map(|(a, b)| *a as f64 * *b as f64)
            .sum::<f64>()
    };
    let best = (0..120).max_by(|&a, &b| corr(a).total_cmp(&corr(b))).unwrap();
    // Centre delay ≈ 1.06 ms + 2 samples ≈ 53 samples at 48 kHz, and at
    // amount 0.5 the wow swings it by up to ±0.5 ms (±24 samples).
    assert!((53 - 26..=53 + 26).contains(&best), "flutter's delay peaks at lag {best}");
}

/// Switching flutter on inserts its centre delay; the crossfade must keep
/// that from being a step. The largest sample-to-sample jump of a smooth
/// low sine through the switch stays of the order of the sine's own
/// slope.
#[test]
fn switching_flutter_on_and_off_does_not_click() {
    use resonance_plugin::ResonancePlugin;
    let mut plugin = resonance_color::ResonanceColor::new();
    let s = Settings {
        mode: Mode::Tape,
        drive: 0.0,
        flutter: 0.0,
        mix: 1.0,
        ..Settings::default()
    };
    apply_settings(&plugin.params, &s);
    plugin.initialize(SR, BLOCK as u32);
    let (l, r) = sine(48_000, 220.0, 0.5);
    let third = 16_000;
    let (mut a, _) = render_with(&mut plugin, &l[..third], &r[..third]);
    plugin.params.flutter.set_value(1.0);
    let (b, _) = render_with(&mut plugin, &l[third..2 * third], &r[third..2 * third]);
    plugin.params.flutter.set_value(0.0);
    let (c, _) = render_with(&mut plugin, &l[2 * third..], &r[2 * third..]);
    a.extend(b);
    a.extend(c);
    // The sine's own largest step: 2π·220/48000 · 0.5 ≈ 0.0144.
    let own = std::f32::consts::TAU * 220.0 / SR * 0.5;
    let worst = a.windows(2).skip(4_800).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(worst < 1.5 * own, "flutter switch stepped by {worst} (sine slope {own})");
}
