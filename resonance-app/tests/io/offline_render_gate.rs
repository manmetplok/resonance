//! The GUI side of the "offline render in progress" gate (code review
//! UPD-06, pairing with `resonance-audio` MIX-02 / ENG-05).
//!
//! A WAV mixdown (`io.bouncing`) drives the live plugin instances from a
//! worker thread like a bounce in place or a freeze, but used to gate
//! nothing: Play went through mid-export, and — for every offline render —
//! the `ProjectIo` family is exempt from the pre-dispatch gates, so Ctrl+O
//! could swap the project out (`ClearAll` + replay) under the renderer.

use resonance_app::message::{Message, ProjectIoMessage, TransportMessage, UiMessage};
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, AudioEvent};

const MIXDOWN: &str = "/tmp/io-offline-render-gate.wav";

fn app_mid_mixdown() -> (
    Resonance,
    resonance_audio::__test_support::Receiver<AudioCommand>,
) {
    let (mut app, _task, cmd_rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::BouncePathSelected(Some(
        MIXDOWN.to_owned(),
    ))));
    assert!(app.test_is_bouncing(), "sanity: the mixdown is in flight");
    // Drain the `BounceToWav` the dialog sent.
    while cmd_rx.try_recv().is_ok() {}
    (app, cmd_rx)
}

#[test]
fn play_is_dropped_while_a_wav_mixdown_renders_and_works_again_after() {
    let (mut app, cmd_rx) = app_mid_mixdown();

    let _ = app.update(Message::Transport(TransportMessage::Play));
    assert!(
        !cmd_rx.try_iter().any(|c| matches!(c, AudioCommand::Play)),
        "no Play may reach the engine while the export owns the plugins"
    );
    assert!(
        !app.test_transport_playing(),
        "the transport mirror must not claim playback started"
    );

    app.test_apply_engine_event(AudioEvent::BounceComplete {
        path: MIXDOWN.to_owned(),
    });
    assert!(!app.test_is_bouncing());
    let _ = app.update(Message::Transport(TransportMessage::Play));
    assert!(
        cmd_rx.try_iter().any(|c| matches!(c, AudioCommand::Play)),
        "once the export is done Play goes through"
    );
}

#[test]
fn opening_another_project_is_refused_while_a_render_is_in_flight() {
    let (mut app, cmd_rx) = app_mid_mixdown();

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(
        "/tmp/io-offline-render-gate-other.rproj".to_owned(),
    ))));
    assert!(
        !cmd_rx
            .try_iter()
            .any(|c| matches!(c, AudioCommand::SetProjectDir(_))),
        "the engine's project dir must not be repointed under the renderer"
    );
    assert!(
        app.test_project_path().is_none(),
        "the app must not adopt the other project's path"
    );
    let banner = app
        .test_error_message()
        .expect("the refusal is surfaced on the error banner");
    assert!(banner.contains("render"), "got: {banner}");
}

#[test]
fn open_recent_is_refused_while_a_render_is_in_flight() {
    let (mut app, cmd_rx) = app_mid_mixdown();

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenRecent(
        std::path::PathBuf::from("/tmp/io-offline-render-gate-recent.rproj"),
    )));
    assert!(app.test_project_path().is_none());
    assert!(
        !cmd_rx
            .try_iter()
            .any(|c| matches!(c, AudioCommand::SetProjectDir(_))),
        "OpenRecent must not repoint the engine either"
    );
    assert!(app
        .test_error_message()
        .is_some_and(|m| m.contains("render")));
}

/// An offline control measurement (`meter.measure` with the default
/// `render` source) holds the same renderer but gates no GUI message, so
/// the open / new-project entry points have to refuse on their own.
#[test]
fn open_and_new_project_are_refused_while_a_measurement_renders() {
    let (mut app, _task, cmd_rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    let response = crate::common::call(&mut app, "meter.measure", serde_json::json!({}));
    assert!(response.error.is_none(), "sanity: the measurement job starts");
    while cmd_rx.try_recv().is_ok() {}

    let _ = app.update(Message::Ui(UiMessage::StartNewProject));
    assert!(
        app.test_error_message()
            .is_some_and(|m| m.contains("render")),
        "New project is refused with the banner"
    );
    // The banner can still be dismissed: a measurement is not a modal.
    let _ = app.update(Message::Ui(UiMessage::DismissError));
    assert!(app.test_error_message().is_none());

    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenProject));
    assert!(
        app.test_error_message()
            .is_some_and(|m| m.contains("render")),
        "Ctrl+O / Open project is refused with the banner"
    );
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::OpenPathSelected(Some(
        "/tmp/io-offline-render-gate-measure.rproj".to_owned(),
    ))));
    assert!(app.test_project_path().is_none());
    assert!(
        !cmd_rx
            .try_iter()
            .any(|c| matches!(c, AudioCommand::SetProjectDir(_))),
        "the engine's project dir must not be repointed under the measurement"
    );
}
