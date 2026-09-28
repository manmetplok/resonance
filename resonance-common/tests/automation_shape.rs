use resonance_common::automation::CurveKind;
use resonance_common::automation_shape::{
    generate_shape, replace_range, BarSpan, GeneratedPoint, ShapeError, ShapeKind, ShapeRequest,
    StepQuantizer, MAX_SHAPE_POINTS_PER_CALL,
};

const TICKS_PER_BAR_4_4: u64 = 1920; // 4 beats * 480 ticks/beat, matches app's TICKS_PER_QUARTER_NOTE
const TICKS_PER_BAR_7_8: u64 = 7 * 240; // 7 eighth-notes * 240 ticks/eighth

fn bars_4_4(count: u64) -> Vec<BarSpan> {
    (0..count)
        .map(|i| BarSpan {
            start_tick: i * TICKS_PER_BAR_4_4,
            end_tick: (i + 1) * TICKS_PER_BAR_4_4,
        })
        .collect()
}

fn one_bar_7_8() -> Vec<BarSpan> {
    vec![BarSpan {
        start_tick: 0,
        end_tick: TICKS_PER_BAR_7_8,
    }]
}

fn base_request(shape: ShapeKind, bars: Vec<BarSpan>) -> ShapeRequest {
    ShapeRequest {
        shape,
        from: 300.0,
        to: 4000.0,
        cycles: None,
        resolution: None,
        seed: None,
        stepped: false,
        step_quantizer: None,
        logarithmic_unit: false,
        bars,
    }
}

fn assert_sorted_ascending(points: &[GeneratedPoint]) {
    for i in 1..points.len() {
        assert!(
            points[i - 1].tick < points[i].tick,
            "points must be strictly ascending by tick: {:?} then {:?}",
            points[i - 1],
            points[i]
        );
    }
}

// --- point counts ---------------------------------------------------------

#[test]
fn ramp_is_always_two_points() {
    let out = generate_shape(&base_request(ShapeKind::Ramp, bars_4_4(3))).unwrap();
    assert_eq!(out.points.len(), 2);
}

#[test]
fn exp_default_resolution_is_16_per_bar_plus_end_point() {
    let out = generate_shape(&base_request(ShapeKind::Exp, bars_4_4(2))).unwrap();
    // 16/bar * 2 bars + 1 forced end point.
    assert_eq!(out.points.len(), 16 * 2 + 1);
}

#[test]
fn sine_default_resolution_is_16_per_bar_plus_end_point() {
    let out = generate_shape(&base_request(ShapeKind::Sine, bars_4_4(1))).unwrap();
    assert_eq!(out.points.len(), 16 + 1);
}

#[test]
fn steps_default_resolution_is_1_per_bar_plus_end_point() {
    let out = generate_shape(&base_request(ShapeKind::Steps, bars_4_4(4))).unwrap();
    assert_eq!(out.points.len(), 4 + 1);
}

#[test]
fn random_walk_default_resolution_is_4_per_bar_plus_end_point() {
    let out = generate_shape(&base_request(ShapeKind::RandomWalk, bars_4_4(3))).unwrap();
    assert_eq!(out.points.len(), 4 * 3 + 1);
}

#[test]
fn explicit_resolution_is_honored() {
    let mut req = base_request(ShapeKind::Sine, bars_4_4(2));
    req.resolution = Some(8);
    let out = generate_shape(&req).unwrap();
    assert_eq!(out.points.len(), 8 * 2 + 1);
}

#[test]
fn triangle_is_two_per_cycle_plus_one() {
    let mut req = base_request(ShapeKind::Triangle, bars_4_4(2));
    req.cycles = Some(3);
    let out = generate_shape(&req).unwrap();
    assert_eq!(out.points.len(), 2 * 3 + 1);
}

#[test]
fn square_is_two_per_cycle_plus_one() {
    let mut req = base_request(ShapeKind::Square, bars_4_4(1));
    req.cycles = Some(2);
    let out = generate_shape(&req).unwrap();
    assert_eq!(out.points.len(), 2 * 2 + 1);
}

#[test]
fn triangle_default_cycles_is_one() {
    let out = generate_shape(&base_request(ShapeKind::Triangle, bars_4_4(1))).unwrap();
    assert_eq!(out.points.len(), 3);
}

// --- end point holds `to` (D3) --------------------------------------------

