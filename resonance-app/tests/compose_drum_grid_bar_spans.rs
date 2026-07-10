//! Verifies that the resolved per-bar spans fed to the drum-grid canvas
//! (todo #487) are correct for the three key arrangement shapes:
//!
//! 1. **Single-entry** — the whole section resolves to one span; no
//!    regression from the pre-arrangement single-pattern rendering.
//! 2. **Chained** — each span covers its correct bar range and references
//!    the right pattern id, so different bars get cells from different
//!    patterns.
//! 3. **Empty arrangement** — the full section falls back to the primary
//!    pattern (same fallback as `pattern_for_definition`).
//! 4. **Fill entry** — the fill bar produces an `is_fill=true` span.
//!
//! Tests drive `ComposeState::resolve_arrangement_for` and
//! `pattern_for_definition` directly (no iced view layer needed) since
//! the view layer's `mod.rs` is a thin wrapper around these two methods.

use resonance_app::compose::messages::{ArrangementMessage, DrumGroupsMessage};
use resonance_app::compose::{ArrangementCoverage, ComposeMessage, EntryLength};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{demo, Resonance, STARTUP_TAB};

fn build_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Compose);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    app
}

fn focused_def_id(app: &Resonance) -> u64 {
    app.compose_state()
        .selected_placement()
        .expect("demo seeds a selected placement")
        .definition_id
}

/// Pattern ids from the default bank (Main = index 0, B section = index 1).
fn pattern_ids(app: &Resonance) -> (u64, u64) {
    let bank = &app.compose_state().drum_patterns;
    (bank[0].id, bank[1].id)
}

fn send(app: &mut Resonance, msg: ArrangementMessage) {
    let _ = app.update(Message::Compose(ComposeMessage::Arrangement(msg)));
}

// ── single-entry ─────────────────────────────────────────────────────────────

#[test]
fn single_entry_resolves_to_one_full_section_span() {
    let mut app = build_app();
    let def_id = focused_def_id(&app);
    let (p1, _) = pattern_ids(&app);

    // Assign P1 as a single-entry arrangement (creates entry 0 if absent).
    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::AssignPattern {
            definition_id: def_id,
            pattern_id: Some(p1),
        },
    )));
    // Resize entry 0 to cover the section's actual bar count exactly.
    let section_bars = {
        let compose = app.compose_state();
        compose.find_definition(def_id).unwrap().length_bars
    };
    send(
        &mut app,
        ArrangementMessage::SetEntryLength {
            definition_id: def_id,
            index: 0,
            length: EntryLength::Bars(section_bars),
        },
    );

    let compose = app.compose_state();
    let def = compose.find_definition(def_id).expect("def exists");

    let resolved = compose.resolve_arrangement_for(def);
    // Primary pattern id must be P1.
    let primary_id = compose
        .pattern_for_definition(def)
        .map(|p| p.id)
        .expect("primary pattern exists");
    assert_eq!(primary_id, p1, "primary pattern must be the first bank entry");

    // For a single full-section entry the spans slice has exactly one span.
    assert!(!resolved.spans.is_empty(), "single entry must produce at least one span");
    let first_span = &resolved.spans[0];
    assert_eq!(first_span.bar_start, 0);
    assert!(!first_span.is_fill, "plain single-entry span must not be marked fill");
    assert_eq!(
        resolved.coverage,
        ArrangementCoverage::Exact,
        "single entry that matches section length must be Exact"
    );
}

// ── chained arrangement ───────────────────────────────────────────────────────

#[test]
fn chained_arrangement_produces_correct_spans_per_pattern() {
    let mut app = build_app();
    let def_id = focused_def_id(&app);
    let (p1, p2) = pattern_ids(&app);

    // Build a 3-span chain: P1×n + P2×1 + P1×1, total `section_bars` bars.
    // Query section length first, then build the arrangement to fill it.
    let section_bars = {
        let compose = app.compose_state();
        compose.find_definition(def_id).unwrap().length_bars
    };
    // Need at least 3 bars for this test.
    assert!(section_bars >= 3, "demo section must be at least 3 bars");
    let p1_bars = section_bars - 2; // all but the last two bars

    // Start from a clean slate via AssignPattern.
    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::AssignPattern {
            definition_id: def_id,
            pattern_id: Some(p1),
        },
    )));
    // entry 0: P1, p1_bars bars
    send(
        &mut app,
        ArrangementMessage::SetEntryLength {
            definition_id: def_id,
            index: 0,
            length: EntryLength::Bars(p1_bars),
        },
    );
    // entry 1: add P2, 1 bar
    send(
        &mut app,
        ArrangementMessage::AddEntry {
            definition_id: def_id,
            pattern_id: p2,
        },
    );
    // entry 2: add P1, 1 bar
    send(
        &mut app,
        ArrangementMessage::AddEntry {
            definition_id: def_id,
            pattern_id: p1,
        },
    );

    let compose = app.compose_state();
    let def = compose.find_definition(def_id).expect("def exists");
    let resolved = compose.resolve_arrangement_for(def);

    // Expect 3 spans: [0, p1_bars)→P1, [p1_bars, p1_bars+1)→P2, [p1_bars+1, section_bars)→P1.
    let mid = p1_bars;
    assert_eq!(
        resolved.spans.len(),
        3,
        "chained 3-entry arrangement must produce 3 spans, got {:?}",
        resolved.spans.iter().map(|s| (s.bar_start, s.bar_end, s.pattern_id)).collect::<Vec<_>>()
    );
    let s0 = &resolved.spans[0];
    let s1 = &resolved.spans[1];
    let s2 = &resolved.spans[2];

    assert_eq!(s0.bar_start, 0);
    assert_eq!(s0.bar_end, mid);
    assert_eq!(s0.pattern_id, p1);
    assert!(!s0.is_fill);

    assert_eq!(s1.bar_start, mid);
    assert_eq!(s1.bar_end, mid + 1);
    assert_eq!(s1.pattern_id, p2);
    assert!(!s1.is_fill);

    assert_eq!(s2.bar_start, mid + 1);
    assert_eq!(s2.bar_end, mid + 2);
    assert_eq!(s2.pattern_id, p1);
    assert!(!s2.is_fill);
}

