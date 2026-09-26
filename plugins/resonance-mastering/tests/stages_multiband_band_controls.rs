//! Per-band controls of the multiband stage (ba todo #1317, audit
//! findings M1 and M2).
//!
//! M1: attack / release / knee / mix were hardcoded inside the stage
//! while each band ran a full `GlueCompressor` that honours all of them,
//! so the defining multiband move — fast release on the lows, slow on
//! the highs — was unreachable by any means, automation included.
//!
//! M2: band Gain was routed as the compressor's makeup, and the
//! compressor returns before applying makeup when it is disabled, so a
//! band whose compressor was off had a silent Gain knob and the stage
//! could not be used as a static four-band tone balancer.

use resonance_mastering::params::MasteringParams;
use resonance_mastering::stages::glue_compressor::{GlueCompressor, GlueCompressorConfig};
use resonance_mastering::stages::multiband::{
    band_gain, BandConfig, Multiband, MultibandConfig, NUM_BANDS,
};

const SR: f32 = 48_000.0;

/// A burst: loud enough to compress for `on_n` frames, then silent, so
/// the release stage is what shapes the tail.
fn burst_stereo(freq: f32, amp: f32, on_n: usize, total: usize) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0f32; total];
    for (i, s) in l.iter_mut().enumerate() {
        if i < on_n {
            *s = (i as f32 / SR * freq * std::f32::consts::TAU).sin() * amp;
        }
    }
    let r = l.clone();
    (l, r)
}

fn sine_stereo(freq: f32, amp: f32, n: usize) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0f32; n];
    for (i, s) in l.iter_mut().enumerate() {
        *s = (i as f32 / SR * freq * std::f32::consts::TAU).sin() * amp;
    }
    let r = l.clone();
    (l, r)
}

fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

fn run(cfg: &MultibandConfig, l: &mut [f32], r: &mut [f32]) {
    let mut mb = Multiband::new(SR, l.len());
    mb.process_stereo(l, r, cfg);
}

// --- M1: the ballistics are reachable -----------------------------------

/// The headline case: two identical mixes, one with a fast release on
/// the low band and one with a slow release, must not sound the same.
#[test]
fn a_per_band_release_change_is_audible() {
    let latency = Multiband::latency_for(SR);
    let on_n = 12_000; // 250 ms of tone
    let total = latency + 36_000;

    let mut fast = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };
    fast.bands[0] = BandConfig {
        enabled: true,
        threshold_db: -30.0,
        ratio: 8.0,
        attack_ms: 1.0,
        release_ms: 10.0,
        ..BandConfig::default()
    };
    let mut slow = fast;
    slow.bands[0].release_ms = 1000.0;

    let (mut fl, mut fr) = burst_stereo(50.0, 0.6, on_n, total);
    run(&fast, &mut fl, &mut fr);
    let (mut sl, mut sr_) = burst_stereo(50.0, 0.6, on_n, total);
    run(&slow, &mut sl, &mut sr_);

    // During the tone the slow release keeps holding gain reduction
    // that the fast release has already let go of, so the two blocks
    // differ substantially — not by a rounding error.
    let window = latency + 2_000..latency + on_n;
    let fast_rms = rms(&fl[window.clone()]);
    let slow_rms = rms(&sl[window.clone()]);
    let diff = rms(&fl[window.clone()]
        .iter()
        .zip(&sl[window])
        .map(|(a, b)| a - b)
        .collect::<Vec<_>>());
    assert!(
        diff > 0.01 * fast_rms.max(slow_rms),
        "release change was inaudible: fast rms {fast_rms}, slow rms {slow_rms}, diff {diff}"
    );
    assert!(
        slow_rms < fast_rms,
        "the slower release should hold more gain reduction \
         (fast {fast_rms}, slow {slow_rms})"
    );
}

#[test]
fn a_per_band_attack_change_is_audible() {
    let latency = Multiband::latency_for(SR);
    let total = latency + 12_000;

    let mut slow = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };
    slow.bands[0] = BandConfig {
        enabled: true,
        threshold_db: -30.0,
        ratio: 8.0,
        attack_ms: 200.0,
        ..BandConfig::default()
    };
    let mut fast = slow;
    fast.bands[0].attack_ms = 1.0;

    let (mut a, mut ar) = sine_stereo(50.0, 0.6, total);
    run(&slow, &mut a, &mut ar);
    let (mut b, mut br) = sine_stereo(50.0, 0.6, total);
    run(&fast, &mut b, &mut br);

    // Right after the tone starts, the slow attack has barely clamped
    // down while the fast one already has.
    let window = latency..latency + 1_000;
    assert!(
        rms(&a[window.clone()]) > rms(&b[window.clone()]) * 1.1,
        "attack change was inaudible: slow {}, fast {}",
        rms(&a[window.clone()]),
        rms(&b[window])
    );
}

