//! Clip / fade / loop drags after playback auto-follow (review VIEW-10).
//!
//! The arrange canvas works in content coordinates — horizontal scroll is
//! owned by the outer `Scrollable` and the canvas's own offset is pinned
//! to 0 — but the drag reducers still added `viewport.scroll_offset`,
//! which playback auto-follow kept writing. Play far enough into the
//! song, stop, and the next clip / fade / loop drag jumped sideways by
//! that stale offset.

use resonance_app::message::{ClipMessage, Message, TransportMessage};
use resonance_app::state::{ClipEdge, ClipState, LoopDragTarget};
use resonance_app::Resonance;
use resonance_audio::types::FadeCurve;

const SR: u32 = 48_000;
const ZOOM: f32 = 100.0;

fn clip() -> ClipState {
    ClipState {
        id: 7,
        track_id: 1,
        start_sample: 0,
        duration_samples: 2 * SR as u64,
        name: "clip".into(),
        total_frames: 2 * SR as u64,
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
        warp: Default::default(),
    }
}

fn fresh() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    let _rx = app.test_capture_engine();
    app.test_set_sample_rate(SR);
    app.test_set_arrange_zoom(ZOOM);
    app.test_push_clip(clip());
    app
}

/// `fresh()` after playing 60 s into the song and stopping: auto-follow
/// ran on the ticks in between.
fn after_playback() -> Resonance {
    let mut app = fresh();
    app.test_dispatch(Message::Transport(TransportMessage::SeekToSample(
        60 * SR as u64,
    )));
    app.test_set_transport_playing(true);
    app.test_dispatch(Message::Tick);
    app.test_dispatch(Message::Tick);
    app.test_set_transport_playing(false);
    app
}

fn clip_start(app: &Resonance) -> u64 {
    app.test_clips().iter().find(|c| c.id == 7).unwrap().start_sample
}

#[test]
fn clip_drag_does_not_jump_after_auto_follow() {
    let mut app = after_playback();
    app.test_dispatch(Message::Clip(ClipMessage::StartClipDrag {
        clip_id: 7,
        grab_offset_x: 50.0,
        start_x: 50.0,
        start_y: 0.0,
    }));
    app.test_dispatch(Message::Clip(ClipMessage::UpdateClipDrag(50.0, 0.0)));
    assert_eq!(clip_start(&app), 0, "a zero-delta drag must not move the clip");
}

#[test]
fn fade_drag_does_not_jump_after_auto_follow() {
    let mut app = after_playback();
    app.test_dispatch(Message::Clip(ClipMessage::StartClipFadeDrag {
        clip_id: 7,
        edge: ClipEdge::Left,
        anchor_x: 0.0,
    }));
    // x = 50 px at 100 px/s = 0.5 s.
    app.test_dispatch(Message::Clip(ClipMessage::UpdateClipFadeDrag(50.0)));
    let fade = app.test_clips().iter().find(|c| c.id == 7).unwrap().fade_in_frames;
    assert_eq!(fade, 24_000);
}

#[test]
fn loop_drag_does_not_jump_after_auto_follow() {
    let drag = |app: &mut Resonance| {
        app.test_dispatch(Message::Transport(TransportMessage::StartLoopDrag(
            LoopDragTarget::In,
        )));
        app.test_dispatch(Message::Transport(TransportMessage::UpdateLoopDrag(100.0)));
        app.test_loop_range().0
    };
    let expected = drag(&mut fresh());
    assert!(expected <= 2 * SR as u64, "baseline lands near 1 s, got {expected}");
    assert_eq!(drag(&mut after_playback()), expected);
}
