//! `TempoMap::position_to_bars` — sub-sample rounding at bar boundaries
//! (ba todo #1230, doc #273).
//!
//! A clip placed exactly at bar 201 reported `{"bar":200,"beat":4.999965}`
//! over the control API: `bar_to_sample` truncates its past-the-horizon
//! extrapolation, so the downbeat lands a fraction under a sample short and
//! the inverse floored into the previous bar. This is the residual of todo
//! #1187, which fixed the *different* "unbounded beat past the table horizon"
//! case; the roll-up of surplus beats into whole bars is not retested here
//! beyond the round trip itself.

use resonance_audio::types::*;

const SR: u32 = 48000;

fn make_tempo_map(bpm: f32, numerator: u8, denominator: u8) -> TempoMap {
    let mut tm = TempoMap::default();
    tm.tempo_points = vec![TempoPoint { bar: 0, bpm }];
    tm.signature_points = vec![SignaturePoint {
        bar: 0,
        numerator,
        denominator,
    }];
    tm.bpm = bpm;
    tm.numerator = numerator;
    tm.denominator = denominator;
    tm.rebuild_bar_table(SR);
    tm
}

/// With a single event at bar 0 the table stops at bar 200 (last event + 200,
/// see `rebuild_bar_table`), so 0-based bars 0..199 are tabulated and
/// everything from 200 up is extrapolated. Both branches must round-trip.
const TABLE_END: u32 = 200;

fn assert_round_trips(tm: &TempoMap, label: &str) {
    // Sweep both sides of the table horizon, including the exact bar from the
    // bug report (0-based 200 == 1-based bar 201).
    let bars: Vec<u32> = (0..8)
        .chain([50, 99, 100, 198, 199, TABLE_END, 201, 202, 271, 500, 1000])
        .collect();
    for bar in bars {
        let sample = tm.bar_to_sample(bar);
        let (got_bar, got_beat, got_frac) = tm.position_to_bars(sample, SR);
        assert_eq!(
            (got_bar, got_beat, got_frac),
            (bar + 1, 1, 0.0),
            "{label}: bar_to_sample({bar}) = {sample} must read back as bar {} beat 1.0",
            bar + 1,
        );
    }
}

#[test]
fn round_trips_at_4_4_120bpm() {
    assert_round_trips(&make_tempo_map(120.0, 4, 4), "120 bpm 4/4");
}

/// 127.3 BPM at 48 kHz is 22,623.723… samples per beat — the residue that
/// produced the original report is worst at a non-integral samples-per-bar.
#[test]
fn round_trips_at_4_4_non_integer_bpm() {
    assert_round_trips(&make_tempo_map(127.3, 4, 4), "127.3 bpm 4/4");
}

#[test]
fn round_trips_at_odd_meters() {
    assert_round_trips(&make_tempo_map(127.3, 7, 8), "127.3 bpm 7/8");
    assert_round_trips(&make_tempo_map(93.7, 5, 4), "93.7 bpm 5/4");
    assert_round_trips(&make_tempo_map(140.0, 3, 4), "140 bpm 3/4");
}

/// The exact figures from the field report: bar 201 (1-based) must not read
/// as bar 200 beat 4.99996.
#[test]
fn bar_201_is_not_bar_200_beat_five() {
    let tm = make_tempo_map(120.0, 4, 4);
    let sample = tm.bar_to_sample(TABLE_END); // 0-based 200 == 1-based 201
    let (bar, beat, frac) = tm.position_to_bars(sample, SR);
    assert_eq!((bar, beat), (201, 1), "got bar {bar} beat {beat} frac {frac}");
    assert_eq!(frac, 0.0);
}

/// The epsilon is one sample wide, so a genuinely off-grid position must keep
/// its true fraction — both inside the table and past its horizon.
#[test]
fn mid_bar_positions_keep_their_fraction() {
    let tm = make_tempo_map(127.3, 4, 4);
    let spb = tm.samples_per_beat(SR);

    for bar in [4u32, 100, TABLE_END, 271] {
        let start = tm.bar_to_sample(bar);

        // Half a beat into the bar: beat 1, fraction 0.5.
        let (b, beat, frac) = tm.position_to_bars(start + (spb * 0.5).round() as u64, SR);
        assert_eq!((b, beat), (bar + 1, 1), "half a beat into bar {bar}");
        assert!(
            (frac - 0.5).abs() < 1e-3,
            "half a beat into bar {bar}: frac {frac} should be ~0.5",
        );

        // Two and a quarter beats in: beat 3, fraction 0.25.
        let (b, beat, frac) = tm.position_to_bars(start + (spb * 2.25).round() as u64, SR);
        assert_eq!((b, beat), (bar + 1, 3), "2.25 beats into bar {bar}");
        assert!(
            (frac - 0.25).abs() < 1e-3,
            "2.25 beats into bar {bar}: frac {frac} should be ~0.25",
        );
    }
}

