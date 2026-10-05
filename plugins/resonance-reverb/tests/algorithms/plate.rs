//! R3: the Plate engine (reverb-algorithms.md §4.4, §5.2).
//!
//! Every render goes through `ReverbDsp::with_engines(SR, &[Plate])`, a
//! bank holding only Plate (so this holds before Plate joins
//! `Algorithm::BUILT`), with every engine setter called explicitly: 100 %
//! wet by construction (`ReverbDsp` returns the wet signal), no pre-delay,
//! return EQ off, ER/tail balance centred.
//!
//! The acceptance tables print with
//!
//!     cargo test -p resonance-reverb --test algorithms plate -- --nocapture
//!
//! The golden is re-blessed with `RESONANCE_BLESS=1` (or
//! `RESONANCE_BLESS_PLATE=1` for this file alone).

use resonance_metering::decay::ImpulseReport;
use resonance_reverb::dsp::Algorithm;

use crate::common::*;

/// The parameter defaults on the Plate: global, not plate-specific.
fn defaults() -> Setup {
    Setup::new(Algorithm::Plate, 0.5, 2.0)
}

/// The Plate voicing of the factory plates (`Vocal Plate`): treble held
/// to 0.8× above 8 kHz.
fn plate() -> Setup {
    defaults().with(|s| s.high_mult = 0.8)
}

/// §5.2's Plate row at one decay, sizes 0.5 and 1.0, the Plate voicing:
/// mid T30 within ±10 % of the knob, treble (8 kHz) ≥ 0.7 × mid, echo
/// density 0.9 by 10 ms, late IACC ≤ 0.3, mono fold no worse than −3.5 dB.
fn plate_row(decay: f32) {
    for size in [0.5, 1.0] {
        let v = Setup {
            size,
            decay,
            ..plate()
        };
        let (l, r) = v.impulse(render_seconds(decay));
        let rep = ImpulseReport::analyze(&l, &r, SR);
        let mid = rep.mid_t30().expect("mid T30");
        let treble = t30_at(&rep, 8_000.0) / mid;
        let dens = rep.echo_density.time_to_reach(0.9);
        println!(
            "Plate size {size:.1} decay {decay:>3.1}s: mid {mid:.3}s ({:+.1} %), \
             8k/mid {treble:.2}\n  {}\n  {rep}",
            100.0 * (mid / decay - 1.0),
            ImpulseReport::table_header()
        );

        let what = format!("size {size} decay {decay}");
        assert!(rep.finite, "{what}: non-finite output");
        assert_not_silent(&what, &l, &r);
        assert!(
            (mid / decay - 1.0).abs() <= 0.10,
            "{what}: mid T30 {mid:.3} s is not within 10 % of the knob"
        );
        assert!(treble >= 0.7, "{what}: treble/mid {treble:.2} < 0.7");
        let dens = dens.expect("echo density never reaches 0.9");
        assert!(
            dens <= 0.010,
            "{what}: echo density reaches 0.9 at {:.1} ms",
            dens * 1e3
        );
        let iacc = rep.late_iacc.unwrap();
        assert!(iacc <= 0.3, "{what}: late IACC {iacc:.3}");
        let mono = rep.mono_fold_db.unwrap();
        assert!(mono >= -3.5, "{what}: mono fold {mono:.2} dB");
    }
}

#[test]
fn plate_row_decay_0_5s() {
    plate_row(0.5);
}

#[test]
fn plate_row_decay_1_5s() {
    plate_row(1.5);
}

#[test]
fn plate_row_decay_3s() {
    plate_row(3.0);
}

#[test]
fn plate_row_decay_6s() {
    plate_row(6.0);
}

/// The numbers at the *global* parameter defaults (treble 0.5×, the
/// voicing's job is the presets'). Only the knob is asserted here; the
/// rest is the report.
#[test]
fn plate_at_the_global_defaults() {
    let v = defaults();
    let (l, r) = v.impulse(render_seconds(v.decay));
    let rep = ImpulseReport::analyze(&l, &r, SR);
    let mid = rep.mid_t30().unwrap();
    println!(
        "Plate at the global defaults: mid {mid:.3}s, 8k/mid {:.2}, 125/mid {:.2}\n  {}\n  {rep}",
        t30_at(&rep, 8_000.0) / mid,
        t30_at(&rep, 125.0) / mid,
        ImpulseReport::table_header()
    );
    assert!((mid / v.decay - 1.0).abs() <= 0.10, "mid T30 {mid:.3}");
    assert_not_silent("defaults", &l, &r);
}

