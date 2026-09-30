//! The command palette (command-palette.md §7, §10 P2): the reducer (open,
//! query, move, run, close-and-remember, unavailable rows), the pinned
//! ranking, recents, and golden snapshots of its four states.

use crate::common;

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::commands::{CommandId, KeyChord, Mods, NamedKey};
use resonance_app::message::{Message, TransportMessage, UiMessage};
use resonance_app::palette::{PaletteItem, PaletteMode, PaletteMsg};
use resonance_app::state::{Overlay, ViewMode};
use resonance_app::update::shortcuts::TypingProbe;
use resonance_app::{demo, theme, Resonance};

const WINDOW: (f32, f32) = (1440.0, 900.0);

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_typing_probe(TypingProbe::Assume { editing: false });
    app
}

fn open(app: &mut Resonance) {
    let _ = app.update(Message::Ui(UiMessage::OpenPalette(PaletteMode::Commands)));
}

fn palette(app: &mut Resonance, m: PaletteMsg) {
    let _ = app.update(Message::Ui(UiMessage::Palette(m)));
}

fn query(app: &mut Resonance, q: &str) {
    palette(app, PaletteMsg::Query(q.to_string()));
}

fn first(app: &Resonance) -> CommandId {
    match &app.test_palette().expect("palette open").rows().next().expect("a row").item {
        PaletteItem::Command(c) => *c,
        other => panic!("not a command row: {other:?}"),
    }
}

fn selected(app: &Resonance) -> CommandId {
    match app.test_palette().unwrap().selected_row().unwrap().item.clone() {
        PaletteItem::Command(c) => c,
        other => panic!("not a command row: {other:?}"),
    }
}

fn recent(app: &Resonance) -> Vec<String> {
    app.test_settings().palette.recent.clone()
}

// ---------------------------------------------------------------------------
// Reducer
// ---------------------------------------------------------------------------

#[test]
fn cmd_k_opens_the_palette_and_toggles_it_closed() {
    let mut app = app();
    let press = |app: &mut Resonance, chord| {
        let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
            chord,
            repeat: false,
            captured: false,
        }));
    };
    press(&mut app, KeyChord::char('k', Mods::cmd()));
    assert_eq!(app.root_overlay(), Some(Overlay::Palette));
    press(&mut app, KeyChord::char('k', Mods::cmd()));
    assert_eq!(app.root_overlay(), None);
    press(&mut app, KeyChord::char('p', Mods::cmd_shift()));
    assert_eq!(app.root_overlay(), Some(Overlay::Palette));
}

#[test]
fn typing_moving_and_enter_run_the_selected_command_and_close() {
    let mut app = app();
    open(&mut app);
    query(&mut app, "toggle");
    let n = app.test_palette().unwrap().row_count();
    assert!(n >= 3, "{n} rows");
    // ↓ ↓ ↵, as the global subscription delivers them.
    let down = KeyChord::named(NamedKey::ArrowDown, Mods::NONE);
    for _ in 0..2 {
        let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
            chord: down,
            repeat: false,
            captured: false,
        }));
    }
    let expected = selected(&app);
    assert_eq!(app.test_palette().unwrap().selected, 2);
    palette(&mut app, PaletteMsg::Submit);
    assert!(app.test_palette().is_none(), "running closes the palette");
    assert_eq!(recent(&app).first().map(String::as_str), Some(expected.key()));
}

#[test]
fn a_run_bypasses_the_typing_gate_the_palette_field_holds() {
    let mut app = app();
    // The palette's own field has the focus: a bare-key command still runs.
    app.test_set_typing_probe(TypingProbe::Assume { editing: true });
    open(&mut app);
    query(&mut app, "toggle metronome");
    assert_eq!(first(&app), CommandId::TransportToggleMetronome);
    palette(&mut app, PaletteMsg::Submit);
    assert_eq!(recent(&app).first().map(String::as_str), Some("TransportToggleMetronome"));
}

#[test]
fn moving_wraps_around() {
    let mut app = app();
    open(&mut app);
    query(&mut app, "play");
    let n = app.test_palette().unwrap().row_count();
    palette(&mut app, PaletteMsg::Move(-1));
    assert_eq!(app.test_palette().unwrap().selected, n - 1);
    palette(&mut app, PaletteMsg::Move(1));
    assert_eq!(app.test_palette().unwrap().selected, 0);
}

