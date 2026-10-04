//! Raising the offline-render gate waits for a callback already in its
//! block (code review RT-09).
//!
//! The callback checks the gate once, at the top of the block. A block
//! that passed it an instant before an export raised it kept processing
//! the live plugin instances while the render worker reset and drove the
//! same ones — the export's first chunk could carry live state, or a
//! reset could race a live `process()`. `OfflineRenderGuard::mark` now
//! returns only once such a block has left.
//!
//! The hook: the note recorder's `process()` takes its record's mutex, so
//! holding that mutex from the test parks the callback mid-block, inside
//! the instrument.

use crate::note_recorder;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use note_recorder::note_recorder;
use resonance_audio::test_support::{MixAudioHarness, OfflineRenderGuard};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const INSTRUMENT: PluginInstanceId = 320;

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn raising_the_gate_waits_for_the_block_in_flight() {
    let mut track = Track::with_type(1, "Synth".into(), TrackType::Instrument);
    track.set_output(TrackOutput::Master);
    track.push_plugin(INSTRUMENT);
    let mut h = MixAudioHarness::new(
        vec![track],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    let (slot, rec) = note_recorder(SR);
    h.edit_plugins(|p| p.insert(INSTRUMENT, Arc::new(slot)));
    h.shared().playing.store(true, Ordering::Relaxed);
    let shared = h.shared_arc();
    assert!(!shared.callback_activity.in_flight());

    // Park the next block inside the instrument's process().
    let hold = rec.lock();
    let callback = std::thread::spawn(move || {
        h.render();
        h
    });
    wait_until("the callback to enter its block", || {
        shared.callback_activity.in_flight()
    });

    // The export raises the gate while that block is still rendering.
    let marked = Arc::new(AtomicBool::new(false));
    let exporter = {
        let shared = Arc::clone(&shared);
        let marked = Arc::clone(&marked);
        std::thread::spawn(move || {
            let guard = OfflineRenderGuard::mark(&shared);
            marked.store(true, Ordering::SeqCst);
            guard
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        !marked.load(Ordering::SeqCst),
        "the render may not take the plugins while a live block still holds them"
    );
    assert!(shared.offline_render_active(), "the gate is already up for new blocks");

    // The block finishes; the render goes ahead.
    drop(hold);
    let mut h = callback.join().expect("callback thread");
    let guard = exporter.join().expect("exporter thread");
    assert!(marked.load(Ordering::SeqCst));
    assert!(!shared.callback_activity.in_flight());

    // Every later block sees the gate: silence, no plugin call.
    let calls = rec.lock().calls;
    for _ in 0..4 {
        assert!(h.render().iter().all(|&s| s == 0.0));
    }
    assert_eq!(rec.lock().calls, calls, "no process() under the gate");
    drop(guard);
}

#[test]
fn raising_the_gate_with_the_callback_idle_does_not_wait() {
    let mut h = MixAudioHarness::new(
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    h.render();
    let shared = h.shared_arc();
    assert_eq!(shared.callback_activity.blocks_started(), 1);
    let start = Instant::now();
    let _guard = OfflineRenderGuard::mark(&shared);
    assert!(start.elapsed() < Duration::from_millis(50));
}