/// Peakiness against Classic at the baseline rows' size/decay/damping
/// (both engines at the global defaults otherwise, Classic computed
/// here). The spec asks for Classic − 2 dB; where that is below the
/// metric's own floor — decaying Gaussian noise at the same T60, which no
/// tail can beat — the bound is that floor + 1 dB instead (see the
/// report: at size 0.9 / 2 s Classic reads 5.6 dB and noise 5.4 dB).
#[test]
fn plate_is_less_coloured_than_classic() {
    let rows = [
        (0.2, 0.5, 8_000.0),
        (0.5, 2.0, 8_000.0),
        (0.9, 2.0, 8_000.0),
        (0.2, 8.0, 8_000.0),
        (0.5, 8.0, 8_000.0),
        (0.5, 8.0, 20_000.0),
    ];
    println!("size decay damp   Plate  Classic  noise  IACC(P)");
    for (size, decay, damping) in rows {
        let v = Setup {
            size,
            decay,
            damping,
            ..defaults()
        };
        let pk = |a| {
            let (l, r) = v.on(a).impulse(1.3);
            let rep = ImpulseReport::analyze(&l, &r, SR);
            (rep.peakiness_db.unwrap(), rep.late_iacc.unwrap())
        };
        let (p, iacc) = pk(Algorithm::Plate);
        let (c, _) = pk(Algorithm::Classic);
        let n = noise_peakiness(decay);
        println!("{size:>4.1} {decay:>4.1}s {damping:>6.0} {p:>6.1} {c:>8.1} {n:>6.1} {iacc:>7.3}");
        let bound = (c - 2.0).max(n + 1.0);
        assert!(
            p <= bound,
            "size {size} decay {decay}: peakiness {p:.1} dB > {bound:.1} \
             (Classic {c:.1}, noise {n:.1})"
        );
        assert!(iacc <= 0.3, "size {size} decay {decay}: IACC {iacc:.3}");
    }
}

/// Bass and treble decay multipliers do what they say: the 125 Hz and
/// 8 kHz band T30s over the 1 kHz one are within ±15 % of the requested
/// multipliers. The crossovers sit two octaves from the measured bands
/// (500 Hz, 2 kHz) so the first-order shelves are near their asymptotes,
/// and their geometric mean is 1 kHz, where the absorption is exact.
/// (Not both shelves deep at once, e.g. ×0.5 and ×0.25 4× apart: there
/// `Absorption`'s mid compensation would need a broadband gain above 1,
/// which it caps to keep the loop passive, so the mid then decays fast —
/// a documented limit of the R2 primitive, not of the plate.)
#[test]
fn bass_and_treble_multipliers_do_what_they_say() {
    for (low, high) in [(2.0, 0.5), (0.5, 0.5), (1.5, 1.0), (1.0, 0.3)] {
        let v = Setup {
            decay: 2.0,
            low_mult: low,
            low_xover: 500.0,
            high_mult: high,
            damping: 2_000.0,
            ..plate()
        };
        let (l, r) = v.impulse(render_seconds(2.0 * low.max(1.0)));
        let rep = ImpulseReport::analyze(&l, &r, SR);
        let mid = t30_at(&rep, 1_000.0);
        let bass = t30_at(&rep, 125.0) / mid;
        let treble = t30_at(&rep, 8_000.0) / mid;
        println!(
            "low ×{low} high ×{high}: 1k {mid:.3}s, 125/1k {bass:.2}, 8k/1k {treble:.2}\n  {rep}"
        );
        assert!((mid / 2.0 - 1.0).abs() <= 0.10, "1 kHz T30 {mid:.3}");
        assert!(
            (bass / low - 1.0).abs() <= 0.15,
            "bass ×{low} measured ×{bass:.2}"
        );
        assert!(
            (treble / high - 1.0).abs() <= 0.15,
            "treble ×{high} measured ×{treble:.2}"
        );
    }
}