#[test]
fn esc_closes_and_keeps_the_query_for_next_time() {
    let mut app = app();
    open(&mut app);
    query(&mut app, "bounce");
    // The query field captures Esc; the palette still closes.
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord: KeyChord::named(NamedKey::Escape, Mods::NONE),
        repeat: false,
        captured: true,
    }));
    assert!(app.test_palette().is_none());
    assert_eq!(app.test_view_mode(), ViewMode::Arrange);
    open(&mut app);
    assert_eq!(app.test_palette().unwrap().query, "bounce");
}

#[test]
fn an_unavailable_command_does_not_run_and_flashes_its_reason() {
    let mut app = app();
    open(&mut app);
    query(&mut app, "record");
    assert_eq!(first(&app), CommandId::TransportRecord);
    palette(&mut app, PaletteMsg::Submit);
    let state = app.test_palette().expect("still open");
    assert_eq!(state.flash, Some("Arm a track to record"));
    assert!(recent(&app).is_empty());
}

#[test]
fn opening_closes_the_overlay_underneath_and_never_opens_over_startup() {
    let mut app = app();
    let _ = app.update(Message::Ui(UiMessage::OpenSettings));
    open(&mut app);
    assert_eq!(app.root_overlay(), Some(Overlay::Palette));
    let _ = app.update(Message::Ui(UiMessage::ClosePalette));
    assert_eq!(app.root_overlay(), None, "Settings was closed, not hidden");

    let (mut fresh, _task) = Resonance::new_for_test();
    open(&mut fresh);
    assert_eq!(fresh.root_overlay(), Some(Overlay::Startup));
}

/// Pinned ranking (§7.2): deterministic, because the goldens and muscle
/// memory both depend on it.
#[test]
fn ranking_is_pinned_for_a_fixed_set_of_queries() {
    let mut app = app();
    let _ = app.update(Message::Transport(TransportMessage::SetLoopRange {
        loop_in: 48_000,
        loop_out: 96_000,
        enabled: None,
    }));
    open(&mut app);
    for (q, expected) in [
        ("loop st", CommandId::PlayheadToLoopStart),
        ("sav", CommandId::SaveProject),
        ("metro", CommandId::TransportToggleMetronome),
        ("play stop", CommandId::TransportTogglePlay),
        // A keyword hit ("mixdown" is Bounce to WAV's alias).
        ("mixdown", CommandId::BounceToWav),
    ] {
        query(&mut app, q);
        assert_eq!(first(&app), expected, "query {q:?}");
    }
}

#[test]
fn the_empty_query_shows_recents_then_suggestions() {
    let mut app = app();
    app.test_run_shortcut(CommandId::TransportToggleMetronome);
    app.test_run_shortcut(CommandId::ToggleGlobalTracks);
    app.test_run_shortcut(CommandId::TransportToggleMetronome);
    open(&mut app);
    let state = app.test_palette().unwrap();
    let titles: Vec<&str> = state.sections.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Recent", "Suggested for this view"]);
    let recent: Vec<PaletteItem> = state.sections[0].rows.iter().map(|r| r.item.clone()).collect();
    assert_eq!(
        recent,
        [
            PaletteItem::Command(CommandId::TransportToggleMetronome),
            PaletteItem::Command(CommandId::ToggleGlobalTracks),
        ],
        "newest first, de-duplicated"
    );
}

// ---------------------------------------------------------------------------
// Argument modes (§7.4)
// ---------------------------------------------------------------------------

#[test]
fn the_go_to_bar_argument_parses_bars_and_beats() {
    use resonance_app::palette::parse_bar;
    assert_eq!(parse_bar("17"), Some((17, 1)));
    assert_eq!(parse_bar(" 17.3 "), Some((17, 3)));
    assert_eq!(parse_bar("0"), None, "bars are 1-based");
    assert_eq!(parse_bar("17.0"), None, "beats are 1-based");
    assert_eq!(parse_bar("x"), None);
    assert_eq!(parse_bar(""), None);
}

#[test]
fn cmd_j_opens_go_to_bar_and_enter_seeks() {
    let mut app = app();
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord: KeyChord::char('j', Mods::cmd()),
        repeat: false,
        captured: false,
    }));
    assert_eq!(app.test_palette().unwrap().query, ":");
    let row = app.test_palette().unwrap().selected_row().unwrap();
    assert!(row.unavailable.is_some(), "no bar typed yet");

    query(&mut app, ":5.2");
    let row = app.test_palette().unwrap().selected_row().unwrap();
    assert_eq!(row.item, PaletteItem::GoTo { bar: 5, beat: 2 });
    assert_eq!(row.name, "Go to bar 5, beat 2");
    palette(&mut app, PaletteMsg::Submit);
    let expected = app
        .test_tempo_map()
        .beat_sample_in_bar(4, 1, 44_100)
        .expect("in the bar table");
    assert_eq!(app.test_playhead(), expected);
}

