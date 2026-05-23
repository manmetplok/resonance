//! Regression test for the `Track::plugin_chain` ArcSwap refactor.
//!
//! Before this change `Track::plugin_ids` was a raw `Vec` that the UI
//! thread mutated under `tracks.write()`. The audio thread, holding
//! `tracks.read()`, would block on that write — every chain edit
//! caused a callback stall. After the refactor mutations publish via
//! `ArcSwap::store`, so the audio thread's `tracks.read()` + `plugins()`
//! pair is wait-free.
//!
//! These tests pin the new semantics:
//!   - basic add / remove / clear / replace round-trips,
//!   - `plugins()` returns a coherent snapshot that doesn't see a
//!     concurrent mutation tearing the chain,
//!   - mutators only need `&self`, so they compose with a read guard
//!     on the surrounding tracks map (which is what audio relies on).
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use indexmap::IndexMap;
use parking_lot::RwLock;
use resonance_audio::{Track, TrackId};

#[test]
fn push_plugin_appends_in_order() {
    let track = Track::new(1, "T1".to_string());
    assert!(track.plugins().is_empty());
    track.push_plugin(10);
    track.push_plugin(20);
    track.push_plugin(30);
    let plugins = track.plugins();
    assert_eq!(plugins.as_slice(), &[10, 20, 30]);
}

#[test]
fn retain_plugins_drops_matching_ids() {
    let track = Track::new(1, "T1".to_string());
    track.push_plugin(10);
    track.push_plugin(20);
    track.push_plugin(30);
    track.retain_plugins(|&id| id != 20);
    let plugins = track.plugins();
    assert_eq!(plugins.as_slice(), &[10, 30]);
}

#[test]
fn clear_plugins_empties_chain() {
    let track = Track::new(1, "T1".to_string());
    track.push_plugin(10);
    track.push_plugin(20);
    track.clear_plugins();
    assert!(track.plugins().is_empty());
}

#[test]
fn set_plugin_chain_replaces_wholesale() {
    let track = Track::new(1, "T1".to_string());
    track.push_plugin(10);
    track.set_plugin_chain(vec![99, 98, 97]);
    let plugins = track.plugins();
    assert_eq!(plugins.as_slice(), &[99, 98, 97]);
}

#[test]
fn plugin_chain_snapshot_outlives_subsequent_mutation() {
    // The snapshot is an Arc, so mutating after the snapshot is taken
    // doesn't change the snapshot's contents. This is the property the
    // engine relies on when it captures the chain to drain plugin
    // instances outside the tracks lock.
    let track = Track::new(1, "T1".to_string());
    track.push_plugin(1);
    track.push_plugin(2);
    let snap = track.plugin_chain_snapshot();
    track.clear_plugins();
    track.push_plugin(99);
    assert_eq!(snap.as_slice(), &[1, 2]);
    assert_eq!(track.plugins().as_slice(), &[99]);
}

/// The whole point of the refactor: mutating the chain only needs a
/// read guard on the enclosing tracks map. This compiled before the
/// refactor only if the test grabbed a `write()` guard; now it works
/// off a `read()`.
#[test]
fn mutation_works_through_read_guard() {
    let tracks: RwLock<IndexMap<TrackId, Track>> = RwLock::new(IndexMap::new());
    tracks.write().insert(1, Track::new(1, "T1".to_string()));
    {
        // Read guard intentionally held over the mutation — this is
        // what the audio thread does while mixing.
        let guard = tracks.read();
        let track = guard.get(&1).unwrap();
        track.push_plugin(7);
        track.push_plugin(8);
        track.retain_plugins(|&id| id != 7);
        assert_eq!(track.plugins().as_slice(), &[8]);
    }
}

/// Stress test: writer thread spams chain mutations while a reader
/// thread (simulating the audio callback) loads the chain in a tight
/// loop. The reader must never observe a torn chain — every snapshot
/// is either the pre-edit or post-edit value, with elements all from
/// the same "generation".
///
/// The writer encodes its generation count into the chain: chain[i] ==
/// `gen * 100 + i`. The reader asserts every element of every snapshot
/// matches the same `gen`. This would fail if `ArcSwap` somehow leaked
/// a half-written Vec, or if a mutation observed a load-in-progress.
#[test]
fn concurrent_load_never_tears_chain() {
    let track = Arc::new(Track::new(1, "T1".to_string()));
    track.set_plugin_chain(vec![0, 1, 2, 3]);

    let stop = Arc::new(AtomicBool::new(false));

    let writer_track = Arc::clone(&track);
    let writer_stop = Arc::clone(&stop);
    let writer = thread::spawn(move || {
        let mut gen: u64 = 1;
        let start = Instant::now();
        while !writer_stop.load(Ordering::Relaxed)
            && start.elapsed() < Duration::from_millis(200)
        {
            let len = 1 + (gen as usize % 8);
            let chain: Vec<_> = (0..len as u64).map(|i| gen * 100 + i).collect();
            writer_track.set_plugin_chain(chain);
            gen += 1;
        }
        gen
    });

    let reader_track = Arc::clone(&track);
    let reader_stop = Arc::clone(&stop);
    let reader = thread::spawn(move || {
        let mut snapshots: u64 = 0;
        let start = Instant::now();
        while !reader_stop.load(Ordering::Relaxed)
            && start.elapsed() < Duration::from_millis(200)
        {
            let plugins = reader_track.plugins();
            if let Some(&first) = plugins.first() {
                // Recover the writer's `gen` from the first element.
                let gen = first / 100;
                let offset = first % 100;
                // Each element must equal `gen * 100 + i` for i in 0..len.
                // Tearing would show up as a mismatched gen or
                // out-of-order offset.
                for (i, &v) in plugins.iter().enumerate() {
                    assert_eq!(
                        v,
                        gen * 100 + offset + i as u64,
                        "torn chain: gen={gen} offset={offset} i={i} v={v} chain={plugins:?}"
                    );
                }
            }
            snapshots += 1;
        }
        snapshots
    });

    let _gen_count = writer.join().unwrap();
    let _snap_count = reader.join().unwrap();
    stop.store(true, Ordering::Relaxed);
}
