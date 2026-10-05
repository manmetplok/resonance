//! "For every algorithm" (§5.2) on Room, Chamber and Ambience: random
//! automation, a 60 s Freeze, reset equals fresh, a size sweep that does
//! not click, and the viz getters.

use resonance_reverb::dsp::Algorithm;

use crate::common::*;

#[test]
fn room_survives_60_s_of_random_automation() {
    survives_random_automation(Algorithm::Room, 0x5eed_0000_524f_4f4d);
}

#[test]
fn chamber_survives_60_s_of_random_automation() {
    survives_random_automation(Algorithm::Chamber, 0x5eed_0043_4841_4d42);
}

#[test]
fn ambience_survives_60_s_of_random_automation() {
    survives_random_automation(Algorithm::Ambience, 0x5eed_414d_4249_454e);
}

#[test]
fn room_freeze_holds_the_tail_for_60_s() {
    assert_freeze_holds_60_s(Setup::new(Algorithm::Room, 0.5, 1.2).with(|s| s.mod_depth = 0.6));
}

#[test]
fn chamber_freeze_holds_the_tail_for_60_s() {
    assert_freeze_holds_60_s(Setup::vocal_chamber());
}

#[test]
fn ambience_freeze_holds_the_tail_for_60_s() {
    assert_freeze_holds_60_s(Setup::new(Algorithm::Ambience, 0.5, 0.8));
}

/// Freeze engaged and released under a sustained sine: no click either
/// way, though the input (into the reflections, the diffusers and the
/// direct diffuse path, Ambience's ×2) is at full level when it lands.
#[test]
fn room_freeze_engage_and_release_do_not_click() {
    assert_freeze_is_click_free(&Setup::new(Algorithm::Room, 0.5, 1.2), FREEZE_CLICK_MARGIN);
}

#[test]
fn chamber_freeze_engage_and_release_do_not_click() {
    assert_freeze_is_click_free(&Setup::vocal_chamber(), FREEZE_CLICK_MARGIN);
}

#[test]
fn ambience_freeze_engage_and_release_do_not_click() {
    assert_freeze_is_click_free(&Setup::new(Algorithm::Ambience, 0.5, 0.8), FREEZE_CLICK_MARGIN);
}

/// A cleared engine renders bit-identically to a fresh one configured
/// with the same values, after a history with size moves (glides and a
/// geometry crossfade), decay and ER changes, Freeze, and the modulators
/// running.
fn reset_equals_fresh(s: Setup) {
    let mut used = s.dsp();
    let mut rng = Rng(7);
    for i in 0..(1.5 * SR) as usize {
        if i == 12_000 {
            used.set_size(0.9);
            used.set_decay(4.0);
            used.set_er_time(0.8);
        }
        if i == 12_500 {
            // Mid-crossfade: queued.
            used.set_size(0.7);
        }
        if i == 30_000 {
            used.set_freeze(true);
        }
        if i == 40_000 {
            used.set_freeze(false);
            used.set_size(0.3);
            used.set_decay(0.9);
            used.set_er_level(0.7);
        }
        let x = 0.3 * rng.gauss();
        used.process(x, -x, s.diffusion, s.width);
    }
    used.clear();

    let mut fresh = s.dsp();
    fresh.set_size(0.3);
    fresh.set_decay(0.9);
    fresh.set_er_time(0.8);
    fresh.set_er_level(0.7);

    let mut rng = Rng(9);
    let mut energy = 0.0f64;
    for i in 0..(0.6 * SR) as usize {
        let x = if i < 2_000 { 0.3 * rng.gauss() } else { 0.0 };
        let a = used.process(x, 0.4 * x, s.diffusion, s.width);
        let b = fresh.process(x, 0.4 * x, s.diffusion, s.width);
        assert_eq!(
            (a.0.to_bits(), a.1.to_bits()),
            (b.0.to_bits(), b.1.to_bits()),
            "{:?}: reset and fresh differ at sample {i}: {a:?} vs {b:?}",
            s.algorithm
        );
        energy += (a.0 as f64).powi(2);
    }
    assert!(
        energy > 1e-3,
        "{:?}: the comparison rendered silence",
        s.algorithm
    );
}

#[test]
fn room_reset_equals_fresh() {
    reset_equals_fresh(Setup::new(Algorithm::Room, 0.5, 1.2).with(|s| {
        s.mod_depth = 0.8;
        s.mod_rate = 3.0;
    }));
}

#[test]
fn chamber_reset_equals_fresh() {
    reset_equals_fresh(Setup::new(Algorithm::Chamber, 0.5, 1.2).with(|s| {
        s.mod_depth = 1.0;
        s.mod_rate = 4.0;
    }));
}

