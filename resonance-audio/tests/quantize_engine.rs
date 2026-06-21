//! Tests for the pure MIDI quantize / humanize / groove engine.

use resonance_audio::quantize::{
    apply_groove, extract_groove, humanize_notes, quantize_notes, stock_grooves, Division,
    GridValue, GrooveTemplate, QuantizeMode,
};
use resonance_audio::types::{MidiNote, SignaturePoint, TempoMap, TICKS_PER_QUARTER_NOTE};

const TPQN: u64 = TICKS_PER_QUARTER_NOTE;

fn note(start: u64, dur: u64, vel: f32, pitch: u8) -> MidiNote {
    MidiNote {
        note: pitch,
        velocity: vel,
        start_tick: start,
        duration_ticks: dur,
    }
}

/// 4/4 default tempo map (no rebuilt bar table needed — quantize works
/// purely in tick space).
fn map_44() -> TempoMap {
    TempoMap::default()
}

fn map_sig(num: u8, den: u8) -> TempoMap {
    let mut tm = TempoMap::default();
    tm.numerator = num;
    tm.denominator = den;
    tm.signature_points = vec![SignaturePoint {
        bar: 0,
        numerator: num,
        denominator: den,
    }];
    tm
}

// ---------------------------------------------------------------------
// Division → ticks
// ---------------------------------------------------------------------

#[test]
fn division_tick_lengths() {
    assert_eq!(Division::straight(GridValue::Quarter).ticks(), TPQN);
    assert_eq!(Division::straight(GridValue::Eighth).ticks(), TPQN / 2);
    assert_eq!(Division::straight(GridValue::Sixteenth).ticks(), TPQN / 4);
    assert_eq!(
        Division::straight(GridValue::ThirtySecond).ticks(),
        TPQN / 8
    );
    // Triplet eighth: two-thirds of a straight eighth (240 → 160).
    assert_eq!(Division::triplet(GridValue::Eighth).ticks(), 160);
    // Dotted eighth: one-and-a-half eighths (240 → 360).
    assert_eq!(Division::dotted(GridValue::Eighth).ticks(), 360);
    // Triplet quarter (480 → 320), dotted quarter (480 → 720).
    assert_eq!(Division::triplet(GridValue::Quarter).ticks(), 320);
    assert_eq!(Division::dotted(GridValue::Quarter).ticks(), 720);
}

// ---------------------------------------------------------------------
// quantize_notes — full strength
// ---------------------------------------------------------------------

#[test]
fn quantize_full_strength_snaps_to_grid() {
    let grid = Division::straight(GridValue::Sixteenth); // 120 ticks
    let notes = vec![
        note(5, 100, 0.8, 60),   // -> 0
        note(118, 100, 0.8, 62), // -> 120
        note(250, 100, 0.8, 64), // -> 240
    ];
    let out = quantize_notes(
        &notes,
        &[0, 1, 2],
        grid,
        1.0,
        0.0,
        QuantizeMode::StartOnly,
        false,
        false,
        &map_44(),
        0,
    );
    assert_eq!(out[0].start_tick, 0);
    assert_eq!(out[1].start_tick, 120);
    assert_eq!(out[2].start_tick, 240);
    // Durations untouched in StartOnly.
    assert_eq!(out[0].duration_ticks, 100);
}

#[test]
fn quantize_partial_strength_moves_halfway() {
    let grid = Division::straight(GridValue::Quarter); // 480
    let notes = vec![note(100, 200, 0.8, 60)];
    let out = quantize_notes(
        &notes,
        &[0],
        grid,
        0.5,
        0.0,
        QuantizeMode::StartOnly,
        false,
        false,
        &map_44(),
        0,
    );
    // Nearest grid line is 0; halfway from 100 → 50.
    assert_eq!(out[0].start_tick, 50);
}

