//! `global.*` — the song-wide tempo and time-signature tracks: the read
//! path (`list_events`, ba todo #1380), the adds (`add_tempo_event` /
//! `add_signature_event`, ba todo #1382), the edits (ba todo #1383) and
//! the removes (ba todo #1384), against design doc #286.
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
use resonance_app::state::{GlobalTrackKind, SelectedGlobalEvent, ViewMode};
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
fn an_added_tempo_event_is_undoable_in_exactly_one_entry() {
    // The contract for every control edit: it lands in undo like a
    // manual one. It only holds because the handler synthesizes a
    // `GlobalTrackMessage` and routes it through `update()`, where
    // `undo::classify` maps `GlobalTrack(_)` to `Record` — writing
    // `app.tempo_events` directly would pass every assertion above and
    // silently bypass undo (doc #286 §3).
    //
    // The COUNT is asserted alongside the round trip, and redo after it,
    // because a value-only round trip passes just as happily when the
    // add recorded two entries: the first undo would restore the track
    // and the second would silently roll back whatever the client did
    // before it (ba todo #1385).
    let mut app = app_with_project();
    let entries_before = undo_entry_count(&app);
    let _: MutationAck = add_tempo(&mut app, 33, 140.0).result().expect("add succeeds");
    assert_eq!(tempo_pairs(&mut app), vec![(1, 120.0), (33, 140.0)]);
    assert_eq!(undo_entry_count(&app), entries_before + 1, "one add, one entry");

    let _ = app.update(Message::Undo);

    assert_eq!(
        tempo_pairs(&mut app),
        vec![(1, 120.0)],
        "undo should take the tempo event back off the track"
    );
    assert_eq!(
        undo_entry_count(&app),
        entries_before,
        "and the single entry is spent: a second undo would otherwise eat the caller's \
         previous edit"
    );

    let _ = app.update(Message::Redo);
    assert_eq!(
        tempo_pairs(&mut app),
        vec![(1, 120.0), (33, 140.0)],
        "and redo puts it back"
    );
}

