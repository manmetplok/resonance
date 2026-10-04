//! Persistence + reconcile coverage for `ProjectFile::loop_record_mode`
//! (W3): cycle-record mode lives on the project — like the loop range it
//! sits beside — because it decides the SHAPE of recorded content for
//! this song's workflow, not a cross-project device preference.

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, TransportMessage, UiMessage};
use resonance_app::project::ProjectFile;
use resonance_app::state::ViewMode;
use resonance_app::{theme, Resonance};
use resonance_audio::test_support::Receiver;
use resonance_audio::types::AudioCommand;

const WINDOW: (f32, f32) = (1440.0, 900.0);

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

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

/// Settings → Recording, through the real widget tree: clicking the
/// toggle emits `TransportMessage::SetLoopRecordMode(true)`, which the
/// real `update()` path turns into both the engine command and an undo
/// entry.
#[test]
fn clicking_the_recording_toggle_sends_the_message_and_flips_the_engine() {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    let _ = app.update(Message::Ui(UiMessage::OpenSettings));
    let rx = app.test_capture_engine();

    let messages: Vec<Message> = {
        let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
        ui.click("Distinct takes per loop pass")
            .expect("the recording toggle is rendered");
        ui.into_messages().collect()
    };
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, Message::Transport(TransportMessage::SetLoopRecordMode(true)))),
        "{messages:?}"
    );
    for m in messages {
        let _ = app.update(m);
    }
    assert!(app.test_loop_record_mode());
    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::SetLoopRecordMode(true))));
}

#[test]
fn loop_record_mode_survives_a_serde_round_trip() {
    let file = ProjectFile {
        loop_record_mode: true,
        ..ProjectFile::default()
    };
    let json = serde_json::to_string(&file).expect("serialize");
    let restored: ProjectFile = serde_json::from_str(&json).expect("deserialize");
    assert!(restored.loop_record_mode);
}

#[test]
fn legacy_project_without_the_field_loads_to_the_engine_default() {
    // A project document authored before `loop_record_mode` existed:
    // every required key is present but it is omitted.
    // `#[serde(default)]` must supply `false` — the engine's own default
    // (`RecordingState::loop_record`) — rather than failing the load.
    let legacy = serde_json::json!({
        "version": ProjectFile::default().version,
        "sample_rate": 44100,
        "bpm": 120.0,
        "time_sig_num": 4,
        "time_sig_den": 4,
        "metronome_enabled": false,
        "master_volume": 0.0,
        "loop_enabled": false,
        "loop_in": 0,
        "loop_out": 0,
        "tracks": [],
        "clips": [],
    });
    let file: ProjectFile = serde_json::from_value(legacy).expect("legacy project loads");
    assert!(!file.loop_record_mode);
}

#[test]
fn build_project_file_mirrors_the_live_transport_flag() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    let _ = app.update(Message::Transport(
        TransportMessage::SetLoopRecordMode(true),
    ));
    let file = app.test_build_project_file();
    assert!(file.loop_record_mode);
}

/// Opening a project sends `AudioCommand::SetLoopRecordMode` so the
/// engine's `rec.loop_record` flag matches the file that is now open,
/// and the app's own mirror (read by the Settings → Recording toggle)
/// follows it.
#[test]
fn opening_a_project_reconciles_loop_record_mode_to_the_engine() {
    let (mut app, _task) = Resonance::new_for_test();
    let rx = app.test_capture_engine();
    app.test_replay_loaded_project(ProjectFile {
        loop_record_mode: true,
        ..ProjectFile::default()
    });
    assert!(app.test_loop_record_mode());
    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::SetLoopRecordMode(true))));
}

/// The diff-replay path (an undo/redo across a change to the field)
/// resends the command too — same reconcile table entry as the loop
/// range, which already behaves this way.
#[test]
fn an_undo_across_a_loop_record_mode_change_resends_the_command() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    // `can_record_undo` needs a saved path to replay snapshots against.
    app.test_set_project_path(std::path::PathBuf::from(
        "/tmp/resonance-test-loop-record-mode.rproj",
    ));
    let _ = app.update(Message::Transport(
        TransportMessage::SetLoopRecordMode(true),
    ));
    assert!(app.test_loop_record_mode());

    let rx = app.test_capture_engine();
    let _ = app.update(Message::Undo);
    assert!(!app.test_loop_record_mode());
    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::SetLoopRecordMode(false))));

    let _ = app.update(Message::Redo);
    assert!(app.test_loop_record_mode());
}