/// Symmetry: the snap must not swallow a position that is genuinely *just
/// before* a downbeat. Half a beat short of the next bar still belongs to the
/// current bar's last beat.
#[test]
fn just_before_a_downbeat_stays_in_the_previous_bar() {
    let tm = make_tempo_map(127.3, 4, 4);
    let spb = tm.samples_per_beat(SR);

    for bar in [5u32, 100, TABLE_END, 271] {
        let next_start = tm.bar_to_sample(bar + 1);
        let pos = next_start - (spb * 0.5).round() as u64;
        let (b, beat, frac) = tm.position_to_bars(pos, SR);
        assert_eq!(
            (b, beat),
            (bar + 1, 4),
            "half a beat before bar {}: got bar {b} beat {beat} frac {frac}",
            bar + 2,
        );
        assert!(
            (frac - 0.5).abs() < 1e-3,
            "half a beat before bar {}: frac {frac} should be ~0.5",
            bar + 2,
        );
    }
}

/// Beats inside a bar round-trip too, not just downbeats.
#[test]
fn beat_boundaries_round_trip_inside_and_past_the_table() {
    let tm = make_tempo_map(127.3, 4, 4);
    let spb = tm.samples_per_beat(SR);

    for bar in [3u32, 150, TABLE_END, 300] {
        let start = tm.bar_to_sample(bar);
        for beat in 0..4u32 {
            let pos = start + (beat as f64 * spb).round() as u64;
            let (b, got_beat, frac) = tm.position_to_bars(pos, SR);
            assert_eq!(
                (b, got_beat),
                (bar + 1, beat as u8 + 1),
                "bar {} beat {}: got bar {b} beat {got_beat} frac {frac}",
                bar + 1,
                beat + 1,
            );
            assert_eq!(frac, 0.0, "bar {} beat {}", bar + 1, beat + 1);
        }
    }
}

/// A tempo change means the horizon moves out to `last event + 200` bars, so
/// the extrapolated branch starts later and runs at the *last* bar's tempo.
#[test]
fn round_trips_across_a_tempo_change() {
    let mut tm = TempoMap::default();
    tm.tempo_points = vec![
        TempoPoint {
            bar: 0,
            bpm: 120.0,
        },
        TempoPoint {
            bar: 64,
            bpm: 91.7,
        },
    ];
    tm.signature_points = vec![SignaturePoint {
        bar: 0,
        numerator: 4,
        denominator: 4,
    }];
    tm.bpm = 120.0;
    tm.rebuild_bar_table(SR);

    for bar in [0u32, 63, 64, 65, 128, 263, 264, 265, 400] {
        let sample = tm.bar_to_sample(bar);
        let (got_bar, got_beat, got_frac) = tm.position_to_bars(sample, SR);
        assert_eq!(
            (got_bar, got_beat, got_frac),
            (bar + 1, 1, 0.0),
            "tempo change: bar_to_sample({bar}) = {sample}",
        );
    }
}

/// The no-bar-table fallback (`rebuild_bar_table` never ran) derives bar/beat
/// from a flat BPM and truncates the same way, so it needs the same snap.
#[test]
fn round_trips_without_a_bar_table() {
    let mut tm = TempoMap::default();
    tm.bpm = 127.3;
    tm.numerator = 4;
    tm.denominator = 4;
    assert_eq!(tm.bar_count(), 0, "this test covers the no-table branch");

    // `bar_to_sample` reads `table_sample_rate`, which is 0 until the table is
    // built, so mirror its flat-BPM formula (a truncating `as u64`) directly.
    let spb = tm.samples_per_beat(SR);
    let bar_sample = |bar: u32| (bar as f64 * spb * 4.0) as u64;

    for bar in [0u32, 1, 7, 200, 201, 512] {
        let sample = bar_sample(bar);
        let (got_bar, got_beat, got_frac) = tm.position_to_bars(sample, SR);
        assert_eq!(
            (got_bar, got_beat, got_frac),
            (bar + 1, 1, 0.0),
            "no table: bar {bar} at sample {sample}",
        );
    }

    // Still not quantising a genuine mid-bar position.
    let (bar, beat, frac) = tm.position_to_bars(bar_sample(9) + (spb * 1.5) as u64, SR);
    assert_eq!((bar, beat), (10, 2));
    assert!((frac - 0.5).abs() < 1e-3, "frac {frac} should be ~0.5");
}