#[test]
fn every_shape_ends_at_to_and_at_the_final_tick() {
    let bars = bars_4_4(2);
    let end_tick = bars.last().unwrap().end_tick;
    for shape in [
        ShapeKind::Ramp,
        ShapeKind::Exp,
        ShapeKind::Sine,
        ShapeKind::Triangle,
        ShapeKind::Square,
        ShapeKind::Steps,
        ShapeKind::RandomWalk,
    ] {
        let out = generate_shape(&base_request(shape, bars.clone())).unwrap();
        let last = out.points.last().unwrap();
        assert_eq!(last.tick, end_tick, "shape {shape:?} last tick");
        assert!(
            (last.value - 4000.0).abs() < 1e-9,
            "shape {shape:?} last value should be `to`, got {}",
            last.value
        );
        assert_sorted_ascending(&out.points);
    }
}

#[test]
fn ramp_first_point_holds_from() {
    let out = generate_shape(&base_request(ShapeKind::Ramp, bars_4_4(1))).unwrap();
    assert_eq!(out.points.first().unwrap().value, 300.0);
}

#[test]
fn exp_first_point_holds_from() {
    let out = generate_shape(&base_request(ShapeKind::Exp, bars_4_4(1))).unwrap();
    let first = out.points.first().unwrap();
    assert!((first.value - 300.0).abs() < 1e-9);
}

// --- 7/8 vs 4/4 point counts are equal ------------------------------------

#[test]
fn seven_eight_bar_gets_same_point_count_as_four_four_bar() {
    for shape in [
        ShapeKind::Exp,
        ShapeKind::Sine,
        ShapeKind::Steps,
        ShapeKind::RandomWalk,
    ] {
        let four_four = generate_shape(&base_request(shape, bars_4_4(1))).unwrap();
        let seven_eight = generate_shape(&base_request(shape, one_bar_7_8())).unwrap();
        assert_eq!(
            four_four.points.len(),
            seven_eight.points.len(),
            "shape {shape:?}: 4/4 bar and 7/8 bar should have the same point count"
        );
    }
}

// --- curve kinds -----------------------------------------------------------

#[test]
fn steps_and_square_default_to_stepped_curve() {
    let steps = generate_shape(&base_request(ShapeKind::Steps, bars_4_4(1))).unwrap();
    assert!(steps.points.iter().all(|p| p.curve == CurveKind::Stepped));

    let mut square_req = base_request(ShapeKind::Square, bars_4_4(1));
    square_req.cycles = Some(1);
    let square = generate_shape(&square_req).unwrap();
    assert!(square.points.iter().all(|p| p.curve == CurveKind::Stepped));
}

#[test]
fn ramp_and_sine_default_to_linear_curve() {
    let ramp = generate_shape(&base_request(ShapeKind::Ramp, bars_4_4(1))).unwrap();
    assert!(ramp.points.iter().all(|p| p.curve == CurveKind::Linear));

    let sine = generate_shape(&base_request(ShapeKind::Sine, bars_4_4(1))).unwrap();
    assert!(sine.points.iter().all(|p| p.curve == CurveKind::Linear));
}

#[test]
fn stepped_flag_forces_stepped_curve_even_on_a_linear_shape() {
    let mut req = base_request(ShapeKind::Ramp, bars_4_4(1));
    req.stepped = true;
    let out = generate_shape(&req).unwrap();
    assert!(out.points.iter().all(|p| p.curve == CurveKind::Stepped));
}

// --- stepped rounding via quantizer ----------------------------------------

#[test]
fn stepped_quantizer_rounds_mute_to_0_or_1() {
    let mut req = ShapeRequest {
        from: 0.0,
        to: 1.0,
        ..base_request(ShapeKind::Ramp, bars_4_4(1))
    };
    req.stepped = true;
    req.step_quantizer = Some(StepQuantizer {
        min: 0.0,
        max: 1.0,
        steps: 2,
    });
    let out = generate_shape(&req).unwrap();
    for p in &out.points {
        assert!(
            p.value == 0.0 || p.value == 1.0,
            "mute value should quantize to 0 or 1, got {}",
            p.value
        );
    }
}

#[test]
fn stepped_quantizer_rounds_sine_onto_discrete_grid() {
    let mut req = ShapeRequest {
        from: 0.0,
        to: 10.0,
        ..base_request(ShapeKind::Sine, bars_4_4(1))
    };
    req.stepped = true;
    req.step_quantizer = Some(StepQuantizer {
        min: 0.0,
        max: 10.0,
        steps: 6, // grid: 0, 2, 4, 6, 8, 10
    });
    let out = generate_shape(&req).unwrap();
    for p in &out.points {
        let nearest = (p.value / 2.0).round() * 2.0;
        assert!(
            (p.value - nearest).abs() < 1e-9,
            "value {} should sit on the 0/2/4/6/8/10 grid",
            p.value
        );
    }
}

