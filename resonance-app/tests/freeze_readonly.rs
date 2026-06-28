//! Read-only gating of frozen inputs + auto-invalidate to stale (ba todo #576).
//!
//! While a track is frozen its *inputs* — notes, lyrics, plugin params,
//! instrument selection — are read-only: an attempted edit must not mutate
//! state or enter the undo stack. Instead it invalidates the freeze to
//! `Stale` (the refreeze banner is the UI todo). The *mixer* controls
//! (volume / pan / mute / solo / routing / sends) stay fully live and never
//! invalidate. These tests drive messages through the full `update()` entry
//! (via `test_update`, which runs the pre-dispatch gates `test_dispatch`
//! skips) and assert both the freeze transition and that no engine command
//! / state mutation / undo entry leaked out of a blocked edit.

use resonance_app::message::{
    FreezeMessage, Message, MidiEditorMessage, PluginMessage, TrackMessage,
};
use resonance_app::state::{FreezeStatus, MidiClipState, PluginSlotState};
use resonance_app::Resonance;
use resonance_audio::__test_support::Receiver;
use resonance_audio::types::{AudioCommand, MidiNote, ParamInfo, TrackId, TrackType};
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

const INSTANCE: u64 = 4242;
const PARAM: u32 = 7;

/// App with a capturing engine + a temp project dir so freeze handlers can
/// derive a cache path (freeze needs a saved project).
///
/// These tests drive messages through the *full* `update()` entry
/// (`test_update`), which runs the pre-dispatch gates. The startup-modal
/// gate swallows every project-mutating message while `has_active_project`
/// is false, so we mark the project active (alongside the path) to lift
/// the modal — exactly the state the app is in once a project is open and
/// a track can actually be frozen.
fn capturing_app() -> (Resonance, Receiver<AudioCommand>, tempfile::TempDir) {
    let (mut app, _task) = Resonance::new();
    let rx = app.test_capture_engine();
    let dir = tempfile::tempdir().expect("temp project dir");
    app.test_set_project_path(dir.path().to_path_buf());
    app.test_set_active_project(true);
    (app, rx, dir)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

fn cache_ref_with_fp(fingerprint: u64) -> FreezeCacheRef {
    FreezeCacheRef::new(
        "freeze_1.wav".to_string(),
        48_000,
        32,
        fingerprint,
        FreezeCacheStatus::Frozen,
    )
}

fn note(pitch: u8) -> MidiNote {
    MidiNote {
        note: pitch,
        velocity: 0.8,
        start_tick: 0,
        duration_ticks: 480,
    }
}

fn midi_clip(id: u64, track_id: TrackId, notes: Vec<MidiNote>) -> MidiClipState {
    MidiClipState {
        id,
        track_id,
        start_sample: 0,
        duration_ticks: 1920,
        name: "clip".to_string(),
        notes,
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

fn synth_plugin() -> PluginSlotState {
    PluginSlotState::new(
        INSTANCE,
        "Test Synth".to_string(),
        "com.test.synth".to_string(),
        "/plugins/test.clap".to_string(),
        vec![ParamInfo {
            id: PARAM,
            name: "Cutoff".to_string(),
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.0,
            current_value: 0.0,
        }],
        false,
    )
}

/// Seed an instrument track that's frozen with a cache whose fingerprint
/// matches its current inputs.
fn frozen_track(app: &mut Resonance, track_id: TrackId) {
    app.test_add_track(track_id, TrackType::Instrument);
    app.test_push_track_plugin(track_id, synth_plugin());
    let fp = app.test_freeze_fingerprint(track_id).expect("fingerprint");
    app.test_set_freeze_status(
        track_id,
        FreezeStatus::Frozen {
            cache_ref: cache_ref_with_fp(fp),
        },
    );
}

// ---------------------------------------------------------------------
// Blocked input edits -> stale
// ---------------------------------------------------------------------

#[test]
fn blocked_plugin_param_edit_marks_stale_without_mutating() {
    let (mut app, rx, _dir) = capturing_app();
    frozen_track(&mut app, 1);
    let _ = drain(&rx);

    app.test_update(Message::Plugin(PluginMessage::SetPluginParam(
        INSTANCE, PARAM, 0.9,
    )));

    // Transitioned to stale, cache kept.
    assert!(
        matches!(app.test_freeze_status(1), FreezeStatus::Stale { .. }),
        "a param edit on a frozen track invalidates it to stale",
    );
    // No mutation: the param value is untouched.
    assert_eq!(app.test_plugin_param(INSTANCE, PARAM), Some(0.0));
    // No engine command leaked out of the blocked edit.
    assert!(
        !drain(&rx)
            .iter()
            .any(|c| matches!(c, AudioCommand::SetPluginParam { .. })),
        "the blocked edit must not reach the engine",
    );
    // No undo entry, project not dirtied.
    assert!(!app.test_can_undo(), "a blocked edit never records undo");
    assert!(!app.test_dirty(), "a blocked edit never dirties the project");
}

#[test]
fn blocked_note_edit_marks_stale() {
    let (mut app, rx, _dir) = capturing_app();
    frozen_track(&mut app, 1);
    app.test_push_midi_clip(midi_clip(10, 1, vec![note(60)]));
    let _ = drain(&rx);

    app.test_update(Message::MidiEditor(MidiEditorMessage::AddNote {
        clip_id: 10,
        note: 64,
        start_tick: 480,
        duration_ticks: 480,
        velocity: 0.7,
    }));

    assert!(matches!(app.test_freeze_status(1), FreezeStatus::Stale { .. }));
    assert!(
        !drain(&rx)
            .iter()
            .any(|c| matches!(c, AudioCommand::AddMidiNote { .. })),
        "the note never reaches the engine while frozen",
    );
    assert!(!app.test_dirty());
}

#[test]
fn blocked_instrument_swap_marks_stale() {
    // Adding a plugin (instrument selection) to a frozen track is blocked.
    let (mut app, rx, _dir) = capturing_app();
    frozen_track(&mut app, 1);
    let _ = drain(&rx);

    app.test_update(Message::Plugin(PluginMessage::RemovePluginFromTrack(
        1, INSTANCE,
    )));

    assert!(matches!(app.test_freeze_status(1), FreezeStatus::Stale { .. }));
    assert!(
        !drain(&rx)
            .iter()
            .any(|c| matches!(c, AudioCommand::RemovePlugin { .. })),
        "the chain edit never reaches the engine while frozen",
    );
}

#[test]
fn stale_track_stays_stale_on_further_edits() {
    let (mut app, rx, _dir) = capturing_app();
    frozen_track(&mut app, 1);
    // First edit flips Frozen -> Stale.
    app.test_update(Message::Plugin(PluginMessage::SetPluginParam(
        INSTANCE, PARAM, 0.5,
    )));
    assert!(matches!(app.test_freeze_status(1), FreezeStatus::Stale { .. }));
    let _ = drain(&rx);

    // A second edit is still a blocked no-op (idempotent stale).
    app.test_update(Message::Plugin(PluginMessage::SetPluginParam(
        INSTANCE, PARAM, 0.6,
    )));
    assert!(matches!(app.test_freeze_status(1), FreezeStatus::Stale { .. }));
    assert!(!drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::SetPluginParam { .. })));
}

// ---------------------------------------------------------------------
// Allowed mixer edits stay live
// ---------------------------------------------------------------------

#[test]
fn allowed_mixer_edit_applies_and_keeps_frozen() {
    let (mut app, rx, _dir) = capturing_app();
    frozen_track(&mut app, 1);
    let _ = drain(&rx);

    app.test_update(Message::Track(TrackMessage::SetTrackVolume(1, -6.0)));

    // The volume edit applied to GUI state...
    let vol = app
        .test_registry()
        .tracks
        .iter()
        .find(|t| t.id == 1)
        .map(|t| t.volume);
    assert_eq!(vol, Some(-6.0), "mixer controls stay live while frozen");
    // ...reached the engine...
    assert!(
        drain(&rx)
            .iter()
            .any(|c| matches!(c, AudioCommand::SetTrackVolume { .. })),
        "a live mixer edit reaches the engine",
    );
    // ...and did NOT invalidate the freeze.
    assert!(
        matches!(app.test_freeze_status(1), FreezeStatus::Frozen { .. }),
        "volume never invalidates a freeze",
    );
}

#[test]
fn mute_solo_pan_never_invalidate() {
    let (mut app, rx, _dir) = capturing_app();
    frozen_track(&mut app, 1);
    let _ = drain(&rx);

    for msg in [
        Message::Track(TrackMessage::ToggleMute(1)),
        Message::Track(TrackMessage::ToggleSolo(1)),
        Message::Track(TrackMessage::SetTrackPan(1, 0.3)),
    ] {
        app.test_update(msg);
        assert!(
            matches!(app.test_freeze_status(1), FreezeStatus::Frozen { .. }),
            "mute/solo/pan are mixer controls and never invalidate",
        );
    }
}

#[test]
fn idle_track_edits_pass_through() {
    // The gate only fires on frozen tracks: an edit on an un-frozen track
    // dispatches normally and reaches the engine.
    let (mut app, rx, _dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);
    app.test_push_track_plugin(1, synth_plugin());
    let _ = drain(&rx);

    app.test_update(Message::Plugin(PluginMessage::SetPluginParam(
        INSTANCE, PARAM, 0.4,
    )));

    assert_eq!(app.test_freeze_status(1), FreezeStatus::Idle);
    assert_eq!(app.test_plugin_param(INSTANCE, PARAM), Some(0.4));
    assert!(drain(&rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::SetPluginParam { .. })));
}

