//! Tests for the reference-track (A/B) command boundary (todo #674).
//!
//! Drives each `AudioCommand::*Reference*` / `*Ref*` / `*AB*` handler
//! directly against a bare `ReferencePlayer` via the `#[doc(hidden)]`
//! test re-exports. That keeps the test headless — no cpal stream, no
//! engine thread, no audio device — while exercising the exact mutation
//! + event emission the dispatch path runs for every command variant.

use std::path::PathBuf;

use crossbeam_channel::{unbounded, Receiver};

use resonance_audio::types::{ABSource, AudioEvent, EngineErrorKind, ReferenceId};
use resonance_audio::{
    handle_add_ref_marker, handle_clear_active_reference, handle_load_reference_track,
    handle_poll_ab_meters, handle_remove_ref_marker,
    handle_remove_reference_track, handle_set_ab_source, handle_set_active_reference,
    handle_set_ref_loop_to_mix, handle_set_ref_loudness_match, handle_set_ref_position,
    handle_set_ref_trim, register_reference, ABMeterTap, ReferencePlayer,
};
use resonance_metering::{LufsMeter, MeterSnapshot};

/// Drain the single event the handler under test just emitted.
fn next_event(rx: &Receiver<AudioEvent>) -> AudioEvent {
    rx.try_recv().expect("handler should emit exactly one event")
}

#[test]
fn register_reference_registers_under_the_given_id() {
    let mut player = ReferencePlayer::new();

    // Registration is a pure mutation — it stores the (unanalysed) entry
    // under the caller-supplied id without emitting any event; the
    // analysis worker emits `ReferenceLoaded` once decode + LUFS
    // measurement land. ARCH-04 D-5: the engine has no allocator of its
    // own left, so the id is mandatory and never invented here.
    let first = register_reference(&mut player, ReferenceId(1), PathBuf::from("/music/ref_master.wav"));
    assert_eq!(first, ReferenceId(1));

    let second = register_reference(&mut player, ReferenceId(2), PathBuf::from("/music/other.flac"));
    assert_eq!(second, ReferenceId(2));
    assert_eq!(player.entry_has_pcm(ReferenceId(1)), Some(false));
    assert_eq!(player.entry_has_pcm(ReferenceId(2)), Some(false));
}

/// ARCH-04 D-5: a second `LoadReferenceTrack` for an id already live is
/// refused (`EngineErrorKind::Internal`), not honoured as a rename/replace
/// — the reference twin of `tests/clap_host/plugin_id_duplicate_rejected.rs`
/// and `tests/engine/bus_id_duplicate_rejected.rs`. Drives the real
/// `handle_load_reference_track` handler (not just `register_reference`),
/// so what's proven is that the refused load never reaches the worker-spawn
/// path either.
#[test]
fn a_duplicate_id_is_refused_and_does_not_replace_the_live_reference() {
    let mut player = ReferencePlayer::new();
    let (event_tx, event_rx) = unbounded::<AudioEvent>();
    let (cmd_tx, _cmd_rx) = unbounded();

    handle_load_reference_track(
        &mut player,
        &event_tx,
        &cmd_tx,
        48_000,
        ReferenceId(1),
        PathBuf::from("/a.wav"),
    );
    assert_eq!(player.entry_has_pcm(ReferenceId(1)), Some(false));

    // A second load asking for the SAME id is refused outright.
    handle_load_reference_track(
        &mut player,
        &event_tx,
        &cmd_tx,
        48_000,
        ReferenceId(1),
        PathBuf::from("/b.wav"),
    );
    let err = event_rx
        .try_recv()
        .expect("the refused load reports an error");
    match err {
        AudioEvent::Error(e) => assert_eq!(
            e.kind,
            EngineErrorKind::Internal,
            "a duplicate id is a caller invariant violation, not a transient Busy condition"
        ),
        other => panic!("expected AudioEvent::Error, got {other:?}"),
    }
    assert!(
        event_rx.try_recv().is_err(),
        "no further event from the refused load"
    );
    // Still exactly the one entry — the refused load did not overwrite it.
    assert_eq!(player.entry_count(), 1);
}

#[test]
fn remove_reference_clears_active_and_emits() {
    let mut player = ReferencePlayer::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    register_reference(&mut player, ReferenceId(1), PathBuf::from("/a.wav"));
    handle_set_active_reference(&mut player, &tx, ReferenceId(1));
    assert!(matches!(next_event(&rx), AudioEvent::ActiveReferenceChanged { id } if id == ReferenceId(1)));

    handle_remove_reference_track(&mut player, &tx, ReferenceId(1));
    assert!(matches!(next_event(&rx), AudioEvent::ReferenceRemoved { id } if id == ReferenceId(1)));

    // Removing an unknown id is a silent no-op (no event).
    handle_remove_reference_track(&mut player, &tx, ReferenceId(99));
    assert!(rx.try_recv().is_err());
}

