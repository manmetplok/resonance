//! "Save as preset…" name prompt (review VIEW-07).
//!
//! `OpenSavePresetPrompt` closes the track context menu and opens the
//! name prompt, but the prompt was only drawn inside the track-menu
//! overlay, which was only stacked while `track_menu` was open — so the
//! prompt never rendered, then took the place of the next context menu.

use crate::common;

use iced::Size;
use resonance_app::message::{Message, TrackMessage, UiMessage};
use resonance_app::state::ViewMode;
use resonance_app::{demo, theme, Resonance};

use iced_test::simulator::Simulator;

const WINDOW: (f32, f32) = (1440.0, 900.0);
/// A name no real user preset has, so the prompt renders "Save" (not
/// "Overwrite") whatever is in the machine's preset directory.
const PRESET_NAME: &str = "Review VIEW-07 golden preset";

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

fn app_with_prompt() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    demo::seed_demo_content(&mut app);
    app.test_dispatch(Message::Ui(UiMessage::OpenTrackMenu {
        id: 1,
        x: 56.0,
        y: 140.0,
    }));
    app.test_dispatch(Message::Track(TrackMessage::OpenSavePresetPrompt(1)));
    app.test_dispatch(Message::Track(TrackMessage::SetSavePresetName(
        PRESET_NAME.into(),
    )));
    app
}

#[test]
fn save_preset_prompt_renders_after_menu_closes() {
    let app = app_with_prompt();
    assert!(
        app.test_track_menu().is_none(),
        "opening the prompt closes the context menu"
    );
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    assert!(
        ui.find("Save track as preset").is_ok(),
        "the name prompt must render once the menu that opened it closed"
    );
}

#[test]
fn closed_prompt_does_not_replace_next_menu() {
    let mut app = app_with_prompt();
    app.test_dispatch(Message::Track(TrackMessage::CloseSavePresetPrompt));
    app.test_dispatch(Message::Ui(UiMessage::OpenTrackMenu {
        id: 1,
        x: 56.0,
        y: 140.0,
    }));
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    assert!(ui.find("Freeze track").is_ok(), "the menu must render");
    assert!(ui.find("Save track as preset").is_err());
}

#[test]
fn save_preset_prompt_golden() {
    let app = app_with_prompt();
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, "tests/snapshots/track_preset_save_prompt.png");
}
