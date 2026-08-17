//! Unit tests for vocal Expression-dock edit transitions (todo #336): the
//! `ExpressionMessage` handlers' state logic, exercised through the
//! exported pure units they're built from — the curve model's edit methods
//! (`compose::expression`), the dock tool state and snap helpers
//! (`compose::expression_edit`). The `resonance-app` binary keeps its
//! runtime state (`ComposeState` / `Resonance`) crate-private, so the
//! handler glue itself isn't reachable here; every decision it makes lives
//! in one of these units (see ARCHITECTURE.md → Test Layout).

use resonance_app::compose::expression::{
    CurveStatus, ExpressionCurve, ExpressionCurves, DEPTH_DEFAULT, DEPTH_RANGE,
    SMOOTHING_DEFAULT_MS, SMOOTHING_RANGE_MS,
};
use resonance_app::compose::expression_edit::{normalized_onsets, snap_time, ExpressionDockState};
use resonance_app::compose::vocal_svs::CurveKind;
use resonance_app::compose::PenMode;
use resonance_audio::types::MidiNote;
use resonance_music_theory::VocalVoicebank;

const EPS: f32 = 1e-5;

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() <= EPS
}

// ---------------------------------------------------------------------------
// Tool-state transitions (SelectCurve / SetPenMode / SetSnap)
// ---------------------------------------------------------------------------

#[test]
fn dock_defaults_are_dynamics_draw_no_snap() {
    let dock = ExpressionDockState::default();
    assert_eq!(dock.active, CurveKind::Dynamics);
    assert_eq!(dock.pen, PenMode::Draw);
    assert!(!dock.snap);
}

#[test]
fn select_curve_sets_active_only() {
    let mut dock = ExpressionDockState::default();
    dock.select_curve(CurveKind::PitchBend);
    assert_eq!(dock.active, CurveKind::PitchBend);
    // Selecting a curve doesn't disturb the pen or snap tool state.
    assert_eq!(dock.pen, PenMode::Draw);
    assert!(!dock.snap);
}

#[test]
fn set_pen_mode_cycles_through_modes() {
    let mut dock = ExpressionDockState::default();
    dock.set_pen(PenMode::Points);
    assert_eq!(dock.pen, PenMode::Points);
    dock.set_pen(PenMode::Line);
    assert_eq!(dock.pen, PenMode::Line);
    dock.set_pen(PenMode::Draw);
    assert_eq!(dock.pen, PenMode::Draw);
}

#[test]
fn set_snap_toggles_both_ways() {
    let mut dock = ExpressionDockState::default();
    dock.set_snap(true);
    assert!(dock.snap);
    dock.set_snap(false);
    assert!(!dock.snap);
}

// ---------------------------------------------------------------------------
// Breakpoint add / move / remove
// ---------------------------------------------------------------------------

#[test]
fn add_breakpoint_marks_edited_and_flips_status() {
    let mut curves = ExpressionCurves::from_baselines(
        vec![0.0, 0.0],
        vec![0.0, 0.0],
        vec![0.0, 0.0],
        vec![0.0, 0.0],
    );
    assert_eq!(
        curves.status(CurveKind::Dynamics, VocalVoicebank::Lilia),
        CurveStatus::Auto
    );

    curves
        .curve_mut(CurveKind::Dynamics)
        .add_breakpoint(0.5, 0.7);

    assert!(curves.is_edited(CurveKind::Dynamics));
    assert_eq!(
        curves.status(CurveKind::Dynamics, VocalVoicebank::Lilia),
        CurveStatus::Edited
    );
    assert!(approx(curves.evaluate(CurveKind::Dynamics, 0.5), 0.7));
}

