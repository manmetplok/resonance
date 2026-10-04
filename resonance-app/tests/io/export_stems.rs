//! Export modal → `AudioCommand::ExportStems`, and the `StemExport*`
//! events back into the modal (code review ARCH2-01). Before the fix
//! `ExportMessage::Confirm` returned `Task::none()`, nothing in the app
//! sent `ExportStems`, and the dispatcher dropped all six events.

use std::path::PathBuf;

use resonance_app::message::{ExportMessage, Message};
use resonance_app::state::{ExportMode, ExportPhase, ExportSource};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{
    AudioCommand, AudioEvent, EngineError, StemBitDepth, StemSource, TrackType,
};

const TRACK: u64 = 7;
const BUS: u64 = 9_000_001;

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

/// A saved project at `dir` with one audio track and one bus, and the
/// Export modal open on it.
fn app_with_dialog(dir: &std::path::Path) -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_project_path(dir.to_path_buf());
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_add_bus(BUS, "Drums/Room");
    let _ = app.update(Message::Export(ExportMessage::Open));
    let _ = drain(&rx);
    (app, rx)
}

fn phase(app: &Resonance) -> ExportPhase {
    app.test_export_dialog().expect("dialog open").phase.clone()
}

fn export(app: &mut Resonance) {
    let _ = app.update(Message::Export(ExportMessage::Confirm));
}

/// Tick master + the bus + the track and confirm: one `ExportStems` with
/// a target per source, written into `<project>/stems`, and the dialog
/// in its Rendering phase.
fn start(app: &mut Resonance, rx: &Receiver<AudioCommand>) -> Vec<(StemSource, PathBuf)> {
    for source in [
        ExportSource::Master,
        ExportSource::Bus(BUS),
        ExportSource::Track(TRACK),
    ] {
        let _ = app.update(Message::Export(ExportMessage::ToggleSource(source)));
    }
    export(app);
    let cmds = drain(rx);
    let stems: Vec<_> = cmds
        .iter()
        .filter_map(|c| match c {
            AudioCommand::ExportStems {
                targets,
                range,
                bit_depth,
                ..
            } => {
                assert_eq!(*range, None, "default range is the whole project");
                assert_eq!(*bit_depth, StemBitDepth::Int24);
                Some(targets.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(stems.len(), 1, "Confirm sends exactly one ExportStems: {cmds:?}");
    stems[0]
        .iter()
        .map(|t| (t.source, PathBuf::from(&t.path)))
        .collect()
}

#[test]
fn confirm_sends_export_stems_for_the_ticked_sources() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, rx) = app_with_dialog(dir.path());
    assert_eq!(
        app.test_export_dialog().unwrap().destination,
        Some(dir.path().join("stems")),
        "a saved project exports into <project>/stems by default"
    );

    let targets = start(&mut app, &rx);
    let stems = dir.path().join("stems");
    assert!(stems.is_dir(), "the destination folder is created");
    // `ExportSource`'s order: tracks, busses, master.
    assert_eq!(
        targets,
        vec![
            (StemSource::Track(TRACK), stems.join("01-Track 1.wav")),
            (StemSource::Bus(BUS), stems.join("02-Drums_Room.wav")),
            (StemSource::Master, stems.join("03-Master.wav")),
        ]
    );
    assert_eq!(phase(&app), ExportPhase::Rendering { done: 0, total: 3 });
}

#[test]
fn an_existing_file_is_not_overwritten_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let stems = dir.path().join("stems");
    std::fs::create_dir_all(&stems).unwrap();
    std::fs::write(stems.join("03-Master.wav"), b"keep me").unwrap();
    let (mut app, rx) = app_with_dialog(dir.path());
    let targets = start(&mut app, &rx);
    assert_eq!(targets[2].1, stems.join("03-Master_1.wav"));
}

#[test]
fn engine_events_drive_the_dialog_to_done() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, rx) = app_with_dialog(dir.path());
    let targets = start(&mut app, &rx);

    // Mid-render the modal can't be dismissed, and the app refuses a
    // second offline render (it shares the live plugin instances).
    let _ = app.update(Message::Export(ExportMessage::Close));
    assert!(app.test_export_dialog().is_some(), "Close is ignored mid-render");

    let paths: Vec<String> = targets
        .iter()
        .map(|(_, p)| p.to_string_lossy().into_owned())
        .collect();
    for (i, path) in paths.iter().enumerate() {
        app.test_apply_engine_event(AudioEvent::StemExportProgress {
            target_index: i,
            total: 3,
            fraction: i as f32 / 3.0,
        });
        assert_eq!(phase(&app), ExportPhase::Rendering { done: i, total: 3 });
        app.test_apply_engine_event(AudioEvent::StemExportTargetDone {
            index: i,
            path: path.clone(),
        });
    }
    app.test_apply_engine_event(AudioEvent::StemExportComplete {
        files: paths.clone(),
    });
    assert_eq!(
        phase(&app),
        ExportPhase::Done(paths.iter().map(PathBuf::from).collect())
    );
    let _ = app.update(Message::Export(ExportMessage::Close));
    assert!(app.test_export_dialog().is_none(), "Close works once done");
}

