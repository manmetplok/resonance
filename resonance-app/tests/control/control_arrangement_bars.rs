//! `arrangement.insert_bars` / `remove_bars` (ba doc #275 P2).
//!
//! Duplicating an 8-bar section mid-song is trivial in the GUI and used
//! to be impossible over the API: every editing method addresses ONE
//! object, so restructuring meant moving ~130 audio clips, 22 MIDI clips
//! and 6 placements by hand, in separate transactions, with nothing to
//! tell you if one was missed. The field report gave up and spliced the
//! WAV outside the project, which left the session no longer matching the
//! delivered audio.
//!
//! These tests drive the real control dispatch, so they cover the wire
//! shape, the confirm gate and the actual mutation together.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::{ClipState, MidiClipState, ViewMode};
use resonance_app::{Resonance};
use resonance_control::methods::arrangement::{
    self as proto, InsertBarsParams, RemoveBarsParams, ShiftResult,
};
use resonance_audio::types::FadeCurve;
use resonance_control::{Request, Response};

const AUDIO: u64 = 40;
const MIDI: u64 = 41;
const SR: u32 = 48_000;
/// 4/4 at 120 BPM: one bar is two seconds.
const BAR: u64 = 2 * SR as u64;

fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-arrangement.rprj"));
    app.test_set_sample_rate(SR);
    app.test_set_flat_tempo(120.0);
    app.test_add_track(AUDIO, resonance_audio::types::TrackType::Audio);
    app.test_add_track(MIDI, resonance_audio::types::TrackType::Instrument);
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