#[test]
fn clear_drops_every_entry() {
    let mut player = ReferencePlayer::new();
    register_reference(&mut player, ReferenceId(5), PathBuf::from("/a.wav"));
    register_reference(&mut player, ReferenceId(6), PathBuf::from("/b.wav"));
    assert_eq!(player.entry_has_pcm(ReferenceId(5)), Some(false));

    player.clear();

    // Entries are gone; id bookkeeping (ARCH-04 D-5) lives entirely on the
    // app side now, so there is no engine-side allocator left to reset.
    assert_eq!(player.entry_has_pcm(ReferenceId(5)), None);
    assert_eq!(player.entry_has_pcm(ReferenceId(6)), None);

    // A ClearAll'd player accepts a fresh registration under any id the
    // app hands it — including one it had already seen before the clear.
    assert_eq!(
        register_reference(&mut player, ReferenceId(5), PathBuf::from("/c.wav")),
        ReferenceId(5)
    );
}

#[test]
fn set_active_reference_requires_existing() {
    let mut player = ReferencePlayer::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    // No reference loaded yet → no-op.
    handle_set_active_reference(&mut player, &tx, ReferenceId(1));
    assert!(rx.try_recv().is_err());

    register_reference(&mut player, ReferenceId(1), PathBuf::from("/a.wav"));
    handle_set_active_reference(&mut player, &tx, ReferenceId(1));
    assert!(matches!(next_event(&rx), AudioEvent::ActiveReferenceChanged { id } if id == ReferenceId(1)));
}

/// `ClearActiveReference` (an undo back to "nothing selected") deselects
/// silently and keeps the entry loaded.
#[test]
fn clear_active_reference_deselects_silently() {
    let mut player = ReferencePlayer::new();
    let (tx, rx) = unbounded::<AudioEvent>();
    register_reference(&mut player, ReferenceId(1), PathBuf::from("/a.wav"));
    handle_set_active_reference(&mut player, &tx, ReferenceId(1));
    let _ = next_event(&rx);

    handle_clear_active_reference(&mut player);
    assert!(rx.try_recv().is_err(), "no echo");
    handle_poll_ab_meters(&player, MeterSnapshot::default(), MeterSnapshot::default(), &tx);
    match next_event(&rx) {
        AudioEvent::ABMeterSnapshot { reference, .. } => {
            assert!(reference.is_none(), "nothing is active any more")
        }
        other => panic!("expected ABMeterSnapshot, got {other:?}"),
    }
    assert_eq!(player.entry_has_pcm(ReferenceId(1)), Some(false), "entry kept");
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

    register_reference(&mut player, ReferenceId(1), PathBuf::from("/a.wav"));

    // The app allocates marker ids (FU-A5a): the engine keeps the one it
    // is handed — it has no allocator that restarts at 1 under a
    // reference whose saved markers only the app restored.
    handle_add_ref_marker(&mut player, &tx, ReferenceId(1), 7, 48_000, "drop".into());
    let first_marker = match next_event(&rx) {
        AudioEvent::RefMarkerAdded {
            ref_id,
            marker_id,
            position_samples,
            label,
        } => {
            assert_eq!(ref_id, ReferenceId(1));
            assert_eq!(marker_id, 7, "the app's id is kept");
            assert_eq!(position_samples, 48_000);
            assert_eq!(label, "drop");
            marker_id
        }
        other => panic!("expected RefMarkerAdded, got {other:?}"),
    };

    handle_add_ref_marker(&mut player, &tx, ReferenceId(1), 3, 96_000, "chorus".into());
    assert!(matches!(
        next_event(&rx),
        AudioEvent::RefMarkerAdded { marker_id: 3, .. }
    ));

    // Re-adding a held id moves that marker instead of duplicating it.
    handle_add_ref_marker(&mut player, &tx, ReferenceId(1), 3, 12_000, "chorus".into());
    assert!(matches!(
        next_event(&rx),
        AudioEvent::RefMarkerAdded { marker_id: 3, position_samples: 12_000, .. }
    ));
    handle_remove_ref_marker(&mut player, &tx, ReferenceId(1), 3);
    assert!(matches!(next_event(&rx), AudioEvent::RefMarkerRemoved { marker_id: 3, .. }));
    handle_remove_ref_marker(&mut player, &tx, ReferenceId(1), 3);
    assert!(rx.try_recv().is_err(), "no second marker 3 was left behind");

    // Adding to an unknown reference is a no-op.
    handle_add_ref_marker(&mut player, &tx, ReferenceId(42), 1, 0, "x".into());
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

    register_reference(&mut player, ReferenceId(1), PathBuf::from("/a.wav"));

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

/// Build `frames` of interleaved stereo PCM from a per-channel generator.
fn interleaved_stereo(frames: usize, mut gen: impl FnMut(usize) -> (f32, f32)) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let (l, r) = gen(i);
        out.push(l);
        out.push(r);
    }
    out
}