#[test]
fn move_breakpoint_keeps_index_and_clamps_to_neighbours() {
    let mut curve = ExpressionCurve::new(CurveKind::Dynamics);
    curve.add_breakpoint(0.2, 0.2);
    curve.add_breakpoint(0.5, 0.5);
    curve.add_breakpoint(0.8, 0.8);

    // Try to drag the middle point past its right neighbour: it clamps to
    // the neighbour's time, so the overlay stays sorted and index 1 still
    // refers to the same (moved) point.
    curve.move_breakpoint(1, 0.95, 0.6);
    let ts: Vec<f32> = curve.overlay().iter().map(|p| p.t).collect();
    assert!(approx(ts[0], 0.2));
    assert!(approx(ts[1], 0.8)); // clamped up to the right neighbour
    assert!(approx(ts[2], 0.8));
    assert!(approx(curve.overlay()[1].value, 0.6));

    // Drag it left past its left neighbour: clamps down to 0.2.
    curve.move_breakpoint(1, -0.5, 0.3);
    assert!(approx(curve.overlay()[1].t, 0.2));
    // Value clamps to the kind's 0..=1 range.
    curve.move_breakpoint(1, 0.25, 5.0);
    assert!(approx(curve.overlay()[1].value, 1.0));
}

#[test]
fn move_breakpoint_out_of_bounds_is_noop() {
    let mut curve = ExpressionCurve::new(CurveKind::Dynamics);
    curve.add_breakpoint(0.5, 0.5);
    curve.move_breakpoint(7, 0.1, 0.1);
    assert_eq!(curve.overlay().len(), 1);
    assert!(approx(curve.overlay()[0].t, 0.5));
}

#[test]
fn remove_breakpoint_drops_point_and_last_removal_returns_to_auto() {
    let mut curve = ExpressionCurve::from_baseline(CurveKind::Dynamics, vec![0.1, 0.9]);
    curve.add_breakpoint(0.3, 0.3);
    curve.add_breakpoint(0.7, 0.7);

    curve.remove_breakpoint(0);
    assert_eq!(curve.overlay().len(), 1);
    assert!(approx(curve.overlay()[0].t, 0.7));
    assert!(curve.is_edited());

    // Removing the last overlay point makes the curve follow its baseline
    // again — back to Auto.
    curve.remove_breakpoint(0);
    assert!(curve.overlay().is_empty());
    assert!(!curve.is_edited());
    assert_eq!(curve.status(true), CurveStatus::Auto);
    assert!(approx(curve.evaluate(0.0), 0.1));
}

#[test]
fn remove_breakpoint_out_of_bounds_is_noop() {
    let mut curve = ExpressionCurve::new(CurveKind::Dynamics);
    curve.add_breakpoint(0.5, 0.5);
    curve.remove_breakpoint(9);
    assert_eq!(curve.overlay().len(), 1);
}

// ---------------------------------------------------------------------------
// Depth / smoothing
// ---------------------------------------------------------------------------

#[test]
fn set_depth_marks_edited_even_without_overlay() {
    let mut curve = ExpressionCurve::from_baseline(CurveKind::Dynamics, vec![0.0, 1.0]);
    assert!(!curve.is_edited());

    curve.set_depth(0.32);
    assert!(approx(curve.depth(), 0.32));
    // Depth alone (no overlay breakpoints) still flips the curve to Edited.
    assert!(curve.is_edited());
    assert_eq!(curve.status(true), CurveStatus::Edited);
}

#[test]
fn set_smoothing_marks_edited_and_clamps_to_range() {
    let mut curve = ExpressionCurve::new(CurveKind::Tension);
    assert!(!curve.is_edited());

    curve.set_smoothing(18.0);
    assert!(approx(curve.smoothing(), 18.0));
    assert!(curve.is_edited());

    // Out-of-range smoothing clamps to the inspector range.
    let (lo, hi) = SMOOTHING_RANGE_MS;
    curve.set_smoothing(hi + 100.0);
    assert!(approx(curve.smoothing(), hi));
    curve.set_smoothing(-5.0);
    assert!(approx(curve.smoothing(), lo));
}

#[test]
fn set_depth_clamps_to_range() {
    let mut curve = ExpressionCurve::new(CurveKind::Dynamics);
    let (lo, hi) = DEPTH_RANGE;
    curve.set_depth(hi + 2.0);
    assert!(approx(curve.depth(), hi));
    curve.set_depth(lo - 2.0);
    assert!(approx(curve.depth(), lo));
}

#[test]
fn returning_depth_to_default_clears_edited() {
    let mut curve = ExpressionCurve::from_baseline(CurveKind::Dynamics, vec![0.0, 1.0]);
    curve.set_depth(0.4);
    assert!(curve.is_edited());
    // Manually setting depth back to neutral (no overlay) -> Auto again.
    curve.set_depth(DEPTH_DEFAULT);
    assert!(!curve.is_edited());
}

