//! Track context menu + Tracks header-cap "Freeze all" button (design
//! doc #181, ba todo #581).
//!
//! Right-clicking an arrange track header opens a floating context menu
//! with the freeze convenience entries — Freeze track (⌘F) / Unfreeze
//! track / Freeze selected tracks / Freeze all tracks (⇧⌘F) / Reveal
//! freeze cache… — and the Tracks column's header cap gains a snowflake
//! "Freeze all" pill that drives the same sequential batch queue.
//!
//! Coverage:
//! - **State machine** (update-driven): `OpenTrackMenu` opens the menu and
//!   selects the track; `CloseTrackMenu` and any freeze action close it;
//!   `RevealFreezeCache` on an unsaved project errors instead of opening.
//! - **Presence** (widget tree): the open menu renders every entry; the
//!   header cap renders the "Freeze all" pill.
//! - **Golden** (`tests/snapshots`): the open menu over a live track and
//!   over a frozen track (dim/enabled states flip between the two).

mod common;

use iced::Size;
use resonance_app::message::{FreezeMessage, Message, UiMessage};
use resonance_app::state::{FreezeStatus, ViewMode};
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

use iced_test::simulator::Simulator;

const WINDOW: (f32, f32) = (1440.0, 900.0);

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

fn frozen_cache_ref() -> FreezeCacheRef {
    FreezeCacheRef::new(
        "freeze_1.wav".to_string(),
        48_000,
        32,
        0,
        FreezeCacheStatus::Frozen,
    )
}

/// Demo app pinned to the Arrange tab.
fn build_arrange_app() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new_for_test();
    demo::seed_demo_content(&mut app);
    app
}

/// Open the context menu on track 1 ("Pattern A", an instrument track) at
/// a fixed arrange-area anchor.
fn open_menu_on_track_1(app: &mut Resonance) {
    app.test_dispatch(Message::Ui(UiMessage::OpenTrackMenu {
        id: 1,
        x: 56.0,
        y: 140.0,
    }));
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, path);
}

// ---------------------------------------------------------------------
// State machine — open / select / close transitions.
// ---------------------------------------------------------------------

#[test]
fn right_click_opens_menu_and_selects_track() {
    let mut app = build_arrange_app();
    open_menu_on_track_1(&mut app);
    let menu = app
        .test_track_menu()
        .expect("OpenTrackMenu must record the open menu state");
    assert_eq!(menu.track_id, 1);
    assert_eq!(
        app.test_selected_tracks(),
        &[1],
        "opening the context menu must select the right-clicked track"
    );
}

#[test]
fn backdrop_close_message_clears_menu() {
    let mut app = build_arrange_app();
    open_menu_on_track_1(&mut app);
    app.test_dispatch(Message::Ui(UiMessage::CloseTrackMenu));
    assert!(
        app.test_track_menu().is_none(),
        "CloseTrackMenu must clear the open menu state"
    );
}

#[test]
fn freeze_action_closes_menu() {
    let mut app = build_arrange_app();
    open_menu_on_track_1(&mut app);
    // Any freeze action closes the menu — CancelFreeze is the cheapest one
    // (a no-op with nothing in flight, but still routed through the freeze
    // handler that owns the close).
    app.test_dispatch(Message::Freeze(FreezeMessage::CancelFreeze));
    assert!(
        app.test_track_menu().is_none(),
        "acting on a menu entry must close the menu"
    );
}

#[test]
fn reveal_cache_on_unsaved_project_errors() {
    let mut app = build_arrange_app();
    app.test_dispatch(Message::Freeze(FreezeMessage::RevealFreezeCache));
    assert!(
        app.test_error_message()
            .is_some_and(|m| m.contains("Save the project")),
        "RevealFreezeCache without a saved project must surface an error, got {:?}",
        app.test_error_message()
    );
}

// ---------------------------------------------------------------------
// Presence — menu entries + header-cap button in the widget tree.
// ---------------------------------------------------------------------

#[test]
fn open_menu_renders_all_entries() {
    let mut app = build_arrange_app();
    open_menu_on_track_1(&mut app);
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    for label in [
        "Freeze track",
        "Unfreeze track",
        "Freeze selected tracks",
        "Freeze all tracks",
        "Reveal freeze cache\u{2026}",
    ] {
        ui.find(label)
            .unwrap_or_else(|_| panic!("open track menu must render {label:?}"));
    }
}

#[test]
fn header_cap_renders_freeze_all_button() {
    let app = build_arrange_app();
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    ui.find("Freeze all")
        .expect("Tracks header cap must render the Freeze all button");
}

// ---------------------------------------------------------------------
// Golden — open menu over a live and a frozen track.
// ---------------------------------------------------------------------

#[test]
fn track_menu_live_track_golden() {
    let mut app = build_arrange_app();
    open_menu_on_track_1(&mut app);
    snapshot_to(&app, "tests/snapshots/track_context_menu_live.png");
}

#[test]
fn track_menu_frozen_track_golden() {
    let mut app = build_arrange_app();
    app.test_set_freeze_status(
        1,
        FreezeStatus::Frozen {
            cache_ref: frozen_cache_ref(),
        },
    );
    open_menu_on_track_1(&mut app);
    snapshot_to(&app, "tests/snapshots/track_context_menu_frozen.png");
}