#[test]
fn quantize_zero_strength_is_noop() {
    let grid = Division::straight(GridValue::Sixteenth);
    let notes = vec![note(37, 200, 0.8, 60)];
    let out = quantize_notes(
        &notes,
        &[0],
        grid,
        0.0,
        0.0,
        QuantizeMode::StartOnly,
        false,
        false,
        &map_44(),
        0,
    );
    assert_eq!(out[0].start_tick, 37);
}

#[test]
fn empty_selection_is_noop() {
    let grid = Division::straight(GridValue::Sixteenth);
    let notes = vec![note(37, 200, 0.8, 60), note(605, 50, 0.5, 62)];
    let out = quantize_notes(
        &notes,
        &[],
        grid,
        1.0,
        0.0,
        QuantizeMode::StartOnly,
        false,
        false,
        &map_44(),
        0,
    );
    assert_eq!(out[0].start_tick, 37);
    assert_eq!(out[1].start_tick, 605);
}

#[test]
fn quantize_only_touches_selected_indices() {
    let grid = Division::straight(GridValue::Sixteenth);
    let notes = vec![note(5, 100, 0.8, 60), note(118, 100, 0.8, 62)];
    let out = quantize_notes(
        &notes,
        &[1],
        grid,
        1.0,
        0.0,
        QuantizeMode::StartOnly,
        false,
        false,
        &map_44(),
        0,
    );
    assert_eq!(out[0].start_tick, 5); // unchanged
    assert_eq!(out[1].start_tick, 120); // snapped
}

#[test]
fn quantize_clip_start_offset_aligns_to_bars() {
    let grid = Division::straight(GridValue::Quarter); // 480
                                                       // Clip starts 100 ticks into the project (not on a bar line). A note
                                                       // at clip-relative tick 50 sits at abs 150 → nearest grid line 0 →
                                                       // clip-relative -100 clamps to... abs snaps to 0, rel = 0 - 100 < 0
                                                       // -> 0. Use a clearer case: abs 150 nearest grid is 0 (dist 150) vs
                                                       // 480 (dist 330) → 0.
    let notes = vec![note(50, 100, 0.8, 60)];
    let out = quantize_notes(
        &notes,
        &[0],
        grid,
        1.0,
        0.0,
        QuantizeMode::StartOnly,
        false,
        false,
        &map_44(),
        100,
    );
    // abs snaps to 0, relative saturates to 0.
    assert_eq!(out[0].start_tick, 0);

    // A note near abs 480: clip-relative 410 → abs 510 → snaps to 480 →
    // relative 380.
    let notes2 = vec![note(410, 100, 0.8, 60)];
    let out2 = quantize_notes(
        &notes2,
        &[0],
        grid,
        1.0,
        0.0,
        QuantizeMode::StartOnly,
        false,
        false,
        &map_44(),
        100,
    );
    assert_eq!(out2[0].start_tick, 380);
}

#[test]
fn quantize_start_and_length_snaps_duration() {
    let grid = Division::straight(GridValue::Sixteenth); // 120
    let notes = vec![note(5, 130, 0.8, 60)];
    let out = quantize_notes(
        &notes,
        &[0],
        grid,
        1.0,
        0.0,
        QuantizeMode::StartAndLength,
        false,
        false,
        &map_44(),
        0,
    );
    assert_eq!(out[0].start_tick, 0);
    // 130 rounds to one 16th step (120).
    assert_eq!(out[0].duration_ticks, 120);
}

#[test]
fn quantize_ends_snaps_note_off() {
    let grid = Division::straight(GridValue::Quarter); // 480
                                                       // start 5 → 0; end 5+490=495 → 480; duration 480.
    let notes = vec![note(5, 490, 0.8, 60)];
    let out = quantize_notes(
        &notes,
        &[0],
        grid,
        1.0,
        0.0,
        QuantizeMode::StartOnly,
        true,
        false,
        &map_44(),
        0,
    );
    assert_eq!(out[0].start_tick, 0);
    assert_eq!(out[0].duration_ticks, 480);
}

