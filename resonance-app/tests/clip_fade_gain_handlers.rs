//! Clip fade/gain edit + drag update handlers and undo wiring (todo #317,
//! arch doc #156). These tests drive the real `ClipMessage` reducers
//! against a `Resonance` whose engine is swapped for a command-capturing
//! stub, so they assert both the GUI-side `ClipState` mutation and the
//! exact `AudioCommand`s the handlers emit — proving the fade-handle drag,
//! the gain-bead drag, and every inspector edit reach the engine through
//! `SetClipFade` / `SetClipGain`, with no engine read-getters involved.
//!
//! Undo is covered two ways: the message classifier (`classify`) assigns
//! the right `Begin`/`Commit`/`Record` action to each message, and the
//! `UndoExtras` snapshot/restore round-trip returns a clip's fade/gain to
//! its pre-edit values while re-syncing the engine.

use std::collections::HashMap;

use resonance_app::message::{ClipMessage, Message};
use resonance_app::state::ClipState;
use resonance_app::undo::{classify, ClipFadeGain, UndoAction, UndoExtras};
use resonance_app::Resonance;
use resonance_audio::__test_support::Receiver;
use resonance_audio::types::{AudioCommand, FadeCurve};

const SR: u32 = 48_000;
const ZOOM: f32 = 100.0; // px per second (default)

/// A 2-second audio clip anchored at the timeline origin: 0..200px wide at
/// the default zoom, so pointer maths in the tests are round numbers.
fn clip(id: u64) -> ClipState {
    ClipState {
        id,
        track_id: 1,
        start_sample: 0,
        duration_samples: 2 * SR as u64, // 2s -> 96_000 frames -> 200px
        name: format!("clip {id}"),
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
    }
}

/// App + capturing engine, sample rate / zoom fixed for deterministic
/// pixel→frame conversions, with `clip(7)` already present.
fn app_with_clip() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task) = Resonance::new();
    let rx = app.test_capture_engine();
    app.test_set_sample_rate(SR);
    app.test_set_arrange_zoom(ZOOM);
    app.test_push_clip(clip(7));
    (app, rx)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

fn clip_of(app: &Resonance, id: u64) -> ClipState {
    app.test_clips()
        .iter()
        .find(|c| c.id == id)
        .unwrap()
        .clone()
}

// ---------------------------------------------------------------------
// Fade-handle drag
// ---------------------------------------------------------------------

#[test]
fn fade_in_drag_sets_length_and_sends_command_on_end() {
    let (mut app, rx) = app_with_clip();

    app.test_dispatch(Message::Clip(ClipMessage::StartClipFadeDrag {
        clip_id: 7,
        edge: resonance_app::state::ClipEdge::Left,
        anchor_x: 0.0,
    }));
    // Start selects the clip but emits no engine command.
    assert!(drain(&rx).is_empty());

    // Drag the handle to x=50px → 0.5s → 24_000 frames.
    app.test_dispatch(Message::Clip(ClipMessage::UpdateClipFadeDrag(50.0)));
    assert_eq!(clip_of(&app, 7).fade_in_frames, 24_000);
    // Mid-drag is mirror-only — nothing sent yet.
    assert!(drain(&rx).is_empty());

    app.test_dispatch(Message::Clip(ClipMessage::EndClipFadeDrag));
    let cmds = drain(&rx);
    assert_eq!(cmds.len(), 1);
    match cmds[0] {
        AudioCommand::SetClipFade {
            clip_id,
            fade_in_frames,
            fade_out_frames,
            ..
        } => {
            assert_eq!(clip_id, 7);
            assert_eq!(fade_in_frames, 24_000);
            assert_eq!(fade_out_frames, 0);
        }
        ref other => panic!("expected SetClipFade, got {other:?}"),
    }
}

