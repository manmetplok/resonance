//! Dynamic EQ bands (warmth-width-depth.md §6.4): a band whose gain is
//! pulled down by the level of its own frequency region.
//!
//! Off by default — `tests/legacy_state.rs` and `dsp_golden.f32` prove the
//! default path is untouched — and measured here: the cut lands where the
//! threshold and ratio put it, only the band's region triggers it, a quiet
//! signal is left alone, and it releases. A pinned golden covers a de-harsh
//! and a backing-off boost, each with a non-silence guard.

use std::path::PathBuf;

use resonance_dsp_test_support as golden;
use resonance_eq::band::{BandKind, BandMs};
use resonance_eq::params::EqParams;
use resonance_eq::ResonanceEq;
use resonance_plugin::{EventIterator, KeyBuffer, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
const TAU: f32 = std::f32::consts::TAU;

fn render(setup: impl Fn(&EqParams), input: impl Fn(u64) -> f32, blocks: usize) -> Vec<f32> {
    let mut plugin = ResonanceEq::new();
    setup(&plugin.params);
    plugin.initialize(SR, BLOCK as u32);
    let mut out = Vec::new();
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    let mut n = 0u64;
    for _ in 0..blocks {
        for i in 0..BLOCK {
            l[i] = input(n + i as u64);
            r[i] = l[i];
        }
        let mut outs = [OutputBuffer {
            left: &mut l,
            right: &mut r,
        }];
        plugin.process(&mut outs, BLOCK, &mut EventIterator::empty(), None);
        n += BLOCK as u64;
        out.extend_from_slice(&l);
    }
    out
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

fn sine(freq: f32, amp: f32) -> impl Fn(u64) -> f32 {
    move |n| amp * (TAU * freq * n as f32 / SR).sin()
}

/// A 0 dB bell at 3 kHz with dynamics: a pure de-harsh cut.
fn deharsh(p: &EqParams) {
    let b = &p.bands[5];
    b.enabled.set_value(true);
    b.kind.set_value(BandKind::Bell.to_index());
    b.freq.set_value(3_000.0);
    b.gain.set_value(0.0);
    b.q.set_value(1.5);
    b.ms.set_value(BandMs::Stereo.to_index());
    b.dyn_on.set_value(true);
    b.dyn_threshold.set_value(-30.0);
    b.dyn_ratio.set_value(4.0);
    b.dyn_attack.set_value(2.0);
    b.dyn_release.set_value(80.0);
}

/// Steady-state level change of a tone through `setup`, dB.
fn level_change_db(setup: impl Fn(&EqParams), freq: f32, amp: f32) -> f32 {
    let out = render(setup, sine(freq, amp), 60);
    let tail = &out[out.len() / 2..];
    let input: Vec<f32> = (0..out.len() as u64).map(sine(freq, amp)).collect();
    db(rms(tail)) - db(rms(&input[input.len() / 2..]))
}

#[test]
fn a_loud_tone_in_the_band_is_cut_by_threshold_and_ratio() {
    // A -6 dBFS tone at the bell's centre reads -6 dB at the detector
    // (a 0 dB-peak band-pass), 24 dB over a -30 dB threshold: at 4:1 the
    // band cuts 24 × 0.75 = 18 dB, and at the centre a bell cut is the
    // whole cut.
    let amp: f32 = 0.5;
    let over = 20.0 * amp.log10() + 30.0;
    let want = -over * (1.0 - 1.0 / 4.0);
    let got = level_change_db(deharsh, 3_000.0, amp);
    assert!((got - want).abs() < 0.5, "cut {got:.2} dB, want {want:.2}");
}

#[test]
fn a_quiet_tone_and_a_tone_outside_the_band_pass_untouched() {
    let quiet = level_change_db(deharsh, 3_000.0, 0.005); // -46 dBFS
    assert!(quiet.abs() < 0.05, "a quiet tone was cut {quiet:.2} dB");
    // Loud, but two and a half octaves below the band: the band-pass
    // detector barely hears it, and the bell has no effect down there.
    let low = level_change_db(deharsh, 500.0, 0.5);
    assert!(low.abs() < 0.3, "a tone outside the band moved {low:.2} dB");
}

#[test]
fn the_cut_releases_when_the_region_quietens() {
    let burst_then_quiet = |n: u64| {
        let amp = if n < 24_000 { 0.5 } else { 0.005 };
        sine(3_000.0, amp)(n)
    };
    let out = render(deharsh, burst_then_quiet, 200);
    // Well after the burst (release 80 ms), the quiet tone is back at unity.
    let late = &out[out.len() - 9_600..];
    let input: Vec<f32> = (out.len() as u64 - 9_600..out.len() as u64)
        .map(sine(3_000.0, 0.005))
        .collect();
    let change = db(rms(late)) - db(rms(&input));
    assert!(change.abs() < 0.1, "still cutting {change:.2} dB after release");
}

#[test]
fn dynamics_are_ignored_on_cut_kinds_and_off_by_default() {
    let p = EqParams::default();
    assert!(p.bands.iter().all(|b| !b.dyn_on.value()));
    let cut = |p: &EqParams, dyn_on: bool| {
        let b = &p.bands[0];
        b.enabled.set_value(true);
        b.kind.set_value(BandKind::LowCut.to_index());
        b.freq.set_value(200.0);
        b.dyn_on.set_value(dyn_on);
        b.dyn_threshold.set_value(-60.0);
    };
    let a = render(|p| cut(p, false), sine(1_000.0, 0.5), 20);
    let b = render(|p| cut(p, true), sine(1_000.0, 0.5), 20);
    assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
}

/// Tilt and LF Lift+Dip have no single gain a dynamic cut could pull
/// down: lowering a tilt's gain *raises* its bottom end, and lowering a
/// lift's raises its dip — a "cut" that boosts the other side up to
/// +24 dB. Dynamics act only on the bell, the shelves and Air; on these
/// two the switch (and every dyn param) is ignored, bit for bit.
#[test]
fn dynamics_are_ignored_on_tilt_and_lf_lift_dip() {
    for kind in [BandKind::Tilt, BandKind::LfLiftDip] {
        assert!(!kind.supports_dyn(), "{kind:?} must not take dynamics");
        let setup = |p: &EqParams, dyn_on: bool| {
            let b = &p.bands[3];
            b.enabled.set_value(true);
            b.kind.set_value(kind.to_index());
            b.freq.set_value(if kind == BandKind::Tilt { 1_000.0 } else { 80.0 });
            b.gain.set_value(4.0);
            b.dyn_on.set_value(dyn_on);
            b.dyn_threshold.set_value(-60.0);
            b.dyn_ratio.set_value(10.0);
            b.dyn_attack.set_value(1.0);
        };
        let input = |n: u64| sine(60.0, 0.5)(n) + sine(4_000.0, 0.4)(n);
        let a = render(|p| setup(p, false), input, 20);
        let b = render(|p| setup(p, true), input, 20);
        assert!(
            a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
            "{kind:?} with dynamics on rendered differently"
        );
    }
    for kind in [BandKind::Bell, BandKind::LowShelf, BandKind::HighShelf, BandKind::Air] {
        assert!(kind.supports_dyn(), "{kind:?} must take dynamics");
    }
}

/// A named, fully pinned setup.
type Scenario = (&'static str, fn(&EqParams));

#[test]
fn dynamic_bands_golden() {
    let scenarios: [Scenario; 2] = [
        ("deharsh", deharsh),
        ("boost_backs_off", |p| {
            let b = &p.bands[2];
            b.enabled.set_value(true);
            b.kind.set_value(BandKind::LowShelf.to_index());
            b.freq.set_value(150.0);
            b.gain.set_value(6.0);
            b.q.set_value(0.707);
            b.ms.set_value(BandMs::Stereo.to_index());
            b.dyn_on.set_value(true);
            b.dyn_threshold.set_value(-20.0);
            b.dyn_ratio.set_value(3.0);
            b.dyn_attack.set_value(10.0);
            b.dyn_release.set_value(150.0);
        }),
    ];
    // Phrases that swell over the thresholds and fall back under them.
    let input = |n: u64| {
        let t = n as f32 / SR;
        let env = 0.5 + 0.45 * (TAU * 1.5 * t).sin();
        env * (0.4 * (TAU * 100.0 * t).sin() + 0.4 * (TAU * 3_000.0 * t).sin())
    };
    let mut all = Vec::new();
    for (name, setup) in scenarios {
        let out = render(setup, input, 48);
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-2, "scenario `{name}` rendered silence");
        let moved = out
            .iter()
            .enumerate()
            .fold(0.0f32, |m, (i, y)| m.max((y - input(i as u64)).abs()));
        assert!(moved > 1e-2, "scenario `{name}` is a no-op");
        all.extend(out);
    }
    let path: PathBuf = golden::golden_path(env!("CARGO_MANIFEST_DIR"), "dynamic_bands.f32");
    if golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_DYNAMIC_BANDS"]) {
        golden::bless_f32(&path, &all);
        return;
    }
    let want = golden::load_golden_f32(&path, all.len(), "RESONANCE_BLESS=1");
    let diff = golden::compare_f32(&all, &want);
    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "dynamic-band render changed: {}/{} samples differ (peak {:.3e}), \
             first at {i} (got {got}, want {want})",
            diff.diff_count,
            all.len(),
            diff.max_abs
        );
    }
}