#[test]
fn quantize_ends_keeps_minimum_duration() {
    let grid = Division::straight(GridValue::Quarter);
    // start and end both snap to 0 → duration clamps to 1.
    let notes = vec![note(5, 3, 0.8, 60)];
    let out = quantize_notes(
        &notes,
        &[0],
        grid,
        1.0,
        0.0,
        QuantizeMode::StartOnly,
        true,
        false,
        &map_44(),
        0,
    );
    assert!(out[0].duration_ticks >= 1);
}

// ---------------------------------------------------------------------
// Swing
// ---------------------------------------------------------------------

#[test]
fn swing_delays_offbeat_only() {
    let grid = Division::straight(GridValue::Eighth); // 240
                                                      // Note near the off-beat (step 1 = 240). With swing 2/3, the off-beat
                                                      // grid line moves to 240 + (2/3 * 240/2) = 240 + 80 = 320 (triplet).
    let notes = vec![note(250, 100, 0.8, 60)];
    let out = quantize_notes(
        &notes,
        &[0],
        grid,
        1.0,
        2.0 / 3.0,
        QuantizeMode::StartOnly,
        false,
        false,
        &map_44(),
        0,
    );
    assert_eq!(out[0].start_tick, 320);

    // The down-beat (step 0) is unaffected by swing.
    let notes2 = vec![note(8, 100, 0.8, 60)];
    let out2 = quantize_notes(
        &notes2,
        &[0],
        grid,
        1.0,
        2.0 / 3.0,
        QuantizeMode::StartOnly,
        false,
        false,
        &map_44(),
        0,
    );
    assert_eq!(out2[0].start_tick, 0);
}

// ---------------------------------------------------------------------
// Odd meters
// ---------------------------------------------------------------------

#[test]
fn odd_meter_grid_anchors_to_each_bar() {
    // 7/8: bar length = 7 * (1920/8) = 7 * 240 = 1680 ticks.
    let tm = map_sig(7, 8);
    let grid = Division::straight(GridValue::Eighth); // 240
                                                      // Second bar begins at 1680. A note at 1690 → snaps to 1680.
    let notes = vec![note(1690, 100, 0.8, 60)];
    let out = quantize_notes(
        &notes,
        &[0],
        grid,
        1.0,
        0.0,
        QuantizeMode::StartOnly,
        false,
        false,
        &tm,
        0,
    );
    assert_eq!(out[0].start_tick, 1680);
}

#[test]
fn odd_meter_bar_end_acts_as_grid_line() {
    // 7/8 with a quarter-note grid (480). Bar length 1680 is not a
    // multiple of 480; the bar's final partial cell ends at the next
    // downbeat (1680), which must itself be a snap target.
    let tm = map_sig(7, 8);
    let grid = Division::straight(GridValue::Quarter); // 480
                                                       // 1600 is closer to the bar end (1680, dist 80) than to 1440 (dist 160).
    let notes = vec![note(1600, 100, 0.8, 60)];
    let out = quantize_notes(
        &notes,
        &[0],
        grid,
        1.0,
        0.0,
        QuantizeMode::StartOnly,
        false,
        false,
        &tm,
        0,
    );
    assert_eq!(out[0].start_tick, 1680);
}

// ---------------------------------------------------------------------
// Invariants: never reorder / merge / drop
// ---------------------------------------------------------------------

