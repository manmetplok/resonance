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

// ---------------------------------------------------------------------------
// `global.edit_tempo_event` / `global.edit_signature_event` (ba todo #1383)
// ---------------------------------------------------------------------------

fn edit_tempo(app: &mut Resonance, params: serde_json::Value) -> Response {
    call(app, proto::EDIT_TEMPO_EVENT, &params)
}

fn edit_signature(app: &mut Resonance, params: serde_json::Value) -> Response {
    call(app, proto::EDIT_SIGNATURE_EVENT, &params)
}

/// A song with a 140 BPM 7/8 bridge at wire bar 33, added through the
/// control surface so every edit test starts from a track a client could
/// itself have written.
fn app_with_a_bridge() -> Resonance {
    let mut app = app_with_project();
    let _: MutationAck = add_tempo(&mut app, 33, 140.0).result().expect("tempo add");
    let _: MutationAck = add_signature(&mut app, 33, 7, 8).result().expect("meter add");
    app
}

fn undo_entry_count(app: &Resonance) -> usize {
    app.test_undo_history().test_undo_entries().len()
}

#[test]
fn editing_a_tempo_event_changes_only_the_named_field() {
    // Omitted fields keep their current value: retuning the bridge must
    // not also move it back to wherever a default would have put it.
    let mut app = app_with_a_bridge();
    let ack: MutationAck = edit_tempo(&mut app, serde_json::json!({"bar": 33, "bpm": 96.0}))
        .result()
        .expect("global.edit_tempo_event succeeds");
    assert_eq!(ack.revision, app.revision(), "the ack carries the post-edit revision");

    assert_eq!(tempo_pairs(&mut app), vec![(1, 120.0), (33, 96.0)]);
}

#[test]
fn editing_a_signature_event_changes_only_the_named_field() {
    // 7/8 edited with `numerator: 5` is 5/8 — the denominator it already
    // had, not a default 4.
    let mut app = app_with_a_bridge();
    let _: MutationAck = edit_signature(&mut app, serde_json::json!({"bar": 33, "numerator": 5}))
        .result()
        .expect("global.edit_signature_event succeeds");

    assert_eq!(meter_triples(&mut app), vec![(1, 4, 4), (33, 5, 8)]);
}

#[test]
fn moving_a_tempo_event_keeps_the_list_sorted() {
    // `new_bar` is the only way to relocate an event on either track.
    // The move goes backwards past another event here, so a handler that
    // forgot `EndTempoDrag` (which re-sorts) would leave the list out of
    // order and every by-bar address after it pointing at the wrong one.
    let mut app = app_with_a_bridge();
    let _: MutationAck = add_tempo(&mut app, 17, 100.0).result().expect("second add");
    assert_eq!(tempo_pairs(&mut app), vec![(1, 120.0), (17, 100.0), (33, 140.0)]);

    let _: MutationAck = edit_tempo(&mut app, serde_json::json!({"bar": 33, "new_bar": 9}))
        .result()
        .expect("the move succeeds");

    assert_eq!(
        tempo_pairs(&mut app),
        vec![(1, 120.0), (9, 140.0), (17, 100.0)],
        "moved to bar 9 at its own tempo, list re-sorted"
    );
}

#[test]
fn one_undo_after_a_tempo_edit_restores_both_the_bpm_and_the_bar() {
    // THE point of this slice. `GlobalTrackMessage::UpdateTempoEvent` is
    // the GUI's drag-MOVE message and is classified `UndoAction::Skip`;
    // its undo entry comes from the `StartTempoDrag` (Begin) /
    // `EndTempoDrag` (Commit) bracket around it. A handler that
    // dispatched `UpdateTempoEvent` bare would pass every assertion in
    // the three tests above and produce an edit that can never be
    // undone (ba doc #286 §3).
    let mut app = app_with_a_bridge();
    let before = tempo_pairs(&mut app);
    let entries_before = undo_entry_count(&app);

    // Both fields at once, so the assertion catches a bracket that
    // restores one of them and not the other.
    let _: MutationAck = edit_tempo(
        &mut app,
        serde_json::json!({"bar": 33, "bpm": 96.0, "new_bar": 41}),
    )
    .result()
    .expect("edit succeeds");
    assert_eq!(tempo_pairs(&mut app), vec![(1, 120.0), (41, 96.0)]);

    let _ = app.update(Message::Undo);

    assert_eq!(
        tempo_pairs(&mut app),
        before,
        "one undo must put back the bpm AND the bar together"
    );
    assert_eq!(
        undo_entry_count(&app),
        entries_before,
        "the bracket is ONE entry: a second undo would otherwise be needed, \
         and would step back past an edit the client never made"
    );
}

#[test]
fn redo_reapplies_a_tempo_edit() {
    let mut app = app_with_a_bridge();
    let _: MutationAck = edit_tempo(
        &mut app,
        serde_json::json!({"bar": 33, "bpm": 96.0, "new_bar": 41}),
    )
    .result()
    .expect("edit succeeds");

    let _ = app.update(Message::Undo);
    let _ = app.update(Message::Redo);

    assert_eq!(tempo_pairs(&mut app), vec![(1, 120.0), (41, 96.0)]);
}