#[test]
fn a_per_band_knee_change_is_audible() {
    let latency = Multiband::latency_for(SR);
    let total = latency + 12_000;

    let mut hard = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };
    // Sit just above the threshold, where the knee decides everything.
    hard.bands[0] = BandConfig {
        enabled: true,
        threshold_db: -12.0,
        ratio: 8.0,
        knee_db: 0.0,
        ..BandConfig::default()
    };
    let mut soft = hard;
    soft.bands[0].knee_db = 12.0;

    let (mut h, mut hr) = sine_stereo(50.0, 0.25, total);
    run(&hard, &mut h, &mut hr);
    let (mut s, mut sr_) = sine_stereo(50.0, 0.25, total);
    run(&soft, &mut s, &mut sr_);

    // A soft knee starts reducing below the threshold, so a signal
    // sitting on the threshold comes out quieter than with a hard knee.
    let window = latency + 4_000..total;
    assert!(
        rms(&h[window.clone()]) > rms(&s[window.clone()]) * 1.02,
        "knee change was inaudible: hard {}, soft {}",
        rms(&h[window.clone()]),
        rms(&s[window])
    );
}

#[test]
fn a_per_band_mix_change_is_audible() {
    let latency = Multiband::latency_for(SR);
    let total = latency + 12_000;

    let mut wet = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };
    wet.bands[0] = BandConfig {
        enabled: true,
        threshold_db: -30.0,
        ratio: 8.0,
        mix: 1.0,
        ..BandConfig::default()
    };
    let mut parallel = wet;
    parallel.bands[0].mix = 0.0;

    let (mut w, mut wr) = sine_stereo(50.0, 0.6, total);
    run(&wet, &mut w, &mut wr);
    let (mut p, mut pr) = sine_stereo(50.0, 0.6, total);
    run(&parallel, &mut p, &mut pr);

    let window = latency + 4_000..total;
    assert!(
        rms(&p[window.clone()]) > rms(&w[window.clone()]) * 1.5,
        "mix change was inaudible: wet {}, dry {}",
        rms(&w[window.clone()]),
        rms(&p[window])
    );
}

// --- M2: band gain works with the compressor off ------------------------

#[test]
fn band_gain_applies_with_the_compressor_disabled() {
    let latency = Multiband::latency_for(SR);
    let total = latency + 12_000;

    let flat = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };
    let mut boosted = flat;
    // Compressor explicitly OFF — this is the tone-balancer case.
    assert!(!boosted.bands[0].enabled);
    boosted.bands[0].gain_db = 6.0;

    let (mut a, mut ar) = sine_stereo(50.0, 0.25, total);
    run(&flat, &mut a, &mut ar);
    let (mut b, mut br) = sine_stereo(50.0, 0.25, total);
    run(&boosted, &mut b, &mut br);

    let window = latency + 4_000..total;
    let ratio = rms(&b[window.clone()]) / rms(&a[window.clone()]);
    let expected = 10f32.powf(6.0 / 20.0);
    assert!(
        (ratio - expected).abs() < 0.05 * expected,
        "+6 dB on the low band with its compressor off gave {ratio}x, expected {expected}x"
    );
}

#[test]
fn band_gain_cuts_as_well_as_boosts_with_the_compressor_disabled() {
    let latency = Multiband::latency_for(SR);
    let total = latency + 12_000;

    let flat = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };
    let mut cut = flat;
    cut.bands[3].gain_db = -12.0;

    // 12 kHz sits in the top band (crossover 3 defaults to 4 kHz).
    let (mut a, mut ar) = sine_stereo(12_000.0, 0.25, total);
    run(&flat, &mut a, &mut ar);
    let (mut b, mut br) = sine_stereo(12_000.0, 0.25, total);
    run(&cut, &mut b, &mut br);

    let window = latency + 4_000..total;
    let ratio = rms(&b[window.clone()]) / rms(&a[window.clone()]);
    let expected = 10f32.powf(-12.0 / 20.0);
    assert!(
        (ratio - expected).abs() < 0.05,
        "−12 dB on the high band gave {ratio}x, expected {expected}x"
    );
}

