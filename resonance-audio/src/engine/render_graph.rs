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
//! Migration state: `midi_clips` (B-1), `busses` and `master` (B-2),
//! `tracks` (B-3), `plugins` (B-4), `clips` (B-5) — every project map the
//! renderers read; none is behind a lock any more.
//!
//! Ownership and threading:
//!
//! - **Writers** are the engine control thread's handlers (MIDI clip and
//!   note CRUD, the bulk edits, live MIDI recording, bus add / remove /
//!   rename / insert-chain edits, master insert-chain edits, track add /
//!   remove / re-route / MIDI-binding edits, `ClearAll`).
//!   Each edit copies the element list (`O(n)` `Arc` clones),
//!   copy-on-writes the one element it changes (`Arc::make_mut` — the
//!   published graph still shares it), and publishes. Plugin add /
//!   remove (track, bus and master chains, track and bus removal,
//!   `ClearAll`, the startup scan, shutdown) edit the plugin map the
//!   same way, and audio-clip CRUD, recording, take parking and the
//!   project-save remaps edit the clip list the same way (B-5). No worker
//!   thread writes the graph — the offline bounce-in-place's cancel posts
//!   `AudioCommand::BounceTargetCancelled` so the engine thread removes
//!   its target track, and the clip-load, pitch-analysis, bounce and
//!   retune-cache workers post their results on `SharedState::inbox`
//!   (`engine::internal`) for the engine thread to apply. The `edit`
//!   mutex serialises the read-modify-publish so a stray second writer
//!   could never lose an update; the audio thread never touches it.
//! - **Bus live state.** A bus's fader, pan, mute, role, chain-bypass
//!   fade, meters and last gains are atomics shared by every
//!   copy-on-write copy of it (`Bus`'s `Clone`), so an edit never loses a
//!   meter write or a fade position the audio thread made on the copy
//!   being replaced. The setters for those write through the published
//!   bus and publish nothing.
//! - **Track live state**, likewise: fader, pan, mute, solo, arm,
//!   monitor, mono, input port, the chain-bypass fade, meters, last
//!   gains, the insert chain, device params, input / MIDI-out device and
//!   the frozen cache are shared by every copy of a track (`Track`'s
//!   `Clone`); their setters write through and publish nothing. Routing
//!   (`Track::output`) and the hardware-MIDI bindings are structural and
//!   go through [`RenderGraphSlot::edit_track`] — so a bus removal and
//!   the re-route of its feeders are one publish
//!   ([`RenderGraphSlot::edit_tracks_and_busses`]).
//! - **The replaced graph** goes through [`retire::publish`] onto
//!   `SharedState::retired`, and the engine loop's sweep drops it once no
//!   reader pins it — so the audio thread is never the last owner of a
//!   graph (or of a clip, bus, track or chain only that graph still
//!   held). That includes an audio clip's samples: a removed or replaced
//!   clip — its mmap, or its in-RAM `Arc<[f32]>` and retune cache — is
//!   freed when the sweep drops the last graph that listed it, on the
//!   engine thread. A removed track therefore needs no retiring of its own: it
//!   rides out on the replaced graph and drops, with its frozen cache and
//!   insert chain, on the engine thread.
//! - **Plugin instances** (B-4). The map holds `Arc<PluginSlot>`, so a
//!   copy of the map shares every slot — and the instance `Mutex` a
//!   block's `process()` locks — with the graph a reader still holds.
//!   The render path still `try_lock`s the *instance* (the offline
//!   renderers `try_lock_with_backoff`), but reaching it is a plain map
//!   lookup on the loaded graph. `ClapInstance::drop` (editor teardown,
//!   deactivate, destroy) must never run on the audio thread, so
//!   [`RenderGraphSlot::edit_plugins`] retires every slot an edit
//!   removed onto the same queue, *individually*: the sweep drops a slot
//!   only once the queue is its sole owner (`strong_count == 1`) — after
//!   every graph that listed it has been swept and no reader holds a
//!   clone of it. Whatever a reader pinned, it only ever releases a
//!   reference the queue still shares, so the destructor runs in the
//!   engine loop's sweep. A slot outlives its removal by up to one sweep
//!   tick (~16 ms) after the last reader lets go; bundles are never
//!   unloaded (`ClapBundle`'s `Drop`), so a late destroy is always safe.
//! - **Chain order stays where it was.** A track's insert chain is the
//!   `ArcSwap<Vec<PluginInstanceId>>` in its shared `TrackRuntime`
//!   (published without a graph edit); bus and master chains are graph
//!   structure already. A block that sees a chain id its graph's plugin
//!   map lacks skips that slot, and a map entry no chain names is never
//!   processed — so add (map, then chain) and remove (chain, then map)
//!   each risk at most one block in which the slot is silent, never one
//!   in which a freed instance is reachable.
//! - **Readers**: the audio callback `load()`s once per block; bounce /
//!   freeze / stem workers load once per chunk (they keep seeing edits at
//!   chunk granularity, as they did through the per-chunk read guards).

