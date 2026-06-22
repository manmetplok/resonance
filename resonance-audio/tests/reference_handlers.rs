//! Tests for the reference-track (A/B) command boundary (todo #674).
//!
//! Drives each `AudioCommand::*Reference*` / `*Ref*` / `*AB*` handler
//! directly against a bare `ReferencePlayer` via the `#[doc(hidden)]`
//! test re-exports. That keeps the test headless — no cpal stream, no
//! engine thread, no audio device — while exercising the exact mutation
//! + event emission the dispatch path runs for every command variant.

use std::path::PathBuf;

use crossbeam_channel::{unbounded, Receiver};

use resonance_audio::types::{ABSource, AudioEvent, ReferenceId};
use resonance_audio::{
    handle_add_ref_marker, handle_poll_ab_meters, handle_remove_ref_marker,
    handle_remove_reference_track, handle_set_ab_source, handle_set_active_reference,
    handle_set_ref_loop_to_mix, handle_set_ref_loudness_match, handle_set_ref_position,
    handle_set_ref_trim, register_reference, ReferencePlayer,
};

/// Drain the single event the handler under test just emitted.
fn next_event(rx: &Receiver<AudioEvent>) -> AudioEvent {
    rx.try_recv().expect("handler should emit exactly one event")
}

#[test]
fn register_reference_allocates_monotonic_ids() {
    let mut player = ReferencePlayer::new();

    // Registration is a pure mutation — it allocates the id and pushes
    // the (unanalysed) entry without emitting any event; the analysis
    // worker emits `ReferenceLoaded` once decode + LUFS measurement land.
    let first = register_reference(&mut player, None, PathBuf::from("/music/ref_master.wav"));
    assert_eq!(first, ReferenceId(1));

    let second = register_reference(&mut player, None, PathBuf::from("/music/other.flac"));
    assert_eq!(second, ReferenceId(2));
}

#[test]
fn register_reference_honours_id_hint_and_bumps_allocator() {
    let mut player = ReferencePlayer::new();

    assert_eq!(
        register_reference(&mut player, Some(ReferenceId(10)), PathBuf::from("/a.wav")),
        ReferenceId(10)
    );

    // A fresh (un-hinted) registration must skip past the hinted id.
    assert_eq!(
        register_reference(&mut player, None, PathBuf::from("/b.wav")),
        ReferenceId(11)
    );
}

#[test]
fn remove_reference_clears_active_and_emits() {
    let mut player = ReferencePlayer::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    register_reference(&mut player, Some(ReferenceId(1)), PathBuf::from("/a.wav"));
    handle_set_active_reference(&mut player, &tx, ReferenceId(1));
    assert!(matches!(next_event(&rx), AudioEvent::ActiveReferenceChanged { id } if id == ReferenceId(1)));

    handle_remove_reference_track(&mut player, &tx, ReferenceId(1));
    assert!(matches!(next_event(&rx), AudioEvent::ReferenceRemoved { id } if id == ReferenceId(1)));

    // Removing an unknown id is a silent no-op (no event).
    handle_remove_reference_track(&mut player, &tx, ReferenceId(99));
    assert!(rx.try_recv().is_err());
}

#[test]
fn set_active_reference_requires_existing() {
    let mut player = ReferencePlayer::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    // No reference loaded yet → no-op.
    handle_set_active_reference(&mut player, &tx, ReferenceId(1));
    assert!(rx.try_recv().is_err());

    register_reference(&mut player, Some(ReferenceId(1)), PathBuf::from("/a.wav"));
    handle_set_active_reference(&mut player, &tx, ReferenceId(1));
    assert!(matches!(next_event(&rx), AudioEvent::ActiveReferenceChanged { id } if id == ReferenceId(1)));
}

#[test]
fn set_ab_source_toggles_and_emits() {
    let mut player = ReferencePlayer::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    handle_set_ab_source(&mut player, &tx, ABSource::Reference);
    assert!(matches!(next_event(&rx), AudioEvent::ABSourceChanged { source } if source == ABSource::Reference));

    handle_set_ab_source(&mut player, &tx, ABSource::Mix);
    assert!(matches!(next_event(&rx), AudioEvent::ABSourceChanged { source } if source == ABSource::Mix));
}