/// The onset (`er_*`) is the first ~50 ms: with the tail balanced out,
/// what is left after 50 ms is 40 dB below the onset's total.
#[test]
fn the_onset_is_the_first_50_ms() {
    let v = plate();
    let mut d = v.dsp();
    d.set_er_tail_balance(-1.0);
    let n = (0.3 * SR) as usize;
    let (mut l, mut r) = (Vec::new(), Vec::new());
    for i in 0..n {
        let x = if i == 0 { 1.0 } else { 0.0 };
        let (a, b) = d.process(x, x, v.diffusion, v.width);
        l.push(a);
        r.push(b);
    }
    assert_not_silent("onset", &l, &r);
    let cut = (0.05 * SR) as usize;
    let total = energy_db(&l, &r);
    let late = energy_db(&l[cut..], &r[cut..]);
    assert!(
        late < total - 40.0,
        "onset after 50 ms at {:.1} dB re total",
        late - total
    );
}

/// 60 s of random automation of every engine setter, as steps and ramps,
/// over noise with impulses: finite throughout, never above +24 dBFS.
#[test]
fn plate_survives_60_s_of_random_automation() {
    survives_random_automation(Algorithm::Plate, 0x5eed_f00d_cafe_d00d);
}

/// Freeze held for 60 s: the tail's energy drifts by at most 0.1 dB.
#[test]
fn freeze_holds_the_tail_for_60_s() {
    assert_freeze_holds_60_s(plate());
}

/// Freeze engaged and released under a sustained sine: no click either
/// way.
#[test]
fn freeze_engage_and_release_do_not_click() {
    assert_freeze_is_click_free(&plate(), FREEZE_CLICK_MARGIN);
}

/// `er_time` swept across its whole range and back under a sustained
/// sine, at block rate as the plugin's smoother delivers it: the onset
/// cascade's lengths move under running audio without a zipper. Against
/// the held render at both ends, step and second difference per unit of
/// level ([`Clicks`]).
#[test]
fn er_time_sweep_under_a_sustained_sine_does_not_zipper() {
    let (warm, window) = (SR as usize, SR as usize);
    // The onset loud against the tail, so its cascade is what is heard.
    let s = plate().with(|s| s.er_level = 1.0);
    let held = |er_time: f32| {
        let s = s.with(|s| s.er_time = er_time);
        let mut d = s.dsp();
        run_sine(&mut d, &s, 0, warm, |_, _| {});
        let (l, r) = run_sine(&mut d, &s, warm, window, |_, _| {});
        Clicks::of(&l, &r)
    };
    let held = held(0.0).max(held(1.0));
    let s = s.with(|s| s.er_time = 0.0);
    let mut d = s.dsp();
    run_sine(&mut d, &s, 0, warm, |_, _| {});
    // 0 → 1 over the first half, back over the second.
    let (l, r) = run_sine(&mut d, &s, warm, window, |d, k| {
        let t = 2.0 * k as f32 / window as f32;
        d.set_er_time(if t <= 1.0 { t } else { 2.0 - t });
    });
    let swept = Clicks::of(&l, &r);
    println!(
        "Plate er_time sweep: step/peak {:.5} (held {:.5}), 2nd diff/peak {:.5} (held {:.5}), \
         peak {:.3}",
        swept.step, held.step, swept.d2, held.d2, swept.peak
    );
    swept.assert_within(&held, 1e-3, "Plate er_time sweep");
}