fn push_audio_clip(app: &mut Resonance, id: u64, start_bar: u64) {
    app.test_push_clip(ClipState {
        id,
        track_id: AUDIO,
        start_sample: (start_bar - 1) * BAR,
        duration_samples: BAR,
        name: format!("take {id}"),
        total_frames: BAR,
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
}

fn push_midi_clip(app: &mut Resonance, id: u64, start_bar: u64) {
    app.test_push_midi_clip(MidiClipState {
        id,
        track_id: MIDI,
        start_sample: (start_bar - 1) * BAR,
        duration_ticks: 4 * 960,
        name: format!("part {id}"),
        notes: Vec::new(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
}

fn clip_start(app: &Resonance, id: u64) -> Option<u64> {
    app.test_clips()
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.start_sample)
}

fn midi_start(app: &Resonance, id: u64) -> Option<u64> {
    app.test_midi_clips()
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.start_sample)
}

fn insert(app: &mut Resonance, at_bar: u32, count: u32) -> ShiftResult {
    call(
        app,
        proto::INSERT_BARS,
        &InsertBarsParams { at_bar, count },
    )
    .result::<ShiftResult>()
    .expect("arrangement.insert_bars succeeds")
}

// ---------------------------------------------------------------------------
// insert_bars
// ---------------------------------------------------------------------------

/// The whole point: one call moves everything after the cut.
#[test]
fn inserting_bars_moves_everything_after_the_cut() {
    let mut app = app_with_project();
    push_audio_clip(&mut app, 1, 1);
    push_audio_clip(&mut app, 2, 9);
    push_midi_clip(&mut app, 10, 9);

    let result = insert(&mut app, 9, 2);

    assert_eq!(clip_start(&app, 1), Some(0), "bar 1 is before the cut");
    assert_eq!(
        clip_start(&app, 2),
        Some(10 * BAR),
        "the clip at bar 9 moves to bar 11"
    );
    assert_eq!(midi_start(&app, 10), Some(10 * BAR), "MIDI moves with it");
    assert_eq!(result.audio_clips_moved, 1);
    assert_eq!(result.midi_clips_moved, 1);
    assert_eq!(result.shift_samples, 2 * BAR as i64);
}

/// A clip that STARTS before the cut is left alone, even when it plays
/// across it. Stretching it would be a different edit — and a silent one.
#[test]
fn a_clip_spanning_the_cut_is_not_moved_or_stretched() {
    let mut app = app_with_project();
    push_audio_clip(&mut app, 1, 5);

    insert(&mut app, 5, 4);
    assert_eq!(
        clip_start(&app, 1),
        Some(8 * BAR),
        "a clip starting exactly at the cut moves: bar 5 -> bar 9"
    );

    let mut app = app_with_project();
    push_audio_clip(&mut app, 1, 5);
    insert(&mut app, 6, 4);
    assert_eq!(
        clip_start(&app, 1),
        Some(4 * BAR),
        "starts before the cut → stays, even though it plays across it"
    );
    let length = app.test_clips()[0].duration_samples;
    assert_eq!(length, BAR, "and keeps its length");
}

/// Bars are 1-based and a zero-bar shift is a typo, not an edit.
#[test]
fn a_zero_bar_or_zero_count_shift_is_refused() {
    let mut app = app_with_project();
    assert!(
        call(
            &mut app,
            proto::INSERT_BARS,
            &InsertBarsParams { at_bar: 0, count: 2 }
        )
        .result::<ShiftResult>()
        .is_err(),
        "at_bar 0 is refused"
    );
    assert!(
        call(
            &mut app,
            proto::INSERT_BARS,
            &InsertBarsParams { at_bar: 4, count: 0 }
        )
        .result::<ShiftResult>()
        .is_err(),
        "count 0 is refused"
    );
}

// ---------------------------------------------------------------------------
// remove_bars
// ---------------------------------------------------------------------------

/// Removing bars pulls later material earlier — the inverse edit.
#[test]
fn removing_bars_pulls_later_material_earlier() {
    let mut app = app_with_project();
    push_audio_clip(&mut app, 1, 1);
    push_audio_clip(&mut app, 2, 9);

    let result = call(
        &mut app,
        proto::REMOVE_BARS,
        &RemoveBarsParams {
            at_bar: 5,
            count: 4,
            confirm: false,
        },
    )
    .result::<ShiftResult>()
    .expect("nothing starts inside bars 5..8, so no confirmation is needed");

    assert_eq!(clip_start(&app, 1), Some(0), "before the cut, unmoved");
    assert_eq!(clip_start(&app, 2), Some(4 * BAR), "bar 9 becomes bar 5");
    assert_eq!(result.shift_samples, -(4 * BAR as i64));
    assert!(result.clips_deleted.is_empty());
}

/// Deleting material needs an explicit confirmation, like `track.delete`.
#[test]
fn removing_bars_that_hold_clips_needs_confirmation() {
    let mut app = app_with_project();
    push_audio_clip(&mut app, 1, 5);
    push_midi_clip(&mut app, 10, 6);

    let refused = call(
        &mut app,
        proto::REMOVE_BARS,
        &RemoveBarsParams {
            at_bar: 5,
            count: 4,
            confirm: false,
        },
    );
    assert!(
        refused.result::<ShiftResult>().is_err(),
        "a removal that would delete clips is refused without confirm"
    );
    assert_eq!(
        clip_start(&app, 1),
        Some(4 * BAR),
        "and changes nothing while refusing"
    );

    let result = call(
        &mut app,
        proto::REMOVE_BARS,
        &RemoveBarsParams {
            at_bar: 5,
            count: 4,
            confirm: true,
        },
    )
    .result::<ShiftResult>()
    .expect("confirmed removal succeeds");

    assert_eq!(clip_start(&app, 1), None, "the clip inside the span is gone");
    assert_eq!(midi_start(&app, 10), None, "including the MIDI clip");
    assert_eq!(result.clips_deleted.len(), 2, "both are reported");
}

// ---------------------------------------------------------------------------
// The collections that have no other coverage
// ---------------------------------------------------------------------------

fn add_marker(app: &mut Resonance, id: u64, start_bar: u64, end_bar: Option<u64>) -> u64 {
    app.test_add_marker(resonance_app::state::ArrangementMarker {
        id,
        name: format!("m{id}"),
        color: [0, 0, 0],
        start_sample: (start_bar - 1) * BAR,
        end_sample: end_bar.map(|b| (b - 1) * BAR),
        seeded: false,
    })
}

fn install_lane(app: &mut Resonance, bars: &[u64]) {
    use resonance_audio::types::AudioEvent;
    use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind};
    let points = bars
        .iter()
        .map(|b| Breakpoint::new((b - 1) * BAR, 0.5, CurveKind::Linear))
        .collect();
    let lane = AutomationLane::new(1, AutomationTarget::MasterGain, points);
    app.test_apply_engine_event(AudioEvent::AutomationLaneChanged { lane });
}

fn lane_points(app: &Resonance) -> Vec<u64> {
    app.test_automation()
        .lanes
        .values()
        .flat_map(|l| l.points.iter().map(|p| p.time_frames))
        .collect()
}

/// Markers and automation breakpoints ride the shift too, and a ranged
/// marker takes its end with it.
#[test]
fn markers_and_automation_move_with_the_cut() {
    let mut app = app_with_project();
    let before_cut = add_marker(&mut app, 1, 3, None);
    let after_cut = add_marker(&mut app, 2, 9, Some(11));
    install_lane(&mut app, &[3, 9, 17]);

    let result = insert(&mut app, 9, 2);

    let marker = |id: u64| {
        app.test_markers()
            .markers
            .iter()
            .find(|m| m.id == id)
            .map(|m| (m.start_sample, m.end_sample))
            .expect("marker survives the shift")
    };
    assert_eq!(marker(before_cut), (2 * BAR, None), "before the cut, unmoved");
    assert_eq!(
        marker(after_cut),
        (10 * BAR, Some(12 * BAR)),
        "bar 9 -> bar 11, and the range end moves with the start"
    );
    assert_eq!(result.markers_moved, 1);

    assert_eq!(
        lane_points(&app),
        vec![2 * BAR, 10 * BAR, 18 * BAR],
        "only the breakpoints at or after the cut move"
    );
    assert_eq!(result.automation_points_moved, 2);
}

/// Removing bars can pull a later breakpoint onto or past one that sat
/// inside the removed span and stayed. The engine requires lanes sorted
/// by time, so the mirror has to re-sort rather than hand it a lane that
/// runs backwards.
#[test]
fn a_removal_leaves_the_automation_lane_sorted() {
    let mut app = app_with_project();
    // Bar 7 is inside the removed 5..8 and does not move; bar 9 lands on
    // bar 5, i.e. BEFORE it.
    install_lane(&mut app, &[1, 7, 9]);

    call(
        &mut app,
        proto::REMOVE_BARS,
        &RemoveBarsParams {
            at_bar: 5,
            count: 4,
            confirm: true,
        },
    )
    .result::<ShiftResult>()
    .expect("nothing starts inside bars 5..8 but the breakpoint");

    let points = lane_points(&app);
    assert!(
        points.windows(2).all(|w| w[0] <= w[1]),
        "the lane is still sorted ascending: {points:?}"
    );
    assert_eq!(points, vec![0, 4 * BAR, 6 * BAR]);
}

/// insert then remove the same span is the identity — the check that the
/// two directions really are inverses.
#[test]
fn insert_then_remove_restores_every_position() {
    let mut app = app_with_project();
    for (id, bar) in [(1u64, 1u64), (2, 3), (3, 9), (4, 17)] {
        push_audio_clip(&mut app, id, bar);
    }
    let before: Vec<u64> = app.test_clips().iter().map(|c| c.start_sample).collect();

    insert(&mut app, 9, 8);
    call(
        &mut app,
        proto::REMOVE_BARS,
        &RemoveBarsParams {
            at_bar: 9,
            count: 8,
            confirm: true,
        },
    )
    .result::<ShiftResult>()
    .expect("removal succeeds");

    let after: Vec<u64> = app.test_clips().iter().map(|c| c.start_sample).collect();
    assert_eq!(before, after, "insert then remove is the identity");
}

// ---------------------------------------------------------------------------
// Bar-count bounds (CTL-05)
// ---------------------------------------------------------------------------

/// A huge `count` used to overflow u32 in the casualty scan — a panic in
/// `update()` in debug, and in release a wrapped (empty) cut span that
/// skipped the confirm gate and then corrupted every marker and event.
#[test]
fn huge_bar_counts_are_invalid_params_and_change_nothing() {
    use resonance_control::ErrorKind;
    let mut app = app_with_project();
    push_audio_clip(&mut app, 1, 5);
    let revision = app.revision();

    let too_big = resonance_control::MAX_BARS + 1;
    for (method, at_bar, count) in [
        (proto::REMOVE_BARS, 2, u32::MAX),
        (proto::REMOVE_BARS, u32::MAX, 1),
        (proto::REMOVE_BARS, 2, too_big),
        (proto::INSERT_BARS, 2, u32::MAX),
        (proto::INSERT_BARS, u32::MAX, 4),
        (proto::INSERT_BARS, resonance_control::MAX_BARS, 2),
    ] {
        let response = if method == proto::REMOVE_BARS {
            call(
                &mut app,
                method,
                &RemoveBarsParams {
                    at_bar,
                    count,
                    confirm: false,
                },
            )
        } else {
            call(&mut app, method, &InsertBarsParams { at_bar, count })
        };
        let error = response
            .error
            .unwrap_or_else(|| panic!("{method} {at_bar}+{count} must be refused"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "{method} {at_bar}+{count}");
    }
    assert_eq!(clip_start(&app, 1), Some(4 * BAR), "nothing moved");
    assert_eq!(app.revision(), revision, "nothing was committed");
}

/// FU-M5a: each call's span fits, but repeated inserts before existing
/// content used to push that content past MAX_BARS. The check is on
/// where the song would END.
#[test]
fn repeated_inserts_cannot_push_content_past_max_bars() {
    use resonance_control::ErrorKind;
    let mut app = app_with_project();
    push_audio_clip(&mut app, 1, 5);
    let raw_insert = |app: &mut Resonance, at_bar: u32, count: u32| {
        call(app, proto::INSERT_BARS, &InsertBarsParams { at_bar, count })
    };

    let half = resonance_control::MAX_BARS / 2;
    let first = raw_insert(&mut app, 1, half);
    assert!(first.error.is_none(), "first insert fits: {:?}", first.error);
    let revision = app.revision();
    let start = clip_start(&app, 1);

    let second = raw_insert(&mut app, 1, half);
    let error = second.error.expect("content would end past MAX_BARS");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert_eq!(clip_start(&app, 1), start, "nothing moved");
    assert_eq!(app.revision(), revision, "nothing was committed");

    // Inserting AFTER the content moves nothing, so only the span counts.
    let after = raw_insert(&mut app, half + 10, 4);
    assert!(after.error.is_none(), "insert past the end fits: {:?}", after.error);
}