#[test]
fn poll_ab_meters_snapshot_reflects_active_reference() {
    let mut player = ReferencePlayer::new();
    let (tx, rx) = unbounded::<AudioEvent>();

    let mix = MeterSnapshot {
        integrated_lufs: -14.0,
        ..MeterSnapshot::default()
    };
    let ref_snap = MeterSnapshot {
        integrated_lufs: -9.0,
        ..MeterSnapshot::default()
    };

    // No active reference → reference meter is None, mix always present.
    handle_poll_ab_meters(&player, mix, ref_snap, &tx);
    match next_event(&rx) {
        AudioEvent::ABMeterSnapshot { mix: m, reference } => {
            assert!(reference.is_none());
            assert_eq!(m.integrated_lufs, -14.0);
        }
        other => panic!("expected ABMeterSnapshot, got {other:?}"),
    }

    register_reference(&mut player, ReferenceId(1), PathBuf::from("/a.wav"));
    handle_set_active_reference(&mut player, &tx, ReferenceId(1));
    let _ = next_event(&rx);

    // Active reference → reference meter is present and carries the
    // published reference snapshot verbatim (so the panel Delta is real).
    handle_poll_ab_meters(&player, mix, ref_snap, &tx);
    match next_event(&rx) {
        AudioEvent::ABMeterSnapshot { mix: m, reference } => {
            assert_eq!(m.integrated_lufs, -14.0);
            let r = reference.expect("active reference → Some snapshot");
            assert_eq!(r.integrated_lufs, -9.0);
        }
        other => panic!("expected ABMeterSnapshot, got {other:?}"),
    }
}

#[test]
fn ab_meter_tap_integrated_matches_offline_analysis() {
    // A 3 s 997 Hz tone at -0.5 amplitude, the canonical metering probe.
    let sr = 48_000u32;
    let frames = sr as usize * 3;
    let freq = 997.0_f32;
    let amp = 0.5_f32;
    let interleaved = interleaved_stereo(frames, |i| {
        let s = amp * (std::f32::consts::TAU * freq * i as f32 / sr as f32).sin();
        (s, s)
    });

    // Stream it through the tap one ~1024-frame block at a time, exactly
    // as the audio callback feeds it.
    let mut tap = ABMeterTap::new(sr as f32);
    for chunk in interleaved.chunks(1024 * 2) {
        let block_frames = chunk.len() / 2;
        tap.feed_interleaved(chunk, 2, block_frames);
    }
    let snap = tap.snapshot();

    // Offline analysis of the same signal (deinterleaved).
    let left: Vec<f32> = interleaved.iter().step_by(2).copied().collect();
    let right: Vec<f32> = interleaved.iter().skip(1).step_by(2).copied().collect();
    let offline = LufsMeter::analyze_offline(sr as f32, &left, &right);

    assert!(
        snap.integrated_lufs.is_finite(),
        "streaming integrated should be finite for a steady tone"
    );
    assert!(
        (snap.integrated_lufs - offline.integrated).abs() < 0.1,
        "streaming integrated {} should match offline {} within tolerance",
        snap.integrated_lufs,
        offline.integrated
    );
    // True peak of a -0.5 (≈ -6 dBFS) tone sits well above the floor.
    assert!(snap.true_peak_max_dbtp > -12.0 && snap.true_peak_max_dbtp < 0.0);
}

#[test]
fn ab_meter_tap_reset_clears_accumulators() {
    let sr = 48_000u32;
    let interleaved = interleaved_stereo(sr as usize, |_| (0.5, 0.5));

    let mut tap = ABMeterTap::new(sr as f32);
    tap.feed_interleaved(&interleaved, 2, sr as usize);
    assert!(tap.snapshot().integrated_lufs.is_finite());

    tap.reset();
    let cleared = tap.snapshot();
    assert_eq!(cleared.integrated_lufs, f32::NEG_INFINITY);
    assert_eq!(cleared.true_peak_max_dbtp, MeterSnapshot::default().true_peak_max_dbtp);
}

#[test]
fn ab_meter_tap_handles_mono_blocks() {
    // A single-channel block should be metered (L duplicated to R), not
    // panic on the stride math.
    let sr = 48_000u32;
    let mono: Vec<f32> = (0..sr as usize)
        .map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / sr as f32).sin())
        .collect();
    let mut tap = ABMeterTap::new(sr as f32);
    tap.feed_interleaved(&mono, 1, mono.len());
    assert!(tap.snapshot().integrated_lufs.is_finite());
}
