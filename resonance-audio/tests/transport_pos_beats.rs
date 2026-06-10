//! `transport_pos_beats` feeds the CLAP transport event's
//! `song_pos_beats`. It must integrate the tempo map (bar table)
//! rather than assume a flat samples-per-beat factor, or tempo-synced
//! plugins drift under tempo ramps.

use resonance_audio::__test_support::transport_pos_beats;
use resonance_audio::types::{SignaturePoint, TempoMap, TempoPoint, TICKS_PER_QUARTER_NOTE};

const SR: u32 = 48_000;

fn map_with_points(points: Vec<TempoPoint>) -> TempoMap {
    let mut tm = TempoMap::default();
    tm.bpm = points[0].bpm;
    tm.tempo_points = points;
    tm.signature_points = vec![SignaturePoint {
        bar: 0,
        numerator: 4,
        denominator: 4,
    }];
    tm.numerator = 4;
    tm.denominator = 4;
    tm.rebuild_bar_table(SR);
    tm
}

#[test]
fn constant_tempo_matches_flat_formula() {
    let tm = map_with_points(vec![TempoPoint { bar: 0, bpm: 120.0 }]);
    // 120 BPM at 48 kHz = 24_000 samples per beat.
    for beat in 0..16u64 {
        let pos = transport_pos_beats(&tm, beat * 24_000, SR);
        assert!(
            (pos - beat as f64).abs() <= 2.0 / TICKS_PER_QUARTER_NOTE as f64,
            "beat {beat}: got {pos}"
        );
    }
}

#[test]
fn tempo_ramp_round_trips_through_bar_table() {
    // 120 BPM ramping to 60 BPM at bar 2. For any tick position the
    // beat count must round-trip through tick_to_abs_sample.
    let tm = map_with_points(vec![
        TempoPoint { bar: 0, bpm: 120.0 },
        TempoPoint { bar: 2, bpm: 60.0 },
    ]);
    for beats in [1u64, 4, 8, 12, 16] {
        let sample = tm.tick_to_abs_sample(0, beats * TICKS_PER_QUARTER_NOTE, SR);
        let pos = transport_pos_beats(&tm, sample, SR);
        assert!(
            (pos - beats as f64).abs() <= 3.0 / TICKS_PER_QUARTER_NOTE as f64,
            "beats {beats}: sample {sample} -> {pos}"
        );
    }
}

#[test]
fn tempo_ramp_diverges_from_flat_formula() {
    // Same ramp: past the slowdown, the flat 120 BPM factor overshoots
    // the true beat position — the bug this helper fixes.
    let tm = map_with_points(vec![
        TempoPoint { bar: 0, bpm: 120.0 },
        TempoPoint { bar: 2, bpm: 60.0 },
    ]);
    let beats = 12u64;
    let sample = tm.tick_to_abs_sample(0, beats * TICKS_PER_QUARTER_NOTE, SR);
    let flat = sample as f64 / SR as f64 * tm.bpm as f64 / 60.0;
    let mapped = transport_pos_beats(&tm, sample, SR);
    assert!((mapped - beats as f64).abs() < 0.01, "mapped={mapped}");
    assert!(
        (flat - beats as f64).abs() > 0.5,
        "flat formula unexpectedly accurate: flat={flat}"
    );
}
