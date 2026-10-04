//! The engine answers every `DeleteClip` with exactly one `ClipDeleted`
//! (code review FU-A13e/f): a delete cancels a load still in flight, and
//! a delete of an id it never loaded — missing media, a cancelled load —
//! echoes too. So a live delete owes its echo whether or not the mirror
//! held the clip yet, and an echo nobody owed is harmless.

use resonance_app::message::{ClipMessage, Message};
use resonance_app::state::ClipState;
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, FadeCurve, TrackType};

const SR: u32 = 48_000;
const TRACK: u64 = 1;
const CLIP: u64 = 7;
/// A clip whose load the engine has not echoed yet: not in the mirror.
const LOADING: u64 = 8;

fn clip(id: u64) -> ClipState {
    ClipState {
        id,
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
        warp: Default::default(),
    }
}

fn app() -> Resonance {
    let (mut app, _task, _cmds) = Resonance::new_for_test_with_capture();
    app.test_set_sample_rate(SR);
    app.test_set_active_project(true);
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_push_clip(clip(CLIP));
    app
}

fn clip_ids(app: &Resonance) -> Vec<u64> {
    app.test_clips().iter().map(|c| c.id).collect()
}

/// Deleting a clip the mirror does not hold yet still owes the echo: the
/// engine sends it, and a clip put back under the id before it arrives
/// (an undo) must survive it.
#[test]
fn a_delete_of_a_clip_not_mirrored_yet_owes_its_echo() {
    let mut app = app();
    let _ = app.update(Message::Clip(ClipMessage::DeleteClip(LOADING)));
    assert!(
        !app.test_restore_echoes_settled(),
        "the engine echoes every DeleteClip, so this one is owed"
    );

    // Put back under the same id before the echo lands.
    app.test_push_clip(clip(LOADING));
    app.test_apply_engine_event(AudioEvent::ClipDeleted { clip_id: LOADING });

    assert!(
        clip_ids(&app).contains(&LOADING),
        "the late echo must not delete the clip put back under its id"
    );
    assert!(app.test_restore_echoes_settled(), "and it settles the ledger");
}

/// An echo for a clip the app never mirrored and never owed — a vocal
/// take's delete of a load the engine cancelled — changes nothing.
#[test]
fn an_unowed_delete_echo_of_an_unknown_clip_changes_nothing() {
    let mut app = app();
    app.test_apply_engine_event(AudioEvent::ClipDeleted { clip_id: 999 });
    assert_eq!(clip_ids(&app), vec![CLIP]);
    assert!(app.test_restore_echoes_settled());
}