#[test]
fn quantize_preserves_count_and_order() {
    let grid = Division::straight(GridValue::Sixteenth);
    let notes = vec![
        note(5, 100, 0.8, 60),
        note(118, 100, 0.7, 62),
        note(119, 100, 0.6, 64), // two notes snap to the same grid line
        note(700, 100, 0.5, 65),
    ];
    let out = quantize_notes(
        &notes,
        &[0, 1, 2, 3],
        grid,
        1.0,
        0.0,
        QuantizeMode::StartOnly,
        false,
        false,
        &map_44(),
        0,
    );
    assert_eq!(out.len(), notes.len());
    // Pitch order preserved index-for-index even though notes 1 and 2
    // both land on tick 120.
    let pitches: Vec<u8> = out.iter().map(|n| n.note).collect();
    assert_eq!(pitches, vec![60, 62, 64, 65]);
    assert_eq!(out[1].start_tick, 120);
    assert_eq!(out[2].start_tick, 120);
}

#[test]
fn out_of_range_selection_indices_ignored() {
    let grid = Division::straight(GridValue::Sixteenth);
    let notes = vec![note(5, 100, 0.8, 60)];
    let out = quantize_notes(
        &notes,
        &[0, 99],
        grid,
        1.0,
        0.0,
        QuantizeMode::StartOnly,
        false,
        false,
        &map_44(),
        0,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].start_tick, 0);
}

// ---------------------------------------------------------------------
// Iterative
// ---------------------------------------------------------------------

#[test]
fn iterative_pulls_closer_than_single_pass() {
    let grid = Division::straight(GridValue::Quarter); // 480
    let notes = vec![note(100, 200, 0.8, 60)];
    let single = quantize_notes(
        &notes,
        &[0],
        grid,
        0.5,
        0.0,
        QuantizeMode::StartOnly,
        false,
        false,
        &map_44(),
        0,
    );
    let iter = quantize_notes(
        &notes,
        &[0],
        grid,
        0.5,
        0.0,
        QuantizeMode::StartOnly,
        false,
        true,
        &map_44(),
        0,
    );
    // Both move toward 0; iterative ends nearer the grid line.
    assert!(iter[0].start_tick < single[0].start_tick);
    assert!(iter[0].start_tick < 50);
}

// ---------------------------------------------------------------------
// Humanize
// ---------------------------------------------------------------------

#[test]
fn humanize_is_deterministic_for_seed() {
    let notes = vec![note(480, 200, 0.8, 60), note(960, 200, 0.7, 62)];
    let a = humanize_notes(&notes, &[0, 1], 20, 0.2, 42);
    let b = humanize_notes(&notes, &[0, 1], 20, 0.2, 42);
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.start_tick, y.start_tick);
        assert_eq!(x.velocity, y.velocity);
    }
}

#[test]
fn humanize_differs_by_seed() {
    let notes = vec![note(480, 200, 0.8, 60)];
    let a = humanize_notes(&notes, &[0], 20, 0.2, 1);
    let b = humanize_notes(&notes, &[0], 20, 0.2, 2);
    // Extremely unlikely to coincide on both fields.
    assert!(a[0].start_tick != b[0].start_tick || a[0].velocity != b[0].velocity);
}

#[test]
fn humanize_respects_bounds_and_count() {
    let notes = vec![note(480, 200, 0.8, 60), note(960, 200, 0.7, 62)];
    let out = humanize_notes(&notes, &[0, 1], 20, 0.2, 7);
    assert_eq!(out.len(), 2);
    for (orig, h) in notes.iter().zip(out.iter()) {
        let delta = h.start_tick as i64 - orig.start_tick as i64;
        assert!(delta.abs() <= 20, "timing offset {delta} out of bounds");
        assert!(h.velocity >= 0.0 && h.velocity <= 1.0);
    }
}

#[test]
fn humanize_velocity_stays_clamped() {
    let notes = vec![note(0, 100, 1.0, 60), note(120, 100, 0.0, 62)];
    let out = humanize_notes(&notes, &[0, 1], 0, 1.0, 99);
    for h in &out {
        assert!(h.velocity >= 0.0 && h.velocity <= 1.0);
    }
}

// ---------------------------------------------------------------------
// Groove
// ---------------------------------------------------------------------

