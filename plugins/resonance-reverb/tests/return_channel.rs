//! The reverb's return-channel controls (warmth-width-depth.md §6.4):
//! the wet HPF/LPF before the tank, ducking from a sidechain key or the
//! dry input, and the ER/tail depth balance.
//!
//! Each feature gets a measured assertion, the defaults get a
//! bit-transparency check, and the whole set gets its own pinned golden
//! (`tests/golden/return_channel.f32`) with a non-silence guard per
//! scenario. The pre-existing `dsp_golden.f32` is untouched by all of
//! this; `tests/legacy_state.rs` proves old state still renders the same.

use std::path::PathBuf;

use resonance_dsp_test_support as golden;
use resonance_plugin::{EventIterator, KeyBuffer, OutputBuffer, ResonancePlugin};
use resonance_reverb::dsp::DUCK_MAX_GR_DB;
use resonance_reverb::params::ReverbParams;
use resonance_reverb::ResonanceReverb;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
const TAU: f32 = std::f32::consts::TAU;

fn sine(freq: f32, amp: f32) -> impl Fn(u64) -> f32 {
    move |n| amp * (TAU * freq * n as f32 / SR).sin()
}

/// A fully wet, unmodulated reverb: the output *is* the return, and the
/// modulation LFOs cannot make two renders differ for reasons that have
/// nothing to do with the control under test.
fn wet_only(p: &ReverbParams) {
    p.mix.set_value(1.0);
    p.mod_depth.set_value(0.0);
    p.size.set_value(0.4);
    p.decay.set_value(1.5);
}

/// Render `blocks` blocks of `input` (mono, both sides) through a fresh
/// plugin, with an optional key. Returns interleaved-by-block L/R.
fn render(
    setup: impl Fn(&ReverbParams),
    input: impl Fn(u64) -> f32,
    key: Option<&dyn Fn(u64) -> f32>,
    blocks: usize,
) -> (Vec<f32>, Vec<f32>) {
    let mut plugin = ResonanceReverb::new();
    setup(&plugin.params);
    plugin.initialize(SR, BLOCK as u32);
    let mut out_l = Vec::new();
    let mut out_r = Vec::new();
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    let mut k = vec![0.0f32; BLOCK];
    let mut n = 0u64;
    for _ in 0..blocks {
        for i in 0..BLOCK {
            let x = input(n + i as u64);
            l[i] = x;
            r[i] = x;
            if let Some(kf) = key {
                k[i] = kf(n + i as u64);
            }
        }
        let mut outs = [OutputBuffer {
            left: &mut l,
            right: &mut r,
        }];
        let mut ev = EventIterator::empty();
        match key {
            Some(_) => {
                let kb = KeyBuffer {
                    left: &k,
                    right: &k,
                };
                plugin.process_with_key(&mut outs, Some(kb), BLOCK, &mut ev, None);
            }
            None => plugin.process(&mut outs, BLOCK, &mut ev, None),
        }
        n += BLOCK as u64;
        out_l.extend_from_slice(&l);
        out_r.extend_from_slice(&r);
    }
    (out_l, out_r)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / x.len().max(1) as f64).sqrt()
        as f32
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

/// Steady-state wet level of a sine through the reverb, dB.
fn wet_level_db(setup: impl Fn(&ReverbParams), freq: f32) -> f32 {
    let (l, _) = render(setup, sine(freq, 0.5), None, 120);
    db(rms(&l[l.len() / 2..]))
}

// ---------------------------------------------------------------------------
// Return EQ
// ---------------------------------------------------------------------------

