//! Coverage for the External-Instrument **patch-by-name** picker (architecture
//! doc #201 §5, epic #40, ba todo #725).
//!
//! When a device preset with a patch list is selected, the numeric
//! Bank/Program pickers give way to a single named-patch picker: picking a
//! named patch resolves to the entry's `bank_msb`/`bank_lsb` + `program` and
//! drives the *existing* Bank Select + Program Change path (the same
//! `AudioCommand::SetExternalInstrumentPatch` the numeric pickers use). The
//! selected named patch persists implicitly via the resolved bank/program, so
//! undo/redo round-trips it. With no device selected the numeric pickers
//! remain.
//!
//! Command-dispatch tests swap in a capturing engine (like
//! `tests/external_instrument_device_preset.rs`); the undo round-trip drives
//! the real reducer. Structural render tests assert the widget tree via
//! `iced_test::Simulator::find`.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{ExternalInstrumentMessage as Eim, Message, UiMessage};
use resonance_app::state::{TrackState, ViewMode};
use resonance_app::undo::{classify, UndoAction};
use resonance_app::{theme, Resonance};
use resonance_audio::__test_support::Receiver;
use resonance_audio::types::{AudioCommand, TrackId};

const TRACK: TrackId = 1;
/// The bundled Moog Muse preset ships in the registry (`resonance-common`
/// bundled definitions), so `Resonance::new_for_test()` always has it available. Its
/// 256 factory patches are grouped by bank/category — Bank 1 · Patch 1 is
/// bank_msb 0 / bank_lsb 0 / program 0, so it resolves to bank `0`, program
/// `0`. Bank 2 · Patch 1 is bank_lsb 1 → combined bank `1`, program `0`.
const MUSE: &str = "moog-muse";

fn external_capturing_app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    let rx = app.test_capture_engine();
    app.test_dispatch(Message::ExternalInstrument(Eim::Enable(TRACK)));
    (app, rx)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

/// The single `SetExternalInstrumentPatch` command in `cmds`. Panics unless
/// exactly one was dispatched, so a test's expectation is unambiguous.
fn one_patch(cmds: &[AudioCommand]) -> (TrackId, Option<u16>, Option<u8>) {
    let mut found = None;
    for cmd in cmds {
        if let AudioCommand::SetExternalInstrumentPatch {
            track_id,
            bank,
            program,
        } = cmd
        {
            assert!(
                found.is_none(),
                "expected exactly one SetExternalInstrumentPatch command"
            );
            found = Some((*track_id, *bank, *program));
        }
    }
    found.expect("a SetExternalInstrumentPatch command was dispatched")
}

#[test]
fn select_named_patch_sets_bank_and_program_in_one_edit() {
    let (mut app, rx) = external_capturing_app();
    app.test_dispatch(Message::ExternalInstrument(Eim::SetDevice(
        TRACK,
        Some(MUSE.to_string()),
    )));
    let _ = drain(&rx); // discard Enable + SetDevice round-trips.

    // "Bank 2 · Patch 1" → bank_lsb 1 (combined bank 1), program 0.
    app.test_dispatch(Message::ExternalInstrument(Eim::SetPatch(
        TRACK,
        Some(1),
        Some(0),
    )));

    // App state records both the bank and the program.
    let ext = app.test_external_instrument(TRACK).expect("still external");
    assert_eq!(ext.bank, Some(1), "named patch resolves its bank");
    assert_eq!(ext.program, Some(0), "named patch resolves its program");

    // Exactly one Bank Select + Program Change fires — the same engine path
    // the numeric Bank/Program pickers use.
    let cmds = drain(&rx);
    let (track_id, bank, program) = one_patch(&cmds);
    assert_eq!(track_id, TRACK);
    assert_eq!(bank, Some(1));
    assert_eq!(program, Some(0));
}