#[test]
fn an_added_signature_event_is_undoable_in_exactly_one_entry() {
    // Same contract as the tempo add, and worth its own test because the
    // two take different routes into the classifier's `GlobalTrack(_) =>
    // Record` arm — and because `AddSignatureEvent` also sets the shelf
    // selection, which is the kind of extra state change that tempts a
    // second undo entry into existence.
    let mut app = app_with_project();
    let entries_before = undo_entry_count(&app);
    let _: MutationAck = add_signature(&mut app, 33, 7, 8).result().expect("add succeeds");
    assert_eq!(meter_triples(&mut app), vec![(1, 4, 4), (33, 7, 8)]);
    assert_eq!(undo_entry_count(&app), entries_before + 1, "one add, one entry");

    let _ = app.update(Message::Undo);

    assert_eq!(
        meter_triples(&mut app),
        vec![(1, 4, 4)],
        "undo should take the meter change back off the track"
    );
    assert_eq!(undo_entry_count(&app), entries_before, "exactly one entry");

    let _ = app.update(Message::Redo);
    assert_eq!(
        meter_triples(&mut app),
        vec![(1, 4, 4), (33, 7, 8)],
        "and redo puts it back"
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

// ---------------------------------------------------------------------------
// `global.remove_tempo_event` / `global.remove_signature_event` (ba todo #1384)
// ---------------------------------------------------------------------------
//
// Two things are under test here that the assertions alone do not make
// obvious.
//
// 1. THE UNDO ROUTE. Removal has no `GlobalTrackMessage` of its own.
//    `Resonance::remove_tempo_event(index)` / `remove_signature_event`
//    are `pub(crate)` methods that splice the vector, rebuild the tempo
//    map and send the engine command — everything a handler appears to
//    need — but they are not messages, so calling one bypasses
//    `Resonance::update` and therefore `undo::classify`. The deletion
//    works, reads back, acks with a revision, and can never be undone.
//    The handler instead dispatches `SelectEvent` (classified `Skip`)
//    then `DeleteSelectedEvent` (which falls to `GlobalTrack(_) =>
//    Record`). The `*_is_undoable` tests below are what make that route
//    load-bearing: replacing the pair with a direct
//    `app.remove_tempo_event(index)` passes every other test in this
//    section and fails those two.
//
// 2. BAR 1 IS REFUSED, NOT IGNORED. Both helpers guard with `index > 0`
//    and then simply return, so a wrapper that dispatched into that
//    guard would report success and delete nothing.

fn remove_tempo(app: &mut Resonance, bar: u32) -> Response {
    call(
        app,
        proto::REMOVE_TEMPO_EVENT,
        &serde_json::json!({ "bar": bar }),
    )
}

fn remove_signature(app: &mut Resonance, bar: u32) -> Response {
    call(
        app,
        proto::REMOVE_SIGNATURE_EVENT,
        &serde_json::json!({ "bar": bar }),
    )
}

#[test]
fn removing_a_tempo_event_leaves_the_previous_tempo_running_through_that_bar() {
    let mut app = app_with_a_bridge();
    let ack: MutationAck = remove_tempo(&mut app, 33)
        .result()
        .expect("global.remove_tempo_event succeeds");
    assert_eq!(ack.revision, app.revision(), "the ack carries the post-removal revision");

    assert_eq!(
        tempo_pairs(&mut app),
        vec![(1, 120.0)],
        "the bridge's tempo change is gone; 120 now runs through bar 33"
    );
}

#[test]
fn removing_a_signature_event_leaves_the_previous_meter_running_through_that_bar() {
    let mut app = app_with_a_bridge();
    let _: MutationAck = remove_signature(&mut app, 33)
        .result()
        .expect("global.remove_signature_event succeeds");

    assert_eq!(meter_triples(&mut app), vec![(1, 4, 4)], "the 7/8 bridge is back in 4/4");
}

#[test]
fn removing_the_middle_of_three_tempo_events_takes_the_named_one() {
    // The handler resolves bar -> index and hands that index to
    // `SelectedGlobalEvent`. An off-by-one there, or an index resolved
    // before some other mutation re-sorted the list, would delete a
    // neighbour — and the ack would look identical.
    let mut app = app_with_a_bridge();
    let _: MutationAck = add_tempo(&mut app, 17, 100.0).result().expect("second add");
    let _: MutationAck = add_tempo(&mut app, 49, 90.0).result().expect("third add");
    assert_eq!(
        tempo_pairs(&mut app),
        vec![(1, 120.0), (17, 100.0), (33, 140.0), (49, 90.0)]
    );

    let _: MutationAck = remove_tempo(&mut app, 33).result().expect("removal succeeds");

    assert_eq!(
        tempo_pairs(&mut app),
        vec![(1, 120.0), (17, 100.0), (49, 90.0)],
        "only bar 33 went"
    );
}

#[test]
fn a_removed_tempo_event_is_undoable() {
    // THE point of this slice, and the reason the handler does not call
    // `Resonance::remove_tempo_event` directly. One undo must put the
    // event back with BOTH its fields, and must cost exactly one entry —
    // `SelectEvent` is `UndoAction::Skip`, so aiming the delete adds
    // nothing to the history.
    let mut app = app_with_a_bridge();
    let before = tempo_pairs(&mut app);
    let entries_before = undo_entry_count(&app);

    let _: MutationAck = remove_tempo(&mut app, 33).result().expect("removal succeeds");
    assert_eq!(tempo_pairs(&mut app), vec![(1, 120.0)]);

    let _ = app.update(Message::Undo);

    assert_eq!(
        tempo_pairs(&mut app),
        before,
        "one undo must restore the removed event with its bar AND its bpm"
    );
    assert_eq!(
        undo_entry_count(&app),
        entries_before,
        "the select+delete pair is ONE entry: SelectEvent is classified Skip, so a second \
         entry would mean the aiming dispatch is recording history of its own"
    );

    let _ = app.update(Message::Redo);
    assert_eq!(tempo_pairs(&mut app), vec![(1, 120.0)], "and redo takes it away again");
}

#[test]
fn a_removed_signature_event_is_undoable() {
    // Same contract for the meter track, asserted separately because it
    // travels through a different helper (`remove_signature_event`, which
    // also re-derives the transport's displayed meter).
    let mut app = app_with_a_bridge();
    let before = meter_triples(&mut app);
    let entries_before = undo_entry_count(&app);

    let _: MutationAck = remove_signature(&mut app, 33).result().expect("removal succeeds");
    assert_eq!(meter_triples(&mut app), vec![(1, 4, 4)]);

    let _ = app.update(Message::Undo);

    assert_eq!(
        meter_triples(&mut app),
        before,
        "one undo must restore the removed event with its bar, numerator AND denominator"
    );
    assert_eq!(undo_entry_count(&app), entries_before, "exactly one entry");

    let _ = app.update(Message::Redo);
    assert_eq!(meter_triples(&mut app), vec![(1, 4, 4)]);
}

#[test]
fn a_removal_records_exactly_one_revision_bump() {
    // Two messages go out and exactly one of them is a committed change,
    // so the counter a client watches for concurrent user edits must move
    // by one — not by two, which would read as "someone else is editing"
    // after every removal.
    let mut app = app_with_a_bridge();
    let before = app.revision();
    let _: MutationAck = remove_tempo(&mut app, 33).result().expect("removal succeeds");
    assert_eq!(app.revision(), before + 1);
}

#[test]
fn an_undone_removal_takes_the_gui_tempo_map_with_it() {
    // The event vectors are not the only thing the removal touched:
    // `rebuild_and_send_tempo` runs inside the domain helper, and the
    // restore path re-runs it. A test reading only the wire lists would
    // miss a tempo map still describing the shortened song.
    let mut app = app_with_a_bridge();
    let _: MutationAck = remove_tempo(&mut app, 33).result().expect("removal succeeds");
    assert_eq!(app.test_tempo_map().tempo_points.len(), 1);

    let _ = app.update(Message::Undo);

    let points = &app.test_tempo_map().tempo_points;
    assert_eq!(points.len(), 2, "{points:?}");
    assert!(
        (points[1].bpm - 140.0).abs() < 1e-4,
        "the tempo map must track the undo, not just the event list: {points:?}"
    );
}

#[test]
fn removing_the_bar_1_tempo_event_is_refused_not_ignored() {
    // `Resonance::remove_tempo_event` guards with `index > 0` and then
    // RETURNS — no error, nothing removed. A wrapper that dispatched into
    // that guard would ack with a revision and leave the track untouched,
    // which is the one outcome a client cannot diagnose.
    let mut app = app_with_a_bridge();
    let before = tempo_pairs(&mut app);
    let entries_before = undo_entry_count(&app);

    let error = remove_tempo(&mut app, 1)
        .error
        .expect("removing the song's initial tempo must be refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(
        error.message.contains("bar 1"),
        "the refusal must name what it refused: {}",
        error.message
    );

    assert_eq!(tempo_pairs(&mut app), before, "nothing removed");
    assert_eq!(
        undo_entry_count(&app),
        entries_before,
        "a refusal must not leave an empty entry in the history"
    );
}

#[test]
fn removing_the_bar_1_signature_event_is_refused_not_ignored() {
    let mut app = app_with_a_bridge();
    let before = meter_triples(&mut app);

    let error = remove_signature(&mut app, 1)
        .error
        .expect("removing the song's initial meter must be refused");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(error.message.contains("bar 1"), "{}", error.message);

    assert_eq!(meter_triples(&mut app), before, "nothing removed");
}

#[test]
fn removing_a_bar_with_no_event_on_it_is_refused_rather_than_acked_as_already_gone() {
    // "It is not there, so consider it removed" is the tempting reading
    // and the wrong one: an empty bar means the change the caller meant
    // to drop is still in the song at some other bar. #1383 set this
    // precedent for `edit_*`.
    let mut app = app_with_a_bridge();

    let error = remove_tempo(&mut app, 20)
        .error
        .expect("bar 20 carries no tempo event");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);
    assert!(error.message.contains("bar 20"), "{}", error.message);

    let error = remove_signature(&mut app, 20)
        .error
        .expect("bar 20 carries no meter event");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);

    assert_eq!(
        tempo_pairs(&mut app),
        vec![(1, 120.0), (33, 140.0)],
        "and nothing else was removed on the way past"
    );
    assert_eq!(meter_triples(&mut app), vec![(1, 4, 4), (33, 7, 8)]);
}