use std::sync::Arc;

use arc_swap::ArcSwap;
use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::clap_host::{PluginMap, PluginSlot};
use crate::mixer::render::slots::{LiveSlots, SlotSupply};
use crate::types::{
    AudioClip, Bus, BusId, ClipId, MasterBus, MidiClip, PluginInstanceId, Track, TrackId,
    TrackMap,
};

use super::retire::{self, Retired};

/// One immutable snapshot of what the renderers read. Every field is its
/// own `Arc`, so publishing an edit to one field never copies another.
#[derive(Debug, Clone)]
pub struct RenderGraph {
    /// Every MIDI clip on the timeline, in insertion order.
    pub midi_clips: Arc<[Arc<MidiClip>]>,
    /// Every audio clip the render plays — timeline clips and take
    /// recordings — in list order (the order the clip mix sums them in,
    /// so it is part of the graph: an edit must keep it exactly).
    pub clips: Arc<[Arc<AudioClip>]>,
    /// Every bus, in insertion order — the mixer gives bus `i` the
    /// `i`-th bus buffer, so the order is part of the graph.
    pub busses: Arc<IndexMap<BusId, Arc<Bus>>>,
    /// The master insert chain.
    pub master: Arc<MasterBus>,
    /// Every track and sub-track, in insertion order (the order stems,
    /// latency comp and the monitor-source pick walk them in).
    pub tracks: Arc<TrackMap>,
    /// Every live plugin instance, across every track, sub-track, bus
    /// and master chain. The chains name slots by id; this owns them.
    pub plugins: Arc<PluginMap>,
}

impl Default for RenderGraph {
    fn default() -> Self {
        Self {
            midi_clips: Arc::from(Vec::new()),
            clips: Arc::from(Vec::new()),
            busses: Arc::new(IndexMap::new()),
            master: Arc::new(MasterBus::default()),
            tracks: Arc::new(TrackMap::new()),
            plugins: Arc::new(PluginMap::new()),
        }
    }
}

#[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
impl RenderGraph {
    /// The MIDI clip with `clip_id`, if any.
    pub fn midi_clip(&self, clip_id: ClipId) -> Option<&MidiClip> {
        self.midi_clips.iter().find(|c| c.id == clip_id).map(|c| &**c)
    }

    /// The audio clip with `clip_id`, if any.
    pub fn clip(&self, clip_id: ClipId) -> Option<&AudioClip> {
        self.clips.iter().find(|c| c.id == clip_id).map(|c| &**c)
    }

    /// Every audio clip on `track_id`, in list order.
    pub fn clips_on(&self, track_id: TrackId) -> impl Iterator<Item = &AudioClip> {
        self.clips
            .iter()
            .filter(move |c| c.track_id == track_id)
            .map(|c| &**c)
    }

    /// The bus with `bus_id`, if any.
    pub fn bus(&self, bus_id: BusId) -> Option<&Bus> {
        self.busses.get(&bus_id).map(|b| &**b)
    }

    /// The track with `track_id`, if any.
    #[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
    pub fn track(&self, track_id: TrackId) -> Option<&Track> {
        self.tracks.get(&track_id).map(|t| &**t)
    }

    /// The plugin slot with `instance_id`, if any.
    #[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
    pub fn plugin(&self, instance_id: PluginInstanceId) -> Option<&PluginSlot> {
        self.plugins.get(&instance_id).map(|p| &**p)
    }
}

/// The published-graph slot on [`SharedState`](super::SharedState).
pub struct RenderGraphSlot {
    graph: ArcSwap<RenderGraph>,
    /// Serialises edits (read, modify, publish). Engine side only.
    edit: Mutex<()>,
    /// Keeps the live callback's per-track render slots at least as many
    /// as the published graph has tracks (`mixer::render::slots`).
    slot_supply: SlotSupply,
}

