//! Deferred drop for snapshots the audio thread may still be reading
//! (code review MIX-04; the primitive every ARCH-02 publish goes through).
//!
//! The engine publishes wait-free snapshots to the audio callback through
//! `ArcSwap`s: the automation lanes, the tempo map, the plugin-delay
//! compensation table, the take-comp table, the aux-send and sidechain
//! tables, a frozen track's cache, the audition and reference PCM. The
//! callback pins one with a `load()` guard (or a `load_full()` `Arc`) for
//! the block. If the publisher simply `store`s a replacement and lets its
//! own reference go, the callback's guard becomes the *last* owner and
//! the value is freed on the realtime thread at the end of the block —
//! for a `LatencyComp` (up to `MAX_COMP_LATENCY` floats per delay line)
//! or a frozen cache that is a `munmap` inside the deadline.
//!
//! [`Retired`] keeps the replaced `Arc` alive on the publishing side:
//! `swap` the slot, [`retire`](Retired::retire) the old value, and the
//! engine loop's [`sweep`](Retired::sweep) drops it once nobody else
//! holds it. The audio thread can then never be the last owner, because
//! this queue is — until the sweep, which runs on the engine thread.
//!
//! Threading: publishers and the sweep both run on the engine control
//! thread (no worker publishes the render graph since ARCH-02 B-3/B-5 — a
//! cancelled offline bounce posts its target-track removal back to the
//! engine thread, and the clip-load, pitch-analysis, bounce and
//! retune-cache workers post their results on `SharedState::inbox`), so
//! the queue is a plain `Mutex`.
//! The audio thread never touches it — the only thing that would need a
//! lock-free path is retiring *from* the callback, and nothing does that:
//! every `Arc` the callback holds is one the engine handed it and still
//! owns here.

use std::any::Any;
use std::sync::Arc;

use parking_lot::Mutex;

/// The type-erased retire queue. See the module docs.
#[derive(Default)]
pub struct Retired {
    queue: Mutex<Vec<Arc<dyn Any + Send + Sync>>>,
}

impl std::fmt::Debug for Retired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Retired").field("len", &self.len()).finish()
    }
}

impl Retired {
    pub const fn new() -> Self {
        Self {
            queue: Mutex::new(Vec::new()),
        }
    }

    /// Keep `old` — the value an `ArcSwap::swap` just replaced — alive
    /// until a [`sweep`](Self::sweep) finds no other owner.
    pub fn retire<T: Any + Send + Sync>(&self, old: Arc<T>) {
        self.queue.lock().push(old);
    }

    /// [`retire`](Self::retire) for an `ArcSwapOption::swap` result.
    pub fn retire_opt<T: Any + Send + Sync>(&self, old: Option<Arc<T>>) {
        if let Some(old) = old {
            self.retire(old);
        }
    }

    /// Drop every entry this queue is the sole owner of, on the calling
    /// (engine) thread. Returns how many were dropped. Entries a reader
    /// still pins stay for the next sweep.
    pub fn sweep(&self) -> usize {
        // Take the freed entries out of the lock before they drop, so a
        // slow destructor (a multi-MB cache) never holds the queue.
        let freed: Vec<Arc<dyn Any + Send + Sync>> = {
            let mut queue = self.queue.lock();
            let (free, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut *queue)
                .into_iter()
                .partition(|a| Arc::strong_count(a) == 1);
            *queue = keep;
            free
        };
        freed.len()
    }

    /// Entries currently held.
    pub fn len(&self) -> usize {
        self.queue.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the queue holds `arc`'s allocation (pointer identity).
    pub fn holds<T: Any + Send + Sync>(&self, arc: &Arc<T>) -> bool {
        let ptr = Arc::as_ptr(arc) as *const ();
        self.queue
            .lock()
            .iter()
            .any(|a| Arc::as_ptr(a) as *const () == ptr)
    }
}

/// `ArcSwap::store` for a slot the audio thread reads: the replaced
/// value goes to `retired` instead of being dropped here.
pub fn publish<T: Any + Send + Sync>(
    slot: &arc_swap::ArcSwap<T>,
    new: Arc<T>,
    retired: &Retired,
) {
    retired.retire(slot.swap(new));
}

/// [`publish`] for an `ArcSwapOption` slot.
pub fn publish_opt<T: Any + Send + Sync>(
    slot: &arc_swap::ArcSwapOption<T>,
    new: Option<Arc<T>>,
    retired: &Retired,
) {
    retired.retire_opt(slot.swap(new));
}
