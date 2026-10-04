//! Busses and the master insert chain on the published render graph
//! (code review ARCH-02 A2-5, refactor todo B-2), driven through the real
//! handlers.
//!
//! The busses and the master chain used to sit behind two
//! `Arc<RwLock<…>>`s the callback `try_read`; they now live in the
//! immutable `RenderGraph` the engine thread republishes on every
//! structural edit. These tests pin what the swap relies on:
//!
//! - a replaced graph — and a bus only it still holds — is freed by the
//!   engine thread's retire sweep, never by the reader that last looked
//!   at it;
//! - a bus edit copy-on-writes only that bus, and the copy shares the
//!   original's live state (fader, meters), so a meter write or a fader
//!   move on either copy is never lost;
//! - an edit that changes nothing (a move / remove of a plugin that is not
//!   on the chain) publishes nothing.

use std::sync::Arc;

use resonance_audio::test_support::{EngineHandlerHarness, RenderGraph};
use resonance_audio::types::*;

const DRUMS: BusId = 7;
const VERB: BusId = 8;

fn two_busses() -> EngineHandlerHarness {
    let mut h = EngineHandlerHarness::new();
    h.add_bus(DRUMS, Some("Drums".into()));
    h.add_bus(VERB, Some("Verb".into()));
    h.drain_events();
    h.sweep_retired();
    h
}

fn bus(g: &RenderGraph, id: BusId) -> Arc<Bus> {
    Arc::clone(g.busses.get(&id).expect("bus is in the graph"))
}

#[test]
fn a_removed_bus_is_freed_by_the_engine_sweep_not_by_its_last_reader() {
    let mut h = two_busses();
    let shared = h.shared_arc();

    // The reader pins the current graph, as the callback does for a block.
    let (pinned_tx, pinned_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let reader = {
        let shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            let pinned = shared.graph.load_full();
            let drums = Arc::downgrade(pinned.busses.get(&DRUMS).unwrap());
            pinned_tx.send((Arc::downgrade(&pinned), drums)).unwrap();
            release_rx.recv().unwrap();
            // The reader lets go on its own thread. It must not be the
            // last owner: the retire queue still holds the graph.
            drop(pinned);
        })
    };
    let (graph, drums) = pinned_rx.recv().unwrap();

    // The real handler unpublishes the bus while the reader holds it.
    h.remove_bus(DRUMS);
    assert_eq!(h.test_bus_ids(), vec![VERB]);
    assert_eq!(h.sweep_retired(), 0, "the pinned graph survives a sweep");
    assert!(drums.upgrade().is_some());

    release_tx.send(()).unwrap();
    reader.join().unwrap();
    assert!(
        graph.upgrade().is_some() && drums.upgrade().is_some(),
        "the reader's drop freed neither the graph nor the bus — the retire queue owns them"
    );
    assert_eq!(h.sweep_retired(), 1, "the engine-thread sweep frees the graph");
    assert!(graph.upgrade().is_none());
    assert!(drums.upgrade().is_none(), "and with it the removed bus");
    assert!(h.shared().retired.is_empty());
}

#[test]
fn a_bus_edit_copies_only_that_bus_and_the_copy_shares_its_live_state() {
    let mut h = two_busses();
    let before = h.render_graph();

    h.set_bus_name(DRUMS, "Kit");
    let after = h.render_graph();
    assert_eq!(h.test_bus_name(DRUMS).as_deref(), Some("Kit"));
    assert_eq!(bus(&before, DRUMS).name, "Drums", "the published copy is immutable");
    assert!(!Arc::ptr_eq(&bus(&before, DRUMS), &bus(&after, DRUMS)));
    assert!(
        Arc::ptr_eq(&bus(&before, VERB), &bus(&after, VERB)),
        "an untouched bus is shared, not copied"
    );
    assert!(Arc::ptr_eq(&before.master, &after.master));
    assert_eq!(
        after.busses.keys().copied().collect::<Vec<_>>(),
        vec![DRUMS, VERB],
        "insertion order — the bus-buffer index — survives the edit"
    );

    // A fader move lands on the published bus and publishes nothing; the
    // replaced copy (still pinned by `before`) sees it too.
    h.set_bus_volume(DRUMS, 0.25);
    assert!(Arc::ptr_eq(&h.render_graph(), &after), "no graph published");
    assert_eq!(bus(&before, DRUMS).volume(), 0.25);

    // A meter write the audio thread made on the replaced copy is read by
    // the engine's meter poll through the new one.
    bus(&before, DRUMS).update_peak_l(0.5);
    assert_eq!(bus(&after, DRUMS).swap_peak_l(), 0.5);
}

#[test]
fn bus_and_master_chain_edits_publish_and_retire_the_old_chain() {
    let mut h = two_busses();
    // Chains of phantom instance ids: the handlers under test only touch
    // the order, never an instance.
    h.shared().edit_bus(DRUMS, |b| b.plugin_ids = vec![10, 20, 30]).unwrap();
    h.shared().edit_master(|m| m.plugin_ids = vec![40, 50]);
    h.sweep_retired();

    let pinned = h.render_graph();
    h.move_plugin(ChainOwner::Bus(DRUMS), 30, 0);
    h.remove_plugin(ChainOwner::Master, 40);
    let now = h.render_graph();
    assert_eq!(now.bus(DRUMS).unwrap().plugin_ids, vec![30, 10, 20]);
    assert_eq!(now.master.plugin_ids, vec![50]);
    // The reader's snapshot keeps the chains it started the block with.
    assert_eq!(pinned.bus(DRUMS).unwrap().plugin_ids, vec![10, 20, 30]);
    assert_eq!(pinned.master.plugin_ids, vec![40, 50]);

    let old_bus = Arc::downgrade(pinned.busses.get(&DRUMS).unwrap());
    let old_master = Arc::downgrade(&pinned.master);
    drop(pinned);
    assert!(old_bus.upgrade().is_some(), "held by the retired graph until the sweep");
    assert_eq!(h.sweep_retired(), 2);
    assert!(old_bus.upgrade().is_none() && old_master.upgrade().is_none());

    // Moving or removing an instance that is not on the chain changes
    // nothing, so it publishes nothing.
    h.move_plugin(ChainOwner::Bus(DRUMS), 99, 0);
    h.remove_plugin(ChainOwner::Bus(DRUMS), 99);
    h.move_plugin(ChainOwner::Master, 99, 0);
    h.remove_plugin(ChainOwner::Master, 99);
    assert!(Arc::ptr_eq(&h.render_graph(), &now));
    assert!(h.shared().retired.is_empty());
}

#[test]
fn clear_all_empties_the_busses_and_the_master_chain() {
    let mut h = two_busses();
    h.shared().edit_master(|m| m.plugin_ids = vec![40]);
    h.sweep_retired();
    let drums = Arc::downgrade(h.render_graph().busses.get(&DRUMS).unwrap());
    h.clear_all();
    let graph = h.render_graph();
    assert!(graph.busses.is_empty());
    assert!(graph.master.plugin_ids.is_empty());
    assert!(drums.upgrade().is_some(), "held by the retired graph until the sweep");
    h.sweep_retired();
    assert!(drums.upgrade().is_none(), "freed by the engine-thread sweep");
}
