//! `global.*` — the song-wide tempo and time-signature tracks: the read
//! path (`list_events`, ba todo #1380) and the adds (`add_tempo_event` /
//! `add_signature_event`, ba todo #1382), against design doc #286.
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
use resonance_control::{ErrorKind, MutationAck, Request, Response};

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

// ---------------------------------------------------------------------------
// `global.add_tempo_event` / `global.add_signature_event` (ba todo #1382)
// ---------------------------------------------------------------------------

fn add_tempo(app: &mut Resonance, bar: u32, bpm: f32) -> Response {
    call(app, proto::ADD_TEMPO_EVENT, &serde_json::json!({"bar": bar, "bpm": bpm}))
}

fn add_signature(app: &mut Resonance, bar: u32, numerator: u8, denominator: u8) -> Response {
    call(
        app,
        proto::ADD_SIGNATURE_EVENT,
        &serde_json::json!({"bar": bar, "numerator": numerator, "denominator": denominator}),
    )
}

fn tempo_pairs(app: &mut Resonance) -> Vec<(u32, f32)> {
    list(app).tempo_events.iter().map(|e| (e.bar, e.bpm)).collect()
}

fn meter_triples(app: &mut Resonance) -> Vec<(u32, u8, u8)> {
    list(app)
        .signature_events
        .iter()
        .map(|e| (e.bar, e.numerator, e.denominator))
        .collect()
}

#[test]
fn an_added_tempo_event_lands_on_the_wire_bar_1_based() {
    let mut app = app_with_project();
    let ack: MutationAck = add_tempo(&mut app, 33, 140.0)
        .result()
        .expect("global.add_tempo_event succeeds");
    assert_eq!(ack.revision, app.revision(), "the ack carries the post-edit revision");

    assert_eq!(tempo_pairs(&mut app), vec![(1, 120.0), (33, 140.0)]);
    // The wire said 33; app state stores these 0-based.
    assert_eq!(app.test_tempo_events()[1].bar, BRIDGE);
}

#[test]
fn an_added_signature_event_lands_on_the_wire_bar_1_based() {
    let mut app = app_with_project();
    let _: MutationAck = add_signature(&mut app, 33, 7, 8)
        .result()
        .expect("global.add_signature_event succeeds");

    assert_eq!(meter_triples(&mut app), vec![(1, 4, 4), (33, 7, 8)]);
    assert_eq!(app.test_signature_events()[1].bar, BRIDGE);
}

#[test]
fn adding_a_tempo_event_twice_at_one_bar_replaces_it() {
    // THE trap this slice exists to close. `AddTempoEvent` used to push
    // unconditionally and re-sort, so a second add at bar 33 left TWO
    // events on that bar: an ambiguous address for every by-bar method
    // and a duplicate bar in the list `song.summary` reports. One bar,
    // one tempo — the second value wins.
    let mut app = app_with_project();
    let _: MutationAck = add_tempo(&mut app, 33, 140.0).result().expect("first add");
    let _: MutationAck = add_tempo(&mut app, 33, 96.0).result().expect("second add");

    assert_eq!(
        tempo_pairs(&mut app),
        vec![(1, 120.0), (33, 96.0)],
        "exactly one event at bar 33, carrying the second value"
    );
    assert_eq!(app.test_tempo_events().len(), 2, "state agrees with the wire view");
}

#[test]
fn adding_a_signature_event_twice_at_one_bar_replaces_it() {
    let mut app = app_with_project();
    let _: MutationAck = add_signature(&mut app, 17, 7, 8).result().expect("first add");
    let _: MutationAck = add_signature(&mut app, 17, 5, 4).result().expect("second add");

    assert_eq!(
        meter_triples(&mut app),
        vec![(1, 4, 4), (17, 5, 4)],
        "exactly one event at bar 17, carrying the second value"
    );
}

#[test]
fn adding_at_bar_1_rewrites_the_initial_events_rather_than_duplicating_them() {
    // Bar 1 is the always-present initial event of each list. An add
    // there is the same edit `transport.set_tempo` /
    // `transport.set_time_signature` make, and must not create a second
    // bar-1 event that the unremovable first one would shadow.
    let mut app = app_with_project();
    let _: MutationAck = add_tempo(&mut app, 1, 90.0).result().expect("tempo add");
    let _: MutationAck = add_signature(&mut app, 1, 3, 4).result().expect("meter add");

    assert_eq!(tempo_pairs(&mut app), vec![(1, 90.0)]);
    assert_eq!(meter_triples(&mut app), vec![(1, 3, 4)]);
}

