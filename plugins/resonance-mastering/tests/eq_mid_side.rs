//! Per-band M/S on the linear-phase EQ stages (warmth-width-depth.md
//! §6.3). A side band must leave the mono sum alone, a mid band the side
//! channel; the latency must not move; and a stage whose bands all go
//! back to Stereo must end up bit-identical to one that never left it.

use resonance_mastering::stages::linear_phase_eq::{
    BandConfig, BandType, FirGeometry, LinearPhaseEq, MsMode, NUM_BANDS,
};

const SR: f32 = 48_000.0;
const BLOCK: usize = 512;

/// Deterministic pseudo-noise from the sample index.
fn noise(n: u64) -> f32 {
    let mut s = n.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1) as u32;
    s ^= s >> 16;
    s = s.wrapping_mul(2_246_822_519);
    s ^= s >> 13;
    (s >> 8) as f32 * (2.0 / (1 << 23) as f32) - 1.0
}

/// Partly correlated stereo noise: real mid and real side content.
fn input(len: usize) -> (Vec<f32>, Vec<f32>) {
    let l = (0..len as u64).map(|n| 0.3 * noise(n) + 0.2 * noise(n + 999_999)).collect();
    let r = (0..len as u64).map(|n| 0.3 * noise(n) - 0.2 * noise(n + 999_999)).collect();
    (l, r)
}

fn band(band_type: BandType, freq_hz: f32, gain_db: f32, ms: MsMode) -> BandConfig {
    BandConfig {
        enabled: true,
        band_type,
        freq_hz,
        q: 0.8,
        gain_db,
        ms,
    }
}

/// Stream `l`/`r` through `eq` in blocks, with `bands_at(block)` as the
/// band set for each block.
fn run(
    eq: &mut LinearPhaseEq,
    l: &[f32],
    r: &[f32],
    bands_at: impl Fn(usize) -> [BandConfig; NUM_BANDS],
) -> (Vec<f32>, Vec<f32>) {
    let (mut ol, mut or) = (l.to_vec(), r.to_vec());
    for (b, start) in (0..l.len()).step_by(BLOCK).enumerate() {
        let end = (start + BLOCK).min(l.len());
        let bands = bands_at(b);
        eq.process_stereo(&mut ol[start..end], &mut or[start..end], &bands);
    }
    (ol, or)
}

fn rms(x: impl Iterator<Item = f32>) -> f64 {
    let (mut s, mut n) = (0.0f64, 0usize);
    for v in x {
        s += (v as f64) * (v as f64);
        n += 1;
    }
    (s / n.max(1) as f64).sqrt()
}

/// Side-only low cut, air shelf and low-mid cut: the moves §6.3 names.
fn side_bands() -> [BandConfig; NUM_BANDS] {
    [
        band(BandType::HighPass, 200.0, 0.0, MsMode::Side),
        band(BandType::HighShelf, 9000.0, 4.0, MsMode::Side),
        band(BandType::Bell, 350.0, -6.0, MsMode::Side),
        BandConfig::off(),
    ]
}

#[test]
fn side_bands_leave_the_mono_sum_unchanged() {
    let mut eq = LinearPhaseEq::new(SR);
    let lat = eq.latency();
    let (l, r) = input(lat + 48_000);
    let (ol, or) = run(&mut eq, &l, &r, |_| side_bands());
    assert!(eq.cross_active(), "a side band must run the cross pair");

    // Past the warm-up, the design and its crossfade.
    let from = lat + 16_384;
    let mut worst = 0.0f32;
    for i in from..l.len() {
        let mono_in = 0.5 * (l[i - lat] + r[i - lat]);
        let mono_out = 0.5 * (ol[i] + or[i]);
        worst = worst.max((mono_out - mono_in).abs());
    }
    // The mid filter is an all-ones design, so only FIR/FFT rounding.
    assert!(worst < 2e-5, "mono sum moved by {worst:.3e}");

    // And the side really was filtered (not a vacuous pass).
    let side_in = rms((from..l.len()).map(|i| 0.5 * (l[i - lat] - r[i - lat])));
    let side_err = rms((from..l.len()).map(|i| 0.5 * (ol[i] - or[i]) - 0.5 * (l[i - lat] - r[i - lat])));
    assert!(side_err > 0.1 * side_in, "side barely changed: {side_err} vs {side_in}");
}