#[test]
fn removing_a_bar_that_only_the_other_track_has_an_event_on_is_refused() {
    // The two tracks are addressed by the same bar numbers but are
    // separate lists. Removing the tempo event at a bar that carries only
    // a meter change must not fall through to the meter track.
    let mut app = app_with_project();
    let _: MutationAck = add_signature(&mut app, 17, 5, 4).result().expect("meter add");

    let error = remove_tempo(&mut app, 17)
        .error
        .expect("bar 17 carries a meter event, not a tempo event");
    assert_eq!(error.kind(), ErrorKind::InvalidParams);

    assert_eq!(meter_triples(&mut app), vec![(1, 4, 4), (17, 5, 4)], "untouched");
}

#[test]
fn bar_0_is_refused_by_the_removes_too() {
    let mut app = app_with_a_bridge();
    for response in [remove_tempo(&mut app, 0), remove_signature(&mut app, 0)] {
        assert_eq!(
            response.error.expect("bar 0 is not a bar").kind(),
            ErrorKind::InvalidParams
        );
    }
    assert_eq!(tempo_pairs(&mut app), vec![(1, 120.0), (33, 140.0)]);
}

#[test]
fn a_removal_clears_the_shelf_selection_rather_than_restoring_the_users() {
    // A decided side effect, pinned so it cannot drift either way.
    //
    // The only message route to a deletion is `DeleteSelectedEvent`,
    // which takes its target from `interaction.selected_global_event`, so
    // the handler has to aim it — overwriting whatever the user had
    // selected — and the message then `take()`s it. The end state is
    // therefore "nothing selected", and it is deliberately NOT restored:
    // `SelectedGlobalEvent` holds an INDEX, every index above the removed
    // one has just shifted down by one, and putting the old value back
    // would leave the shelf's inline pick_lists and its Delete key aimed
    // at the event NEXT to the one the user picked. Cleared is also
    // exactly where the GUI's own delete leaves it.
    let mut app = app_with_a_bridge();
    let _: MutationAck = add_tempo(&mut app, 49, 90.0).result().expect("third event");

    // The user has bar 49 selected — index 2, which the removal below
    // shifts to index 1.
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::SelectEvent(Some(
        SelectedGlobalEvent {
            kind: GlobalTrackKind::Tempo,
            index: 2,
        },
    ))));

    let _: MutationAck = remove_tempo(&mut app, 33).result().expect("removal succeeds");

    assert_eq!(
        app.test_selected_global_event(),
        None,
        "the selection is left cleared: restoring index 2 would now point past the end, and \
         restoring any stale index would re-aim the shelf's Delete key at a neighbour"
    );
}

