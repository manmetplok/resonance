//! A gesture that changes nothing is no edit (code review STATE-07).
//!
//! Every press on a clip body opens an undo transaction (`StartClipDrag`,
//! `Begin`) and every release closes it (`EndClipDrag`, `Commit`). The
//! commit used to record unconditionally, so a plain click to select a
//! clip pushed an empty entry, wiped the redo stack, marked the project
//! dirty and bumped the control revision. The commit now compares the
//! pre-gesture snapshot with the current state and drops the transaction
//! when they match.

use resonance_app::message::{ClipMessage, Message, TrackMessage};
use resonance_app::state::{ClipState, ViewMode};
use resonance_app::Resonance;
use resonance_audio::types::{FadeCurve, TrackType};

const SR: u32 = 48_000;
const TRACK: u64 = 1;
const CLIP: u64 = 7;

fn clip() -> ClipState {
    ClipState {
        id: CLIP,
        track_id: TRACK,
        start_sample: 0,
        duration_samples: 2 * SR as u64,
        name: "clip".into(),
        total_frames: 2 * SR as u64,
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
    }
}

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_sample_rate(SR);
    app.test_set_arrange_zoom(100.0);
    app.test_set_active_project(true);
    // The history only records once the project has a path on disk.
    app.test_set_project_path(std::path::PathBuf::from("/tmp/timeline-undo-noop.rprj"));
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_push_clip(clip());
    app
}

fn press(app: &mut Resonance) {
    let _ = app.update(Message::Clip(ClipMessage::StartClipDrag {
        clip_id: CLIP,
        grab_offset_x: 50.0,
        start_x: 50.0,
        start_y: 0.0,
    }));
}

fn entries(app: &Resonance) -> usize {
    app.test_undo_history().test_undo_entries().len()
}

#[test]
fn a_click_on_a_clip_keeps_redo_dirty_and_revision() {
    let mut app = app();
    let _ = app.update(Message::Track(TrackMessage::SetTrackVolume(TRACK, -6.0)));
    let _ = app.update(Message::Undo);
    assert!(app.test_undo_history().can_redo());
    app.test_set_dirty(false);
    let revision = app.revision();
    let before = entries(&app);

    press(&mut app);
    let _ = app.update(Message::Clip(ClipMessage::EndClipDrag));

    assert!(app.test_undo_history().can_redo(), "a click must not clear redo");
    assert!(!app.is_dirty(), "a click must not mark the project dirty");
    assert_eq!(app.revision(), revision, "a click is not an edit");
    assert_eq!(entries(&app), before, "no empty undo entry");
    assert!(!app.test_undo_history().has_pending(), "the gesture is closed");
}

#[test]
fn a_drag_that_moves_the_clip_still_records_one_entry() {
    let mut app = app();
    app.test_set_dirty(false);
    let revision = app.revision();

    press(&mut app);
    let _ = app.update(Message::Clip(ClipMessage::UpdateClipDrag(150.0, 0.0)));
    let _ = app.update(Message::Clip(ClipMessage::EndClipDrag));

    let start = app.test_clips().iter().find(|c| c.id == CLIP).unwrap().start_sample;
    assert_ne!(start, 0, "the drag moved the clip");
    assert_eq!(entries(&app), 1, "the move is one undo entry");
    assert!(app.is_dirty());
    assert_eq!(app.revision(), revision + 1, "one revision bump per gesture");
}