// ---------------------------------------------------------------------
// Refreeze round-trip
// ---------------------------------------------------------------------

#[test]
fn refreeze_renders_and_returns_to_frozen() {
    let (mut app, rx, dir) = capturing_app();
    frozen_track(&mut app, 1);
    // Invalidate to stale via a blocked edit.
    app.test_update(Message::Plugin(PluginMessage::SetPluginParam(
        INSTANCE, PARAM, 0.9,
    )));
    assert!(matches!(app.test_freeze_status(1), FreezeStatus::Stale { .. }));
    let _ = drain(&rx);

    // Refreeze re-renders: kicks the engine + flips to Freezing.
    app.test_update(Message::Freeze(FreezeMessage::RefreezeTrack(1)));
    assert!(
        drain(&rx)
            .iter()
            .any(|c| matches!(c, AudioCommand::FreezeTrack { track_id: 1, .. })),
        "refreeze re-renders the cache",
    );
    assert!(app.test_freeze_status(1).is_freezing());

    // The engine completion mirror (ba todo #575) lands the cache and
    // returns the track to Frozen.
    let new_fp = app.test_freeze_fingerprint(1).expect("fingerprint");
    app.test_set_freeze_status(
        1,
        FreezeStatus::Frozen {
            cache_ref: cache_ref_with_fp(new_fp),
        },
    );
    assert!(matches!(app.test_freeze_status(1), FreezeStatus::Frozen { .. }));
    let _ = dir; // keep the temp project dir alive
}

