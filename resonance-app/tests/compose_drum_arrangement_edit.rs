//! Behavioural coverage for the drum-arrangement editing messages
//! (`ComposeMessage::Arrangement`). Each test drives the real `update`
//! reducer against the demo project and asserts on the focused section's
//! `arrangement` — add / remove / reorder / length-mode / fill / duplicate,
//! the `Fill to end` / `Trim to fit` remediations, bank-delete cleanup, and
//! an undo/redo round-trip.
//!
//! Kept out of `src` per the project's "tests live in `tests/`" convention;
//! `resonance-app` exposes a `lib.rs` so the integration crate can build a
//! `Resonance`, dispatch messages, and read `compose_state()`.

use resonance_app::compose::messages::{ArrangementMessage, DrumGroupsMessage};
use resonance_app::compose::{ComposeMessage, EntryLength, PatternEntry};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{demo, Resonance, STARTUP_TAB};

/// Demo app pinned to the Compose tab. The demo seed activates a project
/// and selects a placement, so the focused section is always resolvable.
fn build_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Compose);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    app
}

fn focused_definition(app: &Resonance) -> u64 {
    app.compose_state()
        .selected_placement()
        .expect("demo seeds a selected placement")
        .definition_id
}

/// `(main, b_section)` pattern ids from the seeded two-entry bank.
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

fn send(app: &mut Resonance, msg: ArrangementMessage) {
    let _ = app.update(Message::Compose(ComposeMessage::Arrangement(msg)));
}

/// Reset the focused section to an empty arrangement so each test starts
/// from a known state regardless of the demo seed.
fn clear_arrangement(app: &mut Resonance, def: u64) {
    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::AssignPattern {
            definition_id: def,
            pattern_id: None,
        },
    )));
    assert!(arrangement(app, def).is_empty());
}

fn set_section_length(app: &mut Resonance, def: u64, bars: u32) {
    let _ = app.update(Message::Compose(ComposeMessage::ResizeSection {
        definition_id: def,
        length_bars: bars,
    }));
}

#[test]
fn add_entry_appends_and_rejects_unknown_pattern() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    clear_arrangement(&mut app, def);

    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_b });
    // An unknown pattern id is ignored — no phantom entry appended.
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: 9_999_999 });

    let arr = arrangement(&app, def);
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0], PatternEntry::once(p_main));
    assert_eq!(arr[1], PatternEntry::once(p_b));
}

#[test]
fn remove_entry_drops_only_the_target() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    clear_arrangement(&mut app, def);
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_b });

    send(&mut app, ArrangementMessage::RemoveEntry { definition_id: def, index: 0 });
    assert_eq!(arrangement(&app, def), vec![PatternEntry::once(p_b)]);

    // Out-of-range remove is a no-op.
    send(&mut app, ArrangementMessage::RemoveEntry { definition_id: def, index: 7 });
    assert_eq!(arrangement(&app, def), vec![PatternEntry::once(p_b)]);
}

#[test]
fn move_entry_reorders() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    clear_arrangement(&mut app, def);
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_b });

    // Move the second entry up to the front (drag / "move up").
    send(&mut app, ArrangementMessage::MoveEntry { definition_id: def, from: 1, to: 0 });
    let arr = arrangement(&app, def);
    assert_eq!(arr[0].pattern_id, p_b);
    assert_eq!(arr[1].pattern_id, p_main);

    // No-movement / out-of-range requests leave the order untouched.
    send(&mut app, ArrangementMessage::MoveEntry { definition_id: def, from: 0, to: 0 });
    send(&mut app, ArrangementMessage::MoveEntry { definition_id: def, from: 0, to: 9 });
    let arr = arrangement(&app, def);
    assert_eq!(arr[0].pattern_id, p_b);
    assert_eq!(arr[1].pattern_id, p_main);
}

#[test]
fn set_entry_length_switches_mode_and_value() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, _) = pattern_ids(&app);
    clear_arrangement(&mut app, def);
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });

    send(&mut app, ArrangementMessage::SetEntryLength {
        definition_id: def,
        index: 0,
        length: EntryLength::Bars(4),
    });
    assert_eq!(arrangement(&app, def)[0].length, EntryLength::Bars(4));

    send(&mut app, ArrangementMessage::SetEntryLength {
        definition_id: def,
        index: 0,
        length: EntryLength::RepeatN(3),
    });
    assert_eq!(arrangement(&app, def)[0].length, EntryLength::RepeatN(3));
}

#[test]
fn set_entry_fill_toggles_and_validates() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    clear_arrangement(&mut app, def);
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });

    // Choose a fill pattern.
    send(&mut app, ArrangementMessage::SetEntryFill { definition_id: def, index: 0, fill: Some(p_b) });
    assert_eq!(arrangement(&app, def)[0].fill, Some(p_b));

    // A non-existent fill pattern is rejected — the existing fill stands.
    send(&mut app, ArrangementMessage::SetEntryFill { definition_id: def, index: 0, fill: Some(9_999) });
    assert_eq!(arrangement(&app, def)[0].fill, Some(p_b));

    // Clearing the fill is always allowed.
    send(&mut app, ArrangementMessage::SetEntryFill { definition_id: def, index: 0, fill: None });
    assert_eq!(arrangement(&app, def)[0].fill, None);
}