#[test]
fn wet_hpf_cuts_below_its_corner_and_passes_above_it() {
    let hpf = |steep: bool| {
        move |p: &ReverbParams| {
            wet_only(p);
            p.wet_hpf_on.set_value(true);
            p.wet_hpf_freq.set_value(600.0);
            p.wet_filter_slope.set_value(steep as i32);
        }
    };
    for freq in [100.0f32, 4_000.0] {
        let open = wet_level_db(wet_only, freq);
        let cut12 = wet_level_db(hpf(false), freq) - open;
        let cut18 = wet_level_db(hpf(true), freq) - open;
        if freq < 600.0 {
            // 600/100 Hz is ~2.6 octaves: ~31 dB at 12 dB/oct, ~46 at 18.
            assert!(cut12 < -25.0, "12 dB/oct HPF only cut {cut12:.1} dB at {freq} Hz");
            assert!(cut18 < cut12 - 10.0, "18 dB/oct ({cut18:.1}) not steeper than 12 ({cut12:.1})");
        } else {
            assert!(cut12.abs() < 1.0, "HPF moved the passband by {cut12:.2} dB at {freq} Hz");
            assert!(cut18.abs() < 1.0, "18 dB/oct HPF moved the passband by {cut18:.2} dB");
        }
    }
}

#[test]
fn wet_lpf_cuts_above_its_corner_and_passes_below_it() {
    let lpf = |steep: bool| {
        move |p: &ReverbParams| {
            wet_only(p);
            p.damping.set_value(20_000.0);
            p.wet_lpf_on.set_value(true);
            p.wet_lpf_freq.set_value(2_000.0);
            p.wet_filter_slope.set_value(steep as i32);
        }
    };
    let bright = |p: &ReverbParams| {
        wet_only(p);
        p.damping.set_value(20_000.0);
    };
    for freq in [300.0f32, 12_000.0] {
        let open = wet_level_db(bright, freq);
        let cut12 = wet_level_db(lpf(false), freq) - open;
        let cut18 = wet_level_db(lpf(true), freq) - open;
        if freq > 2_000.0 {
            // 2.6 octaves above the corner.
            assert!(cut12 < -25.0, "12 dB/oct LPF only cut {cut12:.1} dB at {freq} Hz");
            assert!(cut18 < cut12 - 10.0, "18 dB/oct ({cut18:.1}) not steeper than 12 ({cut12:.1})");
        } else {
            assert!(cut12.abs() < 1.0, "LPF moved the passband by {cut12:.2} dB at {freq} Hz");
            assert!(cut18.abs() < 1.0, "18 dB/oct LPF moved the passband by {cut18:.2} dB");
        }
    }
}

// ---------------------------------------------------------------------------
// Ducking
// ---------------------------------------------------------------------------

/// With the key held far over the threshold, the wet settles exactly
/// `amount × DUCK_MAX_GR_DB` down. The reverb state is identical in both
/// renders (ducking acts after the tank), so the level ratio of the two
/// is the ducker's gain alone and can be pinned to a tenth of a dB.
#[test]
fn a_loud_key_ducks_the_wet_return_by_the_amount_in_db() {
    let loud_key = sine(220.0, 1.0);
    let silent_key = |_n: u64| 0.0f32;
    for amount in [0.25f32, 0.5] {
        let setup = move |p: &ReverbParams| {
            wet_only(p);
            p.duck_amount.set_value(amount);
            p.duck_threshold.set_value(-30.0);
            p.duck_attack.set_value(5.0);
            p.duck_release.set_value(100.0);
        };
        let input = sine(700.0, 0.01); // far below the threshold
        let (open, _) = render(setup, &input, Some(&silent_key), 80);
        let (ducked, _) = render(setup, &input, Some(&loud_key), 80);
        let half = open.len() / 2;
        let got = db(rms(&ducked[half..])) - db(rms(&open[half..]));
        let want = -amount * DUCK_MAX_GR_DB;
        assert!(
            (got - want).abs() < 0.1,
            "amount {amount}: ducked by {got:.2} dB, want {want:.2}"
        );
    }
}