#[test]
fn a_failed_target_ends_in_error_keeping_the_written_stems() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, rx) = app_with_dialog(dir.path());
    let targets = start(&mut app, &rx);
    let first = targets[0].1.to_string_lossy().into_owned();
    app.test_apply_engine_event(AudioEvent::StemExportTargetError {
        index: 1,
        message: "disk full".into(),
    });
    app.test_apply_engine_event(AudioEvent::StemExportComplete {
        files: vec![first.clone()],
    });
    match phase(&app) {
        ExportPhase::Error {
            written, message, ..
        } => {
            assert_eq!(written, vec![PathBuf::from(first)]);
            assert!(message.contains("disk full"), "{message}");
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn an_export_that_cannot_start_shows_the_engine_reason() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, rx) = app_with_dialog(dir.path());
    let _ = start(&mut app, &rx);
    app.test_apply_engine_event(AudioEvent::StemExportError(EngineError::busy(
        "Stop transport before exporting stems",
    )));
    assert!(matches!(
        phase(&app),
        ExportPhase::Error { ref message, remaining: 3, .. } if message.contains("Stop transport")
    ));
}

#[test]
fn stop_export_sends_cancel_once_and_mirrors_the_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, rx) = app_with_dialog(dir.path());
    let targets = start(&mut app, &rx);
    let _ = app.update(Message::Export(ExportMessage::CancelRender));
    let _ = app.update(Message::Export(ExportMessage::CancelRender));
    let cancels = drain(&rx)
        .into_iter()
        .filter(|c| matches!(c, AudioCommand::CancelStemExport))
        .count();
    assert_eq!(cancels, 1, "one CancelStemExport per render");
    let first = targets[0].1.to_string_lossy().into_owned();
    app.test_apply_engine_event(AudioEvent::StemExportCancelled {
        files: vec![first.clone()],
    });
    assert_eq!(phase(&app), ExportPhase::Cancelled(vec![PathBuf::from(first)]));
}

#[test]
fn nothing_is_sent_without_a_destination_or_in_midi_mode() {
    // Untitled project: no default folder, so Export stays disabled.
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    let _ = app.update(Message::Export(ExportMessage::Open));
    let _ = app.update(Message::Export(ExportMessage::ToggleSource(ExportSource::Master)));
    export(&mut app);
    assert!(
        !drain(&rx).iter().any(|c| matches!(c, AudioCommand::ExportStems { .. })),
        "no destination → no export"
    );
    assert_eq!(phase(&app), ExportPhase::Setup);

    // The MIDI tab has no engine exporter: its action stays inert.
    let dir = tempfile::tempdir().unwrap();
    let _ = app.update(Message::Export(ExportMessage::DestinationChosen(Some(
        dir.path().to_path_buf(),
    ))));
    let _ = app.update(Message::Export(ExportMessage::SetMode(ExportMode::Midi)));
    export(&mut app);
    assert!(!drain(&rx).iter().any(|c| matches!(c, AudioCommand::ExportStems { .. })));
    assert_eq!(phase(&app), ExportPhase::Setup);
}