#[test]
fn mid_bands_leave_the_side_unchanged() {
    let mut eq = LinearPhaseEq::new(SR);
    let lat = eq.latency();
    let (l, r) = input(lat + 48_000);
    let mid = [
        band(BandType::LowShelf, 120.0, 3.0, MsMode::Mid),
        band(BandType::Bell, 2500.0, -4.0, MsMode::Mid),
        BandConfig::off(),
        BandConfig::off(),
    ];
    let (ol, or) = run(&mut eq, &l, &r, |_| mid);
    let from = lat + 16_384;
    let mut worst = 0.0f32;
    for i in from..l.len() {
        let side_in = 0.5 * (l[i - lat] - r[i - lat]);
        let side_out = 0.5 * (ol[i] - or[i]);
        worst = worst.max((side_out - side_in).abs());
    }
    assert!(worst < 2e-5, "side moved by {worst:.3e}");
}

/// A stereo band and a side band together: the stereo band still
/// reaches the mono sum, the side band does not.
#[test]
fn stereo_bands_still_reach_both() {
    let sr = SR;
    let mut ms_eq = LinearPhaseEq::new(sr);
    let mut ref_eq = LinearPhaseEq::new(sr);
    let lat = ms_eq.latency();
    let (l, r) = input(lat + 48_000);
    let stereo = band(BandType::Bell, 1000.0, -8.0, MsMode::Stereo);
    let mixed = [stereo, band(BandType::HighPass, 300.0, 0.0, MsMode::Side), BandConfig::off(), BandConfig::off()];
    let only = [stereo, BandConfig::off(), BandConfig::off(), BandConfig::off()];
    let (ml, mr) = run(&mut ms_eq, &l, &r, |_| mixed);
    let (rl, rr) = run(&mut ref_eq, &l, &r, |_| only);
    let from = lat + 16_384;
    let mut worst = 0.0f32;
    for i in from..l.len() {
        worst = worst.max((0.5 * (ml[i] + mr[i]) - 0.5 * (rl[i] + rr[i])).abs());
    }
    assert!(worst < 2e-5, "mono sum differs from the stereo-only EQ by {worst:.3e}");
}

#[test]
fn latency_is_the_same_with_mid_side_bands() {
    let plain = LinearPhaseEq::new(SR);
    let mut eq = LinearPhaseEq::new(SR);
    let lat = eq.latency();
    assert_eq!(lat, plain.latency());

    // An impulse in the left channel, with a side band engaged well
    // before it: the direct response still peaks at the latency.
    let n = 4 * lat + 32_768;
    let mut l = vec![0.0f32; n];
    let r = vec![0.0f32; n];
    let at = 3 * lat;
    l[at] = 1.0;
    let (ol, _) = run(&mut eq, &l, &r, |_| side_bands());
    assert_eq!(eq.latency(), lat);
    let peak = (0..n).max_by(|&a, &b| ol[a].abs().total_cmp(&ol[b].abs())).unwrap();
    assert_eq!(peak, at + lat, "response peak moved off the reported latency");
}

