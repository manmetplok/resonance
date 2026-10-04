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
        warp: Default::default(),
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

/// Every Begin…Commit gesture, not just the clip drag (FU-M4c).
///
/// The gesture-end check compares the whole snapshot — the `ProjectFile`
/// a save writes (minus the A/B monitor state) and the notes — so "changed nothing
/// snapshotted" means "changed nothing a save or an undo could see". The
/// only other state these gestures touch is transient interaction state
/// (the drag handle, `selected_global_event`), which is deliberately not
/// undoable. Each gesture here must record exactly one entry when it
/// moves something — its effect is visible to the comparison — and none
/// when it is released where it started.
mod every_gesture {
    use super::*;
    use resonance_app::message::{AutomationMessage, GlobalTrackMessage, TransportMessage};
    use resonance_app::state::LoopDragTarget;
    use resonance_common::{AutomationTarget, CurveKind};

    fn file_json(app: &Resonance) -> serde_json::Value {
        serde_json::to_value(app.test_build_project_file()).expect("serializes")
    }

    /// Run `gesture(app, moved)` twice on a fresh app: released in place
    /// it records nothing and leaves the project clean; moved it records
    /// one entry whose undo restores the saved form exactly.
    fn check(name: &str, setup: impl Fn(&mut Resonance), gesture: impl Fn(&mut Resonance, bool)) {
        let mut app = app();
        setup(&mut app);
        app.test_set_dirty(false);
        let before = entries(&app);
        let saved = file_json(&app);
        gesture(&mut app, false);
        assert_eq!(entries(&app), before, "{name}: in place records nothing");
        assert!(!app.is_dirty(), "{name}: in place leaves the project clean");
        assert!(!app.test_undo_history().has_pending(), "{name}: closed");

        gesture(&mut app, true);
        assert_ne!(file_json(&app), saved, "{name}: the move changes the saved form");
        assert_eq!(entries(&app), before + 1, "{name}: one entry per gesture");
        assert!(app.is_dirty(), "{name}: a move marks the project dirty");
        let _ = app.update(Message::Undo);
        assert_eq!(file_json(&app), saved, "{name}: undo restores it");
    }

    #[test]
    fn loop_drag() {
        check(
            "loop drag",
            |_| {},
            |app, moved| {
                let _ = app.update(Message::Transport(TransportMessage::StartLoopDrag(
                    LoopDragTarget::Out,
                )));
                if moved {
                    let _ = app.update(Message::Transport(TransportMessage::UpdateLoopDrag(
                        400.0,
                    )));
                }
                let _ = app.update(Message::Transport(TransportMessage::EndLoopDrag));
            },
        );
    }

    #[test]
    fn tempo_drag() {
        check(
            "tempo drag",
            |app| {
                let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddTempoEvent {
                    bar: 5,
                    bpm: 100.0,
                }));
            },
            |app, moved| {
                let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::StartTempoDrag(1)));
                if moved {
                    let _ = app.update(Message::GlobalTrack(
                        GlobalTrackMessage::UpdateTempoEvent {
                            index: 1,
                            bar: 7,
                            bpm: 90.0,
                        },
                    ));
                }
                let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::EndTempoDrag));
            },
        );
    }

    #[test]
    fn automation_breakpoint_drag() {
        let target = AutomationTarget::TrackGain(TRACK);
        check(
            "breakpoint drag",
            |app| {
                let _ = app.update(Message::Automation(AutomationMessage::AddBreakpoint {
                    target: target.clone(),
                    time_frames: SR as u64,
                    value: 0.5,
                    curve: CurveKind::Linear,
                }));
            },
            |app, moved| {
                let _ = app.update(Message::Automation(AutomationMessage::StartBreakpointDrag {
                    target: target.clone(),
                    index: 0,
                }));
                if moved {
                    let _ = app.update(Message::Automation(AutomationMessage::DragBreakpoint {
                        target: target.clone(),
                        index: 0,
                        time_frames: 2 * SR as u64,
                        value: 0.25,
                    }));
                }
                let _ = app.update(Message::Automation(AutomationMessage::EndBreakpointDrag));
            },
        );
    }
}
