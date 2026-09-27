//! The mutation gate refuses edits while a replay is pending (code review
//! UPD-03).
//!
//! A project load and a template instantiation send `ClearAll` and rebuild
//! everything from the file once the engine answers `AllCleared` — on a
//! later Tick. A control edit landing in between was acknowledged
//! (revision bumped) and then silently wiped by the replay. It must answer
//! `busy` instead, like the other gates.
//!
//! An undo used to open the same window through its `ClearAll` fallback
//! (and this test forced that path); since ARCH-01 A-13j every undo is
//! restored in place, synchronously, so it leaves no window — the second
//! test pins that.

use resonance_app::message::{Message, ProjectIoMessage};
use resonance_app::project::{LoadedProject, ProjectFile};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, TrackType};
use resonance_control::{ErrorKind, MutationAck};
use crate::common::call;

const TRACK: u64 = 1;

fn set_volume(app: &mut Resonance) -> resonance_control::Response {
    call(
        app,
        "mixer.set_volume_db",
        serde_json::json!({"track_id": TRACK, "volume_db": -6.0}),
    )
}

fn app_with_a_track() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/control-gate-loading.rprj"));
    app.test_add_track(TRACK, TrackType::Audio);
    app
}

#[test]
fn a_control_edit_during_a_project_load_is_busy() {
    let mut app = app_with_a_track();
    // The opened project has the same track, so the edit is valid again
    // once it has replayed.
    let file = ProjectFile {
        tracks: vec![app.test_build_project_file().tracks[0].clone()],
        ..ProjectFile::default()
    };
    // ClearAll now, the replay on the engine's AllCleared.
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::ProjectLoaded(Ok(
        Box::new(LoadedProject {
            file,
            project_dir: std::path::PathBuf::from("/tmp/control-gate-loading.rprj"),
            midi_notes: Default::default(),
            plugin_states: Default::default(),
        }),
    ))));
    let revision = app.revision();

    let error = set_volume(&mut app)
        .error
        .expect("an edit mid-replay must not be acknowledged");
    assert_eq!(error.kind(), ErrorKind::Busy, "got {error:?}");
    assert_eq!(app.revision(), revision, "a refused edit bumps nothing");

    // Once the replay has run the project takes edits again.
    app.test_apply_engine_event(AudioEvent::AllCleared);
    let _: MutationAck = set_volume(&mut app).result().expect("edits resume after the replay");
}

#[test]
fn a_control_edit_right_after_a_structural_undo_is_accepted() {
    let mut app = app_with_a_track();
    let before_take = app.test_snapshot_for_undo();
    // A finished take is an undoable edit that adds a clip.
    app.test_apply_engine_event(AudioEvent::RecordingFinished {
        clip_id: 7,
        track_id: TRACK,
        start_sample: 0,
        duration_samples: 48_000,
        name: "take".into(),
        waveform_peaks: Vec::new(),
    });
    // Its undo removes the clip, in place: no ClearAll, nothing pending.
    app.test_begin_restore_from_snapshot(before_take);

    let _: MutationAck = set_volume(&mut app)
        .result()
        .expect("an undo leaves no replay window to refuse edits in");
}
