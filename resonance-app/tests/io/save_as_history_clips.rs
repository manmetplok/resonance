//! Save As brings along the clip WAVs only the undo history names
//! (BUG-60).
//!
//! The engine writes the live clips into the new bundle; a clip deleted
//! before the Save As is named only by an undo snapshot, and its WAV stayed
//! behind in the old bundle. Undo resolves a snapshot's audio against the
//! *current* project, so undoing back to that clip after the Save As played
//! silence. The save now copies every WAV the history names that the new
//! bundle lacks from the old one, off the update loop.

use std::path::{Path, PathBuf};

use resonance_app::message::{ClipMessage, Message, ProjectIoMessage};
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, ClipId, TrackType};

use crate::common::run_task;

const TRACK: u64 = 1;

fn imported(clip_id: ClipId) -> AudioEvent {
    AudioEvent::ClipImported {
        clip_id,
        track_id: TRACK,
        start_sample: 0,
        duration_samples: 48_000,
        name: format!("clip {clip_id}"),
        waveform_peaks: Vec::new(),
    }
}

fn write_wav(bundle: &Path, clip_id: ClipId, bytes: &[u8]) -> PathBuf {
    let audio = bundle.join("audio");
    std::fs::create_dir_all(&audio).unwrap();
    let path = audio.join(format!("clip_{clip_id}.wav"));
    std::fs::write(&path, bytes).unwrap();
    path
}

/// Answer the collecting save's engine round-trip and run its disk write
/// to completion, as the engine and the iced runtime would.
fn complete_save(app: &mut Resonance, clip_files: Vec<(ClipId, String)>) {
    let request = app.test_save_request();
    let _ = app.test_engine_event_task(AudioEvent::ClipsSavedToProjectDir {
        request,
        clip_files,
    });
    let write = app.test_engine_event_task(AudioEvent::AllPluginStatesSaved {
        request,
        states: Vec::new(),
    });
    for message in run_task(write) {
        let _ = app.update(message);
    }
    assert_eq!(app.test_save_in_flight(), None, "the save completed");
}

/// A titled project in `root/A.rproj` with clip 8 live and clip 7 deleted:
/// only the undo history names clip 7.
fn project_with_an_undo_only_clip(root: &Path) -> (Resonance, Receiver<AudioCommand>, PathBuf) {
    let a = root.join("A.rproj");
    write_wav(&a, 7, b"RIFF clip seven");
    write_wav(&a, 8, b"RIFF clip eight");
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_project_path(a.clone());
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_apply_engine_event(imported(7));
    app.test_apply_engine_event(imported(8));
    let _ = app.update(Message::Clip(ClipMessage::DeleteClip(7)));
    assert!(app.test_can_undo());
    (app, rx, a)
}

fn save_as(app: &mut Resonance, target: &Path) {
    let _ = app.update(Message::ProjectIo(ProjectIoMessage::SavePathSelected(Some(
        target.to_string_lossy().into_owned(),
    ))));
}

#[test]
fn undoing_to_a_deleted_clip_after_save_as_finds_its_audio() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, rx, a) = project_with_an_undo_only_clip(root.path());
    let b = root.path().join("B.rproj");

    save_as(&mut app, &b);
    // The engine wrote the live clip into B itself.
    write_wav(&b, 8, b"RIFF clip eight");
    complete_save(&mut app, vec![(8, "audio/clip_8.wav".to_owned())]);
    assert_eq!(app.test_project_path(), Some(b.as_path()));

    assert_eq!(
        std::fs::read(b.join("audio/clip_7.wav")).ok().as_deref(),
        Some(&b"RIFF clip seven"[..]),
        "the history's clip came along"
    );
    assert!(a.join("audio/clip_7.wav").exists(), "copied, not moved");

    while rx.try_recv().is_ok() {}
    let _ = app.update(Message::Undo);
    let loads: Vec<PathBuf> = rx
        .try_iter()
        .filter_map(|c| match c {
            AudioCommand::LoadClipFromWav { clip_id: 7, path, .. } => Some(path),
            _ => None,
        })
        .collect();
    assert_eq!(loads, vec![b.join("audio/clip_7.wav")], "undo reloads clip 7 from B");
    assert!(loads[0].exists(), "and the file is there to play");
}

/// Only what the history names is copied: a stray WAV in the old bundle
/// nothing refers to stays behind.
#[test]
fn save_as_copies_only_the_clips_the_history_names() {
    let root = tempfile::tempdir().unwrap();
    let (mut app, _rx, a) = project_with_an_undo_only_clip(root.path());
    write_wav(&a, 3, b"RIFF stray");
    let b = root.path().join("B.rproj");

    save_as(&mut app, &b);
    complete_save(&mut app, Vec::new());

    assert!(b.join("audio/clip_7.wav").exists());
    assert!(!b.join("audio/clip_3.wav").exists(), "nothing names clip 3");
}
