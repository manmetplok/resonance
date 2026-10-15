//! Save-time GC of `audio/clip_<id>.wav` (code review FU-V5a).
//!
//! Clip WAVs were never deleted: every removed clip, superseded vocal
//! render and deleted take left its file in the bundle forever. A manual
//! save now reaps the ones nothing can name — not the saved project, not
//! a backup or the autosave, not an undo/redo snapshot — and keeps
//! anything it cannot be sure about.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use resonance_app::message::{ClipMessage, Message};
use resonance_app::project::clip_gc::{carry_clip_wavs, reap_unreferenced_clip_wavs};
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, ClipId, TrackType};

fn bundle() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("audio")).unwrap();
    dir
}

fn touch(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join("audio").join(name);
    std::fs::write(&p, b"RIFF").unwrap();
    p
}

fn keep(ids: &[ClipId]) -> BTreeSet<ClipId> {
    ids.iter().copied().collect()
}

#[test]
fn only_unnamed_clip_wavs_below_the_newest_id_are_removed() {
    let dir = bundle();
    let d = dir.path();
    let gone = touch(d, "clip_2.wav");
    let named = touch(d, "clip_3.wav");
    let newer = touch(d, "clip_9.wav"); // above every named id: in flight
    let others = [
        touch(d, "vocal_1.wav"),
        touch(d, "asset_1.wav"),
        touch(d, "clip_1.wav.tmp"),
        touch(d, "clip_x.wav"),
    ];

    let removed = reap_unreferenced_clip_wavs(d, &keep(&[3, 5])).unwrap();

    assert_eq!(removed, vec![gone.clone()]);
    assert!(!gone.exists());
    assert!(named.exists() && newer.exists());
    assert!(others.iter().all(|p| p.exists()), "only clip_<digits>.wav");
}

#[test]
fn a_backup_or_autosave_naming_a_clip_keeps_its_wav() {
    let dir = bundle();
    let d = dir.path();
    let backed_up = touch(d, "clip_2.wav");
    let take = touch(d, "clip_3.wav");
    let autosaved = touch(d, "clip_4.wav");
    std::fs::create_dir_all(d.join("backups")).unwrap();
    std::fs::write(
        d.join("backups/project-2026-01-01T00:00:00Z.json"),
        r#"{"clips":[{"id":2,"audio_file":"audio/clip_2.wav"}],
            "take_groups":[{"takes":[{"content":{"Audio":{"clip_ref":3}}}]}]}"#,
    )
    .unwrap();
    std::fs::write(
        d.join("project.autosave.json"),
        r#"{"clips":[{"id":4,"audio_file":"audio/clip_4.wav"}]}"#,
    )
    .unwrap();

    let removed = reap_unreferenced_clip_wavs(d, &keep(&[10])).unwrap();

    assert!(removed.is_empty(), "{removed:?}");
    assert!(backed_up.exists() && take.exists() && autosaved.exists());
}

#[test]
fn an_unreadable_backup_keeps_everything() {
    let dir = bundle();
    let d = dir.path();
    let wav = touch(d, "clip_2.wav");
    std::fs::create_dir_all(d.join("backups")).unwrap();
    std::fs::write(d.join("backups/project-broken.json"), b"{ not json").unwrap();

    assert!(reap_unreferenced_clip_wavs(d, &keep(&[10])).is_err());
    assert!(wav.exists());
}

#[test]
fn nothing_named_at_all_keeps_everything() {
    let dir = bundle();
    let wav = touch(dir.path(), "clip_2.wav");
    assert!(reap_unreferenced_clip_wavs(dir.path(), &keep(&[])).unwrap().is_empty());
    assert!(wav.exists());
}

fn imported(clip_id: ClipId) -> AudioEvent {
    AudioEvent::ClipImported {
        clip_id,
        track_id: 1,
        start_sample: 0,
        duration_samples: 48_000,
        name: format!("clip {clip_id}"),
        waveform_peaks: Vec::new(),
    }
}

#[test]
fn the_keep_set_covers_the_undo_stack_and_the_engine_report() {
    let (mut app, _task) = Resonance::new_for_test();
    let dir = bundle();
    app.test_set_active_project(true);
    app.test_set_project_path(dir.path().to_path_buf());
    app.test_add_track(1, TrackType::Audio);
    app.test_apply_engine_event(imported(7));
    app.test_apply_engine_event(imported(8));
    // Clip 7 deleted: only the undo stack still names it.
    let _ = app.update(Message::Clip(ClipMessage::DeleteClip(7)));
    assert!(!app.test_clips().iter().any(|c| c.id == 7));
    let keep = app.test_clip_gc_keep(&[]).expect("not recording");
    assert!(keep.contains(&7), "undo can bring clip 7 back: {keep:?}");
    assert!(keep.contains(&8));

    // The engine's own report is kept even when the mirror lags it.
    let keep = app.test_clip_gc_keep(&[11]).unwrap();
    assert!(keep.contains(&11));
}

#[test]
fn the_keep_set_covers_the_redo_stack() {
    let (mut app, _task) = Resonance::new_for_test();
    let dir = bundle();
    app.test_set_active_project(true);
    app.test_set_project_path(dir.path().to_path_buf());
    app.test_add_track(1, TrackType::Audio);
    // Clip 9 lands as a recorded take (an undoable edit); undoing it leaves
    // it named only by the redo snapshot.
    app.test_apply_engine_event(AudioEvent::RecordingFinished {
        clip_id: 9,
        track_id: 1,
        start_sample: 0,
        duration_samples: 48_000,
        name: "take".into(),
        waveform_peaks: Vec::new(),
    });
    let _ = app.update(Message::Undo);
    app.test_apply_engine_event(AudioEvent::AllCleared);
    assert!(!app.test_clips().iter().any(|c| c.id == 9));

    let keep = app.test_clip_gc_keep(&[]).unwrap();
    assert!(keep.contains(&9), "redo can bring clip 9 back: {keep:?}");
}

#[test]
fn no_gc_while_recording() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_transport_recording(true);
    assert!(app.test_clip_gc_keep(&[]).is_none());
}

// ---- carry_clip_wavs (UX-03, BUG-60) ----------------------------------

#[test]
fn a_carry_copies_only_missing_named_clip_wavs() {
    let from = bundle();
    let to = bundle();
    touch(from.path(), "clip_1.wav");
    touch(from.path(), "clip_2.wav");
    touch(from.path(), "vocal_1.wav");
    let kept = to.path().join("audio/clip_2.wav");
    std::fs::write(&kept, b"already here").unwrap();

    let copied = carry_clip_wavs(from.path(), to.path(), Some(&keep(&[1, 2])));

    assert_eq!(copied, vec![to.path().join("audio/clip_1.wav")]);
    assert_eq!(std::fs::read(&kept).unwrap(), b"already here", "never overwritten");
    assert!(!to.path().join("audio/vocal_1.wav").exists());
    assert!(from.path().join("audio/clip_1.wav").exists(), "copied, not moved");
}

#[cfg(unix)]
#[test]
fn a_carry_onto_the_same_dir_through_a_symlink_copies_nothing() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real.rproj");
    std::fs::create_dir_all(real.join("audio")).unwrap();
    std::fs::write(real.join("audio/clip_1.wav"), b"RIFF").unwrap();
    let link = root.path().join("link.rproj");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    assert!(carry_clip_wavs(&real, &link, None).is_empty());
    assert!(carry_clip_wavs(&real, &real, None).is_empty());
}