/// The trim must be a band control, not a compressor control: the same
/// gain has the same effect whether that band's compressor is running.
#[test]
fn band_gain_is_independent_of_the_compressor_enable() {
    let latency = Multiband::latency_for(SR);
    let total = latency + 12_000;
    let window = latency + 4_000..total;

    let mut with_comp = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };
    with_comp.bands[0] = BandConfig {
        enabled: true,
        // Threshold well above the signal: the compressor is running
        // but not reducing, so only the trim moves the level.
        threshold_db: -3.0,
        ..BandConfig::default()
    };
    let mut with_comp_boost = with_comp;
    with_comp_boost.bands[0].gain_db = 6.0;

    let mut no_comp = with_comp;
    no_comp.bands[0].enabled = false;
    let mut no_comp_boost = no_comp;
    no_comp_boost.bands[0].gain_db = 6.0;

    let measure = |cfg: &MultibandConfig| {
        let (mut l, mut r) = sine_stereo(50.0, 0.25, total);
        run(cfg, &mut l, &mut r);
        rms(&l[window.clone()])
    };

    let on_ratio = measure(&with_comp_boost) / measure(&with_comp);
    let off_ratio = measure(&no_comp_boost) / measure(&no_comp);
    assert!(
        (on_ratio - off_ratio).abs() < 0.02,
        "trim behaved differently with the compressor on ({on_ratio}x) and off ({off_ratio}x)"
    );
}

/// With every compressor off and every trim at 0 dB the four bands must
/// still reconstruct the input through the crossover. This is a
/// reconstruction check at audio tolerance — it says nothing about bit
/// exactness, which is what the next two tests are for.
#[test]
fn the_neutral_stage_reconstructs_the_input() {
    let latency = Multiband::latency_for(SR);
    let total = latency + 4_096;
    let cfg = MultibandConfig {
        enabled: true,
        ..MultibandConfig::default()
    };
    for band in &cfg.bands {
        assert_eq!(band.gain_db, 0.0);
    }

    let (mut l, mut r) = sine_stereo(440.0, 0.5, total);
    let (input, _) = sine_stereo(440.0, 0.5, total);
    run(&cfg, &mut l, &mut r);

    let mut max_err = 0.0f32;
    for i in latency..total {
        max_err = max_err.max((l[i] - input[i - latency]).abs());
    }
    assert!(max_err < 5e-3, "reconstruction error = {max_err}");
}

/// The trim at its default must be a true identity, not merely close:
/// multiplying by it has to return the same bits, for every sample.
#[test]
fn zero_gain_is_bit_exact_unity() {
    assert_eq!(band_gain(0.0), 1.0);
    assert_eq!(band_gain(0.0).to_bits(), 1.0f32.to_bits());

    for x in [
        0.0f32,
        -0.0,
        1e-30,
        0.1,
        -0.25,
        0.499_999_97,
        0.999_999_94,
        1.0,
        12.5,
        -1e12,
        f32::MIN_POSITIVE,
    ] {
        assert_eq!(
            (x * band_gain(0.0)).to_bits(),
            x.to_bits(),
            "the neutral trim changed {x}"
        );
    }
}

/// The case this change could most plausibly have broken: a band whose
/// compressor is RUNNING, with the trim left at 0 dB. The old code
/// carried the trim as the compressor's makeup; the new code applies it
/// when the bands are summed. Both expressions are reproduced here and
/// compared bit-for-bit, because `dry + (X - dry)` is not `X` in f32 and
/// the re-association is real even when the algebra says otherwise.
#[test]
fn an_enabled_band_at_zero_gain_matches_the_old_makeup_path_bit_for_bit() {
    let (old, new) = run_old_and_new_gain_paths(0.0);
    for (i, (o, n)) in old.iter().zip(&new).enumerate() {
        assert_eq!(
            o.to_bits(),
            n.to_bits(),
            "sample {i}: old {o} vs new {n} — a compressing band at 0 dB \
             trim must be bit-identical"
        );
    }
}

/// …and with a NON-zero trim the two are no longer bit-identical, by a
/// few ULP, because the multiply moved outside the compressor's dry/wet
/// blend. Pinned so the difference is a documented ~1e-6 relative rather
/// than a surprise in some future null test against an old render.
#[test]
fn a_non_zero_band_gain_differs_from_the_old_makeup_path_by_at_most_a_few_ulp() {
    for gain_db in [-12.0f32, -1.5, 1.5, 6.0, 12.0] {
        let (old, new) = run_old_and_new_gain_paths(gain_db);
        let mut worst = 0.0f32;
        for (o, n) in old.iter().zip(&new) {
            let scale = o.abs().max(n.abs());
            if scale > 1e-6 {
                worst = worst.max((o - n).abs() / scale);
            }
        }
        assert!(
            worst < 1e-5,
            "{gain_db} dB trim drifted {worst} relative from the old path"
        );
    }
}

