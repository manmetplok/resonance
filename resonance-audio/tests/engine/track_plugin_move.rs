//! Reordering a track's insert chain — `Track::move_plugin` behind
//! `AudioCommand::MovePlugin` (ba todo #1224, doc #273).
//!
//! Before this the chain could only be appended to and removed from, so
//! getting an EQ in front of a compressor meant tearing the chain down and
//! rebuilding it in the right order. Order is audible, so the engine needs a
//! first-class reorder.
//!
//! `move_plugin` is copy-on-write like the rest of the chain API (see
//! `track_plugin_chain.rs` for the tearing/read-guard properties it shares):
//! the reordered `Vec` is built off the audio thread and published with a
//! single `ArcSwap::store`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use indexmap::IndexMap;
use parking_lot::RwLock;
use resonance_audio::test_support::affects_latency;
use resonance_audio::types::{AudioCommand, ChainOwner, Track, TrackId};

fn track_with(ids: &[u64]) -> Track {
    let track = Track::new(1, "T1".to_string());
    track.set_plugin_chain(ids.to_vec());
    track
}

#[test]
fn moves_the_last_plugin_to_the_front() {
    let track = track_with(&[10, 20, 30]);
    assert_eq!(track.move_plugin(30, 0), Some(0));
    assert_eq!(track.plugins().as_slice(), &[30, 10, 20]);
}

#[test]
fn moves_the_first_plugin_to_the_end() {
    let track = track_with(&[10, 20, 30]);
    assert_eq!(track.move_plugin(10, 2), Some(2));
    assert_eq!(track.plugins().as_slice(), &[20, 30, 10]);
}

/// Everything between the old and new slot shifts by one; nothing else
/// moves and nothing is duplicated or dropped.
#[test]
fn shifts_only_the_plugins_between_the_two_slots() {
    let track = track_with(&[1, 2, 3, 4, 5]);
    assert_eq!(track.move_plugin(4, 1), Some(1));
    assert_eq!(track.plugins().as_slice(), &[1, 4, 2, 3, 5]);

    let track = track_with(&[1, 2, 3, 4, 5]);
    assert_eq!(track.move_plugin(2, 3), Some(3));
    assert_eq!(track.plugins().as_slice(), &[1, 3, 4, 2, 5]);
}

/// An out-of-range `to_index` clamps to the last slot instead of panicking
/// (`Vec::insert` would panic past `len`).
#[test]
fn out_of_range_index_clamps_to_the_last_slot() {
    for to_index in [3usize, 4, 99, usize::MAX] {
        let track = track_with(&[10, 20, 30]);
        assert_eq!(
            track.move_plugin(10, to_index),
            Some(2),
            "to_index {to_index} must clamp to the last slot",
        );
        assert_eq!(track.plugins().as_slice(), &[20, 30, 10]);
    }
}

/// Moving a plugin to the slot it already occupies is a no-op that still
/// reports success, so a caller can issue it unconditionally.
#[test]
fn no_op_move_leaves_the_chain_untouched() {
    let track = track_with(&[10, 20, 30]);
    assert_eq!(track.move_plugin(20, 1), Some(1));
    assert_eq!(track.plugins().as_slice(), &[10, 20, 30]);

    // Clamping can also turn an out-of-range move of the last plugin into a
    // no-op.
    assert_eq!(track.move_plugin(30, 7), Some(2));
    assert_eq!(track.plugins().as_slice(), &[10, 20, 30]);
}

/// An instance that is not on the chain leaves it untouched and reports
/// `None` — the handler turns that into an error event rather than a panic.
#[test]
fn unknown_instance_leaves_the_chain_untouched() {
    let track = track_with(&[10, 20, 30]);
    assert_eq!(track.move_plugin(99, 0), None);
    assert_eq!(track.plugins().as_slice(), &[10, 20, 30]);
}

#[test]
fn move_on_an_empty_chain_is_none() {
    let track = Track::new(1, "T1".to_string());
    assert_eq!(track.move_plugin(10, 0), None);
    assert!(track.plugins().is_empty());
}

