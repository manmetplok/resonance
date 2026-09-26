//! The immutable render graph the engine publishes to the audio callback
//! and the offline renderers (code review ARCH-02, A2-4…).
//!
//! The callback used to `try_read` five `Arc<parking_lot::RwLock<…>>`
//! project maps and drop the block whenever one of them was write-held
//! or had a writer queued (parking_lot is task-fair). This module is the
//! replacement: the engine thread builds a new [`RenderGraph`] on every
//! edit and publishes it through an `ArcSwap`, so a reader does one
//! wait-free [`load`](RenderGraphSlot::load) and never fails.
//!
//! Migration state: `midi_clips` only (B-1). `busses` / `master`,
//! `tracks`, `plugins` and `clips` join as fields in B-2…B-5; until then
//! they stay behind their locks.
//!
//! Ownership and threading:
//!
//! - **Writers** are the engine control thread's handlers (MIDI clip and
//!   note CRUD, the bulk edits, live MIDI recording, `ClearAll`). Each
//!   edit copies the element list (`O(n)` `Arc` clones), copy-on-writes
//!   the one element it changes (`Arc::make_mut` — the published graph
//!   still shares it), and publishes. No worker thread writes the graph.
//!   The `edit` mutex serialises the read-modify-publish so a stray
//!   second writer could never lose an update; the audio thread never
//!   touches it.
//! - **The replaced graph** goes through [`retire::publish`] onto
//!   `SharedState::retired`, and the engine loop's sweep drops it once no
//!   reader pins it — so the audio thread is never the last owner of a
//!   graph (or of a clip only that graph still held).
//! - **Readers**: the audio callback `load()`s once per block; bounce /
//!   freeze / stem workers load once per chunk (they keep seeing edits at
//!   chunk granularity, as they did through the per-chunk read guards).

use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::Mutex;

use crate::types::{ClipId, MidiClip};

use super::retire::{self, Retired};

/// One immutable snapshot of what the renderers read. Every field is its
/// own `Arc`, so publishing an edit to one field never copies another.
#[derive(Debug, Clone)]
pub struct RenderGraph {
    /// Every MIDI clip on the timeline, in insertion order.
    pub midi_clips: Arc<[Arc<MidiClip>]>,
}

impl Default for RenderGraph {
    fn default() -> Self {
        Self {
            midi_clips: Arc::from(Vec::new()),
        }
    }
}

impl RenderGraph {
    /// The MIDI clip with `clip_id`, if any.
    pub fn midi_clip(&self, clip_id: ClipId) -> Option<&MidiClip> {
        self.midi_clips.iter().find(|c| c.id == clip_id).map(|c| &**c)
    }
}

/// The published-graph slot on [`SharedState`](super::SharedState).
pub struct RenderGraphSlot {
    graph: ArcSwap<RenderGraph>,
    /// Serialises edits (read, modify, publish). Engine side only.
    edit: Mutex<()>,
}

impl Default for RenderGraphSlot {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for RenderGraphSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderGraphSlot")
            .field("midi_clips", &self.graph.load().midi_clips.len())
            .finish()
    }
}

impl RenderGraphSlot {
    pub fn new() -> Self {
        Self {
            graph: ArcSwap::from_pointee(RenderGraph::default()),
            edit: Mutex::new(()),
        }
    }

    /// The current graph. Wait-free, lock-free and allocation-free after
    /// the calling thread's first load — the audio callback's one read.
    #[inline]
    pub fn load(&self) -> arc_swap::Guard<Arc<RenderGraph>> {
        self.graph.load()
    }

    /// The current graph as an owned `Arc`, for a reader that keeps it
    /// past a borrow (an offline chunk, a test).
    pub fn load_full(&self) -> Arc<RenderGraph> {
        self.graph.load_full()
    }

    /// Edit the MIDI clip list and publish the result. `f` gets a copy of
    /// the list (the elements are shared with the published graph until
    /// `Arc::make_mut`ed). Always publishes. Engine thread.
    pub fn edit_midi_clips<R>(
        &self,
        retired: &Retired,
        f: impl FnOnce(&mut Vec<Arc<MidiClip>>) -> R,
    ) -> R {
        let _edit = self.edit.lock();
        let current = self.graph.load_full();
        let mut clips = current.midi_clips.to_vec();
        let out = f(&mut clips);
        self.publish_locked(&current, clips, retired);
        out
    }

    /// Edit the one MIDI clip with `clip_id` and publish. `None` — and
    /// nothing published — when there is no such clip. Engine thread.
    pub fn edit_midi_clip<R>(
        &self,
        retired: &Retired,
        clip_id: ClipId,
        f: impl FnOnce(&mut MidiClip) -> R,
    ) -> Option<R> {
        let _edit = self.edit.lock();
        let current = self.graph.load_full();
        let index = current.midi_clips.iter().position(|c| c.id == clip_id)?;
        let mut clips = current.midi_clips.to_vec();
        let out = f(Arc::make_mut(&mut clips[index]));
        self.publish_locked(&current, clips, retired);
        Some(out)
    }

    fn publish_locked(
        &self,
        current: &RenderGraph,
        midi_clips: Vec<Arc<MidiClip>>,
        retired: &Retired,
    ) {
        // Every other field carries over by `Arc` clone.
        let mut next = current.clone();
        next.midi_clips = midi_clips.into();
        retire::publish(&self.graph, Arc::new(next), retired);
    }
}