#[test]
fn an_added_tempo_event_is_undoable() {
    // The contract for every control edit: it lands in undo like a
    // manual one. It only holds because the handler synthesizes a
    // `GlobalTrackMessage` and routes it through `update()`, where
    // `undo::classify` maps `GlobalTrack(_)` to `Record` — writing
    // `app.tempo_events` directly would pass every assertion above and
    // silently bypass undo (doc #286 §3).
    let mut app = app_with_project();
    let _: MutationAck = add_tempo(&mut app, 33, 140.0).result().expect("add succeeds");
    assert_eq!(tempo_pairs(&mut app), vec![(1, 120.0), (33, 140.0)]);

    let _ = app.update(Message::Undo);

    assert_eq!(
        tempo_pairs(&mut app),
        vec![(1, 120.0)],
        "undo should take the tempo event back off the track"
    );
}

#[test]
fn an_added_signature_event_is_undoable() {
    let mut app = app_with_project();
    let _: MutationAck = add_signature(&mut app, 33, 7, 8).result().expect("add succeeds");
    assert_eq!(meter_triples(&mut app), vec![(1, 4, 4), (33, 7, 8)]);

    let _ = app.update(Message::Undo);

    assert_eq!(
        meter_triples(&mut app),
        vec![(1, 4, 4)],
        "undo should take the meter change back off the track"
    );
}

#[test]
fn bar_0_is_refused_rather_than_underflowing() {
    // Bars are 1-based on this surface; 0 would otherwise wrap to the
    // last bar of the song.
    let mut app = app_with_project();
    for response in [add_tempo(&mut app, 0, 140.0), add_signature(&mut app, 0, 7, 8)] {
        assert_eq!(
            response.error.expect("bar 0 is not a bar").kind(),
            ErrorKind::InvalidParams
        );
    }
    assert_eq!(tempo_pairs(&mut app), vec![(1, 120.0)], "nothing was written");
    assert_eq!(meter_triples(&mut app), vec![(1, 4, 4)]);
}

#[test]
fn an_out_of_range_bpm_is_refused_not_clamped() {
    // The GUI clamps to 20..=300; a client that asked for 500 would
    // otherwise get 300 back with no indication it was overruled.
    let mut app = app_with_project();
    // (A non-finite bpm never survives JSON — serde_json writes NaN as
    // null — so it is refused one layer earlier, when the params fail to
    // parse.)
    for bpm in [19.0, 500.0] {
        let error = add_tempo(&mut app, 33, bpm)
            .error
            .unwrap_or_else(|| panic!("bpm {bpm} should be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for bpm {bpm}");
    }
    assert_eq!(
        tempo_pairs(&mut app),
        vec![(1, 120.0)],
        "a rejected tempo must not reach the track"
    );
}

#[test]
fn an_illegal_meter_is_refused() {
    // Same validator `transport.set_time_signature` uses — the two
    // cannot disagree about what a legal meter is (doc #286 §2). Note
    // 3 is rejected: denominators are RESOLVED (8 for 7/8), never
    // exponents.
    let mut app = app_with_project();
    for (numerator, denominator) in [(0, 4), (33, 4), (7, 3), (4, 64)] {
        let error = add_signature(&mut app, 17, numerator, denominator)
            .error
            .unwrap_or_else(|| panic!("{numerator}/{denominator} should be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for {numerator}/{denominator}");
    }
    assert_eq!(
        meter_triples(&mut app),
        vec![(1, 4, 4)],
        "a rejected meter must not reach the track"
    );
}

#[test]
fn adds_reach_the_engine_and_the_gui_tempo_map() {
    // Routing through `update()` is also what keeps the audio engine and
    // the shelf's own tempo map in step: `rebuild_and_send_tempo` runs
    // as part of the domain message, not as something this handler has
    // to remember.
    let mut app = app_with_project();
    let _: MutationAck = add_tempo(&mut app, 33, 140.0).result().expect("add succeeds");
    let _: MutationAck = add_signature(&mut app, 33, 7, 8).result().expect("add succeeds");

    let map = app.test_tempo_map();
    assert_eq!(map.tempo_points.len(), 2, "{:?}", map.tempo_points);
    assert!((map.tempo_points[1].bpm - 140.0).abs() < 1e-4);
    assert_eq!(map.signature_points.len(), 2);
    assert_eq!(
        (map.signature_points[1].numerator, map.signature_points[1].denominator),
        (7, 8)
    );
}

#[test]
fn the_adds_are_advertised_in_the_hello_capabilities() {
    let capabilities = resonance_control::methods::capabilities();
    assert!(capabilities.contains(&proto::ADD_TEMPO_EVENT));
    assert!(capabilities.contains(&proto::ADD_SIGNATURE_EVENT));
}
