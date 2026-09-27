//! Tracks on the published render graph (code review ARCH-02 A2-6,
//! refactor todo B-3), driven through the real handlers.
//!
//! The track table used to sit behind an `Arc<RwLock<…>>` the callback
//! `try_read` and two bounce paths `write()` from wherever they ran; it now
//! lives in the immutable `RenderGraph` the engine thread republishes on
//! every structural edit. These tests pin what the swap relies on:
//!
//! - a removed track — and the frozen cache only it still holds — is
//!   freed by the engine thread's retire sweep, never by the reader that
//!   last looked at it;
//! - a track edit copy-on-writes only that track, and the copy shares the
//!   original's live state, so a meter write or a fader move on either
//!   copy is never lost; the per-track setters publish nothing at all;
//! - a bus removal and the re-route of its feeders are ONE published
//!   graph, so no reader ever sees a track routed to a bus that is gone;
//! - neither bounce-in-place cancel edits the graph off the engine
//!   thread: the offline worker posts `BounceTargetCancelled` back, the
//!   realtime path tears down in the engine loop's own poll.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use resonance_audio::test_support::{EngineHandlerHarness, RenderGraph};
use resonance_audio::types::*;
use resonance_common::{FreezeCacheRef, FreezeCacheStatus, PlaybackSource};

const DRUMS: BusId = 7;
const SCRATCH: BusId = 9;

fn two_tracks() -> EngineHandlerHarness {
    let mut h = EngineHandlerHarness::new();
    h.add_track(1, Some("one".into()));
    h.add_track(2, Some("two".into()));
    h.drain_events();
    h.sweep_retired();
    h
}

fn track(g: &RenderGraph, id: TrackId) -> Arc<Track> {
    Arc::clone(g.tracks.get(&id).expect("track is in the graph"))
}

fn frozen(frames: usize) -> FrozenSource {
    let cache_ref =
        FreezeCacheRef::new("b3.wav".into(), 48_000, 32, 1, FreezeCacheStatus::Frozen);
    FrozenSource::new(cache_ref, Arc::new(vec![0.25; frames * 2]), 48_000, frames as u64)
}

#[test]
fn a_removed_track_is_freed_by_the_engine_sweep_not_by_its_last_reader() {
    let mut h = two_tracks();
    h.set_track_frozen_source(1, Some(frozen(256)));
    let cache = Arc::downgrade(&h.frozen_source(1).expect("cache attached"));
    let shared = h.shared_arc();

    // The reader pins the current graph, as the callback does for a block.
    let (pinned_tx, pinned_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let reader = {
        let shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            let pinned = shared.graph.load_full();
            let one = Arc::downgrade(pinned.tracks.get(&1).unwrap());
            pinned_tx.send((Arc::downgrade(&pinned), one)).unwrap();
            release_rx.recv().unwrap();
            // The reader lets go on its own thread. It must not be the
            // last owner: the retire queue still holds the graph.
            drop(pinned);
        })
    };
    let (graph, one) = pinned_rx.recv().unwrap();

    // The real handler unpublishes the track while the reader holds it.
    h.remove_track(1);
    assert_eq!(h.test_track_ids(), vec![2]);
    h.sweep_retired();
    assert!(one.upgrade().is_some(), "the pinned graph survives a sweep");
    assert!(cache.upgrade().is_some(), "and so does the track's frozen cache");

    release_tx.send(()).unwrap();
    reader.join().unwrap();
    assert!(
        graph.upgrade().is_some() && one.upgrade().is_some() && cache.upgrade().is_some(),
        "the reader's drop freed neither the graph, the track nor its cache — the retire \
         queue owns them"
    );
    h.sweep_retired();
    assert!(graph.upgrade().is_none(), "the engine-thread sweep frees the graph");
    assert!(one.upgrade().is_none(), "and with it the removed track");
    assert!(cache.upgrade().is_none(), "and the cache only that track held");
    assert!(h.shared().retired.is_empty());
}

#[test]
fn removing_a_parent_takes_its_sub_tracks_in_one_graph() {
    let mut h = two_tracks();
    h.create_sub_track(10, 1, 1);
    h.create_sub_track(11, 1, 2);
    assert_eq!(h.test_track_ids(), vec![1, 2, 10, 11]);
    h.drain_events();
    h.sweep_retired();

    let before = h.shared().retired.len();
    h.remove_track(1);
    assert_eq!(h.test_track_ids(), vec![2], "insertion order of the rest is kept");
    assert_eq!(
        h.shared().retired.len(),
        before + 1,
        "parent and sub-tracks leave in one published graph"
    );
    let removed: Vec<TrackId> = h
        .drain_events()
        .into_iter()
        .filter_map(|e| match e {
            AudioEvent::TrackRemoved { track_id } => Some(track_id),
            _ => None,
        })
        .collect();
    assert_eq!(removed, vec![1, 10, 11]);
}

