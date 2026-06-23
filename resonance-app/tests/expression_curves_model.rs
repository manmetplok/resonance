//! Unit tests for the editable vocal expression-curve data model
//! (`compose::expression`, doc #154 / todo #332). Covers baseline seeding,
//! the evaluate(t) sampler, overlay editing + reset, derived status per
//! voicebank, range clamping, and a serde round-trip.

use resonance_app::compose::expression::{
    Breakpoint, CurveStatus, ExpressionCurve, ExpressionCurves,
};
use resonance_app::compose::vocal_svs::CurveKind;
use resonance_music_theory::VocalVoicebank;

const EPS: f32 = 1e-5;

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() <= EPS
}

#[test]
fn baseline_sampler_interpolates_uniformly() {
    // Four evenly-spaced samples at t = 0, 1/3, 2/3, 1.
    let curve = ExpressionCurve::from_baseline(CurveKind::Dynamics, vec![0.0, 0.3, 0.6, 0.9]);

    assert!(approx(curve.evaluate(0.0), 0.0));
    assert!(approx(curve.evaluate(1.0), 0.9));
    // Endpoints sit exactly on a sample; halfway lands between samples 1&2.
    assert!(approx(curve.evaluate(1.0 / 3.0), 0.3));
    assert!(approx(curve.evaluate(0.5), 0.45));
}

#[test]
fn evaluate_clamps_time_to_unit_interval() {
    let curve = ExpressionCurve::from_baseline(CurveKind::Dynamics, vec![0.2, 0.8]);
    assert!(approx(curve.evaluate(-1.0), 0.2));
    assert!(approx(curve.evaluate(2.0), 0.8));
}

#[test]
fn empty_baseline_and_overlay_yields_neutral() {
    // No baseline, no overlay -> neutral resting value for the kind.
    assert!(approx(ExpressionCurve::new(CurveKind::Dynamics).evaluate(0.5), 0.0));
    assert!(approx(ExpressionCurve::new(CurveKind::PitchBend).evaluate(0.5), 0.0));
}

#[test]
fn single_baseline_sample_holds_flat() {
    let curve = ExpressionCurve::from_baseline(CurveKind::Tension, vec![0.42]);
    assert!(approx(curve.evaluate(0.0), 0.42));
    assert!(approx(curve.evaluate(0.5), 0.42));
    assert!(approx(curve.evaluate(1.0), 0.42));
}

#[test]
fn untouched_curve_is_auto_and_not_edited() {
    let curve = ExpressionCurve::from_baseline(CurveKind::Dynamics, vec![0.1, 0.5]);
    assert!(!curve.is_edited());
    assert_eq!(curve.status(true), CurveStatus::Auto);
}

#[test]
fn overlay_overrides_baseline_and_marks_edited() {
    let mut curve = ExpressionCurve::from_baseline(CurveKind::Dynamics, vec![0.0, 0.0, 0.0]);
    curve.set_overlay(vec![Breakpoint::new(0.0, 0.2), Breakpoint::new(1.0, 0.8)]);

    assert!(curve.is_edited());
    assert_eq!(curve.status(true), CurveStatus::Edited);
    // Overlay (not the all-zero baseline) is sampled.
    assert!(approx(curve.evaluate(0.0), 0.2));
    assert!(approx(curve.evaluate(0.5), 0.5));
    assert!(approx(curve.evaluate(1.0), 0.8));
}

#[test]
fn overlay_holds_flat_outside_breakpoint_span() {
    let mut curve = ExpressionCurve::new(CurveKind::Dynamics);
    curve.set_overlay(vec![Breakpoint::new(0.25, 0.4), Breakpoint::new(0.75, 0.9)]);
    // Before first / after last breakpoint the value is held flat.
    assert!(approx(curve.evaluate(0.0), 0.4));
    assert!(approx(curve.evaluate(0.1), 0.4));
    assert!(approx(curve.evaluate(1.0), 0.9));
    assert!(approx(curve.evaluate(0.5), 0.65));
}

