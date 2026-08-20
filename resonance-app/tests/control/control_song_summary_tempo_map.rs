//! `song.summary` carries the song's whole tempo and meter tracks, and
//! its `tempo_bpm` / `time_signature` are only the reading at the
//! playhead (ba todo #1381, design doc #286 §5).
//!
//! The defect being pinned here is not a crash: `song.summary` used to
//! report the transport's BPM and meter — which follow the PLAYHEAD —
//! with nothing in the response to say that the song changed anywhere
//! else. An agent reading `time_signature: 4/4` on a song that drops
//! into 7/8 at the bridge gets a confident wrong answer, and every bar
//! calculation after it inherits the error silently. The two fields stay
//! (removing them breaks every client), so what makes them safe is that
//! the truth now travels in the same response.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::{GlobalTrackMessage, Message};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_control::methods::global::{SignatureEventView, TempoEventView};
use resonance_control::methods::song::SongSummary;
use resonance_control::methods::transport::{self, SeekParams};
use resonance_control::{PositionSpec, Request, Response};

const SR: u32 = 48_000;

/// The tempo lift, 0-based as app state stores it — wire bar 9.
const LIFT: u32 = 8;
/// The 7/8 bridge, 0-based — wire bar 17.
const BRIDGE: u32 = 16;

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

fn summary(app: &mut Resonance) -> SongSummary {
    call(app, "song.summary", &())
        .result::<SongSummary>()
        .expect("song.summary succeeds")
}

/// Move the playhead to the start of a 1-based bar the way a client
/// would, then let the per-frame tick pull the transport display onto
/// the new position — the same two steps the GUI takes during playback.
fn seek_to_bar(app: &mut Resonance, bar: u32) {
    let params = SeekParams {
        position: PositionSpec::musical(bar, 1.0),
    };
    let response = call(app, transport::SEEK, &params);
    assert!(response.error.is_none(), "seek to bar {bar}: {response:?}");
    let _ = app.update(Message::Tick);
}

/// A song at 120 4/4 that lifts to 140 at bar 9 and drops into 7/8 at
/// bar 17 — the arrangement the acceptance criteria name.
fn app_with_changes() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-song-summary-tempo.rprj"));
    app.test_set_sample_rate(SR);
    app.test_set_flat_tempo(120.0);
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddTempoEvent {
        bar: LIFT,
        bpm: 140.0,
    }));
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddSignatureEvent {
        bar: BRIDGE,
        numerator: 7,
        denominator: 8,
    }));
    // Playback is what makes the transport display track the playhead;
    // without it the tick deliberately leaves the display alone.
    app.test_set_transport_playing(true);
    app
}

fn tempo_pairs(events: &[TempoEventView]) -> Vec<(u32, f32)> {
    events.iter().map(|e| (e.bar, e.bpm)).collect()
}

fn meter_triples(events: &[SignatureEventView]) -> Vec<(u32, u8, u8)> {
    events
        .iter()
        .map(|e| (e.bar, e.numerator, e.denominator))
        .collect()
}

#[test]
fn summary_carries_the_whole_tempo_and_signature_tracks() {
    let mut app = app_with_changes();
    let summary = summary(&mut app);

    // Bars are 1-based on the wire; app state wrote 8 and 16.
    assert_eq!(
        tempo_pairs(&summary.tempo_events),
        vec![(1, 120.0), (9, 140.0)]
    );
    assert_eq!(
        meter_triples(&summary.signature_events),
        vec![(1, 4, 4), (17, 7, 8)]
    );
}

#[test]
fn a_song_with_no_changes_reports_one_event_in_each_list() {
    // The lists are never empty, which is what makes "does this song
    // change tempo or meter?" answerable from the summary's own field
    // lengths, with no second call to global.list_events.
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_sample_rate(SR);
    app.test_set_flat_tempo(120.0);

    let summary = summary(&mut app);
    assert_eq!(
        tempo_pairs(&summary.tempo_events),
        vec![(1, 120.0)],
        "bar 1 is the initial tempo and is always present"
    );
    assert_eq!(meter_triples(&summary.signature_events), vec![(1, 4, 4)]);
}

#[test]
fn tempo_bpm_and_time_signature_follow_the_playhead_while_the_lists_do_not() {
    // The acceptance criterion, and the whole defect in one assertion:
    // the two scalar fields are a local reading, the two lists are the
    // song, and only the former moves when the cursor does.
    let mut app = app_with_changes();

    let at_start = summary(&mut app);
    assert_eq!(at_start.tempo_bpm, 120.0);
    assert_eq!(at_start.time_signature.numerator, 4);
    assert_eq!(at_start.time_signature.denominator, 4);

    // Past the tempo lift but before the bridge: tempo has moved, meter
    // has not — they track the playhead independently.
    seek_to_bar(&mut app, 12);
    let mid = summary(&mut app);
    assert_eq!(mid.tempo_bpm, 140.0, "bar 12 is past the bar-9 lift");
    assert_eq!(mid.time_signature.numerator, 4, "bar 12 is before the bridge");

    // Inside the bridge: both readings have moved.
    seek_to_bar(&mut app, 18);
    let in_bridge = summary(&mut app);
    assert_eq!(in_bridge.tempo_bpm, 140.0);
    assert_eq!(in_bridge.time_signature.numerator, 7);
    assert_eq!(in_bridge.time_signature.denominator, 8);

    // Nothing about the SONG changed across those three reads. A client
    // that trusted `time_signature` would have counted bar 12 in 4/4 and
    // bar 18 in 7/8 and never learned that either was a local reading;
    // the event lists are byte-identical throughout.
    assert_eq!(at_start.tempo_events, mid.tempo_events);
    assert_eq!(at_start.tempo_events, in_bridge.tempo_events);
    assert_eq!(at_start.signature_events, mid.signature_events);
    assert_eq!(at_start.signature_events, in_bridge.signature_events);
    assert_eq!(
        tempo_pairs(&in_bridge.tempo_events),
        vec![(1, 120.0), (9, 140.0)],
        "the song still starts at 120 and lifts at bar 9"
    );
    assert_eq!(
        meter_triples(&in_bridge.signature_events),
        vec![(1, 4, 4), (17, 7, 8)]
    );
}

#[test]
fn summary_and_global_list_events_never_disagree() {
    // Both are built from the same helper on purpose: two independent
    // mappings of the same state are two chances to get the 0-based
    // state bar / 1-based wire bar conversion wrong in only one of them.
    use resonance_control::methods::global::{self as proto, GlobalEvents};

    let mut app = app_with_changes();
    let summary = summary(&mut app);
    let listed = call(&mut app, proto::LIST_EVENTS, &())
        .result::<GlobalEvents>()
        .expect("global.list_events succeeds");

    assert_eq!(summary.tempo_events, listed.tempo_events);
    assert_eq!(summary.signature_events, listed.signature_events);
}

#[test]
fn summary_json_carries_both_event_lists() {
    let mut app = app_with_changes();
    let json: serde_json::Value = call(&mut app, "song.summary", &())
        .result()
        .expect("song.summary succeeds");

    assert_eq!(json["tempo_events"][1]["bar"], 9);
    assert_eq!(json["tempo_events"][1]["bpm"], 140.0);
    assert_eq!(json["signature_events"][1]["bar"], 17);
    assert_eq!(json["signature_events"][1]["numerator"], 7);
    // Resolved denominator (8 for 7/8), never an exponent.
    assert_eq!(json["signature_events"][1]["denominator"], 8);
}