#[test]
fn fade_out_drag_measures_from_the_right_edge() {
    let (mut app, rx) = app_with_clip();
    // Right edge sits at 200px (2s * 100px/s).
    app.test_dispatch(Message::Clip(ClipMessage::StartClipFadeDrag {
        clip_id: 7,
        edge: resonance_app::state::ClipEdge::Right,
        anchor_x: 200.0,
    }));
    // Drag inward to x=150 → 50px from the right → 0.5s → 24_000 frames.
    app.test_dispatch(Message::Clip(ClipMessage::UpdateClipFadeDrag(150.0)));
    let c = clip_of(&app, 7);
    assert_eq!(c.fade_out_frames, 24_000);
    assert_eq!(c.fade_in_frames, 0, "fade-out drag leaves fade-in alone");

    let _ = drain(&rx);
    app.test_dispatch(Message::Clip(ClipMessage::EndClipFadeDrag));
    match drain(&rx).as_slice() {
        [AudioCommand::SetClipFade {
            fade_out_frames, ..
        }] => assert_eq!(*fade_out_frames, 24_000),
        other => panic!("expected one SetClipFade, got {other:?}"),
    }
}

#[test]
fn fade_drag_clamps_to_clip_length() {
    let (mut app, _rx) = app_with_clip();
    app.test_dispatch(Message::Clip(ClipMessage::StartClipFadeDrag {
        clip_id: 7,
        edge: resonance_app::state::ClipEdge::Left,
        anchor_x: 0.0,
    }));
    // Drag way past the right edge (x=10_000px); fade can't exceed the
    // clip's 96_000-frame audible length.
    app.test_dispatch(Message::Clip(ClipMessage::UpdateClipFadeDrag(10_000.0)));
    assert_eq!(clip_of(&app, 7).fade_in_frames, 2 * SR as u64);
}

#[test]
fn fade_drag_past_the_edge_clamps_to_zero() {
    let (mut app, _rx) = app_with_clip();
    app.test_dispatch(Message::Clip(ClipMessage::StartClipFadeDrag {
        clip_id: 7,
        edge: resonance_app::state::ClipEdge::Left,
        anchor_x: 0.0,
    }));
    // Pointer to the left of the clip start → no negative fade.
    app.test_dispatch(Message::Clip(ClipMessage::UpdateClipFadeDrag(-40.0)));
    assert_eq!(clip_of(&app, 7).fade_in_frames, 0);
}

#[test]
fn fade_update_without_start_is_a_no_op() {
    let (mut app, rx) = app_with_clip();
    // No StartClipFadeDrag → no active drag state → nothing happens.
    app.test_dispatch(Message::Clip(ClipMessage::UpdateClipFadeDrag(50.0)));
    assert_eq!(clip_of(&app, 7).fade_in_frames, 0);
    app.test_dispatch(Message::Clip(ClipMessage::EndClipFadeDrag));
    assert!(drain(&rx).is_empty());
}

// ---------------------------------------------------------------------
// Gain-bead drag
// ---------------------------------------------------------------------

#[test]
fn gain_drag_up_increases_db_and_sends_on_end() {
    let (mut app, rx) = app_with_clip();
    app.test_dispatch(Message::Clip(ClipMessage::StartClipGainDrag {
        clip_id: 7,
        anchor_y: 100.0,
    }));
    assert!(drain(&rx).is_empty());

    // Drag up 50px → +7.5 dB at 0.15 dB/px.
    app.test_dispatch(Message::Clip(ClipMessage::UpdateClipGainDrag(50.0)));
    assert!((clip_of(&app, 7).gain_db - 7.5).abs() < 1e-4);
    assert!(drain(&rx).is_empty(), "mid-drag sends nothing");

    app.test_dispatch(Message::Clip(ClipMessage::EndClipGainDrag));
    match drain(&rx).as_slice() {
        [AudioCommand::SetClipGain { clip_id, gain_db }] => {
            assert_eq!(*clip_id, 7);
            assert!((*gain_db - 7.5).abs() < 1e-4);
        }
        other => panic!("expected one SetClipGain, got {other:?}"),
    }
}

#[test]
fn gain_drag_down_decreases_db() {
    let (mut app, _rx) = app_with_clip();
    app.test_dispatch(Message::Clip(ClipMessage::StartClipGainDrag {
        clip_id: 7,
        anchor_y: 100.0,
    }));
    // Drag down 20px → -3 dB.
    app.test_dispatch(Message::Clip(ClipMessage::UpdateClipGainDrag(120.0)));
    assert!((clip_of(&app, 7).gain_db + 3.0).abs() < 1e-4);
}

