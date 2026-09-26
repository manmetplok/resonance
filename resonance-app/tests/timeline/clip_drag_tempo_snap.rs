//! VIEW-22: clip drags snap to the project's tempo map, the same grid the
//! ruler draws. The drag reducers used to snap against
//! `TempoMap::default()` (denominator 4), so in 6/8 a clip snapped to
//! every other drawn bar.

use resonance_app::message::{ClipMessage, Message, TransportMessage};
use resonance_app::state::ClipState;
use resonance_app::Resonance;
use resonance_audio::types::FadeCurve;

const SR: u32 = 48_000;
/// 20 px/s: a 6/8 bar at 120 BPM (1.5 s) is 30 px wide, so the grid is
/// whole bars.
const ZOOM: f32 = 20.0;

fn clip() -> ClipState {
    ClipState {
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
    }
}

#[test]
fn clip_drag_snaps_to_six_eight_bars() {
    let (mut app, _task) = Resonance::new_for_test();
    let _rx = app.test_capture_engine();
    app.test_set_sample_rate(SR);
    app.test_set_flat_tempo(120.0);
    app.test_dispatch(Message::Transport(TransportMessage::SetTimeSignature {
        numerator: 6,
        denominator: 8,
    }));
    assert_eq!(app.test_transport_time_sig(), (6, 8), "precondition: 6/8");
    app.test_set_arrange_zoom(ZOOM);
    app.test_push_clip(clip());

    app.test_dispatch(Message::Clip(ClipMessage::StartClipDrag {
        clip_id: 7,
        grab_offset_x: 0.0,
        start_x: 0.0,
        start_y: 0.0,
    }));
    // 1.2 s: nearest 6/8 bar line is bar 2 at 1.5 s.
    app.test_dispatch(Message::Clip(ClipMessage::UpdateClipDrag(1.2 * ZOOM, 0.0)));

    let start = app.test_clips().iter().find(|c| c.id == 7).unwrap().start_sample;
    assert_eq!(start, app.test_tempo_map().bar_to_sample(1));
    assert_eq!(start, 72_000);
}
