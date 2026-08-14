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
fn a_saved_project_keeps_its_vocals_beside_it() {
    // `audio/` next to the project file, so "save" captures the clip
    // and a moved project folder keeps its renders.
    let dir = vocal_audio_dir(Some(std::path::Path::new("/songs/demo/demo.rson")));
    assert_eq!(dir, std::path::Path::new("/songs/demo/audio"));
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