#[test]
fn at_mode_lists_markers_and_sections_in_timeline_order() {
    use resonance_app::state::ArrangementMarker;
    let mut app = app();
    let late = app.test_tempo_map().bar_to_sample(8);
    let early = app.test_tempo_map().bar_to_sample(1);
    app.test_add_marker(ArrangementMarker::new_point(1, "Bridge".into(), [0; 3], late));
    app.test_add_marker(ArrangementMarker::new_point(2, "Intro".into(), [0; 3], early));
    open(&mut app);
    query(&mut app, "@");
    let names: Vec<String> = app.test_palette().unwrap().rows().map(|r| r.name.clone()).collect();
    assert_eq!(names, ["Intro", "Bridge"]);
    query(&mut app, "@brid");
    let rows: Vec<PaletteItem> = app.test_palette().unwrap().rows().map(|r| r.item.clone()).collect();
    assert_eq!(rows.len(), 1);
    palette(&mut app, PaletteMsg::Submit);
    assert_eq!(app.test_playhead(), late);
}

#[test]
fn hash_mode_selects_a_track_and_plus_mode_needs_one() {
    use resonance_audio::types::TrackType;
    let mut app = app();
    app.test_add_track(1, TrackType::Audio);
    app.test_add_track(2, TrackType::Audio);
    open(&mut app);
    query(&mut app, "+");
    assert!(app.test_palette().unwrap().selected_row().unwrap().unavailable.is_some());
    query(&mut app, "#");
    assert_eq!(app.test_palette().unwrap().row_count(), 2);
    palette(&mut app, PaletteMsg::Move(1));
    let PaletteItem::Track(id) = app.test_palette().unwrap().selected_row().unwrap().item.clone() else {
        panic!("a track row");
    };
    palette(&mut app, PaletteMsg::Submit);
    assert_eq!(app.test_selected_track(), Some(id));
}

// ---------------------------------------------------------------------------
// Goldens
// ---------------------------------------------------------------------------

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

fn demo_app() -> Resonance {
    let mut app = app();
    demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Arrange)));
    app
}

fn golden(app: &Resonance, name: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui.snapshot(&theme::resonance_theme()).expect("snapshot should render");
    common::assert_golden(&snap, &format!("tests/snapshots/{name}.png"));
}

/// Empty query: *Recent* (two commands run) then *Suggested for this view*.
#[test]
fn palette_empty_state_golden() {
    let mut app = demo_app();
    app.test_run_shortcut(CommandId::ToggleGlobalTracks);
    app.test_run_shortcut(CommandId::TransportToggleMetronome);
    open(&mut app);
    assert_eq!(app.test_palette().unwrap().sections.len(), 2);
    golden(&app, "command_palette_empty");
}

/// A query with highlighted matches and one unavailable (dimmed, with its
/// reason) row, the selection moved down one.
#[test]
fn palette_results_golden() {
    let mut app = demo_app();
    open(&mut app);
    query(&mut app, "loop");
    palette(&mut app, PaletteMsg::Move(1));
    let state = app.test_palette().unwrap();
    assert!(state.rows().any(|r| r.unavailable.is_some()), "an unavailable row shows");
    assert!(state.rows().all(|r| !r.ranges.is_empty() || r.name.to_lowercase().contains("loop")));
    golden(&app, "command_palette_results");
}

#[test]
fn palette_no_match_golden() {
    let mut app = demo_app();
    open(&mut app);
    query(&mut app, "zqxv");
    assert_eq!(app.test_palette().unwrap().row_count(), 0);
    golden(&app, "command_palette_no_match");
}

/// Linux keycaps read `Ctrl` `Shift` `S`, never ⌘ (§4.2).
#[test]
fn palette_linux_keycaps_golden() {
    use resonance_app::commands::Platform;
    assert_eq!(
        KeyChord::char('s', Mods::cmd_shift()).keycaps(Platform::Other),
        ["Ctrl", "Shift", "S"]
    );
    assert_eq!(KeyChord::char('s', Mods::cmd_shift()).keycaps(Platform::Mac), ["⇧", "⌘", "S"]);
    assert_eq!(KeyChord::char('s', Mods::cmd_shift()).format_for(Platform::Other), "Ctrl+Shift+S");
    let mut app = demo_app();
    open(&mut app);
    query(&mut app, "save");
    golden(&app, "command_palette_linux_keycaps");
}
