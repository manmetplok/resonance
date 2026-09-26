//! The Settings overlay's autosave row (code review FU-M12a; ba todo
//! #471's controls, ported): an enable toggle and an interval picker,
//! pressed through the real widget tree, persisted, and honoured by the
//! autosave trigger.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, UiMessage};
use resonance_app::settings::load_from;
use resonance_app::state::ViewMode;
use resonance_app::{theme, Resonance};

use crate::common;

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

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view())
}

fn app_with_settings_open() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    let _ = app.update(Message::Ui(UiMessage::OpenSettings));
    app
}

#[test]
fn the_autosave_row_switches_autosave_and_sets_its_interval() {
    let mut app = app_with_settings_open();
    assert!(app.autosave_settings().enabled, "on by default");

    let messages: Vec<Message> = {
        let mut ui = simulator(&app);
        ui.click("Autosave").expect("the autosave toggle is rendered");
        ui.into_messages().collect()
    };
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, Message::Ui(UiMessage::ToggleAutosave))),
        "{messages:?}"
    );
    for m in messages {
        let _ = app.update(m);
    }
    assert!(!app.autosave_settings().enabled);

    let _ = app.update(Message::Ui(UiMessage::ToggleAutosave));
    assert!(app.autosave_settings().enabled);

    // One test, not two: both persist the same process-wide settings file.
    // (The picker's label is drawn by the pick_list itself, not a text
    // widget the simulator can find; the golden below shows it.)
    let _ = app.update(Message::Ui(UiMessage::SetAutosaveInterval(120)));
    assert_eq!(app.autosave_settings().interval_secs, 120);

    // Persisted — into the hermetic config root, never the real one.
    let file = resonance_app::user_dirs::config_dir()
        .expect("config dir")
        .join("resonance/settings.json");
    assert!(file.starts_with(resonance_app::user_dirs::hermetic_root().unwrap()));
    let _ = app.update(Message::Ui(UiMessage::SetAutosaveInterval(45)));
    assert_eq!(load_from(&file).autosave.interval_secs, 45);

    // A zero interval would autosave on every tick.
    let _ = app.update(Message::Ui(UiMessage::SetAutosaveInterval(0)));
    assert_eq!(app.autosave_settings().interval_secs, 1);
}

#[test]
fn settings_overlay_autosave_row() {
    let app = app_with_settings_open();
    let mut ui = simulator(&app);
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, "tests/snapshots/settings_overlay_autosave_row.png");
}
