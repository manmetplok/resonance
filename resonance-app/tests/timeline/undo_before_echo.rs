//! Undo pressed before the engine echo lands (code review STATE-10).
//!
//! The GUI delete of a clip or track only sent the engine command; the
//! mirror dropped the entity on the `ClipDeleted` / `TrackRemoved` echo,
//! drained every 200 ms on an idle transport. An undo in between
//! snapshotted a mirror that still held the entity, saw "no change", took
//! the fast path — and the late echo then deleted it after the undo. The
//! delete now updates the mirror at once, so the undo sees the change and
//! the echo (which the engine sends before the undo's `ClearAll` is
//! processed) finds nothing left to remove.

use resonance_app::message::{ClipMessage, Message, TrackMessage};
use resonance_app::state::{ClipState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, FadeCurve, TrackType};

const SR: u32 = 48_000;
const TRACK: u64 = 1;
const CLIP: u64 = 7;

fn app() -> Resonance {
    let (mut app, _task, _cmds) = Resonance::new_for_test_with_capture();
    let _ = ViewMode::Arrange;
    app.test_set_sample_rate(SR);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/state10-undo.rprj"));
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_push_clip(ClipState {
        id: CLIP,
        track_id: TRACK,
        start_sample: 0,
        duration_samples: SR as u64,
        name: "clip".into(),
        total_frames: SR as u64,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
    });
    app
}

fn has_clip(app: &Resonance) -> bool {
    app.test_clips().iter().any(|c| c.id == CLIP)
}

#[test]
fn undoing_a_clip_delete_before_its_echo_keeps_the_clip() {
    let mut app = app();
    let _ = app.update(Message::Clip(ClipMessage::DeleteClip(CLIP)));
    let _ = app.update(Message::Undo);

    // The engine's echoes, in the order it sends them: the delete, then
    // the undo's clear.
    app.test_apply_engine_event(AudioEvent::ClipDeleted { clip_id: CLIP });
    app.test_apply_engine_event(AudioEvent::AllCleared);

    assert!(has_clip(&app), "the undo restored the clip; the late echo must not undo it");
    assert!(app.test_undo_history().can_redo());
}

#[test]
fn undoing_a_track_delete_before_its_echo_keeps_the_track() {
    let mut app = app();
    let _ = app.update(Message::Track(TrackMessage::RequestRemoveTrack(TRACK)));
    let _ = app.update(Message::Track(TrackMessage::ConfirmRemoveTrack));
    let _ = app.update(Message::Undo);

    app.test_apply_engine_event(AudioEvent::TrackRemoved { track_id: TRACK });
    app.test_apply_engine_event(AudioEvent::AllCleared);

    assert!(has_clip(&app), "the track's clip came back with it");
    assert!(app.test_undo_history().can_redo());
}
