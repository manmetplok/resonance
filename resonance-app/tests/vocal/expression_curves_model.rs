//! The vocal expression-curve model (vocal-mcp-control-and-quality.md
//! §10): the section-relative curves a vocal lane carries in its params,
//! the sampler the render reads, the point edits, and the whole-lane
//! replace that maps song frames back onto sections.

use std::collections::HashSet;

use resonance_app::compose::expression::{
    curve, move_point, normalized_value, real_value, remove_point, sample_at_beat, set_stepped,
    upsert_point,
};
use resonance_app::compose::expression_lanes::{
    plan_set_lane, vocal_lane_id, CurveWindow, PlanError, PointOrigin, VocalLane,
};
use resonance_app::compose::vocal_svs::CurveKind;
use resonance_audio::types::TempoMap;
use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind as Shape};
use resonance_music_theory::{VocalCurvePoint, VocalCurves, VocalParams};

const SR: u32 = 48_000;
/// One beat at 120 BPM / 48 kHz; a 4/4 bar is four.
const BEAT: u64 = 24_000;

fn p(beat: f64, value: f32) -> VocalCurvePoint {
    VocalCurvePoint {
        beat,
        value,
        stepped: false,
    }
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn sampling_is_linear_held_at_the_ends_and_steps_when_asked() {
    let pts = vec![p(1.0, 0.2), p(3.0, 0.6)];
    assert_eq!(sample_at_beat(&[], 1.0), None);
    assert!(approx(sample_at_beat(&pts, 0.0).unwrap(), 0.2), "held before the first");
    assert!(approx(sample_at_beat(&pts, 2.0).unwrap(), 0.4), "linear between");
    assert!(approx(sample_at_beat(&pts, 9.0).unwrap(), 0.6), "held after the last");
    let stepped = vec![
        VocalCurvePoint {
            stepped: true,
            ..p(1.0, 0.2)
        },
        p(3.0, 0.6),
    ];
    assert!(approx(sample_at_beat(&stepped, 2.9).unwrap(), 0.2));
}

#[test]
fn values_map_to_real_units() {
    assert!(approx(real_value(CurveKind::Dynamics, 0.25), 0.25));
    assert!(approx(real_value(CurveKind::PitchBend, 0.5), 0.0));
    assert!(approx(real_value(CurveKind::PitchBend, 1.0), 50.0));
    assert!(approx(normalized_value(CurveKind::PitchBend, -25.0), 0.25));
    assert!(approx(normalized_value(CurveKind::Tension, 2.0), 1.0), "clamped");
}

#[test]
fn point_edits_keep_the_curve_sorted() {
    let mut pts = Vec::new();
    assert_eq!(upsert_point(&mut pts, p(2.0, 0.5)), 0);
    assert_eq!(upsert_point(&mut pts, p(1.0, 0.1)), 0);
    assert_eq!(upsert_point(&mut pts, p(3.0, 0.9)), 2);
    // Same beat replaces.
    assert_eq!(upsert_point(&mut pts, p(2.0, 0.7)), 1);
    assert_eq!(pts.len(), 3);
    assert!(approx(pts[1].value, 0.7));
    // A drag clamps between its neighbours, so the index never moves.
    move_point(&mut pts, 1, 9.0, 2.0, 16.0);
    assert_eq!(pts[1].beat, 3.0);
    assert!(approx(pts[1].value, 1.0));
    move_point(&mut pts, 2, 40.0, 0.5, 16.0);
    assert_eq!(pts[2].beat, 16.0, "the last point stops at the section end");
    set_stepped(&mut pts, 0, true);
    assert!(pts[0].stepped);
    remove_point(&mut pts, 0);
    assert_eq!(pts.len(), 2);
}

#[test]
fn an_old_lane_config_without_curves_loads_with_none() {
    let mut json = serde_json::to_value(VocalParams::default()).unwrap();
    assert!(json.get("curves").is_none(), "empty curves are not written");
    json.as_object_mut().unwrap().remove("curves");
    let back: VocalParams = serde_json::from_value(json).expect("old config loads");
    assert!(back.curves.is_empty());

    let mut with = VocalParams::default();
    with.curves.pitch_bend = vec![p(5.0, 0.75)];
    let json = serde_json::to_value(&with).unwrap();
    assert_eq!(json["curves"]["pitch_bend"][0]["beat"], serde_json::json!(5.0));
    let back: VocalParams = serde_json::from_value(json).unwrap();
    assert_eq!(back.curves, with.curves);
    assert_eq!(curve(&back.curves, CurveKind::PitchBend).len(), 1);
    assert!(curve(&VocalCurves::default(), CurveKind::Dynamics).is_empty());
}

fn tempo_map() -> TempoMap {
    let mut tm = TempoMap::default();
    tm.bpm = 120.0;
    tm.numerator = 4;
    tm.denominator = 4;
    tm.rebuild_bar_table(SR);
    tm
}

fn window(placement_id: u64, start_bar: u64) -> CurveWindow {
    let tm = tempo_map();
    let start_frame = start_bar * 4 * BEAT;
    CurveWindow {
        placement_id,
        definition_id: 7,
        start_frame,
        end_frame: start_frame + 16 * BEAT,
        start_tick: tm.sample_to_abs_tick(start_frame, SR),
        len_beats: 16.0,
        supported: true,
        bank: "Lilia".into(),
    }
}

/// Section 7 placed at bars 0 and 4 with one curve point at beat 1
/// (value 0.3): projected to frames BEAT and 17 * BEAT.
fn lane() -> VocalLane {
    let target = AutomationTarget::VocalCurve {
        track: 3,
        curve: CurveKind::Dynamics,
    };
    let mut lane = AutomationLane::new(vocal_lane_id(3, CurveKind::Dynamics), target, Vec::new());
    lane.points = vec![
        Breakpoint::new(BEAT, 0.3, Shape::Linear),
        Breakpoint::new(17 * BEAT, 0.3, Shape::Linear),
    ];
    VocalLane {
        lane,
        origins: vec![
            PointOrigin {
                placement_id: 1,
                definition_id: 7,
                index: 0,
            },
            PointOrigin {
                placement_id: 2,
                definition_id: 7,
                index: 0,
            },
        ],
        windows: vec![window(1, 0), window(2, 4)],
    }
}

fn plan(points: &[Breakpoint]) -> Result<Vec<(u64, Vec<VocalCurvePoint>)>, PlanError> {
    let current = |_def: u64| vec![p(1.0, 0.3)];
    plan_set_lane(&lane(), points, &current, &tempo_map(), SR)
}

#[test]
fn a_point_at_the_second_placement_lands_on_the_section_beat() {
    let mut points = lane().lane.points;
    points.push(Breakpoint::new(16 * BEAT + 6 * BEAT, 0.9, Shape::Stepped));
    let out = plan(&points).unwrap();
    assert_eq!(out.len(), 1);
    let (def, curve) = &out[0];
    assert_eq!(*def, 7);
    assert_eq!(curve.len(), 2);
    assert_eq!(curve[1].beat, 6.0);
    assert!(curve[1].stepped);
}

#[test]
fn keeping_one_copy_keeps_the_point_and_dropping_all_deletes_it() {
    let one = vec![Breakpoint::new(BEAT, 0.3, Shape::Linear)];
    assert_eq!(plan(&one).unwrap()[0].1.len(), 1, "a surviving copy keeps it");
    let none = vec![Breakpoint::new(3 * BEAT, 0.5, Shape::Linear)];
    let out = plan(&none).unwrap();
    assert_eq!(out[0].1, vec![p(3.0, 0.5)], "no copy survives: replaced");
}

#[test]
fn a_changed_copy_changes_the_section_point() {
    // The copy at the second placement takes a new value; the first's
    // stale copy stays in the list (as `add_points` leaves it).
    let points = vec![
        Breakpoint::new(BEAT, 0.3, Shape::Linear),
        Breakpoint::new(17 * BEAT, 0.8, Shape::Linear),
    ];
    let out = plan(&points).unwrap();
    assert_eq!(out[0].1.len(), 1);
    assert!(approx(out[0].1[0].value, 0.8));
}

#[test]
fn outside_placements_and_conflicts_are_refused() {
    let outside = vec![Breakpoint::new(40 * BEAT, 0.5, Shape::Linear)];
    assert_eq!(
        plan(&outside),
        Err(PlanError::OutsidePlacements { frame: 40 * BEAT })
    );
    let conflict = vec![
        Breakpoint::new(2 * BEAT, 0.1, Shape::Linear),
        Breakpoint::new(18 * BEAT, 0.9, Shape::Linear),
    ];
    assert!(matches!(plan(&conflict), Err(PlanError::Conflict { .. })));
}

#[test]
fn copies_of_a_point_are_found_across_placements() {
    let lane = lane();
    let copies = lane.copies_of(&HashSet::from([1]));
    assert_eq!(copies, HashSet::from([0, 1]));
    assert_eq!(lane.window_at(17 * BEAT).unwrap().placement_id, 2);
    assert!(lane.window_at(40 * BEAT).is_none());
    assert_eq!(
        lane.windows[1].beat_at(&tempo_map(), SR, 22 * BEAT),
        6.0,
        "frame → section beat"
    );
}

/// A section placed under two meters is 12 beats long at its first
/// placement (3/4) and 16 at its second (4/4). A point at beat 14 is
/// projected only at the second; a whole-lane replace that drops that
/// copy deletes the point — the longest placement decides what was
/// visible, not the first one.
#[test]
fn a_point_only_the_longer_placement_shows_is_deletable() {
    let mut short = window(1, 0);
    short.end_frame = short.start_frame + 12 * BEAT;
    short.len_beats = 12.0;
    let long = window(2, 4);
    let target = AutomationTarget::VocalCurve {
        track: 3,
        curve: CurveKind::Dynamics,
    };
    let mut projected = AutomationLane::new(vocal_lane_id(3, CurveKind::Dynamics), target, Vec::new());
    // Section point 0 at beat 1 (both placements), point 1 at beat 14
    // (only the 16-beat placement: frame 16 + 14 beats).
    projected.points = vec![
        Breakpoint::new(BEAT, 0.3, Shape::Linear),
        Breakpoint::new(17 * BEAT, 0.3, Shape::Linear),
        Breakpoint::new(30 * BEAT, 0.6, Shape::Linear),
    ];
    let origin = |placement_id, index| PointOrigin {
        placement_id,
        definition_id: 7,
        index,
    };
    let lane = VocalLane {
        lane: projected,
        origins: vec![origin(1, 0), origin(2, 0), origin(2, 1)],
        windows: vec![short, long],
    };
    let current = |_def: u64| vec![p(1.0, 0.3), p(14.0, 0.6)];
    let keep_first = vec![Breakpoint::new(BEAT, 0.3, Shape::Linear)];
    let out = plan_set_lane(&lane, &keep_first, &current, &tempo_map(), SR).unwrap();
    assert_eq!(out, vec![(7, vec![p(1.0, 0.3)])], "the beat-14 point is deleted");

    // A point past every placement's end still survives untouched.
    let current = |_def: u64| vec![p(1.0, 0.3), p(20.0, 0.6)];
    let out = plan_set_lane(&lane, &keep_first, &current, &tempo_map(), SR).unwrap();
    assert_eq!(out, vec![(7, vec![p(1.0, 0.3), p(20.0, 0.6)])]);
}