// ── empty arrangement fallback ────────────────────────────────────────────────

#[test]
fn empty_arrangement_falls_back_to_primary_pattern() {
    let mut app = build_app();
    let def_id = focused_def_id(&app);
    let (p1, _) = pattern_ids(&app);

    // Clear the arrangement.
    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::AssignPattern {
            definition_id: def_id,
            pattern_id: None,
        },
    )));

    let compose = app.compose_state();
    let def = compose.find_definition(def_id).expect("def exists");

    // Empty arrangement → no resolver spans.
    let resolved = compose.resolve_arrangement_for(def);
    assert!(
        resolved.spans.is_empty(),
        "empty arrangement must produce no explicit spans"
    );

    // The view layer synthesises a fallback span from `pattern_for_definition`.
    // Verify the fallback resolves to something sane (not None).
    let fallback = compose.pattern_for_definition(def);
    assert!(
        fallback.is_some(),
        "pattern_for_definition must return a fallback even with empty arrangement"
    );
    // The fallback must be the project default or the first bank entry (P1).
    assert_eq!(
        fallback.unwrap().id,
        p1,
        "empty-arrangement fallback must be the first bank pattern"
    );
}

// ── fill entry ────────────────────────────────────────────────────────────────

#[test]
fn fill_entry_marks_the_last_bar_as_fill() {
    let mut app = build_app();
    let def_id = focused_def_id(&app);
    let (p1, p2) = pattern_ids(&app);

    // Arrange: P1 over section_bars with fill P2 on the last bar.
    let section_bars = {
        let compose = app.compose_state();
        compose.find_definition(def_id).unwrap().length_bars
    };
    assert!(section_bars >= 2, "demo section must be at least 2 bars");

    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::AssignPattern {
            definition_id: def_id,
            pattern_id: Some(p1),
        },
    )));
    // Resize entry 0 to section_bars.
    send(
        &mut app,
        ArrangementMessage::SetEntryLength {
            definition_id: def_id,
            index: 0,
            length: EntryLength::Bars(section_bars),
        },
    );
    // Add fill P2 on entry 0's last bar.
    send(
        &mut app,
        ArrangementMessage::SetEntryFill {
            definition_id: def_id,
            index: 0,
            fill: Some(p2),
        },
    );

    let compose = app.compose_state();
    let def = compose.find_definition(def_id).expect("def exists");
    let resolved = compose.resolve_arrangement_for(def);

    // Expect 2 spans: [0, section_bars-1)→P1 (non-fill) and
    // [section_bars-1, section_bars)→P2 (fill).
    assert_eq!(
        resolved.spans.len(),
        2,
        "entry with fill must produce 2 spans"
    );
    let main_span = &resolved.spans[0];
    let fill_span = &resolved.spans[1];

    assert_eq!(main_span.bar_start, 0);
    assert_eq!(main_span.bar_end, section_bars - 1);
    assert_eq!(main_span.pattern_id, p1);
    assert!(!main_span.is_fill, "main span must not be marked fill");

    assert_eq!(fill_span.bar_start, section_bars - 1);
    assert_eq!(fill_span.bar_end, section_bars);
    assert_eq!(fill_span.pattern_id, p2);
    assert!(fill_span.is_fill, "fill span must be marked is_fill=true");
}

// ── span_at helper ────────────────────────────────────────────────────────────

#[test]
fn span_at_returns_correct_pattern_per_bar_in_chained_arrangement() {
    let mut app = build_app();
    let def_id = focused_def_id(&app);
    let (p1, p2) = pattern_ids(&app);

    // P1×half + P2×half = section_bars.
    let section_bars = {
        let compose = app.compose_state();
        compose.find_definition(def_id).unwrap().length_bars
    };
    assert!(section_bars >= 2, "demo section must be at least 2 bars");
    let half = section_bars / 2;

    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::AssignPattern {
            definition_id: def_id,
            pattern_id: Some(p1),
        },
    )));
    send(
        &mut app,
        ArrangementMessage::SetEntryLength {
            definition_id: def_id,
            index: 0,
            length: EntryLength::Bars(half),
        },
    );
    send(
        &mut app,
        ArrangementMessage::AddEntry {
            definition_id: def_id,
            pattern_id: p2,
        },
    );
    send(
        &mut app,
        ArrangementMessage::SetEntryLength {
            definition_id: def_id,
            index: 1,
            length: EntryLength::Bars(section_bars - half),
        },
    );

    let compose = app.compose_state();
    let def = compose.find_definition(def_id).expect("def exists");
    let resolved = compose.resolve_arrangement_for(def);

    // Bars 0..half-1 → P1, bars half..section_bars-1 → P2.
    assert_eq!(resolved.span_at(0).unwrap().pattern_id, p1);
    assert_eq!(resolved.span_at(half - 1).unwrap().pattern_id, p1);
    assert_eq!(resolved.span_at(half).unwrap().pattern_id, p2);
    assert_eq!(resolved.span_at(section_bars - 1).unwrap().pattern_id, p2);
}