#[test]
fn quantizer_degenerate_range_collapses_to_min() {
    let q = StepQuantizer {
        min: 5.0,
        max: 5.0,
        steps: 4,
    };
    assert_eq!(q.quantize(100.0), 5.0);
    let q2 = StepQuantizer {
        min: 3.0,
        max: 9.0,
        steps: 1,
    };
    assert_eq!(q2.quantize(100.0), 3.0);
}

// --- exp validation ---------------------------------------------------------

#[test]
fn exp_rejected_for_logarithmic_unit_target() {
    let mut req = base_request(ShapeKind::Exp, bars_4_4(1));
    req.logarithmic_unit = true;
    let err = generate_shape(&req).unwrap_err();
    assert_eq!(err, ShapeError::ExpRejectedLogarithmicUnit);
    assert_eq!(err.to_string(), "dB is already logarithmic; use ramp");
}

#[test]
fn exp_requires_positive_from_and_to() {
    let mut req = base_request(ShapeKind::Exp, bars_4_4(1));
    req.from = 0.0;
    req.to = 100.0;
    assert!(matches!(
        generate_shape(&req).unwrap_err(),
        ShapeError::ExpRequiresPositive { .. }
    ));

    let mut req2 = base_request(ShapeKind::Exp, bars_4_4(1));
    req2.from = -5.0;
    req2.to = 100.0;
    assert!(matches!(
        generate_shape(&req2).unwrap_err(),
        ShapeError::ExpRequiresPositive { .. }
    ));

    let mut req3 = base_request(ShapeKind::Exp, bars_4_4(1));
    req3.from = 100.0;
    req3.to = -5.0;
    assert!(matches!(
        generate_shape(&req3).unwrap_err(),
        ShapeError::ExpRequiresPositive { .. }
    ));
}

#[test]
fn exp_accepts_positive_values_and_is_monotonic() {
    let out = generate_shape(&base_request(ShapeKind::Exp, bars_4_4(1))).unwrap();
    for i in 1..out.points.len() {
        assert!(
            out.points[i].value >= out.points[i - 1].value,
            "exp from a smaller to a larger value should be monotonically increasing"
        );
    }
}

// --- random_walk: bounded and deterministic ---------------------------------

#[test]
fn random_walk_stays_within_bounds() {
    let mut req = base_request(ShapeKind::RandomWalk, bars_4_4(8));
    req.from = 300.0;
    req.to = 4000.0;
    req.seed = Some(42);
    let out = generate_shape(&req).unwrap();
    for p in &out.points {
        assert!(
            (300.0..=4000.0).contains(&p.value),
            "random_walk value {} out of bounds",
            p.value
        );
    }
}

#[test]
fn random_walk_starts_at_from() {
    let mut req = base_request(ShapeKind::RandomWalk, bars_4_4(2));
    req.seed = Some(7);
    let out = generate_shape(&req).unwrap();
    assert_eq!(out.points.first().unwrap().value, req.from);
}

#[test]
fn random_walk_is_deterministic_per_seed() {
    let mut req = base_request(ShapeKind::RandomWalk, bars_4_4(4));
    req.seed = Some(123);
    let out1 = generate_shape(&req).unwrap();
    let out2 = generate_shape(&req).unwrap();
    assert_eq!(out1.points, out2.points);
}

#[test]
fn random_walk_different_seeds_diverge() {
    let mut req_a = base_request(ShapeKind::RandomWalk, bars_4_4(4));
    req_a.seed = Some(1);
    let mut req_b = req_a.clone();
    req_b.seed = Some(2);
    let out_a = generate_shape(&req_a).unwrap();
    let out_b = generate_shape(&req_b).unwrap();
    assert_ne!(out_a.points, out_b.points);
}

#[test]
fn random_walk_echoes_seed_used_default_zero() {
    let req = base_request(ShapeKind::RandomWalk, bars_4_4(1));
    let out = generate_shape(&req).unwrap();
    assert_eq!(out.seed_used, Some(0));
}

#[test]
fn random_walk_echoes_explicit_seed() {
    let mut req = base_request(ShapeKind::RandomWalk, bars_4_4(1));
    req.seed = Some(99);
    let out = generate_shape(&req).unwrap();
    assert_eq!(out.seed_used, Some(99));
}

