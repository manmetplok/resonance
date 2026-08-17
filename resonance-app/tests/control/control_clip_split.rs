//! `clip.split`, and audio clips on external-instrument tracks (ba doc
//! #275 P2).
//!
//! Three hardware performances came back from a session as one
//! continuous audio clip each, spanning bars 1–57 on external-instrument
//! tracks. `clip.trim` worked on them, so a take could be SHORTENED —
//! but `clip.place` refused the track ("audio clips need an audio
//! track") and there was no split at all, so a take could never be cut
//! into pieces, copied, or re-placed. Duplicating an 8-bar section was
//! impossible over the API; the session ended up not matching the
//! delivered audio.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::{ClipState, ViewMode};
use resonance_app::{Resonance};
use resonance_audio::types::{FadeCurve, TrackType};
use resonance_control::methods::clip::{self as proto, SplitParams, SplitResult};
use resonance_control::{PositionSpec, Request, Response};

const AUDIO: u64 = 40;
const EXTERNAL: u64 = 41;
const SR: u32 = 48_000;
/// 4/4 at 120 BPM.
const BAR: u64 = 2 * SR as u64;
const TAKE: u64 = 7;

fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-clip-split.rprj"));
    app.test_set_sample_rate(SR);
    app.test_set_flat_tempo(120.0);
    app.test_add_track(AUDIO, TrackType::Audio);
    app.test_add_track(EXTERNAL, TrackType::Instrument);
    app
}

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let request = Request::new(1, method, params).expect("params serialize");
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request,
        reply,
    })));
    rx.try_recv().expect("every request gets exactly one reply")
}

/// An 8-bar take on `track`, like a recorded hardware performance.
fn push_take(app: &mut Resonance, track_id: u64) {
    app.test_push_clip(ClipState {
        id: TAKE,
        track_id,
        start_sample: 0,
        duration_samples: 8 * BAR,
        name: "Muse take".into(),
        total_frames: 8 * BAR,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 480,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 480,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
    });
}

fn split_at_bar(app: &mut Resonance, bar: u32) -> Response {
    call(
        app,
        proto::SPLIT,
        &SplitParams {
            clip_id: resonance_control::ids::ClipId(TAKE),
            at: PositionSpec::musical(bar, 1.0),
        },
    )
}

fn clip(app: &Resonance, id: u64) -> Option<ClipState> {
    app.test_clips().iter().find(|c| c.id == id).cloned()
}

/// The cut lands where asked and the two halves together are the whole.
#[test]
fn splitting_a_take_yields_two_halves_that_cover_the_original() {
    let mut app = app_with_project();
    push_take(&mut app, AUDIO);

    let result = split_at_bar(&mut app, 5)
        .result::<SplitResult>()
        .expect("clip.split succeeds");

    assert_eq!(u64::from(result.head_clip_id), TAKE, "the head keeps its id");
    assert_eq!(result.head_length_samples, 4 * BAR);
    assert_eq!(result.tail_length_samples, 4 * BAR);

    let head = clip(&app, TAKE).expect("the head is still there");
    let tail = clip(&app, u64::from(result.tail_clip_id)).expect("the tail resolves immediately");

    assert_eq!(head.start_sample, 0);
    assert_eq!(head.duration_samples, 4 * BAR);
    assert_eq!(tail.start_sample, 4 * BAR, "the tail starts at the cut");
    assert_eq!(tail.duration_samples, 4 * BAR);
    assert_eq!(
        head.duration_samples + tail.duration_samples,
        8 * BAR,
        "nothing is lost or duplicated across the cut"
    );
    assert_eq!(
        tail.trim_start_frames, 4 * BAR,
        "the tail plays the SAME source from the split point"
    );
}

/// Fades follow the audible edges: a fade-out left on the head would duck
/// the middle of what was one continuous performance.
#[test]
fn the_cut_edges_carry_no_fade() {
    let mut app = app_with_project();
    push_take(&mut app, AUDIO);

    let result = split_at_bar(&mut app, 5)
        .result::<SplitResult>()
        .expect("clip.split succeeds");

    let head = clip(&app, TAKE).unwrap();
    let tail = clip(&app, u64::from(result.tail_clip_id)).unwrap();
    assert_eq!(head.fade_in_frames, 480, "the head keeps the original fade-in");
    assert_eq!(head.fade_out_frames, 0, "and gets no fade at the cut");
    assert_eq!(tail.fade_in_frames, 0, "nor does the tail");
    assert_eq!(
        tail.fade_out_frames, 480,
        "the tail keeps the original fade-out"
    );
}

/// A cut at or past an edge would make an empty half; refuse rather than
/// hand back a zero-length clip the caller has to notice.
#[test]
fn a_split_outside_the_clip_is_refused() {
    let mut app = app_with_project();
    push_take(&mut app, AUDIO);

    assert!(
        split_at_bar(&mut app, 1).result::<SplitResult>().is_err(),
        "a split at the clip's start is refused"
    );
    assert!(
        split_at_bar(&mut app, 9).result::<SplitResult>().is_err(),
        "a split at the clip's end is refused"
    );
    assert!(
        split_at_bar(&mut app, 20).result::<SplitResult>().is_err(),
        "a split past the clip is refused"
    );
    assert_eq!(
        app.test_clips().len(),
        1,
        "and none of them created anything"
    );
}

/// Splitting records ONE undo entry — the reason it is safe to use on a
/// take that cannot be re-recorded. (The restore itself replays a
/// project snapshot, which needs real audio files on disk; this asserts
/// the classification, which is what the split adds.)
#[test]
fn a_split_records_one_undo_entry() {
    let mut app = app_with_project();
    push_take(&mut app, AUDIO);
    assert!(!app.test_can_undo(), "nothing to undo yet");

    split_at_bar(&mut app, 5)
        .result::<SplitResult>()
        .expect("clip.split succeeds");

    assert!(app.test_can_undo(), "a split is undoable");
    assert_eq!(
        app.test_undo_history().undo_label(),
        Some("clip edit"),
        "and is recorded as one clip edit"
    );
}

/// An external-instrument track holds recorded audio, so it must take a
/// split like any other take-bearing track — this is the case the field
/// report was actually blocked on.
#[test]
fn an_external_instrument_takes_a_split() {
    let mut app = app_with_project();
    let _ = app.update(Message::ExternalInstrument(
        resonance_app::message::ExternalInstrumentMessage::Enable(EXTERNAL),
    ));
    push_take(&mut app, EXTERNAL);

    let result = split_at_bar(&mut app, 5)
        .result::<SplitResult>()
        .expect("splitting a hardware take succeeds");
    let tail = clip(&app, u64::from(result.tail_clip_id)).expect("the tail exists");
    assert_eq!(tail.track_id, EXTERNAL, "and stays on the external track");
}