#[test]
fn clear_named_patch_sends_no_bank_no_program() {
    let (mut app, rx) = external_capturing_app();
    app.test_dispatch(Message::ExternalInstrument(Eim::SetDevice(
        TRACK,
        Some(MUSE.to_string()),
    )));
    app.test_dispatch(Message::ExternalInstrument(Eim::SetPatch(
        TRACK,
        Some(1),
        Some(0),
    )));
    let _ = drain(&rx);

    // The "(no patch)" entry clears both.
    app.test_dispatch(Message::ExternalInstrument(Eim::SetPatch(TRACK, None, None)));

    let ext = app.test_external_instrument(TRACK).expect("still external");
    assert_eq!(ext.bank, None);
    assert_eq!(ext.program, None);

    let cmds = drain(&rx);
    let (_, bank, program) = one_patch(&cmds);
    assert_eq!(bank, None);
    assert_eq!(program, None);
}

#[test]
fn set_patch_is_a_recorded_undoable_edit() {
    // Selecting a named patch is a user-meaningful, reversible edit — the
    // undo classifier records it (the catch-all for config-changing
    // external-instrument variants).
    let action = classify(&Message::ExternalInstrument(Eim::SetPatch(
        TRACK,
        Some(1),
        Some(0),
    )));
    assert!(
        matches!(action, UndoAction::Record),
        "SetPatch should be a recorded undoable edit, got {action:?}"
    );
}

// ---- structural render tests -------------------------------------------

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

fn app_with_external_track() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    app.test_push_track(TrackState::new_instrument(TRACK, 0));
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(Some(TRACK))));
    app
}

fn dispatch(app: &mut Resonance, m: Eim) {
    let _ = app.update(Message::ExternalInstrument(m));
}

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view())
}

/// With a device preset selected, the Patch card shows the named-patch
/// picker: the selected patch's bank/category group is read out beside the
/// numeric tiles, which reflect the resolved bank/program.
#[test]
fn selected_device_shows_named_patch_group_readout() {
    let mut app = app_with_external_track();
    dispatch(&mut app, Eim::Enable(TRACK));
    dispatch(&mut app, Eim::SetDevice(TRACK, Some(MUSE.to_string())));
    // Bank 1 · Patch 1 → bank 0, program 0.
    dispatch(&mut app, Eim::SetPatch(TRACK, Some(0), Some(0)));

    let mut ui = simulator(&app);
    ui.find("PATCH").expect("Patch field header");
    // The selected patch's group (its bank/category) is read out.
    ui.find("Bank 1")
        .expect("named-patch group readout names the selected patch's bank");
    // Tiles reflect the resolved bank/program (zero-padded).
    ui.find("000").expect("bank/program tiles show the resolved 000");
}

/// A different named patch reads out its own group, proving the readout
/// tracks the current selection rather than a constant.
#[test]
fn switching_named_patch_updates_group_readout() {
    let mut app = app_with_external_track();
    dispatch(&mut app, Eim::Enable(TRACK));
    dispatch(&mut app, Eim::SetDevice(TRACK, Some(MUSE.to_string())));
    // Bank 3 · Patch 1 → bank_lsb 2 (combined bank 2), program 0.
    dispatch(&mut app, Eim::SetPatch(TRACK, Some(2), Some(0)));

    let mut ui = simulator(&app);
    ui.find("Bank 3")
        .expect("readout follows the selected patch's group");
}

/// With no device selected, the Patch card keeps the numeric Bank/Program
/// pickers (fallback path) — the named-patch group readout is absent.
#[test]
fn no_device_keeps_numeric_patch_pickers() {
    let mut app = app_with_external_track();
    dispatch(&mut app, Eim::Enable(TRACK));
    dispatch(&mut app, Eim::SetBank(TRACK, Some(31)));
    dispatch(&mut app, Eim::SetProgram(TRACK, Some(12)));

    let mut ui = simulator(&app);
    ui.find("PATCH").expect("Patch field header");
    // Numeric tiles show the raw bank/program.
    ui.find("031").expect("numeric bank tile");
    ui.find("012").expect("numeric program tile");
    // No named-patch group readout without a device.
    assert!(
        ui.find("Named patches").is_err(),
        "the named-patch readout must not appear without a device"
    );
}