#[test]
fn one_undo_after_a_signature_edit_restores_the_previous_meter() {
    // No bracket here, and the asymmetry is deliberate:
    // `UpdateSignatureEvent` has no drag gesture behind it, so it falls
    // to `undo::classify`'s `GlobalTrack(_) => Record` arm and one
    // dispatch is one complete entry. Asserted so that a later
    // "consistency" refactor cannot wrap it in the tempo bracket and
    // turn one entry into two, or drop the tempo bracket to match this.
    let mut app = app_with_a_bridge();
    let before = meter_triples(&mut app);
    let entries_before = undo_entry_count(&app);

    let _: MutationAck = edit_signature(
        &mut app,
        serde_json::json!({"bar": 33, "numerator": 5, "denominator": 4}),
    )
    .result()
    .expect("edit succeeds");
    assert_eq!(meter_triples(&mut app), vec![(1, 4, 4), (33, 5, 4)]);

    let _ = app.update(Message::Undo);

    assert_eq!(meter_triples(&mut app), before, "one undo restores the meter");
    assert_eq!(undo_entry_count(&app), entries_before, "exactly one entry");

    let _ = app.update(Message::Redo);
    assert_eq!(meter_triples(&mut app), vec![(1, 4, 4), (33, 5, 4)]);
}

#[test]
fn a_tempo_edit_records_exactly_one_revision_bump() {
    // Three messages go out (Begin / Skip / Commit) and exactly one of
    // them is a committed change, so the counter a client watches for
    // concurrent user edits must move by one — not by three, which would
    // read as "someone else is editing" on every call.
    let mut app = app_with_a_bridge();
    let before = app.revision();
    let _: MutationAck = edit_tempo(&mut app, serde_json::json!({"bar": 33, "bpm": 96.0}))
        .result()
        .expect("edit succeeds");
    assert_eq!(app.revision(), before + 1);
}

#[test]
fn an_undone_tempo_edit_takes_the_gui_tempo_map_with_it() {
    // The event vectors are not the only thing the edit touched:
    // `rebuild_and_send_tempo` runs as part of the domain messages, and
    // the restore path re-runs it. A test that only read the wire lists
    // would miss a tempo map left describing the edited song.
    let mut app = app_with_a_bridge();
    let _: MutationAck = edit_tempo(&mut app, serde_json::json!({"bar": 33, "bpm": 96.0}))
        .result()
        .expect("edit succeeds");
    assert!((app.test_tempo_map().tempo_points[1].bpm - 96.0).abs() < 1e-4);

    let _ = app.update(Message::Undo);

    let points = &app.test_tempo_map().tempo_points;
    assert_eq!(points.len(), 2, "{points:?}");
    assert!(
        (points[1].bpm - 140.0).abs() < 1e-4,
        "the tempo map must track the undo, not just the event list: {points:?}"
    );
}

#[test]
fn editing_the_bar_1_tempo_event_retunes_the_song_and_the_transport() {
    // The initial event can have its value changed — that is the same
    // edit `transport.set_tempo` makes — and with the playhead at the
    // start the transport display follows.
    let mut app = app_with_a_bridge();
    let _: MutationAck = edit_tempo(&mut app, serde_json::json!({"bar": 1, "bpm": 90.0}))
        .result()
        .expect("retuning bar 1 succeeds");

    assert_eq!(tempo_pairs(&mut app), vec![(1, 90.0), (33, 140.0)]);
    assert!((app.test_transport_bpm() - 90.0).abs() < 1e-4);
}

#[test]
fn moving_the_bar_1_tempo_event_is_refused_not_ignored() {
    // `UpdateTempoEvent` pins `event.bar = 0` for index 0, so the GUI's
    // own path silently ignores the move. Over the wire that is the
    // worst outcome available: `ok` back, list unchanged, and no way to
    // tell a refusal from something having re-created the event.
    let mut app = app_with_a_bridge();
    let before = tempo_pairs(&mut app);

    let error = edit_tempo(&mut app, serde_json::json!({"bar": 1, "new_bar": 5}))
        .error
        .expect("moving the initial tempo event must be refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(
        error.message.contains("bar 1"),
        "the refusal must name what it refused: {}",
        error.message
    );

    assert_eq!(tempo_pairs(&mut app), before, "nothing moved");
}

#[test]
fn editing_a_bar_with_no_event_on_it_is_refused_rather_than_creating_one() {
    // `edit_*` addresses an event; `add_*` creates one. Upserting here
    // would hide a client's wrong idea of where the change sits behind a
    // plausible `ok`, while leaving the event it meant to edit standing.
    let mut app = app_with_a_bridge();

    let error = edit_tempo(&mut app, serde_json::json!({"bar": 20, "bpm": 96.0}))
        .error
        .expect("bar 20 carries no tempo event");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(error.message.contains("bar 20"), "{}", error.message);

    let error = edit_signature(&mut app, serde_json::json!({"bar": 20, "numerator": 5}))
        .error
        .expect("bar 20 carries no meter event");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);

    assert_eq!(tempo_pairs(&mut app), vec![(1, 120.0), (33, 140.0)], "nothing created");
    assert_eq!(meter_triples(&mut app), vec![(1, 4, 4), (33, 7, 8)]);
}