#[test]
fn set_overlay_sorts_unordered_breakpoints() {
    let mut curve = ExpressionCurve::new(CurveKind::Dynamics);
    curve.set_overlay(vec![
        Breakpoint::new(1.0, 1.0),
        Breakpoint::new(0.0, 0.0),
        Breakpoint::new(0.5, 0.5),
    ]);
    let ts: Vec<f32> = curve.overlay().iter().map(|p| p.t).collect();
    assert_eq!(ts, vec![0.0, 0.5, 1.0]);
    assert!(approx(curve.evaluate(0.25), 0.25));
}

#[test]
fn add_breakpoint_keeps_overlay_sorted() {
    let mut curve = ExpressionCurve::new(CurveKind::Dynamics);
    curve.add_breakpoint(0.8, 0.8);
    curve.add_breakpoint(0.2, 0.2);
    curve.add_breakpoint(0.5, 0.5);
    let ts: Vec<f32> = curve.overlay().iter().map(|p| p.t).collect();
    assert_eq!(ts, vec![0.2, 0.5, 0.8]);
}

#[test]
fn reset_to_baseline_clears_overlay_and_restores_baseline() {
    let mut curve = ExpressionCurve::from_baseline(CurveKind::Dynamics, vec![0.1, 0.9]);
    curve.set_overlay(vec![Breakpoint::new(0.0, 0.5), Breakpoint::new(1.0, 0.5)]);
    assert!(curve.is_edited());

    curve.reset_to_baseline();

    assert!(!curve.is_edited());
    assert!(curve.overlay().is_empty());
    // Baseline (provenance) survived the edit and is sampled again.
    assert!(approx(curve.evaluate(0.0), 0.1));
    assert!(approx(curve.evaluate(1.0), 0.9));
    // The baseline samples themselves are intact.
    assert_eq!(curve.baseline(), &[0.1, 0.9]);
}

#[test]
fn values_are_clamped_to_kind_range() {
    // 0..=1 envelope clamps baseline and overlay values.
    let curve = ExpressionCurve::from_baseline(CurveKind::Breathiness, vec![-0.5, 2.0]);
    assert_eq!(curve.baseline(), &[0.0, 1.0]);

    let mut pitch = ExpressionCurve::new(CurveKind::PitchBend);
    pitch.set_overlay(vec![
        Breakpoint::new(-1.0, -200.0), // t and value both out of range
        Breakpoint::new(2.0, 200.0),
    ]);
    let pts = pitch.overlay();
    assert!(approx(pts[0].t, 0.0) && approx(pts[0].value, -50.0));
    assert!(approx(pts[1].t, 1.0) && approx(pts[1].value, 50.0));
}

#[test]
fn pitch_bend_baseline_spans_cents() {
    let curve = ExpressionCurve::from_baseline(CurveKind::PitchBend, vec![-50.0, 0.0, 50.0]);
    assert!(approx(curve.evaluate(0.0), -50.0));
    assert!(approx(curve.evaluate(0.5), 0.0));
    assert!(approx(curve.evaluate(1.0), 50.0));
}

#[test]
fn status_is_na_on_unsupported_voicebank() {
    let curves = ExpressionCurves::from_baselines(
        vec![0.0, 1.0],
        vec![0.0, 1.0],
        vec![0.0, 1.0],
        vec![0.0, 10.0],
    );

    // TIGER accepts only dynamics + pitch bend; tension/breathiness are n/a.
    assert_eq!(
        curves.status(CurveKind::Dynamics, VocalVoicebank::Tiger),
        CurveStatus::Auto
    );
    assert_eq!(
        curves.status(CurveKind::PitchBend, VocalVoicebank::Tiger),
        CurveStatus::Auto
    );
    assert_eq!(
        curves.status(CurveKind::Tension, VocalVoicebank::Tiger),
        CurveStatus::Na
    );
    assert_eq!(
        curves.status(CurveKind::Breathiness, VocalVoicebank::Tiger),
        CurveStatus::Na
    );

    // Lilia accepts all four.
    for kind in CurveKind::ALL {
        assert_eq!(
            curves.status(kind, VocalVoicebank::Lilia),
            CurveStatus::Auto
        );
    }
}