/// Engaging a side band and releasing it again leaves the stage exactly
/// where a stage that never left Stereo is: the cross pair stops, and
/// the output is bit-identical.
#[test]
fn releasing_the_last_mid_side_band_is_bit_exact() {
    let stereo = [
        band(BandType::HighShelf, 9000.0, 4.0, MsMode::Stereo),
        band(BandType::Bell, 350.0, -3.0, MsMode::Stereo),
        BandConfig::off(),
        BandConfig::off(),
    ];
    let mut side = stereo;
    side[0].ms = MsMode::Side;

    let mut toggled = LinearPhaseEq::new(SR);
    let mut steady = LinearPhaseEq::new(SR);
    let lat = toggled.latency();
    let (l, r) = input(lat + 96_000);
    let blocks = l.len().div_ceil(BLOCK);
    let (tl, tr) = run(&mut toggled, &l, &r, |b| if (10..60).contains(&b) { side } else { stereo });
    let (sl, sr) = run(&mut steady, &l, &r, |_| stereo);
    assert!(!toggled.cross_active(), "the cross pair must stop after release");

    // The side band really changed the output while it was engaged.
    let during = (30 * BLOCK)..(60 * BLOCK);
    let moved = during.clone().any(|i| tl[i] != sl[i]);
    assert!(moved, "the side band never reached the output");

    // After release, drain and one more hop of margin: identical bits.
    let from = (60 * BLOCK) + 4 * lat;
    assert!(from + 4096 < blocks * BLOCK, "render too short to check the tail");
    for i in from..l.len() {
        assert_eq!(tl[i].to_bits(), sl[i].to_bits(), "left differs at {i}");
        assert_eq!(tr[i].to_bits(), sr[i].to_bits(), "right differs at {i}");
    }
}

/// A disabled band's M/S setting does nothing, and does not start the
/// cross pair.
#[test]
fn a_disabled_side_band_is_inert() {
    let mut eq = LinearPhaseEq::new(SR);
    let mut reference = LinearPhaseEq::new(SR);
    let lat = eq.latency();
    let (l, r) = input(lat + 16_384);
    let mut off = band(BandType::HighPass, 300.0, 0.0, MsMode::Side);
    off.enabled = false;
    let bands = [off, BandConfig::off(), BandConfig::off(), BandConfig::off()];
    let mut stereo = bands;
    stereo[0].ms = MsMode::Stereo;
    let (a, b) = run(&mut eq, &l, &r, |_| bands);
    let (c, d) = run(&mut reference, &l, &r, |_| stereo);
    assert!(!eq.cross_active());
    assert!(a.iter().zip(&c).all(|(x, y)| x.to_bits() == y.to_bits()));
    assert!(b.iter().zip(&d).all(|(x, y)| x.to_bits() == y.to_bits()));
}

/// Engaging a side band mid-stream, at a block that is not on the hop
/// grid, must keep the mono guarantee through the engage and through
/// every later edit of the side band: the cross pair's crossfades land
/// on the same samples as the direct pair's, so `A + B` stays the mid
/// filter at every sample (review finding M1: the engage used to reset
/// the cross pair onto a grid of its own, so the two halves of each
/// band change landed up to a hop apart).
#[test]
fn mid_stream_side_engage_keeps_the_mono_sum() {
    // The standalone stage (unstaggered) and a chain-like staggered one.
    let hop = FirGeometry::for_sample_rate(SR).hop;
    for stagger in [None, Some([hop / 10, hop / 2 + hop / 10])] {
        let mut eq = LinearPhaseEq::new(SR);
        if let Some(offsets) = stagger {
            eq.set_phase_offsets(offsets);
        }
        let lat = eq.latency();
        let len = lat + 200_000;
        let x: Vec<f32> = (0..len as u64).map(|n| 0.4 * noise(n)).collect();
        let hpf = |hz: f32| {
            [
                band(BandType::HighPass, hz, 0.0, MsMode::Side),
                BandConfig::off(),
                BandConfig::off(),
                BandConfig::off(),
            ]
        };
        // Engage at block 3 (1536 samples: off the 4096 hop grid), then
        // move the side cut every 40 blocks.
        let (ol, or) = run(&mut eq, &x, &x, |b| match b {
            0..=2 => [BandConfig::off(); NUM_BANDS],
            3..=99 => hpf(200.0),
            100..=139 => hpf(450.0),
            140..=179 => hpf(120.0),
            _ => hpf(800.0),
        });
        assert!(eq.cross_active());
        assert_eq!(eq.cross_iteration_countdowns(), eq.iteration_countdowns());
        let mut worst = 0.0f32;
        let mut at = 0;
        for i in lat..len {
            let err = (0.5 * (ol[i] + or[i]) - x[i - lat]).abs();
            if err > worst {
                worst = err;
                at = i;
            }
        }
        assert!(
            worst < 2e-5,
            "stagger {stagger:?}: mono sum moved by {worst:.3e} at sample {at}"
        );
    }
}