/// Stereo input, stereo key (when given) through `process_with_key`;
/// returns the left and right outputs.
fn render_keyed(
    setup: impl Fn(&EqParams),
    input: impl Fn(u64) -> (f32, f32),
    key: Option<&dyn Fn(u64) -> (f32, f32)>,
    blocks: usize,
) -> (Vec<f32>, Vec<f32>) {
    let mut plugin = ResonanceEq::new();
    setup(&plugin.params);
    plugin.initialize(SR, BLOCK as u32);
    let (mut out_l, mut out_r) = (Vec::new(), Vec::new());
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    let mut kl = vec![0.0f32; BLOCK];
    let mut kr = vec![0.0f32; BLOCK];
    let mut n = 0u64;
    for _ in 0..blocks {
        for i in 0..BLOCK {
            (l[i], r[i]) = input(n + i as u64);
            if let Some(k) = key {
                (kl[i], kr[i]) = k(n + i as u64);
            }
        }
        let mut outs = [OutputBuffer {
            left: &mut l,
            right: &mut r,
        }];
        let keybuf = key.map(|_| KeyBuffer {
            left: &kl,
            right: &kr,
        });
        plugin.process_with_key(&mut outs, keybuf, BLOCK, &mut EventIterator::empty(), None);
        n += BLOCK as u64;
        out_l.extend_from_slice(&l);
        out_r.extend_from_slice(&r);
    }
    (out_l, out_r)
}