// ---------------------------------------------------------------------------
// Reset-to-generated
// ---------------------------------------------------------------------------

#[test]
fn reset_clears_overlay_depth_smoothing_and_restores_baseline() {
    let mut curves = ExpressionCurves::from_baselines(
        vec![0.1, 0.9],
        vec![0.0, 1.0],
        vec![0.0, 1.0],
        vec![-10.0, 10.0],
    );
    let dyn_curve = curves.curve_mut(CurveKind::Dynamics);
    dyn_curve.add_breakpoint(0.0, 0.5);
    dyn_curve.add_breakpoint(1.0, 0.5);
    dyn_curve.set_depth(0.4);
    dyn_curve.set_smoothing(25.0);
    assert!(curves.is_edited(CurveKind::Dynamics));

    curves.reset(CurveKind::Dynamics);

    let dyn_curve = curves.curve(CurveKind::Dynamics);
    assert!(!dyn_curve.is_edited());
    assert!(dyn_curve.overlay().is_empty());
    assert!(approx(dyn_curve.depth(), DEPTH_DEFAULT));
    assert!(approx(dyn_curve.smoothing(), SMOOTHING_DEFAULT_MS));
    assert_eq!(
        curves.status(CurveKind::Dynamics, VocalVoicebank::Lilia),
        CurveStatus::Auto
    );
    // Baseline (provenance) survived the edit + reset and is sampled again.
    assert!(approx(curves.evaluate(CurveKind::Dynamics, 0.0), 0.1));
    assert!(approx(curves.evaluate(CurveKind::Dynamics, 1.0), 0.9));
}

#[test]
fn reset_is_scoped_to_one_curve() {
    let mut curves = ExpressionCurves::default();
    curves.curve_mut(CurveKind::Dynamics).add_breakpoint(0.5, 0.5);
    curves.curve_mut(CurveKind::Tension).add_breakpoint(0.5, 0.5);

    curves.reset(CurveKind::Dynamics);

    assert!(!curves.is_edited(CurveKind::Dynamics));
    // Resetting dynamics leaves tension's edit intact.
    assert!(curves.is_edited(CurveKind::Tension));
}

// ---------------------------------------------------------------------------
// Snap-to-syllables maths
// ---------------------------------------------------------------------------

fn note_at(start_tick: u64) -> MidiNote {
    MidiNote {
        note: 60,
        velocity: 0.8,
        start_tick,
        duration_ticks: 240,
    }
}

#[test]
fn normalized_onsets_maps_ticks_into_unit_interval_sorted_deduped() {
    // duration 1000 ticks; notes at 0, 250, 500, and a duplicate at 500.
    let notes = vec![note_at(500), note_at(0), note_at(250), note_at(500)];
    let onsets = normalized_onsets(&notes, 1000);
    assert_eq!(onsets.len(), 3); // duplicate 0.5 collapsed
    assert!(approx(onsets[0], 0.0));
    assert!(approx(onsets[1], 0.25));
    assert!(approx(onsets[2], 0.5));
}

#[test]
fn normalized_onsets_empty_for_zero_length_clip() {
    assert!(normalized_onsets(&[note_at(0)], 0).is_empty());
    assert!(normalized_onsets(&[], 1000).is_empty());
}

#[test]
fn snap_time_picks_nearest_onset() {
    let onsets = vec![0.0, 0.25, 0.5, 1.0];
    assert!(approx(snap_time(0.27, &onsets), 0.25));
    assert!(approx(snap_time(0.40, &onsets), 0.5));
    assert!(approx(snap_time(0.9, &onsets), 1.0));
    // Exactly on a target stays put.
    assert!(approx(snap_time(0.5, &onsets), 0.5));
}

#[test]
fn snap_time_passes_through_when_no_onsets() {
    assert!(approx(snap_time(0.37, &[]), 0.37));
    // And still clamps t to the unit interval.
    assert!(approx(snap_time(2.0, &[]), 1.0));
    assert!(approx(snap_time(-1.0, &[]), 0.0));
}