#[test]
fn with_no_key_the_return_self_ducks_from_the_dry_input() {
    let setup = |amount: f32| {
        move |p: &ReverbParams| {
            wet_only(p);
            p.mix.set_value(0.5);
            p.duck_amount.set_value(amount);
            p.duck_threshold.set_value(-30.0);
        }
    };
    // A loud, continuous dry signal: the self-keyed ducker holds the wet
    // down for the whole render.
    let input = sine(440.0, 0.5);
    let (plain, _) = render(setup(0.0), &input, None, 60);
    let (ducked, _) = render(setup(0.5), &input, None, 60);
    // Compare the wet parts: subtract the (identical) dry half.
    let wet = |out: &[f32]| -> Vec<f32> {
        out.iter()
            .enumerate()
            .map(|(i, y)| y - 0.5 * input(i as u64))
            .collect()
    };
    let half = plain.len() / 2;
    let got = db(rms(&wet(&ducked)[half..])) - db(rms(&wet(&plain)[half..]));
    assert!(
        (got + 0.5 * DUCK_MAX_GR_DB).abs() < 0.2,
        "self-ducked by {got:.2} dB, want {:.2}",
        -0.5 * DUCK_MAX_GR_DB
    );
}

#[test]
fn the_key_never_reaches_the_output() {
    let setup = |p: &ReverbParams| {
        wet_only(p);
        p.duck_amount.set_value(1.0);
    };
    let (l, r) = render(setup, |_| 0.0, Some(&sine(100.0, 1.0)), 20);
    let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
    assert_eq!(peak, 0.0, "a key into a silent reverb produced output");
}

// ---------------------------------------------------------------------------
// ER / tail balance
// ---------------------------------------------------------------------------

#[test]
fn balance_toward_the_tail_removes_the_early_reflections() {
    let impulse = |n: u64| if n == 0 { 1.0 } else { 0.0 };
    let tail_only = |p: &ReverbParams| {
        wet_only(p);
        p.er_level.set_value(0.8);
        p.er_tail_balance.set_value(1.0);
    };
    let no_er = |p: &ReverbParams| {
        wet_only(p);
        p.er_level.set_value(0.0);
    };
    let (a, _) = render(tail_only, impulse, None, 12);
    let (b, _) = render(no_er, impulse, None, 12);
    let worst = a.iter().zip(&b).fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
    assert!(worst < 1e-6, "balance +1 still carries ER: max diff {worst:.3e}");
    assert!(rms(&a) > 1e-4, "the tail itself vanished");
}

#[test]
fn balance_toward_the_room_removes_the_tail() {
    let impulse = |n: u64| if n == 0 { 1.0 } else { 0.0 };
    // No ER and no tail: nothing left of the wet at all.
    let setup = |p: &ReverbParams| {
        wet_only(p);
        p.er_level.set_value(0.0);
        p.er_tail_balance.set_value(-1.0);
    };
    let (l, r) = render(setup, impulse, None, 12);
    let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak < 1e-9, "balance -1 left a tail: {peak:.3e}");

    // With ER on, balance -1 is ER only: the render stops once the last
    // tap (at most ~440 ms) has passed, where the tail would still ring.
    let er_only = |p: &ReverbParams| {
        wet_only(p);
        p.er_level.set_value(1.0);
        p.er_time.set_value(0.0);
        p.decay.set_value(8.0);
        p.er_tail_balance.set_value(-1.0);
    };
    let (l, _) = render(er_only, impulse, None, 60);
    assert!(rms(&l[..BLOCK * 20]) > 1e-4, "ER vanished");
    assert!(rms(&l[BLOCK * 40..]) < 1e-9, "tail still audible at balance -1");
}

// ---------------------------------------------------------------------------
// Defaults are transparent
// ---------------------------------------------------------------------------

/// A key connected to a reverb that is not ducking changes nothing: the
/// render through `process_with_key` with any key equals the unkeyed one.
#[test]
fn a_connected_key_is_inert_while_duck_amount_is_zero() {
    let pluck = |n: u64| {
        let t = n as f32 / SR;
        0.6 * (-200.0 * (t % 0.1)).exp() * (TAU * 330.0 * t).sin()
    };
    let (a_l, a_r) = render(|_| {}, pluck, None, 40);
    let (b_l, b_r) = render(|_| {}, pluck, Some(&sine(90.0, 1.0)), 40);
    assert!(
        a_l.iter().zip(&b_l).all(|(x, y)| x.to_bits() == y.to_bits())
            && a_r.iter().zip(&b_r).all(|(x, y)| x.to_bits() == y.to_bits()),
        "a key changed a reverb whose duck amount is 0"
    );
}