#[test]
fn a_track_edit_copies_only_that_track_and_its_copy_shares_the_live_state() {
    let mut h = two_tracks();
    h.add_bus(DRUMS, Some("Drums".into()));
    let before = h.render_graph();

    // A structural edit: routing is part of the graph since B-3.
    h.set_track_output(1, TrackOutput::Bus(DRUMS));
    let after = h.render_graph();
    assert!(!Arc::ptr_eq(&before, &after), "the edit published a new graph");
    assert!(
        Arc::ptr_eq(&track(&before, 2), &track(&after, 2)),
        "the untouched track is shared, not copied"
    );
    let (old, new) = (track(&before, 1), track(&after, 1));
    assert!(!Arc::ptr_eq(&old, &new), "the edited track was copied");
    assert_eq!(old.output(), TrackOutput::Master, "the old graph is immutable");
    assert_eq!(new.output(), TrackOutput::Bus(DRUMS));

    // The audio thread meters a block on the copy the old graph holds...
    old.update_peak_l(0.5);
    // ...and the engine's meter poll, reading the new graph, sees it.
    assert_eq!(new.swap_peak_l(), 0.5);
    // A fader move on the published copy is what a block still rendering
    // the old graph reads too.
    new.set_volume(0.25);
    assert_eq!(old.volume(), 0.25);
    old.set_last_gains(0.3, 0.4);
    assert_eq!(new.last_gains(), (0.3, 0.4), "the fader ramp carries across the copy");
}

#[test]
fn per_track_setters_write_through_without_publishing() {
    let mut h = two_tracks();
    let before = h.render_graph();
    let retired = h.shared().retired.len();

    for cmd in [
        AudioCommand::SetTrackVolume { track_id: 1, volume: 0.5 },
        AudioCommand::SetTrackPan { track_id: 1, pan: -0.25 },
        AudioCommand::SetTrackMute { track_id: 1, muted: true },
        AudioCommand::SetTrackSolo { track_id: 2, soloed: true },
        AudioCommand::SetTrackRecordArm { track_id: 1, armed: true },
        AudioCommand::SetTrackFxBypass { track_id: 1, bypassed: true },
        AudioCommand::SetTrackPlaybackSource {
            track_id: 1,
            source: PlaybackSource::Recorded,
        },
    ] {
        h.dispatch(cmd);
    }

    let after = h.render_graph();
    assert!(Arc::ptr_eq(&before, &after), "no setter published a graph");
    assert_eq!(h.shared().retired.len(), retired);
    let (one, two) = (track(&after, 1), track(&after, 2));
    assert_eq!(one.volume(), 0.5);
    assert_eq!(one.pan(), -0.25);
    assert!(one.muted());
    assert!(two.soloed());
    assert!(one.record_armed());
    assert!(one.fx_bypassed());
    assert_eq!(one.playback_source(), PlaybackSource::Recorded);
}

#[test]
fn removing_a_bus_reroutes_its_feeders_in_the_same_graph() {
    let mut h = two_tracks();
    h.add_bus(DRUMS, Some("Drums".into()));
    h.set_track_output(1, TrackOutput::Bus(DRUMS));
    h.sweep_retired();
    let before = h.render_graph();
    let retired = h.shared().retired.len();

    h.remove_bus(DRUMS);

    let after = h.render_graph();
    assert_eq!(
        h.shared().retired.len(),
        retired + 1,
        "the bus removal and the re-route are one publish"
    );
    assert!(before.bus(DRUMS).is_some());
    assert_eq!(track(&before, 1).output(), TrackOutput::Bus(DRUMS));
    assert!(after.bus(DRUMS).is_none());
    assert_eq!(track(&after, 1).output(), TrackOutput::Master);
}

/// Every track's bus exists in the SAME graph that routes it there.
fn routing_is_consistent(g: &RenderGraph) -> bool {
    g.tracks.values().all(|t| match t.output() {
        TrackOutput::Bus(b) => g.busses.contains_key(&b),
        TrackOutput::Master => true,
    })
}

