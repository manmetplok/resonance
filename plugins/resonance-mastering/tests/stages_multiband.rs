use resonance_mastering::stages::multiband::{BandConfig, Multiband, MultibandConfig};

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

#[test]
fn oversized_block_processes_all_frames() {
    // Host sends a block larger than the construction-time max_buffer.
    // The stage must chunk internally instead of silently capping (the
    // old behaviour left every frame past max_buffer untouched).
    let sr = 48_000.0_f32;
    let latency = Multiband::latency();
    let n = latency + 2048;
    let max_buffer = 256; // far smaller than the block we send
    let mut mb = Multiband::new(sr, max_buffer);

    let (input_l, _input_r) = sine_stereo(sr, 440.0, 0.5, n);
    let mut l = input_l.clone();
    let mut r = l.clone();
    mb.process_stereo(&mut l, &mut r, &MultibandConfig::default());

    // Disabled config = pure delay, which must hold across the entire
    // oversized block — including the region past max_buffer.
    let mut max_err = 0.0_f32;
    for i in latency..n {
        max_err = max_err.max((l[i] - input_l[i - latency]).abs());
    }
    assert!(max_err < 5e-3, "oversized-block delay error = {max_err}");
}

#[test]
fn oversized_block_matches_chunked_processing_bitwise() {
    // One oversized call must produce exactly what a host sending
    // max_buffer-sized blocks would get.
    let sr = 48_000.0_f32;
    let n = 4096 + 333; // deliberately not a multiple of max_buffer
    let max_buffer = 512;
    let mut cfg = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };
    cfg.bands[0] = BandConfig {
        enabled: true,
        threshold_db: -30.0,
        ratio: 4.0,
        gain_db: 1.5,
    };

    let (input_l, input_r) = sine_stereo(sr, 80.0, 0.5, n);

    let mut one_l = input_l.clone();
    let mut one_r = input_r.clone();
    let mut mb_one = Multiband::new(sr, max_buffer);
    mb_one.process_stereo(&mut one_l, &mut one_r, &cfg);

    let mut many_l = input_l;
    let mut many_r = input_r;
    let mut mb_many = Multiband::new(sr, max_buffer);
    let mut start = 0;
    while start < n {
        let end = (start + max_buffer).min(n);
        mb_many.process_stereo(&mut many_l[start..end], &mut many_r[start..end], &cfg);
        start = end;
    }

    for i in 0..n {
        assert!(
            one_l[i].to_bits() == many_l[i].to_bits()
                && one_r[i].to_bits() == many_r[i].to_bits(),
            "frame {i} differs between oversized and chunked processing"
        );
    }
}
