//! Coverage for `MarkerMessage::SeedFromSections` (todo #371 / doc #161).
//!
//! Seeding derives one ranged arrangement marker per Compose section
//! placement — name + colour copied from the section definition, span
//! computed from `start_bar` + `length_bars` via the tempo map. These
//! tests drive the reducer through the public `Resonance::update` entry
//! point and assert the placement → marker mapping, that hand-placed
//! markers survive a re-seed, and the undo classification.

use resonance_app::compose::ComposeMessage;
use resonance_app::message::{MarkerMessage, Message, TransportMessage};
use resonance_app::state::ArrangementMarker;
use resonance_app::undo::{classify, UndoAction};
use resonance_app::Resonance;

const SAMPLE_RATE: u32 = 48_000;

/// A fresh app with an active project (so Compose / marker messages aren't
/// swallowed by the startup-modal gate) on a deterministic 120 BPM / 4-4
/// grid, matching the marker-reducer test fixture.
fn app() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_sample_rate(SAMPLE_RATE);
    let _ = app.update(Message::Transport(TransportMessage::SetBpmText("120".into())));
    let _ = app.update(Message::Transport(TransportMessage::CommitBpm));
    app
}

/// Create a section definition (and its auto-placement at the first free
/// bar) with the given name/length/colour, returning its definition id.
fn create_section(app: &mut Resonance, name: &str, length_bars: u32, color: [u8; 3]) -> u64 {
    let before: Vec<u64> = app
        .compose_state()
        .definitions
        .iter()
        .map(|d| d.id)
        .collect();
    let _ = app.update(Message::Compose(ComposeMessage::CreateSection {
        name: name.to_string(),
        length_bars,
        color,
    }));
    // The fresh definition is the one id that wasn't there before.
    app.compose_state()
        .definitions
        .iter()
        .map(|d| d.id)
        .find(|id| !before.contains(id))
        .expect("CreateSection should add a definition")
}

#[test]
fn seed_maps_each_placement_to_a_named_coloured_region() {
    let mut app = app();
    create_section(&mut app, "Intro", 4, [10, 20, 30]);
    create_section(&mut app, "Verse", 8, [40, 50, 60]);
    create_section(&mut app, "Chorus", 4, [70, 80, 90]);

    let _ = app.update(Message::Marker(MarkerMessage::SeedFromSections));

    // One marker per placement, all ranged + tagged seeded.
    let placements = app.compose_state().placements.clone();
    assert_eq!(placements.len(), 3, "three sections were created");
    let markers = app.test_markers();
    assert_eq!(markers.len(), placements.len());

    let tempo = app.test_tempo_map().clone();
    for placement in &placements {
        let def = app
            .compose_state()
            .find_definition(placement.definition_id)
            .expect("placement resolves to a definition");
        let expected_start = tempo.bar_to_sample(placement.start_bar);
        let expected_end = tempo.bar_to_sample(placement.start_bar + def.length_bars);

        let marker = markers
            .as_slice()
            .iter()
            .find(|m| m.start_sample == expected_start)
            .unwrap_or_else(|| panic!("no marker at bar {}", placement.start_bar));

        assert!(marker.seeded, "section-derived markers are tagged seeded");
        assert!(marker.is_region(), "a placement becomes a ranged marker");
        assert_eq!(marker.name, def.name, "marker name copies the section name");
        assert_eq!(marker.color, def.color, "marker colour copies the section");
        assert_eq!(marker.end_sample, Some(expected_end), "span ends at start+length");
        assert!(
            expected_end > expected_start,
            "a non-empty section spans a positive range"
        );
    }
}

#[test]
fn reseed_replaces_seeded_and_preserves_hand_placed_markers() {
    let mut app = app();
    create_section(&mut app, "Intro", 4, [10, 20, 30]);

    // A hand-placed point marker the user dropped themselves.
    let hand_id = app.test_add_marker(ArrangementMarker::new_point(
        9_001,
        "My cue".into(),
        [1, 2, 3],
        12_345,
    ));

    let _ = app.update(Message::Marker(MarkerMessage::SeedFromSections));
    assert_eq!(app.test_markers().len(), 2, "one seeded + one hand-placed");

    // Add a second section and re-seed: the seeded set rebuilds to two,
    // the hand-placed marker is untouched, and nothing duplicates.
    create_section(&mut app, "Verse", 8, [40, 50, 60]);
    let _ = app.update(Message::Marker(MarkerMessage::SeedFromSections));

    let markers = app.test_markers();
    let seeded = markers.iter().filter(|m| m.seeded).count();
    let hand = markers.iter().filter(|m| !m.seeded).count();
    assert_eq!(seeded, 2, "re-seed yields exactly one marker per placement");
    assert_eq!(hand, 1, "the hand-placed marker is never clobbered");

    let cue = markers.get(hand_id).expect("hand-placed marker survives");
    assert_eq!(cue.name, "My cue");
    assert_eq!(cue.start_sample, 12_345);
    assert!(cue.is_point());
}

#[test]
fn seeding_with_no_sections_clears_only_seeded_markers() {
    let mut app = app();
    create_section(&mut app, "Intro", 4, [10, 20, 30]);
    let _ = app.update(Message::Marker(MarkerMessage::SeedFromSections));
    assert_eq!(app.test_markers().len(), 1);

    // Remove the only placement, then re-seed: the seeded marker is gone.
    let placement_id = app.compose_state().placements[0].id;
    let _ = app.update(Message::Compose(ComposeMessage::DeleteSectionPlacement {
        placement_id,
    }));
    let _ = app.update(Message::Marker(MarkerMessage::SeedFromSections));
    assert!(
        app.test_markers().is_empty(),
        "no placements => no seeded markers remain"
    );
}

#[test]
fn seed_from_sections_records_an_undo_entry() {
    assert!(
        matches!(
            classify(&Message::Marker(MarkerMessage::SeedFromSections)),
            UndoAction::Record
        ),
        "seeding mutates the marker set, so it must record undo"
    );
}
