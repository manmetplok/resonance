use super::*;

fn sine_stereo(sr: f32, freq: f32, amp: f32, n: usize) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    for i in 0..n {
        let s = (i as f32 / sr * freq * std::f32::consts::TAU).sin() * amp;
        l[i] = s;
        r[i] = s;
    }
    (l, r)
}

#[test]
fn disabled_is_pure_delay() {
    let sr = 48_000.0_f32;
    let latency = Multiband::latency();
    let n = latency + 2048;
    let mut mb = Multiband::new(sr, n);

    let (input_l, _input_r) = sine_stereo(sr, 440.0, 0.5, n);
    let mut l = input_l.clone();
    let mut r = l.clone();
    mb.process_stereo(&mut l, &mut r, &MultibandConfig::default());

    let mut max_err = 0.0_f32;
    for i in latency..n {
        max_err = max_err.max((l[i] - input_l[i - latency]).abs());
    }
    assert!(max_err < 5e-3, "bypass delay error = {max_err}");
}

#[test]
fn enabled_without_compression_reconstructs_delayed_input() {
    // All compressors off → bands sum to delayed input (modulo FIR
    // truncation / Hann-window ripple in the crossover lowpasses).
    let sr = 48_000.0_f32;
    let latency = Multiband::latency();
    let n = latency + 2048;
    let mut mb = Multiband::new(sr, n);
    let cfg = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };

    let (input_l, _input_r) = sine_stereo(sr, 440.0, 0.5, n);
    let mut l = input_l.clone();
    let mut r = l.clone();
    mb.process_stereo(&mut l, &mut r, &cfg);

    let mut max_err = 0.0_f32;
    for i in latency..n {
        max_err = max_err.max((l[i] - input_l[i - latency]).abs());
    }
    assert!(
        max_err < 2e-2,
        "reconstruction error = {max_err} (expected < 0.02)"
    );
}

#[test]
fn compressing_a_band_attenuates_only_that_band() {
    // Feed a 50 Hz sine (lives in band_0) through with only band_0
    // compressing hard. Output should be quieter than input.
    let sr = 48_000.0_f32;
    let latency = Multiband::latency();
    let n = latency + 4096;
    let mut mb = Multiband::new(sr, n);
    let mut cfg = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };
    cfg.bands[0] = BandConfig {
        enabled: true,
        threshold_db: -30.0,
        ratio: 8.0,
        gain_db: 0.0,
    };

    let (mut l, mut r) = sine_stereo(sr, 50.0, 0.5, n);
    mb.process_stereo(&mut l, &mut r, &cfg);
    let tail = &l[latency + 2048..];
    let peak = tail.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
    assert!(peak < 0.3, "band0 compressed 50 Hz peak = {peak}");
}