#[test]
fn gain_drag_clamps_to_engine_range() {
    let (mut app, _rx) = app_with_clip();
    app.test_dispatch(Message::Clip(ClipMessage::StartClipGainDrag {
        clip_id: 7,
        anchor_y: 0.0,
    }));
    // A huge upward drag can't exceed the engine's max gain.
    app.test_dispatch(Message::Clip(ClipMessage::UpdateClipGainDrag(-100_000.0)));
    assert_eq!(clip_of(&app, 7).gain_db, resonance_audio::MAX_CLIP_GAIN_DB);
    // ...and a huge downward drag can't go below the min.
    app.test_dispatch(Message::Clip(ClipMessage::UpdateClipGainDrag(100_000.0)));
    assert_eq!(clip_of(&app, 7).gain_db, resonance_audio::MIN_CLIP_GAIN_DB);
}

// ---------------------------------------------------------------------
// Inspector flyout edits (emitted by todo #319)
// ---------------------------------------------------------------------

#[test]
fn inspector_fade_in_ms_sets_frames_and_sends_full_fade() {
    let (mut app, rx) = app_with_clip();
    app.test_dispatch(Message::Clip(ClipMessage::SetClipFadeInMs {
        clip_id: 7,
        ms: 500.0,
    }));
    // 500 ms at 48 kHz = 24_000 frames.
    assert_eq!(clip_of(&app, 7).fade_in_frames, 24_000);
    match drain(&rx).as_slice() {
        [AudioCommand::SetClipFade {
            clip_id,
            fade_in_frames,
            ..
        }] => {
            assert_eq!(*clip_id, 7);
            assert_eq!(*fade_in_frames, 24_000);
        }
        other => panic!("expected one SetClipFade, got {other:?}"),
    }
}

#[test]
fn inspector_fade_out_ms_clamps_to_clip_length() {
    let (mut app, _rx) = app_with_clip();
    // 10 seconds of fade on a 2-second clip clamps to the clip length.
    app.test_dispatch(Message::Clip(ClipMessage::SetClipFadeOutMs {
        clip_id: 7,
        ms: 10_000.0,
    }));
    assert_eq!(clip_of(&app, 7).fade_out_frames, 2 * SR as u64);
}

#[test]
fn inspector_gain_db_clamps_and_sends() {
    let (mut app, rx) = app_with_clip();
    app.test_dispatch(Message::Clip(ClipMessage::SetClipGainDb {
        clip_id: 7,
        gain_db: -6.0,
    }));
    assert!((clip_of(&app, 7).gain_db + 6.0).abs() < 1e-4);
    match drain(&rx).as_slice() {
        [AudioCommand::SetClipGain { clip_id, gain_db }] => {
            assert_eq!(*clip_id, 7);
            assert!((*gain_db + 6.0).abs() < 1e-4);
        }
        other => panic!("expected one SetClipGain, got {other:?}"),
    }

    // Out-of-range request is clamped before it reaches the engine.
    app.test_dispatch(Message::Clip(ClipMessage::SetClipGainDb {
        clip_id: 7,
        gain_db: 999.0,
    }));
    assert_eq!(clip_of(&app, 7).gain_db, resonance_audio::MAX_CLIP_GAIN_DB);
}

#[test]
fn inspector_curve_picker_changes_curve_and_preserves_lengths() {
    let (mut app, rx) = app_with_clip();
    // Seed a known fade-in length first.
    app.test_dispatch(Message::Clip(ClipMessage::SetClipFadeInMs {
        clip_id: 7,
        ms: 500.0,
    }));
    let _ = drain(&rx);

    app.test_dispatch(Message::Clip(ClipMessage::SetClipFadeInCurve {
        clip_id: 7,
        curve: FadeCurve::Linear,
    }));
    let c = clip_of(&app, 7);
    assert_eq!(c.fade_in_curve, FadeCurve::Linear);
    assert_eq!(c.fade_in_frames, 24_000, "curve change keeps the length");
    match drain(&rx).as_slice() {
        [AudioCommand::SetClipFade {
            fade_in_curve,
            fade_in_frames,
            ..
        }] => {
            assert_eq!(*fade_in_curve, FadeCurve::Linear);
            assert_eq!(*fade_in_frames, 24_000);
        }
        other => panic!("expected one SetClipFade, got {other:?}"),
    }
}

