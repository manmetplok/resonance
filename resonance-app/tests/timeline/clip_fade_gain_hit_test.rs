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
use resonance_app::message::{ClipMessage, Message};
use resonance_app::state::{ClipEdge, ClipState};
use resonance_app::theme::{self, CLIP_EDGE_THRESHOLD};
use resonance_app::view::timeline::hit_test::{
    audio_clip_handles, hit_test_audio, HitKind, FADE_HANDLE_RADIUS, GAIN_HANDLE_RADIUS,
};
use resonance_app::view::timeline::TimelineState;
use resonance_app::Resonance;
use resonance_audio::types::{FadeCurve, TrackType};
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

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

// ---------------------------------------------------------------------
// BUG-14: `TimelineCanvas::clip_handles` used to hardcode 0-length fades
// and `fadeable = true` instead of reading the real `ClipState` /
// `frozen_tracks`. These drive a real press through the canvas's own
// `canvas::Program::update` (`test_timeline_canvas_event`), the code the
// live pointer runs, to pin that the handle geometry now comes from the
// clip's actual fade lengths and the track's actual frozen state.
// ---------------------------------------------------------------------

const SR: u32 = 48_000;
const PRESS_ZOOM: f32 = 100.0; // px per second
const TRACK: u64 = 1;
const CLIP: u64 = 7;

fn press_clip(fade_in_frames: u64) -> ClipState {
    ClipState {
        id: CLIP,
        track_id: TRACK,
        start_sample: 0,
        duration_samples: 2 * SR as u64,
        name: "clip".into(),
        total_frames: 2 * SR as u64,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
        warp: Default::default(),
    }
}

fn press_app(fade_in_frames: u64) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_sample_rate(SR);
    app.test_set_arrange_zoom(PRESS_ZOOM);
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_push_clip(press_clip(fade_in_frames));
    app
}

/// Canvas y of the clip body's top edge, matching `draw::clip_lane_rect`.
fn clip_top_y(app: &Resonance) -> f32 {
    let layout = app.test_arrange_row_layout();
    let (y_top, _height) = layout.track_row_rect(TRACK).expect("track row");
    app.test_arrange_header_offset() + y_top + theme::CLIP_LANE_INSET
}

fn left_press() -> iced::Event {
    iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
}

fn frozen_cache_ref() -> FreezeCacheRef {
    FreezeCacheRef::new("freeze_1.wav".to_string(), SR, 32, 1, FreezeCacheStatus::Frozen)
}

/// A 0.3 s fade-in (30 px at this zoom) puts the real fade-handle bead
/// well clear of the clip's left edge. Before the fix the hit test
/// always assumed a 0-length fade, so a press at the real boundary fell
/// through to a body move instead of grabbing the fade handle.
#[test]
fn fade_handle_hit_test_uses_the_clips_real_fade_length() {
    let fade_frames = (0.3 * SR as f32) as u64;
    let app = press_app(fade_frames);
    let y = clip_top_y(&app);
    let mut state = TimelineState::default();
    let msg = app
        .test_timeline_canvas_event(&mut state, &left_press(), 30.0, y)
        .expect("the press publishes");
    assert!(
        matches!(
            msg,
            Message::Clip(ClipMessage::StartClipFadeDrag {
                clip_id: CLIP,
                edge: ClipEdge::Left,
                ..
            })
        ),
        "a press at the clip's real fade-in boundary must grab the fade handle: {msg:?}"
    );
}

/// With no fade, the same corner press is a plain left-edge trim.
#[test]
fn a_press_short_of_the_fade_boundary_is_not_a_fade_hit() {
    let app = press_app(0);
    let y = clip_top_y(&app);
    let mut state = TimelineState::default();
    let msg = app
        .test_timeline_canvas_event(&mut state, &left_press(), 30.0, y)
        .expect("the press publishes");
    assert!(
        !matches!(msg, Message::Clip(ClipMessage::StartClipFadeDrag { .. })),
        "a zero-length fade has no handle 30px in: {msg:?}"
    );
}

/// A frozen track (playing back from its freeze cache) has no editable
/// sample source, so it is not fadeable: the corner press must trim, not
/// grab a fade handle that design #153 says should not exist there.
#[test]
fn a_frozen_tracks_clip_exposes_no_fade_handle() {
    let mut app = press_app(0);
    app.test_set_freeze_status(
        TRACK,
        resonance_app::state::FreezeStatus::Frozen { cache_ref: frozen_cache_ref() },
    );
    let y = clip_top_y(&app);
    let mut state = TimelineState::default();
    let msg = app
        .test_timeline_canvas_event(&mut state, &left_press(), 0.0, y)
        .expect("the press publishes");
    assert!(
        matches!(
            msg,
            Message::Clip(ClipMessage::StartClipTrim {
                clip_id: CLIP,
                edge: ClipEdge::Left,
                ..
            })
        ),
        "a frozen clip's corner must trim, not fade: {msg:?}"
    );
}

/// The same corner on an idle (non-frozen) track still exposes the fade
/// handle — the control case for the test above.
#[test]
fn the_same_corner_on_an_idle_track_still_exposes_the_fade_handle() {
    let app = press_app(0);
    let y = clip_top_y(&app);
    let mut state = TimelineState::default();
    let msg = app
        .test_timeline_canvas_event(&mut state, &left_press(), 0.0, y)
        .expect("the press publishes");
    assert!(
        matches!(
            msg,
            Message::Clip(ClipMessage::StartClipFadeDrag {
                clip_id: CLIP,
                edge: ClipEdge::Left,
                ..
            })
        ),
        "an idle clip's corner is still the fade handle: {msg:?}"
    );
}