/// Steady-state level change of the left channel, dB, against `reference`.
fn tail_change_db(out: &[f32], reference: impl Fn(u64) -> f32) -> f32 {
    let half = out.len() / 2;
    let input: Vec<f32> = (half as u64..out.len() as u64).map(reference).collect();
    db(rms(&out[half..])) - db(rms(&input))
}

/// A Stereo band's detector hears the louder channel, not the mono sum:
/// a hard-panned tone is cut as deeply as the same tone on both sides,
/// and an antiphase one (whose mono sum is silence) is cut at all.
#[test]
fn a_stereo_band_detects_the_louder_channel_not_the_mono_sum() {
    let amp = 0.5;
    let s = sine(3_000.0, amp);
    let both = level_change_db(deharsh, 3_000.0, amp);
    assert!(both < -12.0, "the dual-mono reference cut only {both:.2} dB");

    let (panned, _) = render_keyed(deharsh, |n| (s(n), 0.0), None, 60);
    let panned = tail_change_db(&panned, &s);
    assert!(
        (panned - both).abs() < 0.5,
        "a hard-left tone was cut {panned:.2} dB, the dual-mono one {both:.2}"
    );

    let (anti, _) = render_keyed(deharsh, |n| (s(n), -s(n)), None, 60);
    let anti = tail_change_db(&anti, &s);
    assert!(
        (anti - both).abs() < 0.5,
        "an antiphase tone was cut {anti:.2} dB, the dual-mono one {both:.2}"
    );
}

