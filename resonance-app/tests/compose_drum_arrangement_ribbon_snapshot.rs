//! Golden-image snapshots for the Compose drum lane's **tiling ribbon**
//! (design doc #170, surface #2 — todo #489).
//!
//! The pure span builder (`build_ribbon_spans`) is unit-tested in
//! `compose_drum_arrangement_ribbon.rs`; those cases prove the span *data*
//! is correct in isolation. These integration tests drive the real `update`
//! reducer to build each arrangement state, assert the ribbon's rendered
//! spans through the app-state path (so the arrangement-edit → ribbon wiring
//! is covered end-to-end and the four span types are provably distinct
//! regardless of the local rasterizer), and then snapshot the whole Compose
//! tab via `app.view()` so the lane + legend are visually verified:
//!
//! 1. **chained + fill** — `Main ×4` (fill on the last bar) chained with
//!    `B section ×4` over an 8-bar section: a normal tinted span, the hatched
//!    warm `FILL` cap, and a second chained normal span (exact 8/8 coverage).
//! 2. **gap (under-fill)** — `Main ×2` in an 8-bar section leaves a 6-bar
//!    hatched warm dashed `GAP` span.
//! 3. **overflow (over-fill)** — `Main ×4` + `B section ×8` overflows the
//!    8-bar section by 4 bars, drawing the pink `OVERFLOW (clipped)` cap past
//!    the section end.
//! 4. **selected entry** — selecting an entry via the ribbon's selection
//!    message highlights its span (the affordance that drives the inspector).
//!
//! Each state renders the static legend (Pattern / Fill / Gap / Overflow)
//! under the ribbon. Window size mirrors `compose_drum_pattern_picker.rs`'s
//! tall viewport so the whole drum lane sits on screen. On first run
//! `matches_image()` writes the goldens under `tests/snapshots/`; subsequent
//! runs diff against them. NOTE: the ribbon paints translucent `Canvas`
//! fills, which the local non-conformant software Vulkan rasterizer renders
//! imprecisely — the goldens are meant to be blessed in CI (see the
//! project's "snapshot goldens diverge in this env" note), so the semantic
//! `assert`s on the ribbon spans below are the authoritative, env-independent
//! proof that the four span types are present and distinct.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::compose::messages::{ArrangementMessage, DrumGroupsMessage};
use resonance_app::compose::{ComposeMessage, EntryLength, SectionDefinitionState, SelectedLane};
use resonance_app::message::Message;
use resonance_app::state::{InstrumentType, ViewMode};
use resonance_app::view::{build_ribbon_spans, RibbonSpan, RibbonSpanKind};
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_audio::types::TrackType;

/// Tall window so the synth lanes + drum lane (with the ribbon + legend
/// under the picker) all fit inside the viewport. Mirrors
/// `compose_drum_pattern_picker.rs`.
const TALL_WINDOW: (f32, f32) = (1440.0, 1600.0);

fn sim_settings() -> iced::Settings {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    fonts.push(theme::ICON_FONT_BYTES.into());
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    }
}

/// Demo app pinned to Compose with the drum lane focused, so the tiling
/// ribbon renders in its lane row. Mirrors `build_compose_app` in the
/// pattern-picker snapshots.
fn build_compose_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Compose);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);

    if let Some(drum_track_id) = app
        .track_registry()
        .tracks
        .iter()
        .find(|t| {
            matches!(t.track_type, TrackType::Instrument)
                && t.sub_track.is_none()
                && t.instrument_type == InstrumentType::Drum
        })
        .map(|t| t.id)
    {
        let _ = app.update(Message::Compose(ComposeMessage::SelectLane(
            SelectedLane::Drums(drum_track_id),
        )));
    }

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

fn send(app: &mut Resonance, msg: ArrangementMessage) {
    let _ = app.update(Message::Compose(ComposeMessage::Arrangement(msg)));
}

/// Empty the focused section's arrangement so each state starts clean,
/// regardless of the demo seed.
fn clear_arrangement(app: &mut Resonance, def: u64) {
    let _ = app.update(Message::Compose(ComposeMessage::DrumGroups(
        DrumGroupsMessage::AssignPattern {
            definition_id: def,
            pattern_id: None,
        },
    )));
}

fn set_section_length(app: &mut Resonance, def: u64, bars: u32) {
    let _ = app.update(Message::Compose(ComposeMessage::ResizeSection {
        definition_id: def,
        length_bars: bars,
    }));
}

/// Append a `RepeatN(repeat)` entry playing `pattern_id`, optionally with a
/// fill on its last bar.
fn add_repeat_entry(app: &mut Resonance, def: u64, pattern_id: u64, repeat: u32, fill: Option<u64>) {
    send(app, ArrangementMessage::AddEntry { definition_id: def, pattern_id });
    let index = app
        .compose_state()
        .find_definition(def)
        .expect("definition exists")
        .arrangement
        .len()
        - 1;
    send(
        app,
        ArrangementMessage::SetEntryLength {
            definition_id: def,
            index,
            length: EntryLength::RepeatN(repeat),
        },
    );
    if let Some(fill_id) = fill {
        send(
            app,
            ArrangementMessage::SetEntryFill {
                definition_id: def,
                index,
                fill: Some(fill_id),
            },
        );
    }
}