#[test]
fn a_refused_removal_leaves_the_shelf_selection_alone() {
    // The refusals happen BEFORE the aiming dispatch, so a rejected call
    // must not disturb what the user is looking at either.
    let mut app = app_with_a_bridge();
    let selected = SelectedGlobalEvent {
        kind: GlobalTrackKind::Tempo,
        index: 1,
    };
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::SelectEvent(Some(
        selected,
    ))));

    assert!(remove_tempo(&mut app, 1).error.is_some());
    assert!(remove_tempo(&mut app, 20).error.is_some());

    assert_eq!(app.test_selected_global_event(), Some(selected));
}

#[test]
fn a_removal_reaches_the_engine_and_the_gui_tempo_map() {
    let mut app = app_with_a_bridge();
    let _: MutationAck = remove_signature(&mut app, 33).result().expect("removal succeeds");

    assert_eq!(
        app.test_tempo_map().signature_points.len(),
        1,
        "the shelf's own tempo map follows the removal, not just the event list"
    );
}

#[test]
fn the_removes_are_advertised_in_the_hello_capabilities() {
    let capabilities = resonance_control::methods::capabilities();
    assert!(capabilities.contains(&proto::REMOVE_TEMPO_EVENT));
    assert!(capabilities.contains(&proto::REMOVE_SIGNATURE_EVENT));
}