/// `dyn_sc` detects on the sidechain key: a loud key in the band's
/// region cuts a quiet programme the band alone would leave alone; the
/// key goes through the band's detector filter, so a loud key outside
/// the region does nothing; with `dyn_sc` off a connected key is
/// ignored, and with `dyn_sc` on but no key the band keys off itself.
#[test]
fn dyn_sc_detects_on_the_sidechain_key() {
    let quiet = sine(3_000.0, 0.005); // -46 dBFS: under the threshold
    let keyed = |sc: bool| {
        move |p: &EqParams| {
            deharsh(p);
            p.bands[5].dyn_sc.set_value(sc);
        }
    };
    let program = |n: u64| (quiet(n), quiet(n));
    let loud_in_band = |n: u64| {
        let v = sine(3_000.0, 0.5)(n);
        (v, v)
    };
    let loud_outside = |n: u64| {
        let v = sine(200.0, 0.5)(n);
        (v, v)
    };

    let (cut, _) = render_keyed(keyed(true), program, Some(&loud_in_band), 60);
    let cut = tail_change_db(&cut, &quiet);
    assert!(cut < -12.0, "a loud key in the band cut only {cut:.2} dB");

    let (outside, _) = render_keyed(keyed(true), program, Some(&loud_outside), 60);
    let outside = tail_change_db(&outside, &quiet);
    assert!(outside.abs() < 0.3, "a key outside the band moved it {outside:.2} dB");

    let (ignored, _) = render_keyed(keyed(false), program, Some(&loud_in_band), 60);
    let ignored = tail_change_db(&ignored, &quiet);
    assert!(ignored.abs() < 0.05, "dyn_sc off, yet the key cut {ignored:.2} dB");

    let (no_key, _) = render_keyed(keyed(true), program, None, 60);
    let no_key = tail_change_db(&no_key, &quiet);
    assert!(no_key.abs() < 0.05, "no key connected, yet the band cut {no_key:.2} dB");

    assert_eq!(ResonanceEq::SIDECHAIN_INPUT, Some(2), "the EQ declares a key port");
    // ...which the host treats as secondary (code review ARCH2-03).
    assert!(resonance_plugin::first_party::SECONDARY_KEY_PLUGINS.contains(&ResonanceEq::CLAP_ID));
    assert!(
        EqParams::default().bands.iter().all(|b| !b.dyn_sc.value()),
        "dyn_sc is off by default"
    );
}

/// Render in `block`-frame blocks, calling `per_block` with the block
/// index before each one, so a test can move a parameter mid-render.
fn render_moving(
    setup: impl Fn(&EqParams),
    per_block: impl Fn(&EqParams, usize),
    input: impl Fn(u64) -> f32,
    block: usize,
    blocks: usize,
) -> Vec<f32> {
    let mut plugin = ResonanceEq::new();
    setup(&plugin.params);
    plugin.initialize(SR, block as u32);
    let mut out = Vec::new();
    let mut l = vec![0.0f32; block];
    let mut r = vec![0.0f32; block];
    let mut n = 0u64;
    for k in 0..blocks {
        per_block(&plugin.params, k);
        for i in 0..block {
            l[i] = input(n + i as u64);
            r[i] = l[i];
        }
        let mut outs = [OutputBuffer {
            left: &mut l,
            right: &mut r,
        }];
        plugin.process(&mut outs, block, &mut EventIterator::empty(), None);
        n += block as u64;
        out.extend_from_slice(&l);
    }
    out
}

/// A band parameter moving every block (automation) must not drop the
/// band's gain reduction. It used to: a changed snapshot re-voiced the
/// band at its static gain, and the GR only came back at the next
/// 16-sample dynamics update — so with blocks that are not a multiple of
/// 16, the first few samples of every block went out uncut, spikes of
/// the whole GR (+18 dB here).
#[test]
fn a_param_moving_every_block_keeps_the_gain_reduction() {
    const BLOCK_100: usize = 100;
    let amp = 0.5;
    let wiggle = |p: &EqParams, k: usize| {
        p.bands[5].gain.set_value(if k % 2 == 0 { 0.0 } else { 0.01 });
    };
    let moving = render_moving(deharsh, wiggle, sine(3_000.0, amp), BLOCK_100, 300);
    let still = render_moving(deharsh, |_, _| {}, sine(3_000.0, amp), BLOCK_100, 300);
    // After the attack has settled, compare peaks over the tail.
    let tail = |x: &[f32]| x[x.len() / 2..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let (got, want) = (tail(&moving), tail(&still));
    assert!(want > 1e-3, "the reference rendered silence");
    let over = db(got) - db(want);
    assert!(
        over < 0.5,
        "moving a param every block spiked the output {over:.2} dB over the steady cut"
    );
}