/// A cleared engine renders bit-identically to a fresh one configured
/// with the same values, after a history with a size glide, a decay
/// change, Freeze, and the modulators running.
#[test]
fn reset_equals_fresh() {
    let v = Setup {
        mod_depth: 0.8,
        mod_rate: 3.0,
        ..plate()
    };
    let mut used = v.dsp();
    let mut rng = Rng(7);
    for i in 0..(1.5 * SR) as usize {
        if i == 12_000 {
            used.set_size(0.9);
            used.set_decay(4.0);
        }
        if i == 30_000 {
            used.set_freeze(true);
        }
        if i == 40_000 {
            used.set_freeze(false);
            used.set_size(0.3);
            used.set_decay(2.5);
        }
        let x = 0.3 * rng.gauss();
        used.process(x, -x, v.diffusion, v.width);
    }
    used.clear();

    let mut fresh = v.dsp();
    fresh.set_size(0.3);
    fresh.set_decay(2.5);

    let mut rng = Rng(9);
    for i in 0..(0.6 * SR) as usize {
        let x = if i < 2_000 { 0.3 * rng.gauss() } else { 0.0 };
        let a = used.process(x, 0.4 * x, v.diffusion, v.width);
        let b = fresh.process(x, 0.4 * x, v.diffusion, v.width);
        assert_eq!(
            (a.0.to_bits(), a.1.to_bits()),
            (b.0.to_bits(), b.1.to_bits()),
            "reset and fresh differ at sample {i}: {a:?} vs {b:?}"
        );
    }
}

/// The tank view reads the plate: eight segment lengths that follow
/// `size`, live segment energies, and no ER taps.
#[test]
fn the_viz_getters_describe_the_tank() {
    let mut d = plate().dsp();
    let small = d.fdn_delay_ms();
    // The paper's D1 of branch A, 4453 samples at 29.761 kHz, at 1×.
    assert!((small[1] - 149.6).abs() < 0.5, "D1A {:.2} ms", small[1]);
    for i in 0..48_000 {
        let x = if i == 0 { 1.0 } else { 0.0 };
        d.process(x, x, 0.8, 1.0);
    }
    assert!(
        d.channel_energies().iter().all(|&e| e > 0.0),
        "{:?}",
        d.channel_energies()
    );
    assert!(d
        .er_tap_times_ms()
        .iter()
        .all(|&(a, b)| a == 0.0 && b == 0.0));
    let mut big = plate().with(|s| s.size = 1.0).dsp();
    big.process(0.0, 0.0, 0.8, 1.0);
    assert!((big.fdn_delay_ms()[1] / small[1] - 1.5).abs() < 0.01);
}

// ---------------------------------------------------------------------
// Golden

fn scenarios() -> Vec<Scenario> {
    vec![
        // The Vocal Plate voicing on an impulse, L at 0 and R at 37: the
        // onset cascade, the input diffusers and the first 250 ms of the
        // tank, tap by tap.
        Scenario {
            name: "impulse_vocal_voicing",
            setup: Setup {
                size: 0.35,
                decay: 1.8,
                damping: 8_000.0,
                diffusion: 0.85,
                er_level: 0.45,
                er_time: 0.3,
                mod_rate: 1.2,
                mod_depth: 0.3,
                low_mult: 1.0,
                low_xover: 250.0,
                high_mult: 0.8,
                width: 1.0,
                ..defaults()
            },
            predelay_ms: 0.0,
            frames: 12_000,
            input: impulse_lr,
            edit: None,
        },
        // A 15 ms noise burst into a big, heavily modulated plate with a
        // shaped decay, a pre-delay and a narrowed width; at 150 ms the
        // size drops, so the glide's fractional reads are pinned too.
        Scenario {
            name: "burst_modulated_glide",
            setup: Setup {
                size: 0.8,
                decay: 4.0,
                damping: 6_000.0,
                diffusion: 0.95,
                er_level: 0.3,
                er_time: 0.7,
                mod_rate: 3.0,
                mod_depth: 1.0,
                low_mult: 1.4,
                low_xover: 400.0,
                high_mult: 0.6,
                width: 0.8,
                ..defaults()
            },
            predelay_ms: 10.0,
            frames: 14_400,
            input: burst,
            edit: Some((7_200, |d| d.set_size(0.5))),
        },
    ]
}

#[test]
fn plate_golden_is_bit_exact() {
    check_golden(
        "plate_golden.f32",
        &["RESONANCE_BLESS", "RESONANCE_BLESS_PLATE"],
        &scenarios(),
    );
}