#[test]
fn non_random_walk_shapes_do_not_echo_a_seed() {
    let out = generate_shape(&base_request(ShapeKind::Ramp, bars_4_4(1))).unwrap();
    assert_eq!(out.seed_used, None);
}

// --- limit errors -------------------------------------------------------

#[test]
fn resolution_over_the_limit_is_rejected_naming_count_limit_and_max_resolution() {
    let mut req = base_request(ShapeKind::Sine, bars_4_4(1));
    req.resolution = Some(3000);
    let err = generate_shape(&req).unwrap_err();
    match err {
        ShapeError::TooManyPoints {
            count,
            limit,
            param_name,
            max_value,
        } => {
            assert_eq!(count, 3000 + 1);
            assert_eq!(limit, MAX_SHAPE_POINTS_PER_CALL);
            assert_eq!(param_name, "resolution");
            // The suggested resolution must actually fit under the limit.
            assert!((max_value as usize) < MAX_SHAPE_POINTS_PER_CALL);
            assert!(max_value < 3000);
        }
        other => panic!("expected TooManyPoints, got {other:?}"),
    }
}

#[test]
fn suggested_max_resolution_actually_fits_across_many_bars() {
    let mut req = base_request(ShapeKind::Exp, bars_4_4(100));
    req.resolution = Some(100); // 100 * 100 + 1 = 10001, way over the limit
    let err = generate_shape(&req).unwrap_err();
    let ShapeError::TooManyPoints { max_value, .. } = err else {
        panic!("expected TooManyPoints");
    };
    // Regenerate with the suggested resolution: it must be accepted.
    let mut retry = req.clone();
    retry.resolution = Some(max_value.max(1));
    let out = generate_shape(&retry);
    assert!(
        out.is_ok(),
        "the suggested max_resolution ({max_value}) should fit but generate_shape returned {out:?}"
    );
}

#[test]
fn cycles_over_the_limit_is_rejected_naming_cycles() {
    let mut req = base_request(ShapeKind::Triangle, bars_4_4(1));
    req.cycles = Some(2000);
    let err = generate_shape(&req).unwrap_err();
    match err {
        ShapeError::TooManyPoints {
            param_name, count, ..
        } => {
            assert_eq!(param_name, "cycles");
            assert_eq!(count, 2 * 2000 + 1);
        }
        other => panic!("expected TooManyPoints, got {other:?}"),
    }
}

#[test]
fn empty_bars_is_rejected() {
    let req = base_request(ShapeKind::Ramp, vec![]);
    assert_eq!(generate_shape(&req).unwrap_err(), ShapeError::EmptyBarSpan);
}

#[test]
fn invalid_bar_span_is_rejected() {
    let req = base_request(
        ShapeKind::Ramp,
        vec![BarSpan {
            start_tick: 100,
            end_tick: 100,
        }],
    );
    assert!(matches!(
        generate_shape(&req).unwrap_err(),
        ShapeError::InvalidBarSpan { .. }
    ));
}

// --- replace_range -----------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct Pt {
    tick: u64,
    value: f64,
}

fn pt(tick: u64, value: f64) -> Pt {
    Pt { tick, value }
}

#[test]
fn replace_range_keeps_points_outside_the_range() {
    let existing = vec![pt(0, 1.0), pt(50, 2.0), pt(100, 3.0), pt(200, 4.0)];
    let generated = vec![pt(60, 9.0), pt(90, 9.5)];
    let result = replace_range(&existing, 50, 100, generated, |p| p.tick);
    // 50 and 100 are inside the closed range and get removed; 0 and 200 stay.
    assert_eq!(
        result,
        vec![pt(0, 1.0), pt(60, 9.0), pt(90, 9.5), pt(200, 4.0)]
    );
}

#[test]
fn replace_range_removes_points_exactly_at_the_edges() {
    let existing = vec![pt(10, 1.0), pt(20, 2.0), pt(30, 3.0)];
    // Range [10, 30] is closed, so both edge points (10 and 30) must be
    // removed even though nothing generated lands exactly on them.
    let generated = vec![pt(15, 8.0)];
    let result = replace_range(&existing, 10, 30, generated, |p| p.tick);
    assert_eq!(result, vec![pt(15, 8.0)]);
}

#[test]
fn replace_range_on_empty_existing_just_inserts_generated() {
    let existing: Vec<Pt> = vec![];
    let generated = vec![pt(5, 1.0), pt(1, 0.0)];
    let result = replace_range(&existing, 0, 10, generated, |p| p.tick);
    assert_eq!(result, vec![pt(1, 0.0), pt(5, 1.0)]);
}

