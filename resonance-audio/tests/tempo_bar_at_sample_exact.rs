//! `TempoMap::bar_at_sample_exact` — the tempo-map-aware inverse of
//! `bar_to_sample` used to re-associate a loaded clip with the bar it was
//! generated for (ba todo #1163). Replaces the old
//! `sample % samples_per_bar == 0` gate against a truncated scalar, which
//! dropped clips whenever the true samples-per-bar was non-integral and was
//! outright wrong under a tempo change.

use resonance_audio::types::*;

const SR: u32 = 48000;

fn make_tempo_map(tempo: &[(u32, f32)], sigs: &[(u32, u8, u8)]) -> TempoMap {
    let mut tm = TempoMap::default();
    tm.tempo_points = tempo
        .iter()
        .map(|&(bar, bpm)| TempoPoint { bar, bpm })
        .collect();
    tm.signature_points = sigs
        .iter()
        .map(|&(bar, num, den)| SignaturePoint {
            bar,
            numerator: num,
            denominator: den,
        })
        .collect();
    if let Some(first) = tm.tempo_points.first() {
        tm.bpm = first.bpm;
    }
    if let Some(first) = tm.signature_points.first() {
        tm.numerator = first.numerator;
        tm.denominator = first.denominator;
    }
    tm.rebuild_bar_table(SR);
    tm
}

/// 108 BPM at 48 kHz in 4/4 has a *non-integral* samples-per-bar:
/// 48000 * 60 / 108 * 4 = 106666.666… The old truncating scalar dropped
/// 0.67 samples per bar, so multiplying it by a bar index drifted early and
/// the divisibility gate never matched a genuine bar boundary.
///
/// `bar_to_sample(N)` must round-trip back through `bar_at_sample_exact`.
#[test]
fn non_integral_spb_round_trips_exactly() {
    let tm = make_tempo_map(&[(0, 108.0)], &[(0, 4, 4)]);

    for bar in [0u32, 1, 4, 8, 72, 73, 100, 112, 113] {
        let sample = tm.bar_to_sample(bar);
        assert_eq!(
            tm.bar_at_sample_exact(sample, 2),
            Some(bar),
            "bar {bar} at sample {sample} must recover to itself",
        );
    }
}

/// Regression on the exact figures from the bug report. The report counts
/// bars 1-based; this API is 0-based, so its "bar 73" is `bar_to_sample(72)`
/// = 7,680,000 (not the truncated-scalar 106666*72 = 7,679,952) and its
/// "bar 113" is `bar_to_sample(112)` = 11,946,667 (not 11,946,592). The
/// recovered bar must match the true placement, and the stale
/// truncated-scalar positions must be rejected — that rejection is the exact
/// case that once orphaned a lane when only placement was fixed.
#[test]
fn recovers_documented_drifting_bars() {
    let tm = make_tempo_map(&[(0, 108.0)], &[(0, 4, 4)]);

    // True tempo-map placement (0-based bars 72 / 112).
    assert_eq!(tm.bar_to_sample(72), 7_680_000);
    assert_eq!(tm.bar_to_sample(112), 11_946_667);

    // Recovery lands on the right bar for the true positions.
    assert_eq!(tm.bar_at_sample_exact(7_680_000, 2), Some(72));
    assert_eq!(tm.bar_at_sample_exact(11_946_667, 2), Some(112));

    // The stale truncated-scalar positions (early by 48 / 75 samples) are
    // *not* within tolerance of a bar boundary, so they are rejected.
    assert_eq!(106_666u64 * 72, 7_679_952);
    assert_eq!(tm.bar_at_sample_exact(7_679_952, 2), None);
    assert_eq!(106_666u64 * 112, 11_946_592);
    assert_eq!(tm.bar_at_sample_exact(11_946_592, 2), None);
}

/// A clip that does not sit on (or adjacent to) any bar boundary — e.g. a
/// hand-placed clip mid-bar — must return `None` rather than being claimed
/// as a derived clip.
#[test]
fn off_grid_position_is_rejected() {
    let tm = make_tempo_map(&[(0, 120.0)], &[(0, 4, 4)]);
    // Bar 4 start, plus a quarter-bar — clearly not a boundary.
    let bar4 = tm.bar_to_sample(4);
    let spb = SR as f64 * 60.0 / 120.0 * 4.0;
    let mid = bar4 + (spb / 4.0) as u64;
    assert_eq!(tm.bar_at_sample_exact(mid, 2), None);
}

/// The tolerance only absorbs sub-sample rounding — it must never be wide
/// enough to claim a neighbouring bar.
#[test]
fn tolerance_does_not_bleed_into_adjacent_bars() {
    let tm = make_tempo_map(&[(0, 120.0)], &[(0, 4, 4)]);
    let bar5 = tm.bar_to_sample(5);
    // A few samples off still resolves to bar 5.
    assert_eq!(tm.bar_at_sample_exact(bar5 + 2, 2), Some(5));
    assert_eq!(tm.bar_at_sample_exact(bar5.saturating_sub(2), 2), Some(5));
    // Far off (half a bar) resolves to neither 5 nor 6.
    let spb = (SR as f64 * 60.0 / 120.0 * 4.0) as u64;
    assert_eq!(tm.bar_at_sample_exact(bar5 + spb / 2, 2), None);
}

/// Under a tempo change the truncated single-scalar math placed clips at
/// positions unrelated to their bars. `bar_at_sample_exact` follows the bar
/// table, so a clip placed with `bar_to_sample(N)` still recovers to bar N
/// even across the tempo step.
#[test]
fn round_trips_across_a_tempo_change() {
    // 120 BPM for the first 4 bars, then 90 BPM.
    let tm = make_tempo_map(&[(0, 120.0), (4, 90.0)], &[(0, 4, 4)]);

    for bar in [0u32, 3, 4, 5, 10] {
        let sample = tm.bar_to_sample(bar);
        assert_eq!(
            tm.bar_at_sample_exact(sample, 2),
            Some(bar),
            "bar {bar} at sample {sample} must recover across the tempo change",
        );
    }
}
