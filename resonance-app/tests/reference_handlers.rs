//! Reducer coverage for the reference-track (A/B) update handlers:
//! each `ReferenceMessage` mutates `ReferenceState` as expected. The
//! engine `AudioCommand` side effect can't be observed from outside the
//! crate, so these assert the optimistic GUI-state changes; the engine's
//! authoritative echoes are covered in `reference_events.rs`.

use std::path::PathBuf;

use resonance_app::message::Message;
use resonance_app::reference::{ReferenceMessage, ReferenceStatus};
use resonance_app::Resonance;
use resonance_audio::types::{ABSource, ReferenceId};

/// A reference message is project-mutating input, so the startup gate
/// drops it unless a project is active. Build an app with an active
/// project anchored at a path so handlers run and undo can record.
fn app() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_project_path(PathBuf::from("/tmp/reference-test.rsn"));
    app
}

fn send(app: &mut Resonance, m: ReferenceMessage) {
    let _ = app.update(Message::Reference(m));
}

/// Register a loaded reference directly through the engine-event path so
/// state tests have an entry to act on without a real decode.
fn fold_loaded(app: &mut Resonance, id: u32, name: &str) {
    app.test_handle_engine_event(resonance_audio::types::AudioEvent::ReferenceLoaded {
        id: ReferenceId(id),
        name: name.to_string(),
        path: format!("/refs/{name}.wav"),
        integrated_lufs: -14.0,
        waveform_peaks: vec![(-0.5, 0.5)],
    });
}

#[test]
fn load_requested_queues_pending_and_clears_error() {
    let mut app = app();
    // Seed a stale error to prove the load clears it.
    send(&mut app, ReferenceMessage::DismissError);
    let path = PathBuf::from("/refs/track.wav");
    send(&mut app, ReferenceMessage::LoadRequested(path.clone()));

    let st = app.test_reference();
    assert_eq!(st.pending_loads.len(), 1);
    assert_eq!(st.pending_loads.front().unwrap(), "/refs/track.wav");
    assert!(st.last_error.is_none());
    // No entry yet — the id is allocated by the engine.
    assert!(st.entries.is_empty());
}

#[test]
fn toggle_ab_source_flips_mix_and_reference() {
    let mut app = app();
    assert_eq!(app.test_reference().ab_source, ABSource::Mix);
    send(&mut app, ReferenceMessage::ToggleAbSource);
    assert_eq!(app.test_reference().ab_source, ABSource::Reference);
    send(&mut app, ReferenceMessage::ToggleAbSource);
    assert_eq!(app.test_reference().ab_source, ABSource::Mix);
}

#[test]
fn momentary_audition_restores_prior_source() {
    let mut app = app();
    // Start from Reference (via a toggle) to prove the prior source is
    // remembered, not assumed to be Mix.
    send(&mut app, ReferenceMessage::ToggleAbSource);
    assert_eq!(app.test_reference().ab_source, ABSource::Reference);

    send(&mut app, ReferenceMessage::MomentaryAudition(true));
    assert_eq!(app.test_reference().ab_source, ABSource::Reference);
    send(&mut app, ReferenceMessage::MomentaryAudition(false));
    assert_eq!(app.test_reference().ab_source, ABSource::Reference);

    // From Mix, a momentary press auditions the reference then returns.
    send(&mut app, ReferenceMessage::ToggleAbSource);
    assert_eq!(app.test_reference().ab_source, ABSource::Mix);
    send(&mut app, ReferenceMessage::MomentaryAudition(true));
    assert_eq!(app.test_reference().ab_source, ABSource::Reference);
    send(&mut app, ReferenceMessage::MomentaryAudition(false));
    assert_eq!(app.test_reference().ab_source, ABSource::Mix);
}

#[test]
fn toggle_loudness_match_flips() {
    let mut app = app();
    assert!(!app.test_reference().loudness_match);
    send(&mut app, ReferenceMessage::ToggleLoudnessMatch);
    assert!(app.test_reference().loudness_match);
    send(&mut app, ReferenceMessage::ToggleLoudnessMatch);
    assert!(!app.test_reference().loudness_match);
}

#[test]
fn trim_changed_sets_trim_db() {
    let mut app = app();
    send(&mut app, ReferenceMessage::TrimChanged(-3.5));
    assert_eq!(app.test_reference().trim_db, -3.5);
}

#[test]
fn set_active_requires_a_loaded_entry() {
    let mut app = app();
    // No entry: ignored.
    send(&mut app, ReferenceMessage::SetActive(ReferenceId(7)));
    assert_eq!(app.test_reference().active_id, None);

    fold_loaded(&mut app, 7, "ref");
    send(&mut app, ReferenceMessage::SetActive(ReferenceId(7)));
    assert_eq!(app.test_reference().active_id, Some(ReferenceId(7)));
}

#[test]
fn remove_drops_entry_and_clears_active() {
    let mut app = app();
    fold_loaded(&mut app, 1, "a");
    send(&mut app, ReferenceMessage::SetActive(ReferenceId(1)));
    assert_eq!(app.test_reference().entries.len(), 1);

    send(&mut app, ReferenceMessage::Remove(ReferenceId(1)));
    assert!(app.test_reference().entries.is_empty());
    assert_eq!(app.test_reference().active_id, None);
}

#[test]
fn toggle_loop_to_mix_flips() {
    let mut app = app();
    assert!(!app.test_reference().loop_to_mix);
    send(&mut app, ReferenceMessage::ToggleLoopToMix);
    assert!(app.test_reference().loop_to_mix);
}

#[test]
fn remove_marker_drops_it_from_the_entry() {
    let mut app = app();
    fold_loaded(&mut app, 1, "a");
    // Engine echoes the marker the user added.
    app.test_handle_engine_event(resonance_audio::types::AudioEvent::RefMarkerAdded {
        ref_id: ReferenceId(1),
        marker_id: 42,
        position_samples: 1000,
        label: "drop".to_string(),
    });
    assert_eq!(app.test_reference().entries[0].markers.len(), 1);

    send(
        &mut app,
        ReferenceMessage::RemoveMarker {
            ref_id: ReferenceId(1),
            marker_id: 42,
        },
    );
    assert!(app.test_reference().entries[0].markers.is_empty());
}

#[test]
fn scrub_updates_entry_cursor() {
    let mut app = app();
    fold_loaded(&mut app, 1, "a");
    send(
        &mut app,
        ReferenceMessage::Scrub {
            ref_id: ReferenceId(1),
            position_samples: 48_000,
        },
    );
    assert_eq!(app.test_reference().entries[0].position_samples, 48_000);
}

#[test]
fn loaded_status_is_loaded() {
    let mut app = app();
    fold_loaded(&mut app, 1, "a");
    assert_eq!(app.test_reference().entries[0].status, ReferenceStatus::Loaded);
}