// ---------------------------------------------------------------------
// Fingerprint recompute confirms staleness
// ---------------------------------------------------------------------

#[test]
fn fingerprint_changes_when_inputs_change() {
    let (mut app, _rx, _dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);
    app.test_push_track_plugin(1, synth_plugin());
    app.test_push_midi_clip(midi_clip(10, 1, vec![note(60)]));

    let fp_before = app.test_freeze_fingerprint(1).expect("fingerprint");
    // Change a plugin param input.
    app.test_set_plugin_param(INSTANCE, PARAM, 0.5);
    let fp_after = app.test_freeze_fingerprint(1).expect("fingerprint");

    assert_ne!(
        fp_before, fp_after,
        "a changed plugin param yields a different fingerprint",
    );
}

#[test]
fn fingerprint_ignores_mixer_controls() {
    let (mut app, _rx, _dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);
    app.test_push_track_plugin(1, synth_plugin());

    let fp_before = app.test_freeze_fingerprint(1).expect("fingerprint");
    // Mixer controls are not render inputs — fingerprint is unchanged.
    app.test_dispatch(Message::Track(TrackMessage::SetTrackVolume(1, -3.0)));
    app.test_dispatch(Message::Track(TrackMessage::ToggleMute(1)));
    let fp_after = app.test_freeze_fingerprint(1).expect("fingerprint");

    assert_eq!(
        fp_before, fp_after,
        "volume / mute do not affect the freeze fingerprint",
    );
}

#[test]
fn revalidate_marks_stale_when_inputs_drift() {
    let (mut app, _rx, _dir) = capturing_app();
    frozen_track(&mut app, 1);

    // Inputs still match the rendered fingerprint -> no transition.
    assert!(!app.test_revalidate_frozen_track(1));
    assert!(matches!(app.test_freeze_status(1), FreezeStatus::Frozen { .. }));

    // Drift an input directly (a change that bypassed the message gate),
    // then revalidate: the fingerprint mismatch confirms staleness.
    app.test_set_plugin_param(INSTANCE, PARAM, 0.7);
    assert!(app.test_revalidate_frozen_track(1));
    assert!(matches!(app.test_freeze_status(1), FreezeStatus::Stale { .. }));
}

#[test]
fn revalidate_is_noop_on_unfrozen_track() {
    let (mut app, _rx, _dir) = capturing_app();
    app.test_add_track(1, TrackType::Instrument);
    app.test_push_track_plugin(1, synth_plugin());

    assert!(!app.test_revalidate_frozen_track(1));
    assert_eq!(app.test_freeze_status(1), FreezeStatus::Idle);
}
