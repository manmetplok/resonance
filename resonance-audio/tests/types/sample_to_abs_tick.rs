//! Round-trip tests for `TempoMap::sample_to_abs_tick`. Verifies
//! the live-MIDI recorder uses the same tempo math as the timeline.

use resonance_audio::types::{
    SignaturePoint, TempoMap, TempoPoint, TICKS_PER_QUARTER_NOTE,
};

const SR: u32 = 48_000;

fn map_with_bpm(bpm: f32) -> TempoMap {
    let mut tm = TempoMap::default();
    tm.tempo_points = vec![TempoPoint { bar: 0, bpm }];
    tm.signature_points = vec![SignaturePoint {
        bar: 0,
        numerator: 4,
        denominator: 4,
    }];
    tm.bpm = bpm;
    tm.numerator = 4;
    tm.denominator = 4;
    tm.rebuild_bar_table(SR);
    tm
}

#[test]
fn round_trip_constant_tempo_at_quarter_note_boundaries() {
    let tm = map_with_bpm(120.0);
    // At 120 BPM and 48 kHz, one quarter note = 24_000 samples.
    for q in 0..16u64 {
        let sample = q * 24_000;
        let tick = tm.sample_to_abs_tick(sample, SR);
        let expected = q * TICKS_PER_QUARTER_NOTE;
        let diff = (tick as i64 - expected as i64).abs();
        assert!(
            diff <= 1,
            "quarter {q}: got {tick}, expected {expected} (diff {diff})"
        );
    }
}

#[test]
fn zero_sample_is_zero_tick() {
    let tm = map_with_bpm(140.0);
    assert_eq!(tm.sample_to_abs_tick(0, SR), 0);
}

#[test]
fn round_trip_via_tick_to_abs_sample() {
    // sample_to_abs_tick must be the (approximate) inverse of
    // tick_to_abs_sample, since the recorder uses one and timeline
    // playback uses the other — divergence would mean a recorded
    // note plays back at a different position than where the user
    // pressed the key.
    let tm = map_with_bpm(100.0);
    for ticks in [0u64, 120, 240, 480, 960, 1920, 4800] {
        let sample = tm.tick_to_abs_sample(0, ticks, SR);
        let back = tm.sample_to_abs_tick(sample, SR);
        let diff = (back as i64 - ticks as i64).abs();
        assert!(diff <= 2, "ticks={ticks} sample={sample} back={back}");
    }
}

#[test]
fn empty_bar_table_falls_back_to_constant_bpm() {
    // No tempo points, no rebuild_bar_table call — exercises the
    // fallback branch that uses the stable `bpm` field directly.
    let mut tm = TempoMap::default();
    tm.bpm = 120.0;
    tm.numerator = 4;
    tm.denominator = 4;
    // 1 second at 120 BPM = 2 quarter notes = 960 ticks.
    let tick = tm.sample_to_abs_tick(SR as u64, SR);
    assert!((tick as i64 - 960).abs() <= 1);
}
