//! One beat unit for positions in and out (code review CTL-01).
//!
//! Every `SongPosition` the app reports counts `beat` in the bar's
//! time-signature beat (an eighth in 6/8), so every `PositionSpec` it
//! accepts must resolve `beat` the same way: a position read back from
//! the app and handed straight to `transport.seek` has to land on the
//! same sample, in any meter and across a signature change.

use resonance_app::state::{ClipState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::{FadeCurve, TrackType};
use resonance_control::methods::clip::SplitResult;
use resonance_control::methods::transport::TransportResult;
use resonance_control::{ErrorKind, SongPosition};

use crate::common::call;

const SR: u32 = 48_000;
/// 120 BPM: a quarter is half a second.
const QUARTER: u64 = SR as u64 / 2;
const EIGHTH: u64 = QUARTER / 2;
/// A 4/4 bar and a 6/8 bar at 120 BPM.
const BAR_4_4: u64 = 4 * QUARTER;
const BAR_6_8: u64 = 6 * EIGHTH;
const AUDIO: u64 = 40;
const TAKE: u64 = 7;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-position-beats.rprj"));
    app.test_set_sample_rate(SR);
    app.test_set_flat_tempo(120.0);
    app
}

fn app_in_6_8() -> Resonance {
    let mut app = app();
    let _: TransportResult = call(
        &mut app,
        "transport.set_time_signature",
        serde_json::json!({ "numerator": 6, "denominator": 8 }),
    )
    .result()
    .expect("set_time_signature succeeds");
    app
}

fn seek(app: &mut Resonance, params: serde_json::Value) -> SongPosition {
    call(app, "transport.seek", params)
        .result::<TransportResult>()
        .expect("seek succeeds")
        .playhead
}

/// Seek by sample, read the reported bar/beat, seek back to exactly that
/// musical position: the playhead must not move.
fn assert_round_trips(app: &mut Resonance, sample: u64) -> SongPosition {
    let read = seek(app, serde_json::json!({ "sample": sample }));
    let back = seek(app, serde_json::json!({ "bar": read.bar, "beat": read.beat }));
    assert_eq!(
        back.sample, sample,
        "bar {} beat {} was read at sample {sample} but resolves to {}",
        read.bar, read.beat, back.sample
    );
    assert_eq!((back.bar, back.beat), (read.bar, read.beat));
    read
}

#[test]
fn a_6_8_beat_is_an_eighth_note() {
    let mut app = app_in_6_8();
    // Bar 2 beat 4 is the 4th eighth of bar 2 — mid-bar, not bar 3.
    let pos = seek(&mut app, serde_json::json!({ "bar": 2, "beat": 4.0 }));
    assert_eq!(pos.sample, BAR_6_8 + 3 * EIGHTH);
    assert_eq!((pos.bar, pos.beat), (2, 4.0));
}

#[test]
fn a_reported_6_8_position_seeks_back_to_the_same_sample() {
    let mut app = app_in_6_8();
    let read = assert_round_trips(&mut app, BAR_6_8 + 5 * EIGHTH);
    assert_eq!((read.bar, read.beat), (2, 6.0));
    // A fractional beat round-trips too.
    let read = assert_round_trips(&mut app, 3 * BAR_6_8 + EIGHTH + EIGHTH / 2);
    assert_eq!((read.bar, read.beat), (4, 2.5));
}

#[test]
fn positions_follow_a_4_4_to_6_8_signature_change() {
    let mut app = app();
    // Bars 1-2 in 4/4, 6/8 from bar 3 on.
    call(
        &mut app,
        "global.add_signature_event",
        serde_json::json!({ "bar": 3, "numerator": 6, "denominator": 8 }),
    )
    .result::<serde_json::Value>()
    .expect("add_signature_event succeeds");

    // Before the change a beat is a quarter...
    let pos = seek(&mut app, serde_json::json!({ "bar": 2, "beat": 3.0 }));
    assert_eq!(pos.sample, BAR_4_4 + 2 * QUARTER);
    // ...after it, an eighth.
    let pos = seek(&mut app, serde_json::json!({ "bar": 3, "beat": 5.0 }));
    assert_eq!(pos.sample, 2 * BAR_4_4 + 4 * EIGHTH);
    let pos = seek(&mut app, serde_json::json!({ "bar": 4, "beat": 2.0 }));
    assert_eq!(pos.sample, 2 * BAR_4_4 + BAR_6_8 + EIGHTH);

    assert_round_trips(&mut app, BAR_4_4 + 3 * QUARTER);
    assert_round_trips(&mut app, 2 * BAR_4_4 + 5 * EIGHTH);
}

#[test]
fn a_beat_past_the_bar_line_is_rejected() {
    let mut app = app_in_6_8();
    // 6/8 has six beats: beat 7 (or 6.99 + anything reaching 7) is the
    // next bar's downbeat and must be written as such.
    for beat in [7.0, 9.5] {
        let response = call(
            &mut app,
            "transport.seek",
            serde_json::json!({ "bar": 2, "beat": beat }),
        );
        assert_eq!(
            response.error.expect("beat past the bar rejected").kind(),
            ErrorKind::InvalidParams,
            "beat {beat}"
        );
    }
    // The last eighth of the bar is fine.
    let pos = seek(&mut app, serde_json::json!({ "bar": 2, "beat": 6.5 }));
    assert_eq!(pos.sample, BAR_6_8 + 5 * EIGHTH + EIGHTH / 2);
}

#[test]
fn loop_set_and_clip_split_use_the_same_beat() {
    let mut app = app_in_6_8();
    let _: TransportResult = call(
        &mut app,
        "transport.loop_set",
        serde_json::json!({
            "start": { "bar": 2, "beat": 4.0 },
            "end": { "bar": 3, "beat": 4.0 },
            "enabled": true
        }),
    )
    .result()
    .expect("loop_set succeeds");
    let (loop_in, loop_out, _) = app.test_loop_range();
    assert_eq!(loop_in, BAR_6_8 + 3 * EIGHTH);
    assert_eq!(loop_out, 2 * BAR_6_8 + 3 * EIGHTH);

    app.test_add_track(AUDIO, TrackType::Audio);
    app.test_push_clip(ClipState {
        id: TAKE,
        track_id: AUDIO,
        start_sample: 0,
        duration_samples: 8 * BAR_6_8,
        name: "take".into(),
        total_frames: 8 * BAR_6_8,
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
    });
    let split: SplitResult = call(
        &mut app,
        "clip.split",
        serde_json::json!({ "clip_id": TAKE, "at": { "bar": 5, "beat": 4.0 } }),
    )
    .result()
    .expect("clip.split succeeds");
    assert_eq!(split.head_length_samples, 4 * BAR_6_8 + 3 * EIGHTH, "cut mid-bar 5");
}
