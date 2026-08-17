//! Selecting a BUS strip in the mixer, and what the inspector then
//! describes.
//!
//! The inspector resolved its selection out of `registry.tracks` only,
//! and nothing ever set a bus selection — so clicking a bus strip left
//! the pane showing the previously selected TRACK. The pane's own empty
//! state said "Select a track or bus", which is what made it read as
//! broken rather than absent.
//!
//! Bus ids and track ids are separate id spaces that overlap
//! numerically, so the selection cannot share one field: `bus 1` and
//! `track 1` both exist. These pin the two-field invariant (exactly one
//! selected channel), the click path, and the cleanup when the selected
//! bus is deleted.

use resonance_app::message::{Message, TrackMessage, UiMessage};
use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::{AudioEvent, TrackOutput, TrackType};

const KICK: u64 = 1;
const BUS: u64 = 1; // deliberately the same number as KICK
const OTHER_BUS: u64 = 2;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    app.test_add_track(KICK, TrackType::Instrument);
    app.test_add_bus(BUS, "Drum Bus");
    app.test_add_bus(OTHER_BUS, "FX Bus");
    app
}

fn select_bus(app: &mut Resonance, id: Option<u64>) {
    let _ = app.update(Message::Ui(UiMessage::SelectBus(id)));
}

fn select_track(app: &mut Resonance, id: Option<u64>) {
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(id)));
}

#[test]
fn selecting_a_bus_records_it_as_the_selected_bus() {
    let mut app = app();
    assert_eq!(app.test_selected_bus(), None, "nothing selected at rest");

    select_bus(&mut app, Some(BUS));
    assert_eq!(app.test_selected_bus(), Some(BUS));

    select_bus(&mut app, Some(OTHER_BUS));
    assert_eq!(app.test_selected_bus(), Some(OTHER_BUS), "selection moves");

    select_bus(&mut app, None);
    assert_eq!(app.test_selected_bus(), None, "and clears");
}

/// The invariant the two fields exist to keep: the inspector describes
/// exactly one channel, so selecting one kind releases the other.
#[test]
fn a_bus_and_a_track_are_never_selected_at_once() {
    let mut app = app();

    select_track(&mut app, Some(KICK));
    assert_eq!(app.test_selected_track(), Some(KICK));
    assert_eq!(app.test_selected_bus(), None);

    select_bus(&mut app, Some(BUS));
    assert_eq!(app.test_selected_bus(), Some(BUS));
    assert_eq!(
        app.test_selected_track(),
        None,
        "the track highlight is released, so the mixer never shows two"
    );
    assert!(
        app.test_selected_tracks().is_empty(),
        "and the multi-selection goes with it"
    );

    select_track(&mut app, Some(KICK));
    assert_eq!(app.test_selected_track(), Some(KICK));
    assert_eq!(app.test_selected_bus(), None, "and back the other way");
}

/// `SelectTrack(None)` is "deselect all", raised by clicking empty
/// space. It must not silently steal a bus selection made a moment ago.
#[test]
fn deselecting_all_tracks_leaves_a_selected_bus_alone() {
    let mut app = app();
    select_bus(&mut app, Some(BUS));
    select_track(&mut app, None);
    assert_eq!(app.test_selected_bus(), Some(BUS));
}

/// Bus 1 and track 1 both exist. Selecting one must not resolve to the
/// other — the whole reason the selection is two fields.
#[test]
fn a_bus_id_that_collides_with_a_track_id_stays_distinct() {
    let mut app = app();
    assert_eq!(BUS, KICK, "this test is meaningless unless the ids collide");

    select_bus(&mut app, Some(BUS));
    assert_eq!(app.test_selected_bus(), Some(BUS));
    assert_eq!(app.test_selected_track(), None);
}

/// Deleting the selected bus must not leave the inspector pointed at a
/// bus that no longer exists.
#[test]
fn deleting_the_selected_bus_clears_the_selection() {
    let mut app = app();
    select_bus(&mut app, Some(BUS));
    app.test_apply_engine_event(AudioEvent::BusRemoved { bus_id: BUS });
    assert_eq!(app.test_selected_bus(), None);

    // A different bus going away leaves the selection alone.
    select_bus(&mut app, Some(OTHER_BUS));
    app.test_add_bus(BUS, "Drum Bus");
    app.test_apply_engine_event(AudioEvent::BusRemoved { bus_id: BUS });
    assert_eq!(app.test_selected_bus(), Some(OTHER_BUS));
}

/// The inspector reads the routing off the tracks to answer "what feeds
/// this bus", so the view has to be built from a state where that
/// routing exists. Rendering it is the golden test's job; this pins that
/// the state it reads is reachable over the normal message path.
#[test]
fn a_track_routed_to_the_selected_bus_is_visible_in_state() {
    let mut app = app();
    let _ = app.update(Message::Track(TrackMessage::SetTrackOutput(
        KICK,
        TrackOutput::Bus(BUS),
    )));
    select_bus(&mut app, Some(BUS));

    let view = app.view();
    // The view must build without panicking with a bus selected — the
    // pane used to take the track path unconditionally.
    drop(view);
    assert_eq!(app.test_selected_bus(), Some(BUS));
}
