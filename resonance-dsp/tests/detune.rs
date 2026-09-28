//! The Doppler detune shifter: 0 cents is an exact delay, a shift moves
//! a tone's frequency by the asked-for ratio, and the crossfade keeps
//! the level.

mod common;

use common::*;
use resonance_dsp::DopplerShifter;

const FS: f32 = 48_000.0;

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&v| v as f64 * v as f64).sum::<f64>() / x.len() as f64).sqrt()
}

#[test]
fn zero_cents_is_an_exact_delay_of_the_mean() {
    // base 10 ms = 480, window 20 ms = 960: the mean delay 960 is whole.
    let mut d = DopplerShifter::new(FS, 30.0, 20.0);
    d.set_base_delay(FS, 10.0);
    d.set_cents(0.0);
    assert_eq!(d.mean_delay_samples(), 960.0);
    let x = noise(8_000, 0.8, 3);
    let y: Vec<f32> = x.iter().map(|&v| d.process(v)).collect();
    for i in 0..x.len() {
        let want = if i >= 960 { x[i - 960] } else { 0.0 };
        assert_eq!(y[i], want, "sample {i}");
    }
}

/// The exact-delay promise also holds where the asked-for times are not
/// whole samples: at 44.1 kHz a 5 ms base is 220.5 samples and a 10 ms
/// window 441 (odd, so its half is fractional too). The base rounds to a
/// whole sample and the window to an even one, so the 0-cent tap is
/// still a whole-sample delay and reads no interpolation.
#[test]
fn zero_cents_is_exact_for_fractional_base_and_odd_window() {
    let fs = 44_100.0;
    let mut d = DopplerShifter::new(fs, 30.0, 10.0);
    d.set_base_delay(fs, 5.0);
    d.set_cents(0.0);
    let window = d.window_samples();
    assert_eq!(window % 2.0, 0.0, "window {window} is not even");
    let mean = d.mean_delay_samples();
    assert_eq!(mean.fract(), 0.0, "mean delay {mean} is not whole");
    let lag = mean as usize;
    let x = noise(8_000, 0.8, 4);
    let y: Vec<f32> = x.iter().map(|&v| d.process(v)).collect();
    for i in 0..x.len() {
        let want = if i >= lag { x[i - lag] } else { 0.0 };
        assert_eq!(y[i], want, "sample {i}");
    }
}

/// Frequency of the strongest bin of `x`, in Hz.
fn peak_hz(x: &[f32]) -> f64 {
    let spec = amplitude_spectrum(x);
    let (k, _) = spec
        .iter()
        .enumerate()
        .skip(1)
        .fold((0, 0.0), |acc, (k, &a)| if a > acc.1 { (k, a) } else { acc });
    k as f64 * FS as f64 / x.len() as f64
}

#[test]
fn a_shift_moves_a_tone_by_the_ratio() {
    let n = 96_000;
    let tone: Vec<f32> = sine(1_000.0, 0.5, SR, n + 48_000).iter().map(|&v| v as f32).collect();
    for cents in [100.0f32, -100.0, 20.0, -20.0] {
        let mut d = DopplerShifter::new(FS, 20.0, 20.0);
        d.set_base_delay(FS, 5.0);
        d.set_cents(cents);
        let y: Vec<f32> = tone.iter().map(|&v| d.process(v)).collect();
        // 2 s analysed after 1 s of warm-up: bins are 0.5 Hz wide.
        let got = peak_hz(&y[48_000..48_000 + n]);
        let want = 1_000.0 * 2f64.powf(cents as f64 / 1200.0);
        assert!(
            (got - want).abs() <= 1.0,
            "{cents} cents: peak at {got:.2} Hz, want {want:.2} Hz"
        );
    }
}

#[test]
fn the_crossfade_keeps_the_level_of_noise() {
    let x = noise(96_000, 0.5, 9);
    for cents in [9.0f32, -9.0, 50.0] {
        let mut d = DopplerShifter::new(FS, 20.0, 20.0);
        d.set_base_delay(FS, 15.0);
        d.set_cents(cents);
        let y: Vec<f32> = x.iter().map(|&v| d.process(v)).collect();
        let ratio_db = db(rms(&y[4_800..]) / rms(&x[4_800..]));
        // Two uncorrelated taps with sin²/cos² gains average −1.25 dB of
        // power over a cycle, and the Hermite read droops white noise's
        // top octave a little more (≈ −2.2 dB total). A broken crossfade
        // (gains not summing to 1, a tap stuck at 0) reads far off.
        assert!(
            (-3.0..=0.5).contains(&ratio_db),
            "{cents} cents: level moved {ratio_db:.2} dB"
        );
        assert!(y.iter().all(|v| v.is_finite()));
    }
}

#[test]
fn reset_clears_the_line_and_non_finite_input_is_dropped() {
    let mut d = DopplerShifter::new(FS, 10.0, 10.0);
    d.set_cents(30.0);
    for _ in 0..2_000 {
        d.process(0.9);
    }
    d.reset();
    for _ in 0..2_000 {
        assert_eq!(d.process(0.0), 0.0);
    }
    for _ in 0..2_000 {
        assert!(d.process(f32::NAN).is_finite());
    }
}