#[test]
fn loudness_match_reports_active_offset() {
    let mut player = ReferencePlayer::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    // With no active reference the offset is reported as 0.
    handle_set_ref_loudness_match(&mut player, &tx, true);
    match next_event(&rx) {
        AudioEvent::RefLoudnessMatchChanged { enabled, offset_db } => {
            assert!(enabled);
            assert_eq!(offset_db, 0.0);
        }
        other => panic!("expected RefLoudnessMatchChanged, got {other:?}"),
    }

    handle_set_ref_loudness_match(&mut player, &tx, false);
    assert!(matches!(
        next_event(&rx),
        AudioEvent::RefLoudnessMatchChanged { enabled: false, .. }
    ));
}

#[test]
fn set_ref_trim_emits_value() {
    let mut player = ReferencePlayer::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    handle_set_ref_trim(&mut player, &tx, -3.5);
    assert!(matches!(next_event(&rx), AudioEvent::RefTrimChanged { db } if db == -3.5));
}

#[test]
fn add_and_remove_markers() {
    let mut player = ReferencePlayer::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    register_reference(&mut player, Some(ReferenceId(1)), PathBuf::from("/a.wav"));

    handle_add_ref_marker(&mut player, &tx, ReferenceId(1), 48_000, "drop".into());
    let first_marker = match next_event(&rx) {
        AudioEvent::RefMarkerAdded {
            ref_id,
            marker_id,
            position_samples,
            label,
        } => {
            assert_eq!(ref_id, ReferenceId(1));
            assert_eq!(position_samples, 48_000);
            assert_eq!(label, "drop");
            marker_id
        }
        other => panic!("expected RefMarkerAdded, got {other:?}"),
    };

    // Marker ids are monotonic per reference.
    handle_add_ref_marker(&mut player, &tx, ReferenceId(1), 96_000, "chorus".into());
    let second_marker = match next_event(&rx) {
        AudioEvent::RefMarkerAdded { marker_id, .. } => marker_id,
        other => panic!("expected RefMarkerAdded, got {other:?}"),
    };
    assert_ne!(first_marker, second_marker);

    // Adding to an unknown reference is a no-op.
    handle_add_ref_marker(&mut player, &tx, ReferenceId(42), 0, "x".into());
    assert!(rx.try_recv().is_err());

    handle_remove_ref_marker(&mut player, &tx, ReferenceId(1), first_marker);
    assert!(matches!(
        next_event(&rx),
        AudioEvent::RefMarkerRemoved { ref_id, marker_id }
            if ref_id == ReferenceId(1) && marker_id == first_marker
    ));

    // Removing a stale marker id is a no-op.
    handle_remove_ref_marker(&mut player, &tx, ReferenceId(1), first_marker);
    assert!(rx.try_recv().is_err());
}

#[test]
fn set_ref_position_seeks_cursor() {
    let mut player = ReferencePlayer::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    register_reference(&mut player, Some(ReferenceId(1)), PathBuf::from("/a.wav"));

    handle_set_ref_position(&mut player, &tx, ReferenceId(1), 123_456);
    assert!(matches!(
        next_event(&rx),
        AudioEvent::RefPositionChanged { ref_id, position_samples }
            if ref_id == ReferenceId(1) && position_samples == 123_456
    ));

    // Unknown reference → no-op.
    handle_set_ref_position(&mut player, &tx, ReferenceId(7), 0);
    assert!(rx.try_recv().is_err());
}

#[test]
fn set_ref_loop_to_mix_emits() {
    let mut player = ReferencePlayer::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    handle_set_ref_loop_to_mix(&mut player, &tx, true);
    assert!(matches!(next_event(&rx), AudioEvent::RefLoopToMixChanged { enabled: true }));

    handle_set_ref_loop_to_mix(&mut player, &tx, false);
    assert!(matches!(next_event(&rx), AudioEvent::RefLoopToMixChanged { enabled: false }));
}

#[test]
fn poll_ab_meters_snapshot_reflects_active_reference() {
    let mut player = ReferencePlayer::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    // No active reference → reference meter is None.
    handle_poll_ab_meters(&player, &tx);
    match next_event(&rx) {
        AudioEvent::ABMeterSnapshot { reference, .. } => assert!(reference.is_none()),
        other => panic!("expected ABMeterSnapshot, got {other:?}"),
    }

    register_reference(&mut player, Some(ReferenceId(1)), PathBuf::from("/a.wav"));
    handle_set_active_reference(&mut player, &tx, ReferenceId(1));
    let _ = next_event(&rx);

    // Active reference → reference meter is present.
    handle_poll_ab_meters(&player, &tx);
    match next_event(&rx) {
        AudioEvent::ABMeterSnapshot { reference, .. } => assert!(reference.is_some()),
        other => panic!("expected ABMeterSnapshot, got {other:?}"),
    }
}