/// Run one band's worth of audio through the compressor twice: once the
/// way the stage used to (trim as `makeup_db`, mix pinned at 1.0), once
/// the way it does now (no makeup, trim applied to the band output).
fn run_old_and_new_gain_paths(gain_db: f32) -> (Vec<f32>, Vec<f32>) {
    let n = 8_192;
    let base = GlueCompressorConfig {
        enabled: true,
        threshold_db: -24.0,
        ratio: 4.0,
        attack_ms: 5.0,
        release_ms: 120.0,
        knee_db: 6.0,
        mix: 1.0,
        makeup_db: 0.0,
    };

    let (mut old_l, mut old_r) = sine_stereo(90.0, 0.5, n);
    let mut old_comp = GlueCompressor::new(SR);
    old_comp.process_stereo(
        &mut old_l,
        &mut old_r,
        &GlueCompressorConfig {
            makeup_db: gain_db,
            ..base
        },
    );

    let (mut new_l, mut new_r) = sine_stereo(90.0, 0.5, n);
    let mut new_comp = GlueCompressor::new(SR);
    new_comp.process_stereo(&mut new_l, &mut new_r, &base);
    let g = band_gain(gain_db);
    for s in new_l.iter_mut() {
        *s *= g;
    }

    (old_l, new_l)
}

// --- the params carry it all the way ------------------------------------

#[test]
fn every_band_exposes_the_glue_stage_vocabulary_as_params() {
    let params = MasteringParams::default();
    for i in 0..NUM_BANDS {
        let band = &params.multiband.bands[i];
        let n = i + 1;
        for (param, expected_id, expected_name) in [
            (
                &band.attack,
                format!("mb_b{i}_attack"),
                format!("MB B{n} Attack"),
            ),
            (
                &band.release,
                format!("mb_b{i}_release"),
                format!("MB B{n} Release"),
            ),
            (&band.knee, format!("mb_b{i}_knee"), format!("MB B{n} Knee")),
            (&band.mix, format!("mb_b{i}_mix"), format!("MB B{n} Mix")),
        ] {
            use resonance_plugin::Param;
            assert_eq!(param.id(), expected_id);
            assert_eq!(param.name(), expected_name);
        }
    }
}

/// The band controls must cover the same ground as the single-band glue
/// stage — same ranges, same defaults, so "Attack" means one thing in
/// this plugin.
#[test]
fn band_controls_match_the_glue_stage_ranges() {
    use resonance_plugin::Param;
    let params = MasteringParams::default();
    let glue = &params.glue_compressor;
    for band in &params.multiband.bands {
        for (b, g, what) in [
            (&band.threshold, &glue.threshold, "threshold"),
            (&band.ratio, &glue.ratio, "ratio"),
            (&band.attack, &glue.attack, "attack"),
            (&band.release, &glue.release, "release"),
            (&band.knee, &glue.knee, "knee"),
            (&band.mix, &glue.mix, "mix"),
        ] {
            assert_eq!(b.min_plain(), g.min_plain(), "{what} min");
            assert_eq!(b.max_plain(), g.max_plain(), "{what} max");
            assert_eq!(b.default_plain(), g.default_plain(), "{what} default");
        }
    }
}

/// Every new control has to reach the DSP, or it is just another dial
/// that does nothing.
#[test]
fn the_param_snapshot_carries_the_new_controls() {
    let params = MasteringParams::default();
    let band = &params.multiband.bands[2];
    band.attack.set_value(4.0);
    band.release.set_value(750.0);
    band.knee.set_value(1.5);
    band.mix.set_value(0.25);
    band.gain.set_value(-5.0);

    let cfg = params.multiband.snapshot().bands[2];
    assert_eq!(cfg.attack_ms, 4.0);
    assert_eq!(cfg.release_ms, 750.0);
    assert_eq!(cfg.knee_db, 1.5);
    assert_eq!(cfg.mix, 0.25);
    assert_eq!(cfg.gain_db, -5.0);
}

/// Defaults are the values the stage used to hardcode, so a project
/// saved before these params existed loads and sounds identical.
#[test]
fn defaults_match_the_previously_hardcoded_settings() {
    let cfg = MasteringParams::default().multiband.snapshot().bands[0];
    assert_eq!(cfg.attack_ms, 30.0);
    assert_eq!(cfg.release_ms, 150.0);
    assert_eq!(cfg.knee_db, 6.0);
    assert_eq!(cfg.mix, 1.0);
    assert_eq!(cfg.gain_db, 0.0);
}