#[test]
fn extract_groove_measures_offset_and_velocity() {
    let grid = Division::straight(GridValue::Sixteenth); // 120, 16 steps/bar
                                                         // Step 0 notes on the grid; step 1 (tick 120) consistently +20 late
                                                         // and louder.
    let notes = vec![
        note(0, 60, 0.6, 60),
        note(140, 60, 0.9, 62), // step 1, +20, vel 0.9
        note(480, 60, 0.6, 64), // step 4 (next beat), on grid
    ];
    let groove = extract_groove(&notes, grid, &map_44());
    assert_eq!(groove.steps_per_bar, 16);
    assert_eq!(groove.timing_offsets_ticks[1], 20);
    // Step 1 louder than mean → scale > 1.
    assert!(groove.velocity_scale[1] > 1.0);
    // Steps with no notes are neutral.
    assert_eq!(groove.timing_offsets_ticks[2], 0);
    assert_eq!(groove.velocity_scale[2], 1.0);
}

#[test]
fn apply_groove_shifts_timing_by_strength() {
    let mut template = GrooveTemplate::identity(16);
    template.timing_offsets_ticks[1] = 40;
    template.velocity_scale[1] = 1.2;

    // A note exactly on step 1 (tick 120).
    let notes = vec![note(120, 60, 0.5, 60)];
    let out = apply_groove(&notes, &[0], &template, 0.5, &map_44());
    // Half of +40 → +20.
    assert_eq!(out[0].start_tick, 140);
    // velocity scaled by 1 + (1.2 - 1) * 0.5 = 1.1.
    assert!((out[0].velocity - 0.55).abs() < 1e-5);
}

#[test]
fn extract_then_apply_round_trips() {
    let grid = Division::straight(GridValue::Sixteenth);
    // A grooved performance: every off-16th is +30 late.
    let notes = vec![
        note(0, 60, 0.7, 60),
        note(150, 60, 0.7, 62), // step 1 +30
        note(240, 60, 0.7, 64), // step 2 on grid
        note(390, 60, 0.7, 65), // step 3 +30
    ];
    let groove = extract_groove(&notes, grid, &map_44());
    // Apply the extracted groove at full strength to straight notes; the
    // off-beats should pick up the +30 feel.
    let straight = vec![
        note(0, 60, 0.7, 60),
        note(120, 60, 0.7, 62),
        note(240, 60, 0.7, 64),
        note(360, 60, 0.7, 65),
    ];
    let out = apply_groove(&straight, &[0, 1, 2, 3], &groove, 1.0, &map_44());
    assert_eq!(out[0].start_tick, 0);
    assert_eq!(out[1].start_tick, 150);
    assert_eq!(out[2].start_tick, 240);
    assert_eq!(out[3].start_tick, 390);
}

#[test]
fn apply_groove_rejects_malformed_template() {
    let bad = GrooveTemplate {
        steps_per_bar: 16,
        timing_offsets_ticks: vec![10; 4], // wrong length
        velocity_scale: vec![1.0; 16],
    };
    let notes = vec![note(120, 60, 0.5, 60)];
    let out = apply_groove(&notes, &[0], &bad, 1.0, &map_44());
    assert_eq!(out[0].start_tick, 120); // unchanged
}

#[test]
fn stock_grooves_are_well_formed() {
    let grooves = stock_grooves();
    assert!(grooves.len() >= 2);
    for (name, g) in &grooves {
        assert!(!name.is_empty());
        assert_eq!(g.timing_offsets_ticks.len(), g.steps_per_bar as usize);
        assert_eq!(g.velocity_scale.len(), g.steps_per_bar as usize);
    }
    // MPC swing delays the off-16ths.
    let (_, mpc) = grooves.iter().find(|(n, _)| n == "MPC Swing").unwrap();
    assert!(mpc.timing_offsets_ticks[1] > 0);
    assert_eq!(mpc.timing_offsets_ticks[0], 0);
}
