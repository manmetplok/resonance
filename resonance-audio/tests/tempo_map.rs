use resonance_audio::types::*;

const SR: u32 = 48000;

// ---- Helper to build a TempoMap with events and a rebuilt bar table ----

fn make_tempo_map(
    tempo: &[(u32, f32)],
    sigs: &[(u32, u8, u8)],
) -> TempoMap {
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

// ========================================================================
// bpm_at_bar — free function
// ========================================================================

#[test]
fn bpm_at_bar_empty_defaults_to_120() {
    assert_eq!(bpm_at_bar(0.0, &[]), 120.0);
    assert_eq!(bpm_at_bar(5.0, &[]), 120.0);
}

#[test]
fn bpm_at_bar_single_event_constant() {
    let pts = [TempoPoint { bar: 0, bpm: 140.0 }];
    assert_eq!(bpm_at_bar(0.0, &pts), 140.0);
    assert_eq!(bpm_at_bar(100.0, &pts), 140.0);
}

#[test]
fn bpm_at_bar_linear_interpolation() {
    let pts = [
        TempoPoint { bar: 0, bpm: 120.0 },
        TempoPoint { bar: 4, bpm: 160.0 },
    ];
    assert!((bpm_at_bar(0.0, &pts) - 120.0).abs() < 1e-9);
    assert!((bpm_at_bar(2.0, &pts) - 140.0).abs() < 1e-9);
    assert!((bpm_at_bar(4.0, &pts) - 160.0).abs() < 1e-9);
    // Past last event → holds
    assert!((bpm_at_bar(8.0, &pts) - 160.0).abs() < 1e-9);
}

#[test]
fn bpm_at_bar_step_change_last_wins() {
    // Two events at bar 2: 100 then 150. Departure = 150.
    let pts = [
        TempoPoint { bar: 0, bpm: 120.0 },
        TempoPoint { bar: 2, bpm: 100.0 },
        TempoPoint { bar: 2, bpm: 150.0 },
    ];
    assert!((bpm_at_bar(2.0, &pts) - 150.0).abs() < 1e-9);
}

// ========================================================================
// arrival_bpm_at_bar
// ========================================================================

#[test]
fn arrival_bpm_returns_first_event_at_bar() {
    let pts = [
        TempoPoint { bar: 0, bpm: 120.0 },
        TempoPoint { bar: 2, bpm: 100.0 },
        TempoPoint { bar: 2, bpm: 150.0 },
    ];
    // Arrival at bar 2 = first event at bar 2 = 100
    assert!((arrival_bpm_at_bar(2, &pts) - 100.0).abs() < 1e-9);
    // Departure at bar 2 = last event = 150
    assert!((bpm_at_bar(2.0, &pts) - 150.0).abs() < 1e-9);
}

#[test]
fn arrival_bpm_no_event_at_bar_interpolates() {
    let pts = [
        TempoPoint { bar: 0, bpm: 120.0 },
        TempoPoint { bar: 4, bpm: 160.0 },
    ];
    // No event at bar 2 → arrival is the interpolated value
    assert!((arrival_bpm_at_bar(2, &pts) - 140.0).abs() < 1e-9);
}

#[test]
fn arrival_equals_departure_without_step_change() {
    let pts = [
        TempoPoint { bar: 0, bpm: 120.0 },
        TempoPoint { bar: 2, bpm: 100.0 },
    ];
    assert_eq!(arrival_bpm_at_bar(2, &pts), bpm_at_bar(2.0, &pts));
}

// ========================================================================
// avg_bpm_for_bar
// ========================================================================

#[test]
fn avg_bpm_constant_tempo() {
    let pts = [TempoPoint { bar: 0, bpm: 120.0 }];
    assert!((avg_bpm_for_bar(0, &pts) - 120.0).abs() < 1e-9);
    assert!((avg_bpm_for_bar(5, &pts) - 120.0).abs() < 1e-9);
}

#[test]
fn avg_bpm_linear_ramp() {
    let pts = [
        TempoPoint { bar: 0, bpm: 100.0 },
        TempoPoint { bar: 2, bpm: 200.0 },
    ];
    // Bar 0: depart 100, arrive at bar 1 = 150 → avg = 125
    assert!((avg_bpm_for_bar(0, &pts) - 125.0).abs() < 1e-9);
    // Bar 1: depart 150, arrive at bar 2 = 200 → avg = 175
    assert!((avg_bpm_for_bar(1, &pts) - 175.0).abs() < 1e-9);
}

#[test]
fn avg_bpm_with_step_change() {
    // 150 at bar 0, ramp to 109 at bar 2, step back to 150 at bar 2.
    let pts = [
        TempoPoint { bar: 0, bpm: 150.0 },
        TempoPoint { bar: 2, bpm: 109.0 },
        TempoPoint { bar: 2, bpm: 150.0 },
    ];
    // Bar 1: depart = bpm_at_bar(1) = 150+(109-150)*0.5 = 129.5
    //         arrive at bar 2 = 109
    //         avg = (129.5 + 109) / 2 = 119.25
    assert!((avg_bpm_for_bar(1, &pts) - 119.25).abs() < 1e-9);
    // Bar 2: depart = 150 (step change), arrive at bar 3 = 150 (constant)
    //         avg = 150
    assert!((avg_bpm_for_bar(2, &pts) - 150.0).abs() < 1e-9);
}

// ========================================================================
// tick_frac_to_sample_frac / sample_frac_to_tick_frac
// ========================================================================

#[test]
fn frac_conversion_identity_at_constant_bpm() {
    for f in [0.0, 0.25, 0.5, 0.75, 1.0] {
        assert!((tick_frac_to_sample_frac(f, 120.0, 120.0) - f).abs() < 1e-12);
        assert!((sample_frac_to_tick_frac(f, 120.0, 120.0) - f).abs() < 1e-12);
    }
}

#[test]
fn frac_conversion_boundaries() {
    // At boundaries 0 and 1 the result should be 0 and 1 regardless of BPM ratio.
    for (s, e) in [(100.0, 200.0), (200.0, 100.0), (80.0, 160.0)] {
        assert!((tick_frac_to_sample_frac(0.0, s, e) - 0.0).abs() < 1e-12);
        assert!((tick_frac_to_sample_frac(1.0, s, e) - 1.0).abs() < 1e-12);
        assert!((sample_frac_to_tick_frac(0.0, s, e) - 0.0).abs() < 1e-12);
        assert!((sample_frac_to_tick_frac(1.0, s, e) - 1.0).abs() < 1e-12);
    }
}

#[test]
fn frac_conversion_round_trip() {
    // tick → sample → tick should be identity
    for f in [0.1, 0.3, 0.5, 0.7, 0.9] {
        let sf = tick_frac_to_sample_frac(f, 120.0, 80.0);
        let back = sample_frac_to_tick_frac(sf, 120.0, 80.0);
        assert!(
            (back - f).abs() < 1e-10,
            "round trip failed: {f} → {sf} → {back}"
        );
    }
}

#[test]
fn frac_slower_tempo_means_more_samples_early() {
    // When BPM drops (slower), early ticks take fewer samples (BPM still
    // high at the start), so the sample fraction should be LESS than
    // the tick fraction at the midpoint.
    let sf = tick_frac_to_sample_frac(0.5, 150.0, 100.0);
    assert!(sf < 0.5, "expected sf < 0.5 with decelerating BPM, got {sf}");
}

// ========================================================================
// TempoMap — rebuild_bar_table
// ========================================================================

#[test]
fn bar_table_built_for_single_tempo() {
    let tm = make_tempo_map(&[(0, 120.0)], &[(0, 4, 4)]);
    assert!(tm.bar_count() > 0, "bar table should always be built when sample_rate > 0");
}

#[test]
fn bar_table_populated_for_two_tempos() {
    let tm = make_tempo_map(&[(0, 120.0), (4, 160.0)], &[(0, 4, 4)]);
    assert!(tm.bar_count() > 4);
}

// ========================================================================
// TempoMap::bpm_at — sample-position lookup
// ========================================================================

#[test]
fn bpm_at_no_bar_table_returns_field() {
    let tm = make_tempo_map(&[(0, 140.0)], &[(0, 4, 4)]);
    assert_eq!(tm.bpm_at(0, SR), 140.0);
    assert_eq!(tm.bpm_at(1_000_000, SR), 140.0);
}

#[test]
fn bpm_at_with_ramp() {
    let tm = make_tempo_map(
        &[(0, 120.0), (4, 120.0)],
        &[(0, 4, 4)],
    );
    // Constant 120 → should stay ~120 everywhere
    let bpm = tm.bpm_at(0, SR);
    assert!((bpm - 120.0).abs() < 0.5, "expected ~120, got {bpm}");
}

#[test]
fn bpm_at_bar_start_matches_departure() {
    // 150 → 120 over 4 bars
    let tm = make_tempo_map(
        &[(0, 150.0), (4, 120.0)],
        &[(0, 4, 4)],
    );
    // At sample 0 (bar 0), BPM should be departure = 150
    let bpm = tm.bpm_at(0, SR);
    assert!((bpm - 150.0).abs() < 0.1, "bar 0 start: expected ~150, got {bpm}");
}

// ========================================================================
// TempoMap::position_to_bars
// ========================================================================

#[test]
fn position_to_bars_constant_tempo() {
    let tm = make_tempo_map(&[(0, 120.0)], &[(0, 4, 4)]);
    let (bar, beat, _) = tm.position_to_bars(0, SR);
    assert_eq!((bar, beat), (1, 1));

    // One beat at 120 BPM = 24000 samples
    let (bar, beat, _) = tm.position_to_bars(24000, SR);
    assert_eq!((bar, beat), (1, 2));

    // One bar = 4 beats = 96000 samples
    let (bar, beat, _) = tm.position_to_bars(96000, SR);
    assert_eq!((bar, beat), (2, 1));
}

#[test]
fn position_to_bars_with_tempo_change() {
    // 150 BPM for bar 0, 120 BPM from bar 1 onwards
    let tm = make_tempo_map(
        &[(0, 150.0), (1, 120.0)],
        &[(0, 4, 4)],
    );
    // Bar 0 at 150 BPM: samples_per_beat = 48000*60/150 = 19200
    // But with ramp to 120, avg_bpm_for_bar(0) = (150+120)/2 = 135
    // bar 0 samples ≈ 4 * 48000*60/135 ≈ 85333
    // At sample 0 → bar 1 beat 1
    let (bar, beat, _) = tm.position_to_bars(0, SR);
    assert_eq!((bar, beat), (1, 1));
}

// ========================================================================
// TempoMap::tick_to_abs_sample
// ========================================================================

#[test]
fn tick_to_abs_sample_zero_offset_returns_clip_start() {
    let tm = make_tempo_map(&[(0, 120.0), (4, 160.0)], &[(0, 4, 4)]);
    assert_eq!(tm.tick_to_abs_sample(12345, 0, SR), 12345);
}

#[test]
fn tick_to_abs_sample_constant_tempo() {
    // With constant BPM the tick→sample is linear.
    let tm = make_tempo_map(&[(0, 120.0)], &[(0, 4, 4)]);
    // spt = (48000 * 60 / 120) / 480 = 24000 / 480 = 50
    let result = tm.tick_to_abs_sample(0, 480, SR);
    // 480 ticks = 1 beat = 24000 samples
    assert_eq!(result, 24000);
}

#[test]
fn tick_to_abs_sample_one_bar_constant() {
    let tm = make_tempo_map(&[(0, 120.0)], &[(0, 4, 4)]);
    // 4 beats = 1920 ticks = 96000 samples at 120 BPM
    let result = tm.tick_to_abs_sample(0, 1920, SR);
    assert_eq!(result, 96000);
}

#[test]
fn tick_to_abs_sample_with_tempo_ramp() {
    // Bar 0: 120 BPM, Bar 2: 180 BPM. A clip at bar 0 with ticks
    // spanning into bar 1 should account for the ramp.
    let tm = make_tempo_map(
        &[(0, 120.0), (2, 180.0)],
        &[(0, 4, 4)],
    );
    // 1920 ticks = 1 bar. In a constant 120 BPM world that's 96000 samples.
    // With the ramp, bar 0's avg BPM is (120 + 150)/2 = 135, so
    // bar 0 samples ≈ 4 * 48000*60/135 ≈ 85333.
    let result = tm.tick_to_abs_sample(0, 1920, SR);
    // Should be roughly 85333, NOT 96000 (flat 120) or 64000 (flat 180).
    assert!(
        result > 80000 && result < 90000,
        "expected ~85333, got {result}"
    );
}

#[test]
fn tick_to_abs_sample_clip_starting_mid_bar() {
    // Constant 120 BPM. Clip starts at sample 48000 (beat 2 of bar 0).
    // 480 ticks further = 1 beat = 24000 samples.
    let tm = make_tempo_map(&[(0, 120.0)], &[(0, 4, 4)]);
    let result = tm.tick_to_abs_sample(48000, 480, SR);
    assert_eq!(result, 48000 + 24000);
}

#[test]
fn tick_to_abs_sample_with_step_change() {
    // 150 at bar 0, step change at bar 2: arrival 109, departure 150.
    let tm = make_tempo_map(
        &[
            (0, 150.0),
            (2, 109.0),
            (2, 150.0),
        ],
        &[(0, 4, 4)],
    );
    // A clip from bar 0, one bar of ticks (1920), should span bar 0
    // which ramps from 150 to the arrival at bar 1.
    // arrival_bpm_at_bar(1) = interpolated = 150+(109-150)*0.5 = 129.5
    // avg = (150 + 129.5) / 2 = 139.75
    // bar 0 samples ≈ 4 * 48000*60/139.75 ≈ 82377
    let result = tm.tick_to_abs_sample(0, 1920, SR);
    assert!(
        result > 80000 && result < 86000,
        "expected ~82377, got {result}"
    );
}

// ========================================================================
// TempoMap::bar_index_at / beats_in_bar / beat_sample_in_bar
// ========================================================================

#[test]
fn bar_index_at_single_tempo() {
    let tm = make_tempo_map(&[(0, 120.0)], &[(0, 4, 4)]);
    // Bar table is always built now, so bar 0 should be found.
    assert_eq!(tm.bar_index_at(0), Some(0));
}

#[test]
fn bar_index_at_start() {
    let tm = make_tempo_map(
        &[(0, 120.0), (4, 160.0)],
        &[(0, 4, 4)],
    );
    assert_eq!(tm.bar_index_at(0), Some(0));
}

#[test]
fn beats_in_bar_default_4_4() {
    let tm = make_tempo_map(
        &[(0, 120.0), (4, 160.0)],
        &[(0, 4, 4)],
    );
    assert_eq!(tm.beats_in_bar(0), 4);
    assert_eq!(tm.beats_in_bar(1), 4);
}

#[test]
fn beat_sample_in_bar_beat_0_is_bar_start() {
    let tm = make_tempo_map(
        &[(0, 120.0), (4, 160.0)],
        &[(0, 4, 4)],
    );
    // Beat 0 of bar 0 should be at sample 0.
    assert_eq!(tm.beat_sample_in_bar(0, 0, SR), Some(0));
}

#[test]
fn beat_sample_in_bar_out_of_range() {
    let tm = make_tempo_map(
        &[(0, 120.0), (4, 160.0)],
        &[(0, 4, 4)],
    );
    // Beat 4 in a 4/4 bar doesn't exist (0-based: 0,1,2,3).
    assert_eq!(tm.beat_sample_in_bar(0, 4, SR), None);
}

#[test]
fn beat_sample_in_bar_constant_tempo_evenly_spaced() {
    // Constant 120 BPM: beats at 0, 24000, 48000, 72000.
    let tm = make_tempo_map(
        &[(0, 120.0), (4, 120.0)],
        &[(0, 4, 4)],
    );
    let expected_spb: u64 = 24000;
    for beat in 0..4u32 {
        let s = tm.beat_sample_in_bar(0, beat, SR).unwrap();
        let expected = beat as u64 * expected_spb;
        assert!(
            (s as i64 - expected as i64).unsigned_abs() < 2,
            "beat {beat}: expected {expected}, got {s}"
        );
    }
}

// ========================================================================
// TempoMap::sync_bpm_at
// ========================================================================

#[test]
fn sync_bpm_at_updates_field() {
    let mut tm = make_tempo_map(
        &[(0, 120.0), (4, 160.0)],
        &[(0, 4, 4)],
    );
    assert!((tm.bpm - 120.0).abs() < 0.01);
    // Move to a position mid-ramp and sync
    let mid = tm.tick_to_abs_sample(0, 1920 * 2, SR); // ~bar 2
    tm.sync_bpm_at(mid, SR);
    // Should be somewhere between 120 and 160
    assert!(tm.bpm > 125.0 && tm.bpm < 155.0, "bpm = {}", tm.bpm);
}

// ========================================================================
// TempoMap::format_position / format_time
// ========================================================================

#[test]
fn format_position_at_origin() {
    let tm = TempoMap::default();
    assert_eq!(tm.format_position(0, SR), "1.1");
}

#[test]
fn format_time_at_origin() {
    let tm = TempoMap::default();
    assert_eq!(tm.format_time(0, SR), "00:00.000");
}

#[test]
fn format_time_at_one_second() {
    let tm = TempoMap::default();
    assert_eq!(tm.format_time(48000, SR), "00:01.000");
}

// ========================================================================
// Step change scenario (user's bug report)
// ========================================================================

#[test]
fn user_scenario_150_rit_109_back_to_150() {
    // User's exact setup: 150 at bar 0, 150 at bar 1 (hold),
    // 109 at bar 2 (ramp target), 150 at bar 2 (step back up).
    let pts = [
        TempoPoint { bar: 0, bpm: 150.0 },
        TempoPoint { bar: 1, bpm: 150.0 },
        TempoPoint { bar: 2, bpm: 109.0 },
        TempoPoint { bar: 2, bpm: 150.0 },
    ];

    // Bar 0: constant 150 (depart 150, arrive bar 1 = 150)
    assert!((avg_bpm_for_bar(0, &pts) - 150.0).abs() < 1e-9);

    // Bar 1: ritardando from 150 → 109
    // Departure at bar 1 = 150, arrival at bar 2 = 109
    let avg1 = avg_bpm_for_bar(1, &pts);
    assert!(
        (avg1 - 129.5).abs() < 1e-9,
        "bar 1 avg should be (150+109)/2 = 129.5, got {avg1}"
    );

    // Bar 2: back to constant 150 (departure 150, arrival at bar 3 = 150)
    assert!((avg_bpm_for_bar(2, &pts) - 150.0).abs() < 1e-9);

    // Also verify the interpolated ramp within bar 1
    let mid = bpm_at_bar(1.5, &pts);
    // At bar 1.5: between bar 1 (dep 150) and bar 2 (first event = 109)
    // t = (1.5 - 1) / (2 - 1) = 0.5 → 150 + (109-150)*0.5 = 129.5
    assert!(
        (mid - 129.5).abs() < 1e-9,
        "bpm at bar 1.5 should be 129.5, got {mid}"
    );
}

#[test]
fn user_scenario_bar_table_consistent() {
    // Same setup: verify the bar table produces consistent positions.
    let tm = make_tempo_map(
        &[
            (0, 150.0),
            (1, 150.0),
            (2, 109.0),
            (2, 150.0),
        ],
        &[(0, 4, 4)],
    );

    // Bar 0 at 150 BPM: spb = 48000*60/150 = 19200, bar = 76800
    let bar0_end = tm.tick_to_abs_sample(0, 1920, SR);
    let expected_bar0 = (4.0 * 48000.0 * 60.0 / 150.0) as u64; // 76800
    assert!(
        (bar0_end as i64 - expected_bar0 as i64).unsigned_abs() < 10,
        "bar 0 duration: expected ~{expected_bar0}, got {bar0_end}"
    );

    // Bar 1 ritardando 150→109, avg = 129.5: bar ≈ 4*48000*60/129.5 ≈ 89035
    let bar1_start = bar0_end;
    let bar1_end = tm.tick_to_abs_sample(0, 1920 * 2, SR);
    let bar1_dur = bar1_end - bar1_start;
    let expected_bar1 = (4.0 * 48000.0 * 60.0 / 129.5) as u64;
    assert!(
        (bar1_dur as i64 - expected_bar1 as i64).unsigned_abs() < 100,
        "bar 1 duration: expected ~{expected_bar1}, got {bar1_dur}"
    );

    // Bar 2 back to 150 BPM: should be ~76800 again
    let bar2_end = tm.tick_to_abs_sample(0, 1920 * 3, SR);
    let bar2_dur = bar2_end - bar1_end;
    assert!(
        (bar2_dur as i64 - expected_bar0 as i64).unsigned_abs() < 10,
        "bar 2 duration: expected ~{expected_bar0}, got {bar2_dur}"
    );
}

// ========================================================================
// Round-trip: tick_to_abs_sample + position_to_bars
// ========================================================================

#[test]
fn tick_to_sample_and_back_constant() {
    let tm = make_tempo_map(&[(0, 120.0), (4, 120.0)], &[(0, 4, 4)]);
    // 3 bars of ticks from sample 0
    let sample = tm.tick_to_abs_sample(0, 1920 * 3, SR);
    let (bar, beat, _) = tm.position_to_bars(sample, SR);
    assert_eq!((bar, beat), (4, 1), "3 bars from 0 should be bar 4 beat 1");
}

#[test]
fn tick_to_sample_and_back_with_ramp() {
    let tm = make_tempo_map(
        &[(0, 120.0), (4, 180.0)],
        &[(0, 4, 4)],
    );
    // 2 bars of ticks from sample 0 should land at bar 3 beat 1
    let sample = tm.tick_to_abs_sample(0, 1920 * 2, SR);
    let (bar, beat, frac) = tm.position_to_bars(sample, SR);
    assert_eq!(bar, 3, "expected bar 3, got {bar}");
    assert_eq!(beat, 1, "expected beat 1, got {beat}");
    assert!(frac < 0.01, "expected frac ~0, got {frac}");
}
