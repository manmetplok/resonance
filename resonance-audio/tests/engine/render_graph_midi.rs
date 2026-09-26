//! MIDI clips on the published render graph (code review ARCH-02 A2-4,
//! refactor todo B-1), driven through the real handlers.
//!
//! The audio callback used to `try_read` an `RwLock<Vec<MidiClip>>` and
//! drop the block whenever a handler held (or had queued) its write
//! lock. The clips now live in an immutable `RenderGraph` the engine
//! thread republishes on every edit; readers `load()` it and never wait.
//! These tests pin the two properties the swap relies on:
//!
//! - a reader loading the graph while handlers hammer it always sees a
//!   whole, consistent snapshot — never a half-applied edit — and every
//!   edit lands;
//! - a replaced graph is freed by the engine thread's retire sweep, never
//!   by the reader that last looked at it.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use resonance_audio::test_support::{EngineHandlerHarness, RenderGraph};
use resonance_audio::types::*;

const CLIPS: u64 = 500;
const FIRST_CLIP: ClipId = 1_000;

fn note(start_tick: u64, pitch: u8) -> MidiNote {
    MidiNote {
        note: pitch,
        velocity: 0.8,
        start_tick,
        duration_ticks: 120,
    }
}

/// A 500-clip project, loaded through the real `LoadMidiClipDirect`
/// handler as a project load would.
fn big_project() -> EngineHandlerHarness {
    let mut h = EngineHandlerHarness::new();
    for i in 0..CLIPS {
        h.load_midi_clip_direct(FIRST_CLIP + i, 1 + i % 8);
    }
    h.drain_events();
    h
}

#[test]
fn a_reader_sees_whole_snapshots_while_handlers_edit_a_500_clip_project() {
    let mut h = big_project();
    let shared = h.shared_arc();
    let done = Arc::new(AtomicBool::new(false));
    let loads = Arc::new(AtomicU64::new(0));

    // The "audio thread": load the graph in a loop and check every
    // snapshot is internally consistent. `SetMidiClipNotes` below always
    // writes `k` notes all at pitch `k`, so a torn clip would show a
    // mixed or mis-sized note array.
    let reader = {
        let shared = Arc::clone(&shared);
        let done = Arc::clone(&done);
        let loads = Arc::clone(&loads);
        std::thread::spawn(move || {
            let mut max_notes_seen = 0usize;
            while !done.load(Ordering::Acquire) {
                let graph = shared.graph.load();
                assert_eq!(graph.midi_clips.len(), CLIPS as usize);
                let target = &graph.midi_clips[0];
                assert_eq!(target.id, FIRST_CLIP);
                let k = target.notes.len();
                assert!(
                    target.notes.iter().all(|n| n.note as usize == k),
                    "torn clip: {} notes {:?}",
                    k,
                    target.notes.iter().map(|n| n.note).collect::<Vec<_>>()
                );
                assert!(k >= max_notes_seen, "snapshots never go backwards");
                max_notes_seen = k;
                loads.fetch_add(1, Ordering::Relaxed);
            }
        })
    };

    for k in 1..=100u8 {
        let notes: Vec<MidiNote> = (0..k as u64).map(|i| note(i * 240, k)).collect();
        h.set_midi_clip_notes(FIRST_CLIP, notes);
        // A single-note add on another clip in between, the other
        // handler shape (find, copy-on-write one element, publish).
        h.add_midi_note(FIRST_CLIP + 1 + k as u64, note(0, 60));
        // Keep the retire queue bounded the way the engine loop does.
        if k.is_multiple_of(10) {
            h.sweep_retired();
        }
        // Let the reader load at least once more before the next edit, so
        // the two really interleave over all 100 rounds.
        let seen = loads.load(Ordering::Relaxed);
        while loads.load(Ordering::Relaxed) == seen && !reader.is_finished() {
            std::thread::yield_now();
        }
    }
    done.store(true, Ordering::Release);
    reader.join().expect("reader saw a consistent graph every time");
    assert!(loads.load(Ordering::Relaxed) > 0, "the reader actually ran");

    let graph = h.render_graph();
    assert_eq!(graph.midi_clips.len(), CLIPS as usize);
    assert_eq!(h.midi_notes(FIRST_CLIP).len(), 100);
    for k in 1..=100u64 {
        assert_eq!(h.midi_notes(FIRST_CLIP + 1 + k).len(), 1, "clip {k} got its note");
    }
    // Untouched clips are shared with the pre-edit graphs, not copied:
    // the last clip was never edited, so every graph published since the
    // load holds the same allocation.
    let untouched = FIRST_CLIP + CLIPS - 1;
    let before = h.render_graph();
    h.add_midi_note(FIRST_CLIP, note(99_999, 1));
    let after = h.render_graph();
    let find = |g: &RenderGraph, id| g.midi_clips.iter().find(|c| c.id == id).cloned().unwrap();
    assert!(Arc::ptr_eq(&find(&before, untouched), &find(&after, untouched)));
    assert!(!Arc::ptr_eq(&find(&before, FIRST_CLIP), &find(&after, FIRST_CLIP)));
}

#[test]
fn a_replaced_graph_is_freed_by_the_engine_sweep_not_by_its_last_reader() {
    let mut h = big_project();
    h.sweep_retired();
    let shared = h.shared_arc();

    // The reader pins the current graph, as the callback does for a block.
    let (pinned_tx, pinned_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let reader = {
        let shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            let pinned = shared.graph.load_full();
            pinned_tx.send(Arc::downgrade(&pinned)).unwrap();
            release_rx.recv().unwrap();
            // The reader lets go on its own thread. It must not be the
            // last owner: the retire queue still holds the graph.
            drop(pinned);
        })
    };
    let weak = pinned_rx.recv().unwrap();

    // A handler replaces the graph while the reader holds it.
    h.add_midi_note(FIRST_CLIP, note(0, 64));
    assert_eq!(h.sweep_retired(), 0, "the pinned graph survives a sweep");
    assert!(weak.upgrade().is_some());

    release_tx.send(()).unwrap();
    reader.join().unwrap();
    assert!(
        weak.upgrade().is_some(),
        "the reader's drop did not free the graph — the retire queue owns it"
    );
    assert_eq!(h.sweep_retired(), 1, "the engine-thread sweep frees it");
    assert!(weak.upgrade().is_none());
    assert!(h.shared().retired.is_empty());
}

#[test]
fn clear_all_empties_the_graph_and_retires_the_clips() {
    let mut h = big_project();
    h.sweep_retired();
    let before = h.render_graph();
    let one = Arc::downgrade(&before.midi_clips[0]);
    drop(before);
    h.clear_all();
    assert!(h.render_graph().midi_clips.is_empty());
    assert!(one.upgrade().is_some(), "held by the retired graph until the sweep");
    h.sweep_retired();
    assert!(one.upgrade().is_none(), "freed by the engine-thread sweep");
}