#[test]
fn a_reader_never_sees_a_track_routed_to_a_bus_that_is_gone() {
    let mut h = two_tracks();
    let shared = h.shared_arc();
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let shared = Arc::clone(&shared);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let (mut loads, mut routed) = (0u64, 0u64);
            while !stop.load(Ordering::Acquire) {
                let g = shared.graph.load();
                assert!(routing_is_consistent(&g), "a graph routed a track to a missing bus");
                if g.tracks.values().any(|t| t.output() != TrackOutput::Master) {
                    routed += 1;
                }
                loads += 1;
            }
            (loads, routed)
        })
    };

    for round in 0..2_000u64 {
        h.add_bus(SCRATCH, None);
        h.set_track_output(1 + round % 2, TrackOutput::Bus(SCRATCH));
        h.remove_bus(SCRATCH);
        if round.is_multiple_of(64) {
            h.sweep_retired();
        }
    }
    stop.store(true, Ordering::Release);
    let (loads, routed) = reader.join().expect("reader");
    assert!(loads > 0 && routed > 0, "the reader raced the edits ({loads} loads, {routed} routed)");
    assert!(routing_is_consistent(&h.render_graph()));
    assert!(
        h.test_track_ids().iter().all(|&id| track(&h.render_graph(), id).output()
            == TrackOutput::Master),
        "every feeder of the removed bus went back to master"
    );
    h.sweep_retired();
    assert!(h.shared().retired.is_empty());
}

/// A source instrument track (1) with a MIDI clip to bounce, and the
/// freshly-added empty target track (2) a bounce-in-place renders into.
fn bounce_fixture() -> EngineHandlerHarness {
    let mut h = EngineHandlerHarness::new();
    h.dispatch(AudioCommand::AddInstrumentTrack { id: 1, name: None });
    h.add_track(2, Some("bounce target".into()));
    h.push_midi_clip(MidiClip {
        id: 50,
        track_id: 1,
        start_sample: 0,
        duration_ticks: 16 * TICKS_PER_QUARTER_NOTE,
        notes: vec![MidiNote {
            note: 60,
            velocity: 0.8,
            start_tick: 0,
            duration_ticks: TICKS_PER_QUARTER_NOTE,
        }],
        name: "src".into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    h.drain_events();
    h.sweep_retired();
    h
}

/// The teardown events, in order, with the rest (progress) filtered out.
fn teardown_events(events: Vec<AudioEvent>) -> Vec<AudioEvent> {
    events
        .into_iter()
        .filter(|e| {
            matches!(
                e,
                AudioEvent::TrackRemoved { .. } | AudioEvent::TrackBounceCancelled { .. }
            )
        })
        .collect()
}

fn assert_target_torn_down(events: Vec<AudioEvent>) {
    let events = teardown_events(events);
    assert!(
        matches!(
            events.as_slice(),
            [
                AudioEvent::TrackRemoved { track_id: 2 },
                AudioEvent::TrackBounceCancelled { target_track_id: 2 },
            ]
        ),
        "expected TrackRemoved then TrackBounceCancelled for the target, got {events:?}"
    );
}

#[test]
fn a_cancelled_offline_bounce_hands_the_target_removal_to_the_engine_thread() {
    let mut h = bounce_fixture();

    // Park the worker before its render, so the cancel is seen before a
    // single chunk renders.
    let (cancel, parked) = h.bounce_track_to_audio_parked(1, 2, 99);
    cancel.store(true, Ordering::Relaxed);
    drop(parked);

    let deadline = Instant::now() + Duration::from_secs(10);
    let posted = loop {
        let cmds = h.take_retry_commands();
        if !cmds.is_empty() {
            break cmds;
        }
        assert!(Instant::now() < deadline, "the worker never reported its cancel");
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(
        matches!(
            posted.as_slice(),
            [AudioCommand::BounceTargetCancelled { target_track_id: 2 }]
        ),
        "the worker posts the teardown back, got {posted:?}"
    );
    // The worker itself edited nothing: the target is still published
    // until the engine thread runs the command.
    assert_eq!(h.test_track_ids(), vec![1, 2]);
    assert!(teardown_events(h.drain_events()).is_empty());

    for cmd in posted {
        h.dispatch(cmd);
    }
    assert_eq!(h.test_track_ids(), vec![1]);
    assert_target_torn_down(h.drain_events());
    assert!(h.clip_ids().is_empty(), "the bounce never pushed its clip");
}

#[test]
fn a_cancelled_realtime_bounce_removes_its_target_in_the_engine_poll() {
    let mut h = bounce_fixture();
    let before = h.render_graph();
    let target = Arc::downgrade(before.tracks.get(&2).unwrap());
    drop(before);

    h.cancel_pending_realtime_bounce(1, 2);

    assert_eq!(h.test_track_ids(), vec![1]);
    assert_target_torn_down(h.drain_events());
    assert!(target.upgrade().is_some(), "the removed target rides out on the retired graph");
    h.sweep_retired();
    assert!(target.upgrade().is_none(), "and the engine-thread sweep frees it");
}