#[test]
fn ambience_reset_equals_fresh() {
    reset_equals_fresh(Setup::new(Algorithm::Ambience, 0.5, 0.8));
}

/// A size sweep under a sustained sine (0.2 → 0.9 over 2 s, stepped per
/// 128-sample block as the plugin's block-rate smoother delivers it) does
/// not click.
///
/// The `switching.rs` check (largest sample step ≤ the held render's +
/// 1e-3) assumes the level does not move, and here it does: the lines
/// glide, the modes near 220 Hz move under the sine, and the steady-state
/// level rises by up to 2× along the sweep, which raises a sine's largest
/// step by the same factor without any discontinuity. So both metrics are
/// taken per unit of level ([`Clicks`]) against the held rooms' worst:
/// the step, and the click detector proper, the second difference (a
/// sine's is `(2πf/fs)²` of its level, a discontinuity's is the jump).
fn size_sweep_does_not_click(s: Setup) {
    let (warm, window) = (SR as usize, 2 * SR as usize);
    let held = |size: f32| {
        let s = s.with(|s| s.size = size);
        let mut d = s.dsp();
        run_sine(&mut d, &s, 0, warm, |_, _| {});
        let (l, r) = run_sine(&mut d, &s, warm, window, |_, _| {});
        Clicks::of(&l, &r)
    };
    let held = held(0.2).max(held(0.9));
    let s = s.with(|s| s.size = 0.2);
    let mut d = s.dsp();
    run_sine(&mut d, &s, 0, warm, |_, _| {});
    let (l, r) = run_sine(&mut d, &s, warm, window, |d, k| {
        d.set_size(0.2 + 0.7 * k as f32 / window as f32);
    });
    let swept = Clicks::of(&l, &r);
    println!(
        "{:?} size sweep: step/peak {:.5} (held {:.5}), 2nd diff/peak {:.5} (held {:.5}), \
         peak {:.3}",
        s.algorithm, swept.step, held.step, swept.d2, held.d2, swept.peak
    );
    assert!(swept.peak > 0.05, "{:?}: the sweep is near silent", s.algorithm);
    swept.assert_within(&held, 1e-3, &format!("{:?} size sweep", s.algorithm));
}

#[test]
fn room_size_sweep_does_not_click() {
    size_sweep_does_not_click(Setup::new(Algorithm::Room, 0.2, 1.2));
}

#[test]
fn chamber_size_sweep_does_not_click() {
    size_sweep_does_not_click(Setup::vocal_chamber());
}

#[test]
fn ambience_size_sweep_does_not_click() {
    size_sweep_does_not_click(Setup::new(Algorithm::Ambience, 0.2, 0.8));
}

/// The tank view and ER view read the engine: eight line-pair lengths
/// that follow `size`, live energies after an impulse, and the first 12
/// shoebox taps (ascending, inside the box's reach, with gains).
#[test]
fn the_viz_getters_describe_the_room() {
    for algorithm in [Algorithm::Room, Algorithm::Chamber, Algorithm::Ambience] {
        let s = Setup::new(algorithm, 0.2, 1.0);
        let mut d = s.dsp();
        let small = d.fdn_delay_ms();
        let small_er = d.er_tap_times_ms();
        d.set_size(0.9);
        let big = d.fdn_delay_ms();
        let big_er = d.er_tap_times_ms();
        for k in 0..8 {
            assert!(
                big[k] > small[k],
                "{algorithm:?}: line pair {k} did not grow"
            );
            if k > 0 {
                assert!(
                    small[k] >= small[k - 1],
                    "{algorithm:?}: pairs not ascending"
                );
            }
        }
        assert!(
            small[0] > 1.0 && big[7] < 150.0,
            "{algorithm:?}: {small:?} / {big:?}"
        );
        for k in 0..12 {
            let (l, r) = small_er[k];
            assert!(
                l > 0.0 && r > 0.0 && l < 100.0,
                "{algorithm:?}: tap {k} at {l}/{r} ms"
            );
            if k > 0 {
                assert!(l >= small_er[k - 1].0, "{algorithm:?}: taps not ascending");
            }
        }
        assert!(
            big_er[11].0 > small_er[11].0,
            "{algorithm:?}: ER did not grow with size"
        );
        let gains = d.er_tap_gains();
        assert!(gains.iter().all(|g| g.0.abs() > 0.0 && g.1.abs() > 0.0));

        for i in 0..4_800 {
            let x = if i == 0 { 1.0 } else { 0.0 };
            d.process(x, x, s.diffusion, s.width);
        }
        let e = d.channel_energies();
        assert!(
            e.iter().all(|&v| v > 0.0),
            "{algorithm:?}: dead line pair {e:?}"
        );
        println!("{algorithm:?}: lines {small:.1?} -> {big:.1?} ms, energies {e:.4?}");
    }
}