#[test]
fn duplicate_entry_inserts_a_copy_after() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    clear_arrangement(&mut app, def);
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_b });
    send(&mut app, ArrangementMessage::SetEntryFill { definition_id: def, index: 0, fill: Some(p_b) });

    send(&mut app, ArrangementMessage::DuplicateEntry { definition_id: def, index: 0 });
    let arr = arrangement(&app, def);
    assert_eq!(arr.len(), 3);
    // The copy carries the fill and sits immediately after the original.
    assert_eq!(arr[0], arr[1]);
    assert_eq!(arr[1].pattern_id, p_main);
    assert_eq!(arr[1].fill, Some(p_b));
    assert_eq!(arr[2].pattern_id, p_b);
}

#[test]
fn fill_to_end_closes_a_trailing_gap() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, _) = pattern_ids(&app);
    set_section_length(&mut app, def, 8);
    clear_arrangement(&mut app, def);
    // One single-bar repeat leaves a 7-bar gap on an 8-bar section.
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });

    send(&mut app, ArrangementMessage::FillToEnd { definition_id: def });
    let arr = arrangement(&app, def);
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[1].pattern_id, p_main);
    assert_eq!(arr[1].length, EntryLength::Bars(7));

    // Now exact — a second Fill-to-end is a no-op.
    send(&mut app, ArrangementMessage::FillToEnd { definition_id: def });
    assert_eq!(arrangement(&app, def).len(), 2);
}

#[test]
fn fill_to_end_on_empty_section_lays_down_default() {
    let mut app = build_app();
    let def = focused_definition(&app);
    set_section_length(&mut app, def, 4);
    clear_arrangement(&mut app, def);

    send(&mut app, ArrangementMessage::FillToEnd { definition_id: def });
    let arr = arrangement(&app, def);
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0].length, EntryLength::Bars(4));
}

#[test]
fn fill_to_end_extends_a_bars_entry_in_place() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, _) = pattern_ids(&app);
    set_section_length(&mut app, def, 8);
    clear_arrangement(&mut app, def);
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });
    send(&mut app, ArrangementMessage::SetEntryLength {
        definition_id: def,
        index: 0,
        length: EntryLength::Bars(3),
    });

    send(&mut app, ArrangementMessage::FillToEnd { definition_id: def });
    let arr = arrangement(&app, def);
    // The Bars entry grows in place rather than spawning a new one.
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0].length, EntryLength::Bars(8));
}

#[test]
fn trim_to_fit_shrinks_last_entry() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, _) = pattern_ids(&app);
    set_section_length(&mut app, def, 4);
    clear_arrangement(&mut app, def);
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });
    send(&mut app, ArrangementMessage::SetEntryLength {
        definition_id: def,
        index: 0,
        length: EntryLength::Bars(10),
    });

    send(&mut app, ArrangementMessage::TrimToFit { definition_id: def });
    let arr = arrangement(&app, def);
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0].length, EntryLength::Bars(4));
}

#[test]
fn trim_to_fit_drops_then_shrinks_across_entries() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    set_section_length(&mut app, def, 4);
    clear_arrangement(&mut app, def);
    // [Bars(6), Bars(3)] = 9 bars over a 4-bar section. Trimming drops the
    // tail entry (3 bars) entirely, then clips the first to land on bar 4.
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });
    send(&mut app, ArrangementMessage::SetEntryLength {
        definition_id: def,
        index: 0,
        length: EntryLength::Bars(6),
    });
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_b });
    send(&mut app, ArrangementMessage::SetEntryLength {
        definition_id: def,
        index: 1,
        length: EntryLength::Bars(3),
    });

    send(&mut app, ArrangementMessage::TrimToFit { definition_id: def });
    let arr = arrangement(&app, def);
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0].pattern_id, p_main);
    assert_eq!(arr[0].length, EntryLength::Bars(4));
}

#[test]
fn deleting_a_bank_pattern_keeps_arrangements_valid() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    clear_arrangement(&mut app, def);
    // Entry 0 references p_b only as a fill; entry 1 plays p_b outright.
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });
    send(&mut app, ArrangementMessage::SetEntryFill { definition_id: def, index: 0, fill: Some(p_b) });
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_b });

    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::DeletePattern { pattern_id: p_b },
    )));

    let arr = arrangement(&app, def);
    // The p_b-playing entry is dropped; the p_b fill is cleared. No entry
    // references the deleted pattern.
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0].pattern_id, p_main);
    assert_eq!(arr[0].fill, None);
    assert!(arr.iter().all(|e| e.pattern_id != p_b && e.fill != Some(p_b)));
}

#[test]
fn undo_redo_round_trips_a_multi_entry_arrangement() {
    let mut app = build_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    // Anchor a project path so the undo machinery records snapshots.
    app.test_set_project_path(std::path::PathBuf::from("/tmp/resonance-undo-test"));

    clear_arrangement(&mut app, def);
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_main });
    let before = arrangement(&app, def);
    assert_eq!(before, vec![PatternEntry::once(p_main)]);

    // The edit under test: append a second entry.
    send(&mut app, ArrangementMessage::AddEntry { definition_id: def, pattern_id: p_b });
    let after = arrangement(&app, def);
    assert_eq!(after.len(), 2);

    // Undo restores the single-entry arrangement…
    let _ = app.update(Message::Undo);
    assert_eq!(arrangement(&app, def), before);

    // …and redo brings the second entry back.
    let _ = app.update(Message::Redo);
    assert_eq!(arrangement(&app, def), after);
}
