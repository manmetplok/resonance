//! End-to-end coverage for the "Ext Instrument" entry in the Add Track menu
//! (doc #251 gap 1 affordance 2, view half — ba todo #1067).
//!
//! Drives the menu message through the public `update()` reducer, lands the
//! new track via the engine echo (`InstrumentTrackAdded`), selects it, and
//! asserts that the Mixer inspector then renders the external-instrument
//! onboarding (Unassigned / Unconfigured) card for the freshly-created track.
//!
//! The view half is a single button in `view/menus.rs` that dispatches
//! `TrackMessage::AddExternalInstrumentTrack` (implemented in #1066). Because
//! the new track only lands in the registry once the engine echoes it back,
//! the test pumps that echo before rendering — exactly as the app does at
//! runtime.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, TrackMessage, UiMessage};
use resonance_app::state::ViewMode;
use resonance_app::{theme, Resonance};
use resonance_audio::types::{AudioCommand, AudioEvent, TrackId};

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

fn drain(rx: &resonance_audio::test_support::Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

/// The app-allocated id the `AddExternalInstrumentTrack` reducer hands to the
/// engine. Read back from the emitted command so the test stays correct even
/// if the base sub-track counter changes.
fn allocated_id_from_add(cmds: &[AudioCommand]) -> TrackId {
    for cmd in cmds {
        if let AudioCommand::AddInstrumentTrack {
            id_hint: Some(id), ..
        } = cmd
        {
            return *id;
        }
    }
    panic!("expected an AddInstrumentTrack with an app-allocated id_hint; got {cmds:?}");
}

/// Dispatching the Add Track menu's "Ext Instrument" message creates the
/// external-instrument track and, once the engine echoes it, the Mixer
/// inspector renders the onboarding (Unconfigured) card for that track.
#[test]
fn ext_instrument_menu_entry_renders_onboarding_card() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    // Belt-and-braces in case another test in this binary set the tab first
    // (the OnceLock `set` above becomes a no-op then).
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));

    let rx = app.test_capture_engine();

    // This is exactly the message the Add Track menu button dispatches.
    let _ = app.update(Message::Track(TrackMessage::AddExternalInstrumentTrack));

    let id = allocated_id_from_add(&drain(&rx));

    // The track only appears in the registry once the engine echoes it back —
    // pump that echo, then select it so the inspector renders it.
    app.test_apply_engine_event(AudioEvent::InstrumentTrackAdded { track_id: id });
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(Some(id))));

    // Precondition: the new track is a fresh external-instrument track.
    assert!(
        app.test_external_instrument(id).is_some(),
        "the menu entry must create an external-instrument track"
    );

    let mut ui = simulator(&app);

    // The inspector shows the External Instrument onboarding card: the
    // Unconfigured status badge plus the dashed setup-guidance block.
    ui.find("Unconfigured")
        .expect("fresh external-instrument track shows the Unconfigured status badge");
    ui.find("External instrument track. Pair a hardware synth's MIDI output with its \
         audio return so it plays and records in-line like a built-in instrument. \
         To set it up:")
        .expect("Mixer inspector renders the external-instrument onboarding card");
}

/// The Add Track menu offers an "Ext Instrument" entry alongside the existing
/// Audio / Instrument / Vocal entries.
#[test]
fn add_track_menu_lists_ext_instrument_entry() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));

    // Open the Add Track popover menu.
    let _ = app.update(Message::Ui(UiMessage::OpenAddTrackMenu));

    let mut ui = simulator(&app);
    ui.find("Ext Instrument")
        .expect("Add Track menu lists the Ext Instrument entry");
    // Sanity: the sibling entries are still there.
    ui.find("Instrument").expect("built-in Instrument entry");
    ui.find("Audio").expect("Audio entry");
    ui.find("Vocal").expect("Vocal entry");
}
