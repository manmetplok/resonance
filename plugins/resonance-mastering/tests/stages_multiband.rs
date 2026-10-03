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
    // The bypass path is a plain delay line (the crossovers don't run),
    // so the output must be bit-exactly the input delayed by the
    // reported latency, zero-padded at the start.
    let sr = 48_000.0_f32;
    let latency = Multiband::latency_for(sr);
    let n = latency + 2048;
    let mut mb = Multiband::new(sr, n);

    let (input_l, _input_r) = sine_stereo(sr, 440.0, 0.5, n);
    let mut l = input_l.clone();
    let mut r = l.clone();
    mb.process_stereo(&mut l, &mut r, &MultibandConfig::default());

    for i in 0..n {
        let expected = if i < latency {
            0.0
        } else {
            input_l[i - latency]
        };
        assert!(
            l[i].to_bits() == expected.to_bits() && r[i].to_bits() == expected.to_bits(),
            "bypass output differs from delayed input at frame {i}"
        );
    }
}

#[test]
fn toggling_enable_stays_continuous() {
    // Disabled → enabled → disabled with all band compressors off. The
    // crossovers restart from silence on the enable edge, but the
    // subtraction topology sums the bands to the delayed input for any
    // filter state, so the output must track the delayed input across
    // both toggles (and through the filters' warm-up) without a glitch.
    let sr = 48_000.0_f32;
    let latency = Multiband::latency_for(sr);
    let block = 512;
    let seg = latency + 2048;
    let n = 3 * seg;
    let mut mb = Multiband::new(sr, block);

    let off = MultibandConfig::default();
    let on = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };

    let (input_l, _input_r) = sine_stereo(sr, 440.0, 0.5, n);
    let mut l = input_l.clone();
    let mut r = l.clone();
    let mut start = 0;
    while start < n {
        let end = (start + block).min(n);
        let cfg = if start < seg || start >= 2 * seg {
            &off
        } else {
            &on
        };
        mb.process_stereo(&mut l[start..end], &mut r[start..end], cfg);
        start = end;
    }

    let mut max_err = 0.0_f32;
    for i in latency..n {
        max_err = max_err.max((l[i] - input_l[i - latency]).abs());
    }
    assert!(max_err < 2e-2, "toggle continuity error = {max_err}");
}

#[test]
fn enabled_without_compression_reconstructs_delayed_input() {
    // All compressors off → bands sum to delayed input (modulo FIR
    // truncation / Hann-window ripple in the crossover lowpasses).
    let sr = 48_000.0_f32;
    let latency = Multiband::latency_for(sr);
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
    let latency = Multiband::latency_for(sr);
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
        ..BandConfig::default()
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
    let latency = Multiband::latency_for(sr);
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
        ..BandConfig::default()
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

#[test]
fn side_only_high_band_content_is_compressed() {
    // Anti-phase 9 kHz at 0.8 per channel — pure side content, landing
    // in the top band with the default crossovers. The old per-band
    // mono-sum detector read this as silence and the band compressor
    // never engaged; the max-of-channels detector must compress it.
    let sr = 48_000.0_f32;
    let latency = Multiband::latency_for(sr);
    let block = 512;
    let n = latency + 24_000;
    let mut mb = Multiband::new(sr, block);
    let mut cfg = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };
    cfg.bands[3] = BandConfig {
        enabled: true,
        threshold_db: -30.0,
        ratio: 8.0,
        attack_ms: 1.0,
        release_ms: 50.0,
        knee_db: 0.0,
        ..BandConfig::default()
    };

    let (mut l, mut r) = sine_stereo(sr, 9_000.0, 0.8, n);
    for s in r.iter_mut() {
        *s = -*s;
    }
    let mut start = 0;
    while start < n {
        let end = (start + block).min(n);
        let (lh, rh) = (&mut l[start..end], &mut r[start..end]);
        mb.process_stereo(lh, rh, &cfg);
        start = end;
    }

    // 0.8 ≈ −1.94 dBFS is 28 dB over the −30 dB threshold at 8:1 →
    // ~24.5 dB of steady-state GR; anything close to zero means the
    // detector cancelled the side content again.
    let gr = mb.band_gr_db()[3];
    assert!(gr > 10.0, "top-band GR on side-only content = {gr} dB");
    let tail = &l[n - 4096..];
    let peak = tail.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
    assert!(peak < 0.3, "side-only 9 kHz settled peak = {peak}");
}

// ---- DSP2-05 / DSP2-08: switching and trims must not step ----

/// Run `n` frames of a stereo sine through `mb` in `block`-sized blocks,
/// picking each block's config with `cfg(block_index)`.
fn run_blocks(
    mb: &mut Multiband,
    sr: f32,
    freq: f32,
    n: usize,
    block: usize,
    cfg: impl Fn(usize) -> MultibandConfig,
) -> Vec<f32> {
    let (mut l, mut r) = sine_stereo(sr, freq, 0.5, n);
    let mut start = 0;
    let mut k = 0;
    while start < n {
        let end = (start + block).min(n);
        mb.process_stereo(&mut l[start..end], &mut r[start..end], &cfg(k));
        start = end;
        k += 1;
    }
    l
}