#[test]
fn single_plugin_chain_clamps_to_its_only_slot() {
    let track = track_with(&[10]);
    assert_eq!(track.move_plugin(10, 5), Some(0));
    assert_eq!(track.plugins().as_slice(), &[10]);
}

/// Like the rest of the chain mutators, reordering only needs a *read*
/// guard on the enclosing tracks map — that is what lets the engine
/// handler run it without ever blocking the audio callback.
#[test]
fn move_works_through_a_read_guard() {
    let tracks: RwLock<IndexMap<TrackId, Track>> = RwLock::new(IndexMap::new());
    tracks.write().insert(1, Track::new(1, "T1".to_string()));
    {
        // Read guard held across the mutation, as the audio thread does.
        let guard = tracks.read();
        let track = guard.get(&1).unwrap();
        track.set_plugin_chain(vec![7, 8, 9]);
        assert_eq!(track.move_plugin(9, 0), Some(0));
        assert_eq!(track.plugins().as_slice(), &[9, 7, 8]);
    }
}

/// A snapshot taken before the move keeps the old order, which is what the
/// engine relies on when it hands a chain off to another scope.
#[test]
fn snapshot_taken_before_the_move_keeps_the_old_order() {
    let track = track_with(&[1, 2, 3]);
    let snap = track.plugin_chain_snapshot();
    track.move_plugin(3, 0);
    assert_eq!(snap.as_slice(), &[1, 2, 3]);
    assert_eq!(track.plugins().as_slice(), &[3, 1, 2]);
}

/// Reordering must refresh plugin-delay compensation. This is *not*
/// obvious — a chain's latency is the sum of its plugins, so a reorder
/// looks neutral — but on an instrument track the first plugin is the
/// instrument, whose latency is treated separately and is inherited by
/// every sub-track (`latency::chain_latencies`). Moving a plugin into or
/// out of slot 0 changes the comp table, so the engine loop has to rebuild
/// it after this command.
#[test]
fn move_plugin_refreshes_the_comp_table() {
    assert!(affects_latency(&AudioCommand::MovePlugin {
        owner: ChainOwner::Track(1),
        instance_id: 2,
        to_index: 0,
    }));
}

/// The reorder must never expose a torn chain to the audio thread. A
/// writer thread rotates a chain of known ids while a reader loads it in a
/// tight loop; every snapshot must be a permutation of the original set,
/// with no duplicates and no losses.
#[test]
fn concurrent_load_never_sees_a_torn_reorder() {
    const IDS: [u64; 6] = [1, 2, 3, 4, 5, 6];

    let track = Arc::new(Track::new(1, "T1".to_string()));
    track.set_plugin_chain(IDS.to_vec());

    let stop = Arc::new(AtomicBool::new(false));

    let writer_track = Arc::clone(&track);
    let writer_stop = Arc::clone(&stop);
    let writer = thread::spawn(move || {
        let start = Instant::now();
        let mut i = 0usize;
        while !writer_stop.load(Ordering::Relaxed) && start.elapsed() < Duration::from_millis(200)
        {
            // Rotate by repeatedly moving the current head to the tail.
            let head = writer_track.plugins()[0];
            writer_track.move_plugin(head, IDS.len() - 1);
            i += 1;
        }
        i
    });

    let reader_track = Arc::clone(&track);
    let reader_stop = Arc::clone(&stop);
    let reader = thread::spawn(move || {
        let start = Instant::now();
        while !reader_stop.load(Ordering::Relaxed) && start.elapsed() < Duration::from_millis(200)
        {
            let plugins = reader_track.plugins();
            let mut seen = plugins.to_vec();
            seen.sort_unstable();
            assert_eq!(
                seen.as_slice(),
                IDS.as_slice(),
                "torn reorder: chain={plugins:?}",
            );
        }
    });

    let _rotations = writer.join().unwrap();
    reader.join().unwrap();
    stop.store(true, Ordering::Relaxed);
}