#[test]
fn inspector_reset_clears_fades_and_gain() {
    let (mut app, rx) = app_with_clip();
    // Dirty the clip first.
    app.test_dispatch(Message::Clip(ClipMessage::SetClipFadeInMs {
        clip_id: 7,
        ms: 500.0,
    }));
    app.test_dispatch(Message::Clip(ClipMessage::SetClipFadeOutMs {
        clip_id: 7,
        ms: 250.0,
    }));
    app.test_dispatch(Message::Clip(ClipMessage::SetClipGainDb {
        clip_id: 7,
        gain_db: 4.0,
    }));
    let _ = drain(&rx);

    app.test_dispatch(Message::Clip(ClipMessage::ResetClipFadeGain { clip_id: 7 }));
    let c = clip_of(&app, 7);
    assert_eq!(c.fade_in_frames, 0);
    assert_eq!(c.fade_out_frames, 0);
    assert_eq!(c.fade_in_curve, FadeCurve::default());
    assert_eq!(c.fade_out_curve, FadeCurve::default());
    assert_eq!(c.gain_db, 0.0);

    // Reset re-syncs both the fade and the gain on the engine.
    let cmds = drain(&rx);
    assert!(cmds.iter().any(|c| matches!(
        c,
        AudioCommand::SetClipFade {
            fade_in_frames: 0,
            fade_out_frames: 0,
            ..
        }
    )));
    assert!(cmds
        .iter()
        .any(|c| matches!(c, AudioCommand::SetClipGain { gain_db, .. } if *gain_db == 0.0)));
}

#[test]
fn inspector_edit_on_unknown_clip_is_a_no_op() {
    let (mut app, rx) = app_with_clip();
    app.test_dispatch(Message::Clip(ClipMessage::SetClipGainDb {
        clip_id: 999,
        gain_db: -6.0,
    }));
    // No such clip → mirror untouched and no command emitted.
    assert_eq!(clip_of(&app, 7).gain_db, 0.0);
    assert!(drain(&rx).is_empty());
}

// ---------------------------------------------------------------------
// Undo wiring
// ---------------------------------------------------------------------

#[test]
fn fade_and_gain_drags_are_a_single_undo_transaction() {
    // Start opens a transaction; End commits it; mid-drag updates skip.
    assert!(matches!(
        classify(&Message::Clip(ClipMessage::StartClipFadeDrag {
            clip_id: 7,
            edge: resonance_app::state::ClipEdge::Left,
            anchor_x: 0.0,
        })),
        UndoAction::Begin
    ));
    assert!(matches!(
        classify(&Message::Clip(ClipMessage::UpdateClipFadeDrag(1.0))),
        UndoAction::Skip
    ));
    assert!(matches!(
        classify(&Message::Clip(ClipMessage::EndClipFadeDrag)),
        UndoAction::Commit
    ));
    assert!(matches!(
        classify(&Message::Clip(ClipMessage::StartClipGainDrag {
            clip_id: 7,
            anchor_y: 0.0,
        })),
        UndoAction::Begin
    ));
    assert!(matches!(
        classify(&Message::Clip(ClipMessage::UpdateClipGainDrag(1.0))),
        UndoAction::Skip
    ));
    assert!(matches!(
        classify(&Message::Clip(ClipMessage::EndClipGainDrag)),
        UndoAction::Commit
    ));
}

#[test]
fn inspector_edits_each_record_one_undo_entry() {
    for msg in [
        ClipMessage::SetClipFadeInMs {
            clip_id: 7,
            ms: 1.0,
        },
        ClipMessage::SetClipFadeOutMs {
            clip_id: 7,
            ms: 1.0,
        },
        ClipMessage::SetClipGainDb {
            clip_id: 7,
            gain_db: 1.0,
        },
        ClipMessage::SetClipFadeInCurve {
            clip_id: 7,
            curve: FadeCurve::Linear,
        },
        ClipMessage::SetClipFadeOutCurve {
            clip_id: 7,
            curve: FadeCurve::Exp,
        },
        ClipMessage::ResetClipFadeGain { clip_id: 7 },
    ] {
        assert!(
            matches!(classify(&Message::Clip(msg.clone())), UndoAction::Record),
            "{msg:?} should record one undo entry"
        );
    }
}

