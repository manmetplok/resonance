//! Arrange-view playhead follow (review FU-D1).
//!
//! The arrange timeline is a fixed-width canvas inside an outer horizontal
//! `Scrollable`; nothing ever scrolled it during playback, so the playhead
//! walked off the right edge. The tick now pages the `Scrollable` (via
//! `scroll_to`) when the playhead leaves the visible range — only while
//! playing, and never while the user scrolls or drags by hand.

use resonance_app::message::{ClipMessage, Message, TransportMessage, UiMessage, ViewportMessage};
use resonance_app::state::{ClipState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::FadeCurve;

const SR: u32 = 48_000;
/// 100 px per second.
const ZOOM: f32 = 100.0;
const VISIBLE_W: f32 = 1000.0;
const CONTENT_W: f32 = 20_000.0;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let _rx = app.test_capture_engine();
    app.test_set_sample_rate(SR);
    app.test_set_arrange_zoom(ZOOM);
    scrolled(&mut app, 0.0);
    app
}

/// The outer `Scrollable`'s `on_scroll` report.
fn scrolled(app: &mut Resonance, offset_x: f32) {
    app.test_dispatch(Message::Viewport(ViewportMessage::ArrangeScrolled {
        offset_x,
        visible_width: VISIBLE_W,
        content_width: CONTENT_W,
    }));
}

fn seek_seconds(app: &mut Resonance, seconds: u64) {
    app.test_dispatch(Message::Transport(TransportMessage::SeekToSample(
        seconds * SR as u64,
    )));
}

/// Tick once; `true` when the tick returned a task (the `scroll_to`).
fn tick(app: &mut Resonance) -> bool {
    app.update(Message::Tick).units() > 0
}

#[test]
fn follow_pages_when_the_playhead_leaves_the_view() {
    let mut app = app();
    app.test_set_transport_playing(true);
    // 5 s = 500 px: inside [0, 1000).
    seek_seconds(&mut app, 5);
    assert!(!tick(&mut app), "a visible playhead must not scroll");
    assert_eq!(app.test_arrange_scroll_x(), 0.0);

    // 12 s = 1200 px: past the right edge → page so it sits just inside
    // the left edge (5 % lead of the visible width).
    seek_seconds(&mut app, 12);
    assert!(tick(&mut app), "expected a scroll_to task");
    assert_eq!(app.test_arrange_scroll_x(), 1200.0 - 50.0);

    // The Scrollable echoes the requested offset: not a manual scroll.
    scrolled(&mut app, 1150.0);
    assert!(!app.test_follow_paused());
    assert!(!tick(&mut app));
}

#[test]
fn follow_catches_a_playhead_behind_the_view() {
    let mut app = app();
    scrolled(&mut app, 3000.0);
    app.test_set_transport_playing(true);
    // Loop wrap / seek back to 2 s = 200 px, left of the view.
    seek_seconds(&mut app, 2);
    assert!(tick(&mut app));
    assert_eq!(app.test_arrange_scroll_x(), 150.0);
}

#[test]
fn no_follow_while_stopped() {
    let mut app = app();
    seek_seconds(&mut app, 30);
    assert!(!tick(&mut app));
    assert_eq!(app.test_arrange_scroll_x(), 0.0);
}

#[test]
fn manual_scroll_during_playback_pauses_follow_until_stop() {
    let mut app = app();
    app.test_set_transport_playing(true);
    seek_seconds(&mut app, 5);
    tick(&mut app);
    // The user drags the scrollbar / wheels away from the playhead.
    scrolled(&mut app, 4000.0);
    assert!(app.test_follow_paused());
    assert!(!tick(&mut app), "follow must not yank the view back");
    assert_eq!(app.test_arrange_scroll_x(), 4000.0);

    // Stopping re-arms follow for the next playback.
    app.test_set_transport_playing(false);
    tick(&mut app);
    assert!(!app.test_follow_paused());
    app.test_set_transport_playing(true);
    assert!(tick(&mut app));
    assert_eq!(app.test_arrange_scroll_x(), 450.0);
}

#[test]
fn no_follow_while_dragging_a_clip() {
    let mut app = app();
    app.test_push_clip(ClipState {
        id: 7,
        track_id: 1,
        start_sample: 0,
        duration_samples: SR as u64,
        name: "clip".into(),
        total_frames: SR as u64,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
    });
    app.test_dispatch(Message::Clip(ClipMessage::StartClipDrag {
        clip_id: 7,
        grab_offset_x: 10.0,
        start_x: 10.0,
        start_y: 0.0,
    }));
    app.test_set_transport_playing(true);
    seek_seconds(&mut app, 12);
    assert!(!tick(&mut app), "content must not page away under a drag");
    assert_eq!(app.test_arrange_scroll_x(), 0.0);
}

#[test]
fn no_follow_outside_the_arrange_view() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    let _rx = app.test_capture_engine();
    app.test_set_sample_rate(SR);
    app.test_set_arrange_zoom(ZOOM);
    scrolled(&mut app, 0.0);
    app.test_set_transport_playing(true);
    seek_seconds(&mut app, 12);
    assert!(!tick(&mut app));
}

// ---------------- FU-V3b: on/off switch + resume ----------------

#[test]
fn follow_can_be_switched_off_and_the_choice_is_a_setting() {
    let mut app = app();
    assert!(app.test_settings().arrange.follow_playhead, "on by default");
    app.test_dispatch(Message::Ui(UiMessage::ToggleFollowPlayhead));
    assert!(!app.test_settings().arrange.follow_playhead);

    app.test_set_transport_playing(true);
    seek_seconds(&mut app, 12);
    assert!(!tick(&mut app), "follow is off: the view stays put");
    assert_eq!(app.test_arrange_scroll_x(), 0.0);

    app.test_dispatch(Message::Ui(UiMessage::ToggleFollowPlayhead));
    assert!(tick(&mut app), "switched back on, it pages again");
}

#[test]
fn follow_resumes_when_the_playhead_reenters_the_view() {
    let mut app = app();
    app.test_set_transport_playing(true);
    seek_seconds(&mut app, 5);
    tick(&mut app);
    // The user scrolls ahead of the playhead: follow pauses.
    scrolled(&mut app, 4000.0);
    assert!(app.test_follow_paused());

    // The playhead catches up into the visible range [4000, 5000)...
    seek_seconds(&mut app, 42);
    assert!(!tick(&mut app), "it is visible: nothing to scroll");
    assert!(!app.test_follow_paused(), "follow resumes once it is back in view");

    // ...so when it runs off the right edge, the view pages with it.
    seek_seconds(&mut app, 52);
    assert!(tick(&mut app));
    assert_eq!(app.test_arrange_scroll_x(), 5200.0 - 50.0);
}
