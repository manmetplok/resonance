//! Pure-geometry tests for the timeline fade/gain/crossfade rendering
//! helpers (todo #320, design doc #153, arch doc #156).
//!
//! These cover the allocation-free math the Canvas draw path relies on —
//! the gain-tint colour ramp, the signed dB header tag, the clip-overlap
//! range that derives the automatic crossfade, and the fade-ramp envelope
//! polyline — without going through `wgpu`, so they are deterministic and
//! environment-independent (the golden-image lock lives in the snapshot
//! test alongside).

use resonance_app::view::timeline::draw::{
    fade_envelope, format_gain_db, gain_tinted_body, overlap_range,
};
use resonance_audio::types::FadeCurve;

const TOL: f32 = 1e-4;

#[test]
fn gain_tint_brightens_when_louder_darkens_when_quieter() {
    let unity = gain_tinted_body(0.0).a;
    let loud = gain_tinted_body(9.0).a;
    let quiet = gain_tinted_body(-9.0).a;

    assert!((unity - 0.10).abs() < TOL, "unity sits at the base wash");
    assert!(loud > unity, "louder => brighter (higher alpha)");
    assert!(quiet < unity, "quieter => darker (lower alpha)");
}

#[test]
fn gain_tint_clamps_to_a_readable_band() {
    // Extreme gains stay within a readable alpha band so the wash never
    // washes out or vanishes: ±18 dB saturates the ±0.08 swing around the
    // 0.10 base, and the lower end is floored at 0.03.
    let very_loud = gain_tinted_body(120.0).a;
    let very_quiet = gain_tinted_body(-120.0).a;
    assert!((very_loud - 0.18).abs() < TOL, "loud saturates at 0.18");
    assert!((very_quiet - 0.03).abs() < TOL, "quiet floors at 0.03");
}

#[test]
fn gain_tag_is_signed_and_collapses_unity() {
    assert_eq!(format_gain_db(3.0), "+3.0 dB");
    assert_eq!(format_gain_db(-6.0), "-6.0 dB");
    assert_eq!(format_gain_db(0.0), "+0.0 dB");
    // A hair off unity still reads as +0.0 (never "-0.0").
    assert_eq!(format_gain_db(-0.01), "+0.0 dB");
    assert_eq!(format_gain_db(12.34), "+12.3 dB");
}

#[test]
fn overlap_range_detects_and_bounds_the_seam() {
    // [0, 100) and [80, 200) overlap on [80, 100).
    assert_eq!(overlap_range(0, 100, 80, 120), Some((80, 100)));
    // Order independent.
    assert_eq!(overlap_range(80, 120, 0, 100), Some((80, 100)));
    // Containment: inner clip fully inside the outer.
    assert_eq!(overlap_range(0, 100, 20, 30), Some((20, 50)));
}

#[test]
fn overlap_range_rejects_disjoint_and_touching_clips() {
    // Fully disjoint.
    assert_eq!(overlap_range(0, 50, 100, 50), None);
    // Edge-touching (a ends exactly where b starts) is not an overlap.
    assert_eq!(overlap_range(0, 100, 100, 50), None);
}

#[test]
fn fade_in_envelope_runs_silence_to_unity() {
    let env = fade_envelope(FadeCurve::Linear, 10.0, 40.0, 100.0, 80.0, true);
    assert_eq!(env.len(), 17, "16 segments => 17 points");

    let first = env.first().unwrap();
    let last = env.last().unwrap();
    // Left edge: silence => amplitude 0 => bottom of the clip body.
    assert!((first.x - 10.0).abs() < TOL);
    assert!((first.y - 180.0).abs() < TOL, "silence sits at the bottom");
    // Ramp end: unity => top of the clip body.
    assert!((last.x - 50.0).abs() < TOL);
    assert!((last.y - 100.0).abs() < TOL, "unity sits at the top");

    // x increases left->right; y rises (decreases) toward unity.
    for w in env.windows(2) {
        assert!(w[1].x >= w[0].x - TOL, "x is monotonic across the ramp");
        assert!(w[1].y <= w[0].y + TOL, "fade-in rises toward the top");
    }
}

#[test]
fn fade_out_envelope_runs_unity_to_silence() {
    let env = fade_envelope(FadeCurve::Linear, 10.0, 40.0, 100.0, 80.0, false);
    let first = env.first().unwrap();
    let last = env.last().unwrap();
    // Left edge: unity (fade-out begins) => top.
    assert!((first.y - 100.0).abs() < TOL, "fade-out starts at unity");
    // Right edge: silence => bottom.
    assert!((last.y - 180.0).abs() < TOL, "fade-out ends in silence");

    for w in env.windows(2) {
        assert!(w[1].y >= w[0].y - TOL, "fade-out falls toward the bottom");
    }
}

#[test]
fn equal_power_envelope_curves_above_the_linear_chord() {
    // Equal-power fade-in is convex: at the midpoint its amplitude
    // (sin(45°) ≈ 0.707) is well above linear's 0.5, so its midpoint y
    // sits higher (smaller) than the linear ramp's midpoint.
    let eq = fade_envelope(FadeCurve::EqualPower, 0.0, 16.0, 0.0, 100.0, true);
    let lin = fade_envelope(FadeCurve::Linear, 0.0, 16.0, 0.0, 100.0, true);
    let mid = eq.len() / 2;
    assert!(
        eq[mid].y < lin[mid].y - 1.0,
        "equal-power rises faster than linear at the midpoint"
    );
}
