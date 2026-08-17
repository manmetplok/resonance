//! Behavioural coverage for the right-rail Entry-inspector messages added
//! for the drum-arrangement inspector + pattern-bank drag source (todo
//! #490, doc #170): pattern swap (`SetEntryPattern`), entry selection
//! (`SelectEntry`, incl. auto-select on add and clamp on remove), and the
//! undo-skip classification of a pure selection.
//!
//! Like the sibling `compose_drum_arrangement_edit` suite these drive the
//! real `update` reducer against the demo project and assert on the focused
//! section's `arrangement` + the drumroll view state.

use resonance_app::compose::messages::{ArrangementMessage, DrumGroupsMessage};
use resonance_app::compose::{ComposeMessage, EntryLength, PatternEntry};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{demo, Resonance, STARTUP_TAB};

fn build_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Compose);
    let (mut app, _task) = Resonance::new_for_test();
    demo::seed_demo_content(&mut app);
    app
}

fn focused_definition(app: &Resonance) -> u64 {
    app.compose_state()
        .selected_placement()
        .expect("demo seeds a selected placement")
        .definition_id
}

fn pattern_ids(app: &Resonance) -> (u64, u64) {
    let bank = &app.compose_state().drum_patterns;
    (bank[0].id, bank[1].id)
}

fn arrangement(app: &Resonance, def: u64) -> Vec<PatternEntry> {
    app.compose_state()
        .find_definition(def)
        .expect("definition exists")
        .arrangement
        .clone()
}

fn selected_entry(app: &Resonance) -> Option<usize> {
    app.compose_state().drumroll.selected_entry_index
}

fn send(app: &mut Resonance, msg: ArrangementMessage) {
    let _ = app.update(Message::Compose(ComposeMessage::Arrangement(msg)));
}

fn clear_arrangement(app: &mut Resonance, def: u64) {
    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::AssignPattern {
            definition_id: def,
            pattern_id: None,
        },
    )));
    assert!(arrangement(app, def).is_empty());
}

#[test]
fn set_entry_pattern_swaps_and_validates() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    clear_arrangement(&mut app, def);
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });
    // Give the entry a fill + a fixed length so we can prove the swap
    // preserves everything but the pattern.
    send(&mut app, ArrangementMessage::SetEntryLength {
        definition_id: def,
        index: 0,
        length: EntryLength::Bars(3),
    });
    send(&mut app, ArrangementMessage::SetEntryFill { definition_id: def, index: 0, fill: Some(p_b) });

    send(&mut app, ArrangementMessage::SetEntryPattern {
        definition_id: def,
        index: 0,
        pattern_id: p_b,
    });
    let arr = arrangement(&app, def);
    assert_eq!(arr[0].pattern_id, p_b);
    assert_eq!(arr[0].length, EntryLength::Bars(3));
    assert_eq!(arr[0].fill, Some(p_b));

    // An unknown pattern id is rejected — the pattern stands.
    send(&mut app, ArrangementMessage::SetEntryPattern {
        definition_id: def,
        index: 0,
        pattern_id: 9_999_999,
    });
    assert_eq!(arrangement(&app, def)[0].pattern_id, p_b);

    // Out-of-range index is a no-op (no panic, no phantom entry).
    send(&mut app, ArrangementMessage::SetEntryPattern {
        definition_id: def,
        index: 7,
        pattern_id: p_main,
    });
    assert_eq!(arrangement(&app, def).len(), 1);
}

#[test]
fn adding_a_bank_pattern_selects_the_new_entry() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    clear_arrangement(&mut app, def);

    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });
    assert_eq!(selected_entry(&app), Some(0));

    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_b });
    assert_eq!(selected_entry(&app), Some(1));

    // A rejected add (unknown pattern) doesn't move the selection.
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: 42_424_242 });
    assert_eq!(selected_entry(&app), Some(1));
}

#[test]
fn select_entry_focuses_and_clears() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    clear_arrangement(&mut app, def);
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_b });

    send(&mut app, ArrangementMessage::SelectEntry { index: Some(0) });
    assert_eq!(selected_entry(&app), Some(0));

    send(&mut app, ArrangementMessage::SelectEntry { index: None });
    assert_eq!(selected_entry(&app), None);
}

#[test]
fn removing_an_entry_clamps_the_selection() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    clear_arrangement(&mut app, def);
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_b });

    // Select the tail entry, then remove it → selection clamps to the new last.
    send(&mut app, ArrangementMessage::SelectEntry { index: Some(1) });
    send(&mut app, ArrangementMessage::RemoveEntry { definition_id: def, index: 1 });
    assert_eq!(arrangement(&app, def).len(), 1);
    assert_eq!(selected_entry(&app), Some(0));

    // Removing the final entry clears the selection entirely.
    send(&mut app, ArrangementMessage::RemoveEntry { definition_id: def, index: 0 });
    assert!(arrangement(&app, def).is_empty());
    assert_eq!(selected_entry(&app), None);
}

#[test]
fn selecting_an_entry_is_not_undoable() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/resonance-inspector-undo-test"));

    clear_arrangement(&mut app, def);
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });
    let before = arrangement(&app, def);

    // The real edit: append a second entry.
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_b });
    assert_eq!(arrangement(&app, def).len(), 2);

    // A pure selection between the edit and the undo must NOT consume the
    // undo step — Undo still reverts the append.
    send(&mut app, ArrangementMessage::SelectEntry { index: Some(0) });
    let _ = app.update(Message::Undo);
    assert_eq!(arrangement(&app, def), before);
}
