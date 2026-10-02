//! `SaveClipsToProjectDir` is answered exactly once, success or failure
//! (code review STATE2-02). The app's save collector waits for that one
//! reply; a failed copy or transcode used to send only `AudioEvent::Error`
//! and wedge every later save, open and new for the rest of the session.

use std::path::PathBuf;

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::transcode_to_wav;
use resonance_audio::types::*;

const TRACK: TrackId = 3;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-save-clips-answers-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn clip(id: ClipId, source: ClipSource) -> AudioClip {
    AudioClip {
        id,
        track_id: TRACK,
        start_sample: 0,
        source,
        name: format!("clip {id}"),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: Default::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

fn pcm() -> Vec<f32> {
    (0..1024).map(|i| i as f32 / 1024.0).collect()
}

/// Run the save and return its replies: the completion and failure
/// events, in order (any other event is ignored).
fn save_replies(h: &mut EngineHandlerHarness) -> Vec<AudioEvent> {
    h.dispatch(AudioCommand::SaveClipsToProjectDir);
    h.drain_events()
        .into_iter()
        .filter(|e| {
            matches!(
                e,
                AudioEvent::ClipsSavedToProjectDir { .. } | AudioEvent::ClipsSaveFailed { .. }
            )
        })
        .collect()
}

fn single_failure(replies: Vec<AudioEvent>) -> String {
    match replies.as_slice() {
        [AudioEvent::ClipsSaveFailed { error }] => error.clone(),
        other => panic!("expected exactly one ClipsSaveFailed, got {other:?}"),
    }
}

#[test]
fn a_successful_save_answers_with_the_clip_files() {
    let dir = scratch("ok");
    let mut h = EngineHandlerHarness::new();
    h.set_project_dir(dir.clone());
    h.push_clip(clip(7, ClipSource::memory(pcm())));

    match save_replies(&mut h).as_slice() {
        [AudioEvent::ClipsSavedToProjectDir { clip_files }] => {
            assert_eq!(clip_files, &vec![(7, "audio/clip_7.wav".to_owned())]);
        }
        other => panic!("expected exactly one ClipsSavedToProjectDir, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_project_dir_still_answers() {
    let mut h = EngineHandlerHarness::new();
    h.push_clip(clip(7, ClipSource::memory(pcm())));
    let error = single_failure(save_replies(&mut h));
    assert!(error.contains("no project directory"), "{error}");
}

#[test]
fn a_failed_transcode_still_answers() {
    // The project "directory" is a plain file: nothing can be written
    // under it, the way a full disk or a read-only mount refuses.
    let dir = scratch("encode");
    let blocker = dir.join("not-a-dir.rproj");
    std::fs::write(&blocker, b"x").unwrap();
    let mut h = EngineHandlerHarness::new();
    h.set_project_dir(blocker);
    h.push_clip(clip(7, ClipSource::memory(pcm())));

    let error = single_failure(save_replies(&mut h));
    assert!(error.contains("Transcode clip 7"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_copy_still_answers() {
    // A mapped clip from another bundle (the Save As case) whose source
    // file vanished before the copy.
    let dir = scratch("copy");
    let src = dir.join("elsewhere.wav");
    transcode_to_wav(&src, &pcm(), 48_000).unwrap();
    let source = ClipSource::open_wav(&src).unwrap();
    std::fs::remove_file(&src).unwrap();
    let project = dir.join("song.rproj");
    let mut h = EngineHandlerHarness::new();
    h.set_project_dir(project);
    h.push_clip(clip(9, source));

    let error = single_failure(save_replies(&mut h));
    assert!(error.contains("Copy clip 9"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clips_written_before_a_failure_are_remapped() {
    // Clip 1 encodes into the bundle; clip 2's copy source is gone. The
    // save fails, but clip 1 already plays from its bundle file, so the
    // retry has nothing left to do for it.
    let dir = scratch("partial");
    let src = dir.join("elsewhere.wav");
    transcode_to_wav(&src, &pcm(), 48_000).unwrap();
    let mapped = ClipSource::open_wav(&src).unwrap();
    std::fs::remove_file(&src).unwrap();
    let project = dir.join("song.rproj");
    let mut h = EngineHandlerHarness::new();
    h.set_project_dir(project.clone());
    h.push_clip(clip(1, ClipSource::memory(pcm())));
    h.push_clip(clip(2, mapped));

    let _ = single_failure(save_replies(&mut h));
    let target = project.join("audio").join("clip_1.wav");
    match &h.clip(1).expect("clip 1").source {
        ClipSource::Mapped { path, .. } => assert_eq!(path, &target),
        ClipSource::Memory(_) => panic!("clip 1 was written but not remapped"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
