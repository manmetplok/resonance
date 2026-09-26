//! A full-reload undo must find the audio of every clip it restores
//! (code review FU-V5b).
//!
//! The slow path reloads each audio clip from `audio/clip_<id>.wav`, the
//! only name an undo snapshot records — and before the fix that file was
//! written only by a save. A clip that lived in RAM (a bounce, a pool
//! placement) or in a vocal render's `vocal_*.wav` came back silent when
//! a structural edit made since was undone before saving.
//!
//! The engine now writes the file on `PersistClipWavs` (pinned end to end
//! in `resonance-audio`'s `engine::persist_clip_wavs`); what the app owes
//! is sending it every time it captures a snapshot, *before* the edit's
//! own commands — the engine runs commands in order, so the clip is
//! persisted while it still exists — and before the undo's `ClearAll`.

use resonance_app::message::{ClipMessage, Message, TrackMessage};
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, AudioEvent, TrackType};

const SR: u32 = 48_000;
const TRACK: u64 = 1;
const CLIP: u64 = 7;

fn app() -> (Resonance, crossbeam_channel::Receiver<AudioCommand>, tempfile::TempDir) {
    let (mut app, _task, cmds) = Resonance::new_for_test_with_capture();
    let dir = tempfile::tempdir().unwrap();
    app.test_set_sample_rate(SR);
    app.test_set_active_project(true);
    app.test_set_project_path(dir.path().to_path_buf());
    app.test_add_track(TRACK, TrackType::Audio);
    // An in-RAM clip lands: the engine echoes it, nothing is on disk.
    app.test_apply_engine_event(AudioEvent::ClipImported {
        clip_id: CLIP,
        track_id: TRACK,
        start_sample: 0,
        duration_samples: SR as u64,
        name: "bounce".into(),
        waveform_peaks: Vec::new(),
    });
    while cmds.try_recv().is_ok() {}
    (app, cmds, dir)
}

fn drain(cmds: &crossbeam_channel::Receiver<AudioCommand>) -> Vec<AudioCommand> {
    cmds.try_iter().collect()
}

fn position(sent: &[AudioCommand], pred: impl Fn(&AudioCommand) -> bool) -> Option<usize> {
    sent.iter().position(pred)
}

#[test]
fn an_edit_persists_clip_audio_before_its_own_commands() {
    let (mut app, cmds, _dir) = app();
    let _ = app.update(Message::Clip(ClipMessage::DeleteClip(CLIP)));
    let sent = drain(&cmds);

    let persist = position(&sent, |c| matches!(c, AudioCommand::PersistClipWavs))
        .expect("capturing the pre-delete snapshot persists the clip's audio");
    let delete = position(&sent, |c| matches!(c, AudioCommand::DeleteClip { .. }))
        .expect("the delete itself");
    assert!(
        persist < delete,
        "persisted while the clip still exists in the engine: {sent:?}"
    );
}

#[test]
fn undo_persists_the_current_clips_before_clearing_the_engine() {
    let (mut app, cmds, _dir) = app();
    // A structural edit that leaves the clip in place — deleting a second
    // one — so the undo takes the slow path while the redo snapshot names
    // the clip.
    app.test_apply_engine_event(AudioEvent::ClipImported {
        clip_id: CLIP + 1,
        track_id: TRACK,
        start_sample: SR as u64,
        duration_samples: SR as u64,
        name: "other".into(),
        waveform_peaks: Vec::new(),
    });
    let _ = app.update(Message::Clip(ClipMessage::DeleteClip(CLIP + 1)));
    while cmds.try_recv().is_ok() {}

    let _ = app.update(Message::Undo);
    let sent = drain(&cmds);
    let persist = position(&sent, |c| matches!(c, AudioCommand::PersistClipWavs))
        .expect("the redo snapshot's clips are persisted");
    let clear = position(&sent, |c| matches!(c, AudioCommand::ClearAll))
        .expect("a structural undo clears the engine");
    assert!(persist < clear, "{sent:?}");

    // The replay then reloads the clip from exactly the file persisted.
    app.test_apply_engine_event(AudioEvent::AllCleared);
    let sent = drain(&cmds);
    let reload = sent.iter().find_map(|c| match c {
        AudioCommand::LoadClipFromWav { clip_id, path, .. } if *clip_id == CLIP => Some(path),
        _ => None,
    });
    assert_eq!(
        reload.map(|p| p.file_name().unwrap().to_owned()),
        Some(format!("clip_{CLIP}.wav").into()),
        "{sent:?}"
    );
}

#[test]
fn nothing_is_persisted_without_audio_clips() {
    let (mut app, _task, cmds) = Resonance::new_for_test_with_capture();
    let dir = tempfile::tempdir().unwrap();
    app.test_set_active_project(true);
    app.test_set_project_path(dir.path().to_path_buf());
    app.test_add_track(TRACK, TrackType::Audio);
    while cmds.try_recv().is_ok() {}
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -6.0)));
    assert!(
        !drain(&cmds)
            .iter()
            .any(|c| matches!(c, AudioCommand::PersistClipWavs)),
        "a project with no audio clips sends nothing extra"
    );
}