#[test]
fn na_status_wins_even_when_edited() {
    let mut curves = ExpressionCurves::default();
    curves
        .curve_mut(CurveKind::Tension)
        .set_overlay(vec![Breakpoint::new(0.0, 0.5), Breakpoint::new(1.0, 0.9)]);
    assert!(curves.is_edited(CurveKind::Tension));
    // Edited but unsupported -> still reported n/a.
    assert_eq!(
        curves.status(CurveKind::Tension, VocalVoicebank::Tiger),
        CurveStatus::Na
    );
    assert_eq!(
        curves.status(CurveKind::Tension, VocalVoicebank::Lilia),
        CurveStatus::Edited
    );
}

#[test]
fn bundle_routes_each_kind_to_its_curve() {
    let curves = ExpressionCurves::from_baselines(
        vec![0.1],
        vec![0.2],
        vec![0.3],
        vec![10.0],
    );
    assert!(approx(curves.evaluate(CurveKind::Dynamics, 0.5), 0.1));
    assert!(approx(curves.evaluate(CurveKind::Tension, 0.5), 0.2));
    assert!(approx(curves.evaluate(CurveKind::Breathiness, 0.5), 0.3));
    assert!(approx(curves.evaluate(CurveKind::PitchBend, 0.5), 10.0));
    assert_eq!(curves.curve(CurveKind::Dynamics).kind(), CurveKind::Dynamics);
}

#[test]
fn default_bundle_is_untouched() {
    let curves = ExpressionCurves::default();
    assert!(!curves.any_edited());
    for kind in CurveKind::ALL {
        assert!(!curves.is_edited(kind));
        assert_eq!(curves.status(kind, VocalVoicebank::Lilia), CurveStatus::Auto);
    }
}

#[test]
fn any_edited_and_reset_all() {
    let mut curves = ExpressionCurves::from_baselines(
        vec![0.0, 1.0],
        vec![0.0, 1.0],
        vec![0.0, 1.0],
        vec![0.0, 5.0],
    );
    assert!(!curves.any_edited());

    curves
        .curve_mut(CurveKind::Dynamics)
        .add_breakpoint(0.5, 0.7);
    assert!(curves.any_edited());
    assert!(curves.is_edited(CurveKind::Dynamics));

    curves.reset(CurveKind::Dynamics);
    assert!(!curves.any_edited());

    // reset_all drops every overlay but keeps baselines.
    curves.curve_mut(CurveKind::Tension).add_breakpoint(0.3, 0.4);
    curves.curve_mut(CurveKind::Breathiness).add_breakpoint(0.6, 0.2);
    assert!(curves.any_edited());
    curves.reset_all();
    assert!(!curves.any_edited());
    assert!(approx(curves.evaluate(CurveKind::Tension, 1.0), 1.0));
}

#[test]
fn serde_round_trips_an_edited_bundle() {
    let mut curves = ExpressionCurves::from_baselines(
        vec![0.0, 0.5, 1.0],
        vec![0.2, 0.4],
        vec![],
        vec![-25.0, 25.0],
    );
    curves
        .curve_mut(CurveKind::Dynamics)
        .set_overlay(vec![Breakpoint::new(0.0, 0.1), Breakpoint::new(1.0, 0.9)]);

    let json = serde_json::to_string(&curves).expect("serialize");
    let back: ExpressionCurves = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(curves, back);
    // Behaviour survives the round-trip: edited dynamics follows overlay,
    // untouched tension follows its baseline.
    assert!(approx(back.evaluate(CurveKind::Dynamics, 0.5), 0.5));
    assert!(approx(back.evaluate(CurveKind::Tension, 1.0), 0.4));
    assert!(back.is_edited(CurveKind::Dynamics));
    assert!(!back.is_edited(CurveKind::Tension));
}