#[test]
fn moving_a_tempo_event_onto_an_occupied_bar_is_refused() {
    // One bar, one tempo — the invariant `AddTempoEvent`'s upsert
    // enforces (ba todo #1382). A move must not slip a duplicate past it
    // through the back door and leave a bar that cannot be addressed
    // unambiguously.
    let mut app = app_with_a_bridge();
    let _: MutationAck = add_tempo(&mut app, 17, 100.0).result().expect("second add");

    let error = edit_tempo(&mut app, serde_json::json!({"bar": 33, "new_bar": 17}))
        .error
        .expect("bar 17 is taken");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);

    assert_eq!(
        tempo_pairs(&mut app),
        vec![(1, 120.0), (17, 100.0), (33, 140.0)],
        "no duplicate, and nothing moved"
    );
}

#[test]
fn moving_a_tempo_event_onto_its_own_bar_is_a_permitted_no_move() {
    // `new_bar == bar` is not a collision with itself; a client
    // re-sending the position it already believes is safe.
    let mut app = app_with_a_bridge();
    let _: MutationAck = edit_tempo(
        &mut app,
        serde_json::json!({"bar": 33, "new_bar": 33, "bpm": 96.0}),
    )
    .result()
    .expect("edit succeeds");

    assert_eq!(tempo_pairs(&mut app), vec![(1, 120.0), (33, 96.0)]);
}

#[test]
fn an_edit_that_names_no_field_is_refused_rather_than_acked_as_a_no_op() {
    // Otherwise a caller that omitted the field it meant to send reads
    // an unchanged track back and cannot tell whether the call or its
    // idea of the song is wrong. It also keeps an empty entry out of the
    // undo history.
    let mut app = app_with_a_bridge();
    let entries_before = undo_entry_count(&app);

    for response in [
        edit_tempo(&mut app, serde_json::json!({"bar": 33})),
        edit_signature(&mut app, serde_json::json!({"bar": 33})),
    ] {
        assert_eq!(
            response.error.expect("an edit must change something").kind(),
            ErrorKind::InvalidParams
        );
    }
    assert_eq!(undo_entry_count(&app), entries_before, "no empty undo entry");
}

#[test]
fn an_edited_bpm_out_of_range_is_refused_not_clamped() {
    // `UpdateTempoEvent` clamps to 20..=300 for the GUI's drag, so
    // without an explicit check this call would silently become 300 —
    // and `global.add_tempo_event`, which refuses, would disagree with
    // `global.edit_tempo_event` about what a legal tempo is. Two paths
    // into one list must not have two answers (ba doc #286 §2).
    let mut app = app_with_a_bridge();
    for bpm in [19.0, 500.0] {
        let error = edit_tempo(&mut app, serde_json::json!({"bar": 33, "bpm": bpm}))
            .error
            .unwrap_or_else(|| panic!("bpm {bpm} should be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for bpm {bpm}");
    }
    assert_eq!(
        tempo_pairs(&mut app),
        vec![(1, 120.0), (33, 140.0)],
        "no clamped tempo reached the track"
    );
}

#[test]
fn an_edited_meter_is_validated_as_the_pair_it_becomes() {
    // The omitted half is filled in from the event before validation, so
    // `denominator: 3` against a 7/8 event is rejected as 7/3 rather
    // than slipping through because the numerator was fine.
    let mut app = app_with_a_bridge();
    for params in [
        serde_json::json!({"bar": 33, "denominator": 3}),
        serde_json::json!({"bar": 33, "numerator": 0}),
        serde_json::json!({"bar": 33, "numerator": 33}),
    ] {
        let error = edit_signature(&mut app, params.clone())
            .error
            .unwrap_or_else(|| panic!("{params} should be rejected"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams, "for {params}");
    }
    assert_eq!(
        meter_triples(&mut app),
        vec![(1, 4, 4), (33, 7, 8)],
        "a rejected meter must not reach the track"
    );
}

#[test]
fn bar_0_is_refused_by_the_edits_too() {
    let mut app = app_with_a_bridge();
    for response in [
        edit_tempo(&mut app, serde_json::json!({"bar": 0, "bpm": 96.0})),
        edit_tempo(&mut app, serde_json::json!({"bar": 33, "new_bar": 0})),
        edit_signature(&mut app, serde_json::json!({"bar": 0, "numerator": 5})),
    ] {
        assert_eq!(
            response.error.expect("bar 0 is not a bar").kind(),
            ErrorKind::InvalidParams
        );
    }
    assert_eq!(tempo_pairs(&mut app), vec![(1, 120.0), (33, 140.0)]);
}

#[test]
fn the_edits_are_advertised_in_the_hello_capabilities() {
    let capabilities = resonance_control::methods::capabilities();
    assert!(capabilities.contains(&proto::EDIT_TEMPO_EVENT));
    assert!(capabilities.contains(&proto::EDIT_SIGNATURE_EVENT));
}