fn max_step(x: &[f32]) -> f32 {
    x.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

fn rms_db(x: &[f32]) -> f32 {
    let ms = x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64;
    10.0 * ms.log10() as f32
}

/// Largest per-sample step of a 0.5-amplitude sine at `freq`.
fn sine_step(sr: f32, freq: f32) -> f32 {
    0.5 * std::f32::consts::TAU * freq / sr
}

fn top_band_boost() -> MultibandConfig {
    let mut cfg = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };
    cfg.bands[3].gain_db = 6.0;
    cfg
}

#[test]
fn enabling_does_not_route_the_whole_mix_through_the_top_band() {
    // 1 kHz sits in band 2, so with band 3 at +6 dB the enabled steady
    // state is unity. The crossovers restart from silence on the enable
    // edge, and while they refill `band_3 = delayed − y3` is the whole
    // mix: it used to get band 3's +6 dB for one FIR length.
    let sr = 48_000.0_f32;
    let block = 128;
    let toggle_block = 400; // ~1.07 s in, well past the bypass latency.
    let n = (toggle_block + 800) * block;
    let mut mb = Multiband::new(sr, block);
    let on = top_band_boost();
    let off = MultibandConfig {
        enabled: false,
        ..on
    };
    let out = run_blocks(&mut mb, sr, 1_000.0, n, block, |k| {
        if k < toggle_block {
            off
        } else {
            on
        }
    });
    let t = toggle_block * block;
    let steady = rms_db(&out[n - 8192..]);
    let first_85ms = rms_db(&out[t..t + 4096]);
    assert!(
        (first_85ms - steady).abs() < 0.5,
        "first 85 ms after enable at {first_85ms:.2} dB vs steady {steady:.2} dB"
    );
    let step = max_step(&out[t - 1..]);
    assert!(
        step < 1.1 * sine_step(sr, 1_000.0),
        "enable step {step} vs sine's own {}",
        sine_step(sr, 1_000.0)
    );
}

#[test]
fn enabling_and_disabling_a_boosted_band_ramps() {
    // 8 kHz sits in band 3: the trim takes the level from 0.5 to 1.0
    // when enabled and back when disabled. Both edges must ramp.
    let sr = 48_000.0_f32;
    let block = 128;
    let (on_at, off_at) = (400, 1_200);
    let n = 1_600 * block;
    let mut mb = Multiband::new(sr, block);
    let on = top_band_boost();
    let off = MultibandConfig {
        enabled: false,
        ..on
    };
    let out = run_blocks(&mut mb, sr, 8_000.0, n, block, |k| {
        if (on_at..off_at).contains(&k) {
            on
        } else {
            off
        }
    });
    // A per-sample bound is useless this close to Nyquist (the sine's own
    // step is ~0.5), so bound the envelope instead: the peak of each
    // 6-sample period may move by at most a few percent of the 0.5
    // amplitude difference (a 10 ms ramp moves it ~0.6 % per period; a
    // hard switch moves it 100 % in one).
    for (name, at) in [("enable", on_at), ("disable", off_at)] {
        let t = at * block;
        let env: Vec<f32> = out[t - 60..t + 16_384]
            .chunks_exact(6)
            .map(|c| c.iter().fold(0.0f32, |m, v| m.max(v.abs())))
            .collect();
        let jump = max_step(&env);
        assert!(jump < 0.05, "{name}: envelope jumps {jump} in one period");
    }
    // And the boost actually lands once settled.
    let t = off_at * block;
    let boosted = rms_db(&out[t - 4096..t]);
    let plain = rms_db(&out[n - 4096..]);
    assert!(
        (boosted - plain - 6.0).abs() < 0.3,
        "boost {boosted:.2} dB vs {plain:.2} dB"
    );
}

#[test]
fn band_gain_sweep_does_not_zipper() {
    // Band 0 trim swept 0 -> 6 dB over 20 blocks on a 50 Hz sine: block-
    // rate steps of 0.3 dB used to land as one-sample jumps.
    let sr = 48_000.0_f32;
    let block = 128;
    let start_block = 600;
    let n = (start_block + 200) * block;
    let mut mb = Multiband::new(sr, block);
    let out = run_blocks(&mut mb, sr, 50.0, n, block, |k| {
        let mut cfg = MultibandConfig {
            enabled: true,
            ..MultibandConfig::default()
        };
        let p = (k.saturating_sub(start_block) as f32 / 20.0).min(1.0);
        cfg.bands[0].gain_db = 6.0 * p;
        cfg
    });
    let t = start_block * block;
    // The 50 Hz sine at up to +6 dB steps at most 2x its own step.
    let limit = 2.0 * sine_step(sr, 50.0) * 1.1;
    let step = max_step(&out[t - 1..t + 40 * block]);
    assert!(step < limit, "sweep step {step} vs limit {limit}");
}
