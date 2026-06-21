//! Hit-testing coverage for the audio-clip fade/gain handle beads added in
//! todo #318 (design doc #153, arch doc #156).
//!
//! The fade handles are circular beads on the clip's two top corners (handle
//! x = ramp end) with a ~10px hit radius — deliberately larger than the 6px
//! `CLIP_EDGE_THRESHOLD` so fade wins at the very top corner while trim keeps
//! the rest of the vertical edge. The gain bead sits at the clip's
//! top-centre and stays available even when the clip is non-fadeable
//! (frozen / no source); fade hits are suppressed in that case.

use iced::{Point, Rectangle};
use resonance_app::state::ClipEdge;
use resonance_app::theme::CLIP_EDGE_THRESHOLD;
use resonance_app::view::timeline::hit_test::{
    audio_clip_handles, hit_test_audio, HitKind, FADE_HANDLE_RADIUS, GAIN_HANDLE_RADIUS,
};

/// A clip rect well clear of the origin so corner math is unambiguous.
fn rect() -> Rectangle {
    Rectangle {
        x: 100.0,
        y: 40.0,
        width: 400.0,
        height: 90.0,
    }
}

const ZOOM: f32 = 100.0; // px per second

#[test]
fn zero_length_fades_put_handles_at_top_corners() {
    let r = rect();
    let h = audio_clip_handles(r, 0.0, 0.0, ZOOM, true);
    assert_eq!(h.fade_in_x, r.x, "fade-in bead at the left edge when fade=0");
    assert_eq!(
        h.fade_out_x,
        r.x + r.width,
        "fade-out bead at the right edge when fade=0"
    );
    assert_eq!(h.gain_x, r.x + r.width / 2.0, "gain bead at top-centre");
    assert_eq!(h.top_y, r.y, "beads ride the clip's top edge");
}

#[test]
fn fade_handles_ride_inward_with_length() {
    let r = rect();
    // 0.5s fade-in at 100 px/s -> handle 50 px in from the left edge.
    let h = audio_clip_handles(r, 0.5, 0.25, ZOOM, true);
    assert_eq!(h.fade_in_x, r.x + 50.0);
    assert_eq!(h.fade_out_x, r.x + r.width - 25.0);
}

#[test]
fn over_long_fades_clamp_within_clip() {
    let r = rect();
    // 10s fade on a 4s clip would overshoot — must clamp inside the rect.
    let h = audio_clip_handles(r, 10.0, 10.0, ZOOM, true);
    assert!(h.fade_in_x >= r.x && h.fade_in_x <= r.x + r.width);
    assert!(h.fade_out_x >= r.x && h.fade_out_x <= r.x + r.width);
}

#[test]
fn top_corner_hits_fade_not_trim() {
    let r = rect();
    let h = audio_clip_handles(r, 0.0, 0.0, ZOOM, true);
    // Right on the top-left corner -> fade-in (bead wins over trim).
    assert_eq!(
        hit_test_audio(Point::new(r.x, r.y), r, CLIP_EDGE_THRESHOLD, &h),
        HitKind::FadeIn
    );
    // Top-right corner -> fade-out.
    assert_eq!(
        hit_test_audio(
            Point::new(r.x + r.width, r.y),
            r,
            CLIP_EDGE_THRESHOLD,
            &h
        ),
        HitKind::FadeOut
    );
}

#[test]
fn left_edge_below_bead_is_trim_not_fade() {
    let r = rect();
    let h = audio_clip_handles(r, 0.0, 0.0, ZOOM, true);
    // On the left vertical edge, but below the fade bead's radius: trim.
    let y = r.y + FADE_HANDLE_RADIUS + 5.0;
    assert_eq!(
        hit_test_audio(Point::new(r.x + 1.0, y), r, CLIP_EDGE_THRESHOLD, &h),
        HitKind::Trim(ClipEdge::Left)
    );
}

#[test]
fn top_centre_hits_gain() {
    let r = rect();
    let h = audio_clip_handles(r, 0.0, 0.0, ZOOM, true);
    assert_eq!(
        hit_test_audio(
            Point::new(r.x + r.width / 2.0, r.y),
            r,
            CLIP_EDGE_THRESHOLD,
            &h
        ),
        HitKind::Gain
    );
}

#[test]
fn clip_body_is_move() {
    let r = rect();
    let h = audio_clip_handles(r, 0.0, 0.0, ZOOM, true);
    // Centre of the clip, away from every bead and edge.
    let hit = hit_test_audio(
        Point::new(r.x + r.width / 2.0, r.y + r.height / 2.0),
        r,
        CLIP_EDGE_THRESHOLD,
        &h,
    );
    assert!(matches!(hit, HitKind::Move { .. }), "got {hit:?}");
}

#[test]
fn non_fadeable_clip_exposes_no_fade_hits_but_keeps_gain() {
    let r = rect();
    let h = audio_clip_handles(r, 0.0, 0.0, ZOOM, false);
    // Top-left corner of a frozen clip falls through fade -> trim.
    assert_eq!(
        hit_test_audio(Point::new(r.x, r.y), r, CLIP_EDGE_THRESHOLD, &h),
        HitKind::Trim(ClipEdge::Left)
    );
    // Gain still works on a frozen clip (design: gain still applies).
    assert_eq!(
        hit_test_audio(
            Point::new(r.x + r.width / 2.0, r.y),
            r,
            CLIP_EDGE_THRESHOLD,
            &h
        ),
        HitKind::Gain
    );
}

#[test]
fn gain_radius_bounds_the_bead() {
    let r = rect();
    let h = audio_clip_handles(r, 0.0, 0.0, ZOOM, true);
    let cx = r.x + r.width / 2.0;
    // Just outside the gain radius horizontally -> not gain (body move).
    let hit = hit_test_audio(
        Point::new(cx + GAIN_HANDLE_RADIUS + 2.0, r.y + 2.0),
        r,
        CLIP_EDGE_THRESHOLD,
        &h,
    );
    assert!(matches!(hit, HitKind::Move { .. }), "got {hit:?}");
}

#[test]
fn miss_outside_clip() {
    let r = rect();
    let h = audio_clip_handles(r, 0.0, 0.0, ZOOM, true);
    assert_eq!(
        hit_test_audio(
            Point::new(r.x - 50.0, r.y + r.height / 2.0),
            r,
            CLIP_EDGE_THRESHOLD,
            &h
        ),
        HitKind::Miss
    );
}