// ---------------------------------------------------------------------------
// Golden
// ---------------------------------------------------------------------------

struct Scenario {
    name: &'static str,
    setup: fn(&ReverbParams),
    keyed: bool,
}

fn golden_input(n: u64) -> f32 {
    let t = n as f32 / SR;
    if n == 0 {
        1.0
    } else {
        0.5 * (-150.0 * (t % 0.125)).exp()
            * ((TAU * 180.0 * t).sin() + 0.4 * (TAU * 5_400.0 * t).sin())
    }
}

fn golden_key(n: u64) -> f32 {
    // A phrase on/off every ~85 ms, so attack and release both act.
    if (n / 4_096).is_multiple_of(2) {
        0.8 * (TAU * 150.0 * n as f32 / SR).sin()
    } else {
        0.0
    }
}

/// Every scenario pins every parameter it depends on, from defaults up.
fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "return_eq_12db",
            setup: |p| {
                wet_only(p);
                p.wet_hpf_on.set_value(true);
                p.wet_hpf_freq.set_value(600.0);
                p.wet_lpf_on.set_value(true);
                p.wet_lpf_freq.set_value(10_000.0);
                p.wet_filter_slope.set_value(0);
            },
            keyed: false,
        },
        Scenario {
            name: "return_eq_18db",
            setup: |p| {
                wet_only(p);
                p.wet_hpf_on.set_value(true);
                p.wet_hpf_freq.set_value(300.0);
                p.wet_lpf_on.set_value(true);
                p.wet_lpf_freq.set_value(4_000.0);
                p.wet_filter_slope.set_value(1);
            },
            keyed: false,
        },
        Scenario {
            name: "ducked_by_key",
            setup: |p| {
                wet_only(p);
                p.mix.set_value(0.4);
                p.duck_amount.set_value(0.5);
                p.duck_threshold.set_value(-24.0);
                p.duck_attack.set_value(15.0);
                p.duck_release.set_value(150.0);
            },
            keyed: true,
        },
        Scenario {
            name: "self_ducked",
            setup: |p| {
                wet_only(p);
                p.mix.set_value(0.4);
                p.duck_amount.set_value(0.4);
                p.duck_threshold.set_value(-18.0);
                p.duck_attack.set_value(10.0);
                p.duck_release.set_value(250.0);
            },
            keyed: false,
        },
        Scenario {
            name: "balance_close",
            setup: |p| {
                wet_only(p);
                p.er_level.set_value(0.7);
                p.er_tail_balance.set_value(-0.6);
            },
            keyed: false,
        },
        Scenario {
            name: "balance_far",
            setup: |p| {
                wet_only(p);
                p.er_level.set_value(0.7);
                p.er_tail_balance.set_value(0.6);
            },
            keyed: false,
        },
    ]
}

#[test]
fn return_channel_golden() {
    let mut all = Vec::new();
    for s in scenarios() {
        let key: Option<&dyn Fn(u64) -> f32> = if s.keyed { Some(&golden_key) } else { None };
        let (l, r) = render(s.setup, golden_input, key, 24);
        let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-3, "scenario `{}` rendered silence", s.name);
        // The last quarter is well past the impulse: a real tail, not
        // only the pluck train's dry-less echo.
        assert!(rms(&l[l.len() * 3 / 4..]) > 1e-4, "scenario `{}` has no tail", s.name);
        assert!(l.iter().chain(&r).all(|x| x.is_finite()), "`{}` non-finite", s.name);
        all.extend(l);
        all.extend(r);
    }
    let path: PathBuf = golden::golden_path(env!("CARGO_MANIFEST_DIR"), "return_channel.f32");
    if golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_RETURN_CHANNEL"]) {
        golden::bless_f32(&path, &all);
        return;
    }
    let want = golden::load_golden_f32(&path, all.len(), "RESONANCE_BLESS=1");
    let diff = golden::compare_f32(&all, &want);
    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "return-channel render changed: {}/{} samples differ (peak {:.3e}), \
             first at {i} (got {got}, want {want})",
            diff.diff_count,
            all.len(),
            diff.max_abs
        );
    }
}