/// The ribbon spans the view would render for the focused section — built
/// through the same public path (`resolve_arrangement_for` + the bank
/// lookups) as `ribbon::ribbon`. Lets each test assert the four span types
/// independently of the flaky Canvas rasterizer.
fn ribbon_spans(app: &Resonance, def: u64) -> Vec<RibbonSpan> {
    let compose = app.compose_state();
    let definition: &SectionDefinitionState = compose.find_definition(def).expect("definition");
    let resolved = compose.resolve_arrangement_for(definition);
    build_ribbon_spans(
        &definition.arrangement,
        definition.length_bars,
        &resolved,
        |id| compose.find_pattern(id).map(|p| p.bar_span()).unwrap_or(1),
        |id| {
            compose
                .find_pattern(id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "?".to_string())
        },
        |id| compose.find_pattern(id).map(|p| p.color).unwrap_or([0, 0, 0]),
    )
}

fn kinds(spans: &[RibbonSpan]) -> Vec<RibbonSpanKind> {
    spans.iter().map(|s| s.kind).collect()
}

fn snapshot(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(
        sim_settings(),
        Size::new(TALL_WINDOW.0, TALL_WINDOW.1),
        app.view(),
    );
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image(path).expect("matches_image i/o"),
        "snapshot diverged from golden: {path}"
    );
}

#[test]
fn ribbon_chained_with_fill() {
    // Main ×4 (fill on the last bar, 1-bar pattern → 4 bars) then B section
    // ×4 → exact 8/8 coverage: normal span, hatched warm FILL cap, second
    // chained normal span.
    let mut app = build_compose_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    clear_arrangement(&mut app, def);
    set_section_length(&mut app, def, 8);

    add_repeat_entry(&mut app, def, p_main, 4, Some(p_b));
    add_repeat_entry(&mut app, def, p_b, 4, None);

    let spans = ribbon_spans(&app, def);
    assert_eq!(
        kinds(&spans),
        vec![
            RibbonSpanKind::Normal,
            RibbonSpanKind::Fill,
            RibbonSpanKind::Normal
        ],
        "chained+fill = tinted span + FILL cap + second tinted span"
    );
    assert_eq!(spans[0].label, "Main \u{00d7}4");
    assert_eq!(spans[1].label, "FILL");
    assert_eq!(spans[2].label, "B section \u{00d7}4");

    snapshot(&app, "tests/snapshots/compose_ribbon_chained_fill.png");
}

#[test]
fn ribbon_under_fill_gap() {
    // Main ×2 (1-bar pattern → 2 bars) in an 8-bar section → a 6-bar hatched
    // warm dashed GAP tail. (×2 keeps a clear normal span *and* a wide gap.)
    let mut app = build_compose_app();
    let def = focused_definition(&app);
    let (p_main, _p_b) = pattern_ids(&app);
    clear_arrangement(&mut app, def);
    set_section_length(&mut app, def, 8);

    add_repeat_entry(&mut app, def, p_main, 2, None);

    let spans = ribbon_spans(&app, def);
    assert_eq!(
        kinds(&spans),
        vec![RibbonSpanKind::Normal, RibbonSpanKind::Gap],
        "under-fill = a tinted span followed by a GAP span"
    );
    let gap = spans.last().unwrap();
    assert_eq!(gap.label, "GAP");
    assert_eq!((gap.bar_start, gap.bar_end), (2, 8));
    assert_eq!(gap.entry_index, None, "a gap belongs to no entry");

    snapshot(&app, "tests/snapshots/compose_ribbon_gap.png");
}

#[test]
fn ribbon_over_fill_overflow() {
    // Main ×4 (4 bars) + B section ×8 (8 bars) = 12 bars in an 8-bar section
    // → 4 bars overflow. The pink OVERFLOW (clipped) cap hangs past the end.
    let mut app = build_compose_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    clear_arrangement(&mut app, def);
    set_section_length(&mut app, def, 8);

    add_repeat_entry(&mut app, def, p_main, 4, None);
    add_repeat_entry(&mut app, def, p_b, 8, None);

    let spans = ribbon_spans(&app, def);
    assert_eq!(
        spans.last().map(|s| s.kind),
        Some(RibbonSpanKind::Overflow),
        "over-fill ends in an OVERFLOW cap"
    );
    let cap = spans.last().unwrap();
    assert_eq!(cap.label, "OVERFLOW (clipped)");
    assert_eq!(cap.bar_start, 8, "cap begins at the section end");
    assert!(cap.bar_end > 8, "cap extends past the section end");

    snapshot(&app, "tests/snapshots/compose_ribbon_overflow.png");
}

#[test]
fn ribbon_selected_entry_highlights_span() {
    // Selecting an entry via the ribbon's selection message marks it as the
    // focused arrangement entry (drives the highlighted-border draw + the
    // right-rail inspector).
    let mut app = build_compose_app();
    let def = focused_definition(&app);
    let (p_main, p_b) = pattern_ids(&app);
    clear_arrangement(&mut app, def);
    set_section_length(&mut app, def, 8);

    add_repeat_entry(&mut app, def, p_main, 4, Some(p_b));
    add_repeat_entry(&mut app, def, p_b, 4, None);
    let _ = app.update(Message::Compose(ComposeMessage::SelectArrangementEntry(
        Some(0),
    )));
    assert_eq!(
        app.compose_state().drumroll.selected_entry_index,
        Some(0),
        "selection message sets the focused entry"
    );

    snapshot(&app, "tests/snapshots/compose_ribbon_selected_entry.png");
}