// ---------------------------------------------------------------------------
// The undo contract across the whole namespace (ba todo #1385)
// ---------------------------------------------------------------------------
//
// The three slices above each pin their own mutators' undo behaviour, and
// between them all six are round-tripped. Two things no per-slice test can
// reach are left, and they are what this section is:
//
// 1. THE CLASSIFICATION ITSELF. Every handler's correctness rests on four
//    `undo::classify` arms, and until now all four were asserted only
//    INDIRECTLY, through the state a round trip leaves behind. Reclassify
//    `SelectEvent` from `Skip` to `Record` and the removes quietly start
//    recording two entries per deletion; reclassify `UpdateTempoEvent`
//    from `Skip` to `Record` and every tempo edit records two. Both
//    failures surface a long way from the change, as a second undo doing
//    something surprising. `classify` is a pure function of the message,
//    so the arms can simply be asserted.
//
// 2. A MIXED SEQUENCE. Every test above starts from a clean project and
//    performs one edit, so all of them pass even if the entry accounting
//    is off by a constant. It takes a run of edits across BOTH tracks,
//    undone one at a time back to the start, to show that each edit's
//    entry restores exactly its own edit and no more.

/// Every `GlobalTrackMessage` variant's undo classification, as the
/// `global.*` handlers rely on it.
///
/// The match is exhaustive on purpose: a new variant will not compile
/// until someone states what undo should do with it, rather than
/// inheriting `Record` from the classifier's `GlobalTrack(_)` catch-all
/// by default. That default is right for a message that mutates the
/// track and wrong for anything gesture-shaped or view-only, and the
/// difference is invisible at the call site.
fn expected_action(message: &GlobalTrackMessage) -> &'static str {
    match message {
        // Aiming the shelf (and the removes' `DeleteSelectedEvent`) is
        // view state, not an edit.
        GlobalTrackMessage::SelectEvent(_) => "Skip",
        // The drag bracket: `UpdateTempoEvent` is the mid-gesture move,
        // so its entry comes from the Begin/Commit pair around it. This
        // is why `global.edit_tempo_event` dispatches all three.
        GlobalTrackMessage::StartTempoDrag(_) => "Begin",
        GlobalTrackMessage::UpdateTempoEvent { .. } => "Skip",
        GlobalTrackMessage::EndTempoDrag => "Commit",
        // The plain edits: one dispatch, one entry.
        GlobalTrackMessage::AddTempoEvent { .. } => "Record",
        GlobalTrackMessage::AddSignatureEvent { .. } => "Record",
        GlobalTrackMessage::UpdateSignatureEvent { .. } => "Record",
        GlobalTrackMessage::DeleteSelectedEvent => "Record",
    }
}

fn action_name(action: &resonance_app::undo::UndoAction) -> &'static str {
    use resonance_app::undo::UndoAction;
    match action {
        UndoAction::Skip => "Skip",
        UndoAction::Record => "Record",
        UndoAction::RecordCoalesced(_) => "RecordCoalesced",
        UndoAction::Begin => "Begin",
        UndoAction::Commit => "Commit",
    }
}

#[test]
fn the_classifier_arms_the_global_handlers_depend_on_are_what_they_claim() {
    let messages = [
        GlobalTrackMessage::AddTempoEvent { bar: 32, bpm: 140.0 },
        GlobalTrackMessage::UpdateTempoEvent {
            index: 1,
            bar: 32,
            bpm: 140.0,
        },
        GlobalTrackMessage::StartTempoDrag(1),
        GlobalTrackMessage::EndTempoDrag,
        GlobalTrackMessage::AddSignatureEvent {
            bar: 32,
            numerator: 7,
            denominator: 8,
        },
        GlobalTrackMessage::UpdateSignatureEvent {
            index: 1,
            numerator: 7,
            denominator: 8,
        },
        GlobalTrackMessage::SelectEvent(Some(SelectedGlobalEvent {
            kind: GlobalTrackKind::Tempo,
            index: 1,
        })),
        GlobalTrackMessage::SelectEvent(None),
        GlobalTrackMessage::DeleteSelectedEvent,
    ];

    for message in messages {
        let expected = expected_action(&message);
        let actual = action_name(&resonance_app::undo::classify(&Message::GlobalTrack(
            message.clone(),
        )));
        assert_eq!(
            actual, expected,
            "undo::classify({message:?}) is {actual}, not {expected} — the global.* handlers \
             are built on this classification, and changing it changes how many undo entries \
             every one of them records"
        );
    }
}