#[test]
fn snapshot_captures_fade_gain_and_restore_round_trips() {
    let (mut app, rx) = app_with_clip();

    // Edit the clip, then snapshot the *post-edit* state.
    app.test_dispatch(Message::Clip(ClipMessage::SetClipFadeInMs {
        clip_id: 7,
        ms: 500.0,
    }));
    app.test_dispatch(Message::Clip(ClipMessage::SetClipGainDb {
        clip_id: 7,
        gain_db: -6.0,
    }));
    let _ = drain(&rx);

    let snapshot = app.test_snapshot_for_undo();
    let captured = snapshot
        .extras
        .clip_fade_gain
        .get(&7)
        .copied()
        .expect("snapshot captured clip 7 fade/gain");
    assert_eq!(captured.fade_in_frames, 24_000);
    assert!((captured.gain_db + 6.0).abs() < 1e-4);

    // Now change the clip again (simulating a later edit)...
    app.test_dispatch(Message::Clip(ClipMessage::ResetClipFadeGain { clip_id: 7 }));
    assert_eq!(clip_of(&app, 7).fade_in_frames, 0);
    assert_eq!(clip_of(&app, 7).gain_db, 0.0);
    let _ = drain(&rx);

    // ...and restore the captured extras: the mirror returns to the
    // snapshot's values and the engine is re-synced via SetClipFade/Gain.
    app.test_apply_clip_fade_gain_restore(&snapshot.extras.clip_fade_gain);
    let c = clip_of(&app, 7);
    assert_eq!(c.fade_in_frames, 24_000);
    assert!((c.gain_db + 6.0).abs() < 1e-4);

    let cmds = drain(&rx);
    assert!(cmds.iter().any(|c| matches!(
        c,
        AudioCommand::SetClipFade {
            fade_in_frames: 24_000,
            ..
        }
    )));
    assert!(cmds.iter().any(
        |c| matches!(c, AudioCommand::SetClipGain { gain_db, .. } if (*gain_db + 6.0).abs() < 1e-4)
    ));
}

#[test]
fn restore_skips_clips_whose_fade_gain_is_unchanged() {
    let (mut app, rx) = app_with_clip();
    let _ = drain(&rx);

    // The clip is at defaults; a restore map that matches the current
    // state must not emit any engine command.
    let mut map: HashMap<u64, ClipFadeGain> = HashMap::new();
    map.insert(
        7,
        ClipFadeGain {
            fade_in_frames: 0,
            fade_in_curve: FadeCurve::default(),
            fade_out_frames: 0,
            fade_out_curve: FadeCurve::default(),
            gain_db: 0.0,
        },
    );
    app.test_apply_clip_fade_gain_restore(&map);
    assert!(
        drain(&rx).is_empty(),
        "no-op restore should not re-send commands"
    );
}

#[test]
fn restore_into_finalize_undo_path_reapplies_fade_gain() {
    // The full slow-path restore (`finalize_undo_restore`) also re-applies
    // clip fade/gain from the extras.
    let (mut app, rx) = app_with_clip();
    let _ = drain(&rx);

    let mut extras = UndoExtras::default();
    extras.clip_fade_gain.insert(
        7,
        ClipFadeGain {
            fade_in_frames: 12_000,
            fade_in_curve: FadeCurve::Exp,
            fade_out_frames: 0,
            fade_out_curve: FadeCurve::default(),
            gain_db: 2.0,
        },
    );
    app.test_finalize_undo_restore(extras);

    let c = clip_of(&app, 7);
    assert_eq!(c.fade_in_frames, 12_000);
    assert_eq!(c.fade_in_curve, FadeCurve::Exp);
    assert!((c.gain_db - 2.0).abs() < 1e-4);
}
