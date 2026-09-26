//! The filesystem layer of the vocal render pipeline (ba todo #1259).
//!
//! Directory layout, WAV naming and the superseded-file unlink used to
//! be private helpers inside the update handler. They decide where a
//! user's rendered vocals live — and whether a save captures them — so
//! they get pinned here now that they have a module of their own.

use resonance_app::update::compose::vocal_audio_io::{
    render_wav_filename, unlink_if_exists, vocal_audio_dir, write_rendered_wav,
};

#[test]
fn a_saved_project_keeps_its_vocals_inside_it() {
    // `project_path` is the `.rproj` project DIRECTORY, so renders go to
    // its own `audio/` — not a sibling `audio/` shared with every other
    // project in the same parent folder (FU-B3).
    let dir = vocal_audio_dir(Some(std::path::Path::new("/songs/demo.rproj")));
    assert_eq!(dir, std::path::Path::new("/songs/demo.rproj/audio"));
}

#[test]
fn an_unsaved_session_renders_to_a_temp_dir() {
    let dir = vocal_audio_dir(None);
    assert_eq!(dir, std::env::temp_dir().join("resonance_vocal"));
}

#[test]
fn every_take_gets_its_own_filename() {
    // A fresh render must never overwrite the WAV the engine may still
    // be playing from; the superseded file is unlinked separately.
    let a = render_wav_filename();
    let b = render_wav_filename();
    assert!(a.starts_with("vocal_"), "{a}");
    assert!(a.ends_with(".wav"), "{a}");
    assert_ne!(a, b);
}

#[test]
fn a_rendered_take_lands_in_the_destination_dir() {
    let dir = tempfile::tempdir().expect("temp dir");
    let samples = vec![0.0f32; 128];

    let path = write_rendered_wav(dir.path(), &samples, 44_100).expect("write");

    assert_eq!(path.parent(), Some(dir.path()));
    assert!(path.exists(), "{}", path.display());
    let reader = hound::WavReader::open(&path).expect("readable WAV");
    assert_eq!(reader.spec().channels, 2, "vocals are written as stereo");
    assert_eq!(reader.spec().sample_rate, 44_100);
}

#[test]
fn the_destination_directory_is_created_on_demand() {
    // A project's `audio/` folder need not exist yet — the first render
    // after a save must not fail on that.
    let dir = tempfile::tempdir().expect("temp dir");
    let fresh = dir.path().join("audio");
    assert!(!fresh.exists());

    let path = write_rendered_wav(&fresh, &[0.0f32; 8], 48_000).expect("write");
    assert_eq!(path.parent(), Some(fresh.as_path()));
    assert!(path.exists());
}

#[test]
fn unlinking_is_best_effort() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = write_rendered_wav(dir.path(), &[0.0f32; 8], 48_000).expect("write");

    unlink_if_exists(&path);
    assert!(!path.exists());
    // A second pass (a stale-epoch result racing the tear-down) must not
    // fail the regen.
    unlink_if_exists(&path);
}

/// FU-B3: a manual save garbage-collects rendered vocal WAVs in the
/// project's `audio/` that nothing references any more — conservatively:
/// only `vocal_*.wav`, only inside this project, never one an installed
/// vocal clip still points at, and never on autosave.
#[test]
fn a_manual_save_reaps_orphaned_vocal_wavs_only() {
    use resonance_app::message::{Message, ProjectIoMessage};
    use resonance_app::state::ViewMode;
    use resonance_app::Resonance;

    let root = tempfile::tempdir().expect("temp dir");
    let project = root.path().join("song.rproj");
    let audio = project.join("audio");
    std::fs::create_dir_all(&audio).expect("audio dir");
    let orphan = audio.join("vocal_1.wav");
    let live = audio.join("vocal_2.wav");
    let saved_clip = audio.join("clip_5.wav");
    let user_file = audio.join("vocal_notes.txt");
    let sibling = root.path().join("audio").join("vocal_3.wav");
    std::fs::create_dir_all(sibling.parent().unwrap()).expect("sibling dir");
    for f in [&orphan, &live, &saved_clip, &user_file, &sibling] {
        std::fs::write(f, b"x").expect("write fixture");
    }

    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(project.clone());
    app.test_install_vocal_audio_clip(1, 2, 3, 40, live.clone());

    let saved = |autosave| Message::ProjectIo(ProjectIoMessage::ProjectSaved(Ok(()), autosave));
    let _ = app.update(saved(true));
    assert!(orphan.exists(), "an autosave never reaps");

    let _ = app.update(saved(false));
    assert!(!orphan.exists(), "the unreferenced render is reaped");
    assert!(live.exists(), "an installed vocal clip's WAV stays");
    assert!(saved_clip.exists(), "clip_<id>.wav belongs to the saved project");
    assert!(user_file.exists(), "only vocal_*.wav are candidates");
    assert!(sibling.exists(), "nothing outside the project directory is touched");
}
