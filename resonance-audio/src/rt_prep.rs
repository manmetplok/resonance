//! Per-thread state a realtime thread must own before its first block.
//!
//! `arc-swap` gives every thread a debt-list node on the thread's first
//! `load` of ANY `ArcSwap`, and keeps it until the thread exits (then it
//! goes back to a global list for the next new thread). When the list has
//! no free node, that first load `Box`es a new one. The render path loads
//! `ArcSwap`s every block (the render graph, a track's frozen source), so
//! a realtime thread that has not claimed its node yet allocates inside
//! its first block. Each realtime thread claims its node off the RT path:
//!
//! - render pool workers, at spawn ([`claim_arc_swap_node`]);
//! - the native PipeWire output's data-loop thread, through an invoke
//!   queued on that loop right after `connect`, before the first cycle
//!   (`output_pipewire`);
//! - a thread we cannot run code on before its first callback (cpal's
//!   audio worker): [`seed_arc_swap_nodes`] leaves free nodes in the list
//!   so that thread's first load takes one without allocating.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

/// Claim the calling thread's `arc-swap` debt-list node now. Allocates at
/// most once per thread; allocation-free on every later call.
pub(crate) fn claim_arc_swap_node() {
    let probe = arc_swap::ArcSwapOption::<()>::const_empty();
    let _ = probe.load();
}

/// Leave at least `n` free `arc-swap` debt-list nodes in the global list,
/// for a realtime thread about to start that we cannot run code on first.
/// `n` short-lived threads each claim a node, all hold them at once, then
/// exit, which returns every node to the list. Best effort: a thread that
/// starts between this and the realtime thread's first load may take one.
/// Allocates (spawns); engine side.
pub(crate) fn seed_arc_swap_nodes(n: usize) {
    let gate = Arc::new(RwLock::new(()));
    let claimed = Arc::new(AtomicUsize::new(0));
    let hold = gate.write().unwrap_or_else(|e| e.into_inner());
    let seeders: Vec<_> = (0..n)
        .filter_map(|_| {
            let gate = Arc::clone(&gate);
            let claimed = Arc::clone(&claimed);
            std::thread::Builder::new()
                .name("arc-swap-seed".into())
                .spawn(move || {
                    claim_arc_swap_node();
                    claimed.fetch_add(1, Ordering::SeqCst);
                    // Keep the node until every seeder has one.
                    drop(gate.read());
                })
                .ok()
        })
        .collect();
    while claimed.load(Ordering::SeqCst) < seeders.len() {
        std::thread::yield_now();
    }
    drop(hold);
    for seeder in seeders {
        let _ = seeder.join();
    }
}
