//! The Settings overlay's Plugins section, pressed rather than read
//! (ba todo #1307, finding X10).
//!
//! `control_plugins_rescan` proves the control method and the app's
//! reducer; it dispatches `PluginMessage::RescanPlugins` directly, which
//! is the message the button is *supposed* to carry. That leaves the GUI
//! half of the dual surface — a section that renders, a button wired to
//! that message, a label that says which state the scan is in, and the
//! failure lines — asserted only by reading the source. A mis-wired
//! button would pass every other test in this todo.
//!
//! So this drives the real widget tree: open Settings, click the button
//! by its label, and follow the label through the scan.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, PluginMessage, UiMessage};
use resonance_app::state::ViewMode;
use resonance_app::{theme, Resonance};
use resonance_audio::types::{AudioCommand, AudioEvent, PluginScanFailure, ScannedPlugin};

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

fn drain(rx: &resonance_audio::__test_support::Receiver<AudioCommand>) -> Vec<AudioCommand> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

/// An app with Settings open and one plugin in the catalog.
fn app_with_settings_open() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    // Without an active project the startup modal owns the screen and no
    // other overlay renders at all.
    app.test_set_active_project(true);
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![ScannedPlugin {
            clap_file_path: "/plugins/reverb.clap".to_owned(),
            clap_plugin_id: "com.resonance.reverb".to_owned(),
            name: "Resonance Reverb".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: false,
            ..Default::default()
        }],
    });
    let _ = app.update(Message::Ui(UiMessage::OpenSettings));
    app
}

#[test]
fn the_settings_button_runs_the_scan() {
    let mut app = app_with_settings_open();

    // The section is there, and says what the last scan found — the
    // number is the context for pressing the button at all.
    {
        let mut ui = simulator(&app);
        ui.find("Plugins").expect("the Plugins section is rendered");
        ui.find("1 plugin(s) available — 0 instrument(s), 1 effect(s)")
            .expect("the section summarises the catalog");
    }

    // Press it by its label, exactly as a user would.
    let messages: Vec<Message> = {
        let mut ui = simulator(&app);
        ui.click("Rescan Plugins")
            .expect("the rescan button is clickable");
        ui.into_messages().collect()
    };
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, Message::Plugin(PluginMessage::RescanPlugins))),
        "the button carries the rescan message, got {messages:?}"
    );

    // And that message reaches the engine as the additive rescan.
    let rx = app.test_capture_engine();
    for m in messages {
        app.test_dispatch(m);
    }
    let commands = drain(&rx);
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, AudioCommand::RescanPlugins)),
        "pressing the button asks the engine to look again: {commands:?}"
    );
    assert!(
        !commands
            .iter()
            .any(|c| matches!(c, AudioCommand::ScanPlugins)),
        "and never the destructive startup scan"
    );
}

#[test]
fn the_button_says_a_scan_is_running_and_stops_saying_it() {
    let mut app = app_with_settings_open();

    let messages: Vec<Message> = {
        let mut ui = simulator(&app);
        ui.click("Rescan Plugins").expect("clickable");
        ui.into_messages().collect()
    };
    for m in messages {
        app.test_dispatch(m);
    }

    // While the engine is scanning the label reports it.
    {
        let mut ui = simulator(&app);
        ui.find("Scanning...")
            .expect("the button reports the scan in progress");
        assert!(
            simulator(&app).find("Rescan Plugins").is_err(),
            "the idle label is gone while the scan runs"
        );
    }

    // `PluginsScanned` is the ONLY thing that clears the flag, and a
    // scan that finds nothing still sends one — otherwise the button
    // would read "Scanning..." for the rest of the session.
    app.test_apply_engine_event(AudioEvent::PluginsScanned { plugins: Vec::new() });
    let mut ui = simulator(&app);
    ui.find("Rescan Plugins")
        .expect("the button returns to its idle label once the scan reports");
    ui.find("0 plugin(s) available — 0 instrument(s), 0 effect(s)")
        .expect("and the summary follows the new catalog");
    drop(ui);
}

#[test]
fn a_failed_bundle_is_drawn_in_the_section() {
    // "Reported rather than swallowed" means reported TO THE USER: a
    // `.clap` that will not load is otherwise indistinguishable from one
    // that was never installed.
    let mut app = app_with_settings_open();
    app.test_apply_engine_event(AudioEvent::PluginScanFailed {
        failures: vec![PluginScanFailure {
            path: "/plugins/broken.clap".to_owned(),
            reason: "missing clap_entry symbol".to_owned(),
        }],
    });

    {
        let mut ui = simulator(&app);
        ui.find("Failed: /plugins/broken.clap — missing clap_entry symbol")
            .expect("the failing bundle and the loader's reason are both on screen");
    }

    // A clean rescan clears it, so a fixed install stops being reported.
    let _ = app.update(Message::Plugin(PluginMessage::RescanPlugins));
    app.test_apply_engine_event(AudioEvent::PluginsScanned { plugins: Vec::new() });
    assert!(
        simulator(&app)
            .find("Failed: /plugins/broken.clap — missing clap_entry symbol")
            .is_err(),
        "the previous run's failure is not still on screen after a clean scan"
    );
}