impl Default for RenderGraphSlot {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for RenderGraphSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let graph = self.graph.load();
        f.debug_struct("RenderGraphSlot")
            .field("midi_clips", &graph.midi_clips.len())
            .field("clips", &graph.clips.len())
            .field("busses", &graph.busses.len())
            .field("master_plugins", &graph.master.plugin_ids.len())
            .field("tracks", &graph.tracks.len())
            .field("plugins", &graph.plugins.len())
            .finish()
    }
}

#[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
impl RenderGraphSlot {
    pub fn new() -> Self {
        Self {
            graph: ArcSwap::from_pointee(RenderGraph::default()),
            edit: Mutex::new(()),
            slot_supply: SlotSupply::default(),
        }
    }

    /// Hand a new live callback its per-track render slot pool, sized for
    /// the current graph and grown on every later publish. Engine side;
    /// allocates.
    pub(crate) fn attach_live_slots(&self, max_frames: usize) -> LiveSlots {
        let _edit = self.edit.lock();
        let tracks = self.graph.load().tracks.len();
        self.slot_supply.attach(max_frames, tracks)
    }

    /// The current graph. Wait-free, lock-free and allocation-free after
    /// the calling thread's first load — the audio callback's one read.
    #[inline]
    pub fn load(&self) -> arc_swap::Guard<Arc<RenderGraph>> {
        self.graph.load()
    }