#[test]
fn replace_range_with_generated_shape_output_end_to_end() {
    let bars = bars_4_4(1);
    let out = generate_shape(&base_request(ShapeKind::Ramp, bars.clone())).unwrap();
    let existing = vec![
        GeneratedPoint {
            tick: 0,
            value: 0.0,
            curve: CurveKind::Linear,
        },
        GeneratedPoint {
            tick: TICKS_PER_BAR_4_4 / 2,
            value: 999.0,
            curve: CurveKind::Linear,
        },
        GeneratedPoint {
            tick: TICKS_PER_BAR_4_4 * 5,
            value: -1.0,
            curve: CurveKind::Linear,
        },
    ];
    let result = replace_range(
        &existing,
        bars[0].start_tick,
        bars[0].end_tick,
        out.points,
        |p| p.tick,
    );
    // The far-outside point at tick 5*bar survives; the point inside the
    // bar (at bar/2) is gone, replaced by the ramp's own two points.
    assert!(result.iter().any(|p| p.tick == TICKS_PER_BAR_4_4 * 5));
    assert!(!result.iter().any(|p| p.value == 999.0));
    assert_eq!(result.len(), 3); // ramp start + ramp end + the far point
}

// --- values follow time, not point index ------------------------------------

/// A 7/8 bar followed by a 4/4 bar, back to back.
fn seven_eight_then_four_four() -> Vec<BarSpan> {
    vec![
        BarSpan {
            start_tick: 0,
            end_tick: TICKS_PER_BAR_7_8,
        },
        BarSpan {
            start_tick: TICKS_PER_BAR_7_8,
            end_tick: TICKS_PER_BAR_7_8 + TICKS_PER_BAR_4_4,
        },
    ]
}

#[test]
fn exp_over_mixed_meters_is_monotonic_and_geometric_at_the_tick_midpoint() {
    let bars = seven_eight_then_four_four();
    let out = generate_shape(&base_request(ShapeKind::Exp, bars)).unwrap();
    // Per-bar placement is unchanged: 16 per bar + the end point.
    assert_eq!(out.points.len(), 16 * 2 + 1);
    for w in out.points.windows(2) {
        assert!(w[1].value > w[0].value, "strictly rising: {w:?}");
    }
    // The span's midpoint in ticks (1800) is a grid point of the 4/4 bar
    // (1680 + 1 * 120); an exponential sweep sits at the geometric mean
    // of `from` and `to` halfway through in time.
    let mid_tick = (TICKS_PER_BAR_7_8 + TICKS_PER_BAR_4_4) / 2;
    let mid = out
        .points
        .iter()
        .find(|p| p.tick == mid_tick)
        .expect("a point at the tick midpoint");
    let geometric_mean = (300.0f64 * 4000.0).sqrt();
    assert!(
        (mid.value - geometric_mean).abs() < 1e-9,
        "midpoint {} != geometric mean {geometric_mean}",
        mid.value
    );
}

#[test]
fn a_short_bar_covers_less_of_a_sweep_than_a_long_one() {
    let mut req = base_request(ShapeKind::Steps, seven_eight_then_four_four());
    req.from = 0.0;
    req.to = 3600.0; // one unit per tick, so value == tick
    let out = generate_shape(&req).unwrap();
    // One step per bar: the 4/4 bar's step starts where the 7/8 bar
    // ends in time (1680 of 3600 ticks), not halfway.
    assert_eq!(out.points.len(), 3);
    assert_eq!(out.points[1].tick, TICKS_PER_BAR_7_8);
    for p in &out.points {
        assert!((p.value - p.tick as f64).abs() < 1e-9, "{p:?}");
    }
}

#[test]
fn sine_phase_follows_ticks_across_mixed_meters() {
    let bars = seven_eight_then_four_four();
    let span = (TICKS_PER_BAR_7_8 + TICKS_PER_BAR_4_4) as f64;
    let out = generate_shape(&base_request(ShapeKind::Sine, bars)).unwrap();
    // Every point but the forced end point sits on the one-cycle sine of
    // its own time fraction.
    for p in &out.points[..out.points.len() - 1] {
        let t = p.tick as f64 / span;
        let expected = 300.0 + 3700.0 * (1.0 - (2.0 * std::f64::consts::PI * t).cos()) / 2.0;
        assert!((p.value - expected).abs() < 1e-9, "{p:?} vs {expected}");
    }
}