#[test]
fn a_run_of_edits_across_both_tracks_undoes_one_edit_at_a_time_back_to_the_start() {
    // The off-by-one detector. Four edits of four different shapes — an
    // add on each track, the bracketed tempo edit, and a removal (which
    // is itself two dispatches) — then four undos, each asserted to land
    // on the state the previous step left. If any single edit records
    // two entries, or none, the run walks off by that much and one of
    // these intermediate assertions fails; the per-slice tests above all
    // still pass, because each of them starts clean and stops after one
    // undo.
    let mut app = app_with_project();
    assert_eq!(
        undo_entry_count(&app),
        0,
        "a freshly opened project has nothing to undo"
    );

    // The four states the run passes through, oldest first.
    let initial = (vec![(1, 120.0)], vec![(1, 4, 4)]);
    let after_tempo_add = (vec![(1, 120.0), (33, 140.0)], vec![(1, 4, 4)]);
    let after_meter_add = (vec![(1, 120.0), (33, 140.0)], vec![(1, 4, 4), (33, 7, 8)]);
    let after_tempo_edit = (vec![(1, 120.0), (17, 96.0)], vec![(1, 4, 4), (33, 7, 8)]);
    let after_meter_remove = (vec![(1, 120.0), (17, 96.0)], vec![(1, 4, 4)]);

    let _: MutationAck = add_tempo(&mut app, 33, 140.0).result().expect("tempo add");
    let _: MutationAck = add_signature(&mut app, 33, 7, 8).result().expect("meter add");
    // Retune AND relocate in one call, so the undone entry has to carry
    // both halves of the bracket's edit.
    let _: MutationAck = edit_tempo(&mut app, serde_json::json!({"bar": 33, "bpm": 96.0, "new_bar": 17}))
        .result()
        .expect("tempo edit");
    let _: MutationAck = remove_signature(&mut app, 33).result().expect("meter removal");

    assert_eq!(state(&mut app), after_meter_remove.clone());
    assert_eq!(
        undo_entry_count(&app),
        4,
        "four client edits, four undo entries — the selection dispatches the removal and the \
         edit bracket also make are classified Skip and must not add any"
    );

    for expected in [
        after_tempo_edit.clone(),
        after_meter_add.clone(),
        after_tempo_add.clone(),
        initial.clone(),
    ] {
        let _ = app.update(Message::Undo);
        assert_eq!(
            state(&mut app),
            expected,
            "each undo must step back exactly one client edit"
        );
    }
    assert_eq!(undo_entry_count(&app), 0, "and the history is spent, not overdrawn");

    // A fifth undo has nothing of ours left to take, and must not reach
    // behind the start of the run.
    let _ = app.update(Message::Undo);
    assert_eq!(state(&mut app), initial, "nothing left to undo");

    // Forwards again, for the same reason: a redo stack that gained or
    // lost an entry replays the run out of step.
    for expected in [
        after_tempo_add,
        after_meter_add,
        after_tempo_edit,
        after_meter_remove,
    ] {
        let _ = app.update(Message::Redo);
        assert_eq!(
            state(&mut app),
            expected,
            "each redo must step forward exactly one client edit"
        );
    }
}

/// Both tracks as the wire reports them, for step-by-step comparison
/// through an undo run.
fn state(app: &mut Resonance) -> (Vec<(u32, f32)>, Vec<(u32, u8, u8)>) {
    (tempo_pairs(app), meter_triples(app))
}

// ---------------- bar bounds (CTL-05) ----------------

/// A global event past `MAX_BARS` is refused like bar 0: nothing sane
/// lives there, and bar arithmetic downstream is plain u32.
#[test]
fn events_past_max_bars_are_invalid_params() {
    let mut app = app_with_project();
    let before = tempo_pairs(&mut app);
    for bar in [resonance_control::MAX_BARS + 1, u32::MAX] {
        let error = add_tempo(&mut app, bar, 100.0)
            .error
            .unwrap_or_else(|| panic!("bar {bar} must be refused"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams);
        let error = add_signature(&mut app, bar, 3, 4)
            .error
            .unwrap_or_else(|| panic!("bar {bar} must be refused"));
        assert_eq!(error.kind(), ErrorKind::InvalidParams);
    }
    assert_eq!(tempo_pairs(&mut app), before);
}
