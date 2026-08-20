//! `global.list_events` — the read path onto the song-wide tempo and
//! time-signature tracks (ba todo #1380, design doc #286 §2/§5).
//!
//! The app has always kept per-bar tempo and meter changes on the
//! global-tracks shelf, and until this method none of the 118 control
//! methods could see them. `song.summary` reports `tempo_bpm` and
//! `time_signature` from the TRANSPORT, i.e. the values at the playhead,
//! so on a song that changes meter it hands a client a confident wrong
//! answer with nothing in the response to hint that changes exist.
//!
//! These tests drive the real control dispatch, so they cover the wire
//! shape and the 0-based-state / 1-based-wire conversion together.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::{GlobalTrackMessage, Message};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_control::methods::global::{self as proto, GlobalEvents};
use resonance_control::{ErrorKind, Request, Response};

const SR: u32 = 48_000;

/// The bar the bridge starts on, 0-based as app state stores it. On the
/// wire that is bar 33.
const BRIDGE: u32 = 32;

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

fn list(app: &mut Resonance) -> GlobalEvents {
    call(app, proto::LIST_EVENTS, &())
        .result::<GlobalEvents>()
        .expect("global.list_events succeeds")
}

/// A song in 4/4 at 120 with an open project and nothing else.
fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-global-events.rprj"));
    app.test_set_sample_rate(SR);
    app.test_set_flat_tempo(120.0);
    app
}

#[test]
fn a_song_with_no_changes_reports_one_initial_event_in_each_list() {
    // Neither list is ever empty: bar 1 carries the song's initial tempo
    // and initial meter. That is what makes "does this song change?"
    // answerable by list length alone, with no second call.
    let mut app = app_with_project();
    let events = list(&mut app);

    assert_eq!(events.tempo_events.len(), 1, "{:?}", events.tempo_events);
    assert_eq!(events.tempo_events[0].bar, 1, "bars are 1-based on the wire");
    assert_eq!(events.tempo_events[0].bpm, 120.0);

    assert_eq!(events.signature_events.len(), 1, "{:?}", events.signature_events);
    assert_eq!(events.signature_events[0].bar, 1);
    assert_eq!(events.signature_events[0].numerator, 4);
    assert_eq!(events.signature_events[0].denominator, 4);
}

#[test]
fn added_events_are_reported_in_bar_order_with_1_based_bars() {
    // The bridge drops into 7/8 at 140 BPM. App state writes it at bar
    // 32 (0-based); a client must see bar 33.
    let mut app = app_with_project();
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddTempoEvent {
        bar: BRIDGE,
        bpm: 140.0,
    }));
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddSignatureEvent {
        bar: BRIDGE,
        numerator: 7,
        denominator: 8,
    }));

    let events = list(&mut app);

    let tempo: Vec<(u32, f32)> = events.tempo_events.iter().map(|e| (e.bar, e.bpm)).collect();
    assert_eq!(tempo, vec![(1, 120.0), (33, 140.0)]);

    let meter: Vec<(u32, u8, u8)> = events
        .signature_events
        .iter()
        .map(|e| (e.bar, e.numerator, e.denominator))
        .collect();
    assert_eq!(meter, vec![(1, 4, 4), (33, 7, 8)]);

    // Every state bar maps to exactly one wire bar, always +1 — no
    // list-position leaks into the reply.
    for (view, state) in events.tempo_events.iter().zip(app.test_tempo_events()) {
        assert_eq!(view.bar, state.bar + 1);
    }
    for (view, state) in events.signature_events.iter().zip(app.test_signature_events()) {
        assert_eq!(view.bar, state.bar + 1);
    }
}

#[test]
fn events_stay_sorted_when_an_earlier_bar_is_added_afterwards() {
    // Both lists re-sort on every mutation, which is exactly why the
    // namespace addresses events by bar and never by index: a client
    // holding "index 1" here would be pointing at a different event
    // after this second add.
    let mut app = app_with_project();
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddSignatureEvent {
        bar: BRIDGE,
        numerator: 7,
        denominator: 8,
    }));
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddSignatureEvent {
        bar: 8,
        numerator: 3,
        denominator: 4,
    }));

    let bars: Vec<u32> = list(&mut app).signature_events.iter().map(|e| e.bar).collect();
    assert_eq!(bars, vec![1, 9, 33], "sorted by bar, 1-based");
}

#[test]
fn the_reply_carries_the_current_revision() {
    let mut app = app_with_project();
    let before = list(&mut app).revision;
    assert_eq!(before, app.revision(), "read reports the app's own counter");

    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddTempoEvent {
        bar: 16,
        bpm: 90.0,
    }));

    let after = list(&mut app);
    assert_eq!(after.revision, app.revision());
    assert!(
        after.revision > before,
        "the edit bumped the revision the client watches: {before} -> {}",
        after.revision
    );
    // Reading twice does not move it — `global.list_events` mutates
    // nothing.
    assert_eq!(list(&mut app).revision, after.revision);
}

#[test]
fn list_events_is_busy_without_an_active_project() {
    // Deliberately below the mutation gate, like `master.summary`: with
    // nothing open, reporting the default 120 BPM 4/4 would read like a
    // real song's tempo map (doc #286; the namespace list in
    // control_mutation_gate.rs pins this for every `global.*` method).
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    let response = call(&mut app, proto::LIST_EVENTS, &());
    assert_eq!(
        response.error.expect("no project means no tempo map").kind(),
        ErrorKind::Busy
    );
}

#[test]
fn list_events_is_advertised_in_the_hello_capabilities() {
    assert!(resonance_control::methods::capabilities().contains(&proto::LIST_EVENTS));
}