    /// The current graph as an owned `Arc`, for a reader that keeps it
    /// past a borrow (an offline chunk, a test).
    #[cfg_attr(not(feature = "test-internals"), allow(dead_code))]
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
        self.publish_locked(&current, retired, |g| g.midi_clips = clips.into());
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
        self.publish_locked(&current, retired, |g| g.midi_clips = clips.into());
        Some(out)
    }

    /// Edit the audio clip list and publish the result. `f` gets a copy of
    /// the list (the clips are shared with the published graph until
    /// `Arc::make_mut`ed — a clip copy shares its samples, see
    /// [`AudioClip`]). Always publishes. Engine thread.
    ///
    /// A clip `f` removes or replaces rides out on the replaced graph,
    /// which the retire sweep drops on the engine thread — so its audio is
    /// never freed by a reader (see the module docs).
    pub fn edit_clips<R>(
        &self,
        retired: &Retired,
        f: impl FnOnce(&mut Vec<Arc<AudioClip>>) -> R,
    ) -> R {
        let _edit = self.edit.lock();
        let current = self.graph.load_full();
        let mut clips = current.clips.to_vec();
        let out = f(&mut clips);
        self.publish_locked(&current, retired, |g| g.clips = clips.into());
        out
    }

    /// Edit the one audio clip with `clip_id` (copy-on-write) and publish.
    /// `None` — and nothing published — when there is no such clip.
    /// Engine thread.
    pub fn edit_clip<R>(
        &self,
        retired: &Retired,
        clip_id: ClipId,
        f: impl FnOnce(&mut AudioClip) -> R,
    ) -> Option<R> {
        let _edit = self.edit.lock();
        let current = self.graph.load_full();
        let index = current.clips.iter().position(|c| c.id == clip_id)?;
        let mut clips = current.clips.to_vec();
        let out = f(Arc::make_mut(&mut clips[index]));
        self.publish_locked(&current, retired, |g| g.clips = clips.into());
        Some(out)
    }

    /// Edit the bus map (add, remove, clear) and publish the result. `f`
    /// gets a copy of the map whose buses are shared with the published
    /// graph until `Arc::make_mut`ed. Always publishes. Engine thread.
    pub fn edit_busses<R>(
        &self,
        retired: &Retired,
        f: impl FnOnce(&mut IndexMap<BusId, Arc<Bus>>) -> R,
    ) -> R {
        let _edit = self.edit.lock();
        let current = self.graph.load_full();
        let mut busses = (*current.busses).clone();
        let out = f(&mut busses);
        self.publish_locked(&current, retired, |g| g.busses = Arc::new(busses));
        out
    }

    /// Edit the one bus with `bus_id` (copy-on-write; the copy shares the
    /// published bus's live state) and publish. `None` — and nothing
    /// published — when there is no such bus. Engine thread.
    pub fn edit_bus<R>(
        &self,
        retired: &Retired,
        bus_id: BusId,
        f: impl FnOnce(&mut Bus) -> R,
    ) -> Option<R> {
        let _edit = self.edit.lock();
        let current = self.graph.load_full();
        if !current.busses.contains_key(&bus_id) {
            return None;
        }
        let mut busses = (*current.busses).clone();
        let out = f(Arc::make_mut(busses.get_mut(&bus_id)?));
        self.publish_locked(&current, retired, |g| g.busses = Arc::new(busses));
        Some(out)
    }

    /// Edit the master insert chain and publish. Always publishes.
    /// Engine thread.
    pub fn edit_master<R>(&self, retired: &Retired, f: impl FnOnce(&mut MasterBus) -> R) -> R {
        let _edit = self.edit.lock();
        let current = self.graph.load_full();
        let mut master = (*current.master).clone();
        let out = f(&mut master);
        self.publish_locked(&current, retired, |g| g.master = Arc::new(master));
        out
    }

    /// Edit the track map (add, remove, clear, reorder) and publish the
    /// result. `f` gets a copy of the map whose tracks are shared with the
    /// published graph until `Arc::make_mut`ed. Always publishes. Engine
    /// thread.
    pub fn edit_tracks<R>(&self, retired: &Retired, f: impl FnOnce(&mut TrackMap) -> R) -> R {
        let _edit = self.edit.lock();
        let current = self.graph.load_full();
        let mut tracks = (*current.tracks).clone();
        let out = f(&mut tracks);
        self.publish_locked(&current, retired, |g| g.tracks = Arc::new(tracks));
        out
    }

    /// Edit the one track with `track_id` (copy-on-write; the copy shares
    /// the published track's live state) and publish. `None` — and
    /// nothing published — when there is no such track. Engine thread.
    pub fn edit_track<R>(
        &self,
        retired: &Retired,
        track_id: TrackId,
        f: impl FnOnce(&mut Track) -> R,
    ) -> Option<R> {
        let _edit = self.edit.lock();
        let current = self.graph.load_full();
        if !current.tracks.contains_key(&track_id) {
            return None;
        }
        let mut tracks = (*current.tracks).clone();
        let out = f(Arc::make_mut(tracks.get_mut(&track_id)?));
        self.publish_locked(&current, retired, |g| g.tracks = Arc::new(tracks));
        Some(out)
    }

    /// Edit the track map and the bus map together and publish both in
    /// ONE graph — for an edit whose halves must never be seen apart by
    /// a block (a bus removal and the re-route of the tracks that fed
    /// it). Always publishes. Engine thread.
    pub fn edit_tracks_and_busses<R>(
        &self,
        retired: &Retired,
        f: impl FnOnce(&mut TrackMap, &mut IndexMap<BusId, Arc<Bus>>) -> R,
    ) -> R {
        let _edit = self.edit.lock();
        let current = self.graph.load_full();
        let mut tracks = (*current.tracks).clone();
        let mut busses = (*current.busses).clone();
        let out = f(&mut tracks, &mut busses);
        self.publish_locked(&current, retired, |g| {
            g.tracks = Arc::new(tracks);
            g.busses = Arc::new(busses);
        });
        out
    }

    /// Edit the plugin map (insert, remove, drain) and publish the
    /// result. `f` gets a copy of the map whose slots are shared with the
    /// published graph. Always publishes. Engine thread.
    ///
    /// Drop discipline: every slot the published map held that `f`'s map
    /// no longer holds (removed, or replaced under the same id) is
    /// retired onto `retired` on its own, so its `ClapInstance` is
    /// destroyed by the engine loop's sweep once nothing else owns it —
    /// never by a reader that pinned an older graph, and never inside
    /// this call (see the module docs).
    pub fn edit_plugins<R>(&self, retired: &Retired, f: impl FnOnce(&mut PluginMap) -> R) -> R {
        let _edit = self.edit.lock();
        let current = self.graph.load_full();
        let mut plugins = (*current.plugins).clone();
        let out = f(&mut plugins);
        for (id, slot) in current.plugins.iter() {
            if !plugins.get(id).is_some_and(|kept| Arc::ptr_eq(kept, slot)) {
                retired.retire(Arc::clone(slot));
            }
        }
        self.publish_locked(&current, retired, |g| g.plugins = Arc::new(plugins));
        out
    }

    /// Publish a copy of `current` with the fields `set` replaces; every
    /// other field carries over by `Arc` clone. Called with `edit` held.
    fn publish_locked(
        &self,
        current: &RenderGraph,
        retired: &Retired,
        set: impl FnOnce(&mut RenderGraph),
    ) {
        let mut next = current.clone();
        set(&mut next);
        // Before the store: a callback that sees `next` must find a slot
        // pool that fits it (see `SlotSupply::ensure`).
        self.slot_supply.ensure(next.tracks.len());
        retire::publish(&self.graph, Arc::new(next), retired);
    }
}
