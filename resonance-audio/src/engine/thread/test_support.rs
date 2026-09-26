//! Headless harness over the engine control thread's own
//! [`HandlerCtx`] + [`HandlerState`], so an integration test can run a
//! real command handler without an audio device, a CLAP plugin or the
//! engine thread (ba todo #1399).
//!
//! Same purpose as `mixer::test_support` one layer down, and the same
//! rule: it drives the *real* handler, and it starts from
//! [`HandlerState::new`] — the state the live engine thread starts from —
//! so what the harness proves is what the engine does.
//!
//! The exposed surface is deliberately narrow: `HandlerCtx` and
//! `HandlerState` stay `pub(crate)`, and each accessor here is one thing
//! a test needs to observe. Widen it a method at a time.

use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};
use ringbuf::traits::Split;

use resonance_common::{CompSegment, TakeGroup, TakeGroupId, TakeId};

use crate::clap_host::PluginMap;
use crate::engine::{
    automation::AutomationSnapshot, takes, tracks, transport, OfflineRenderGuard, SharedState,
};
use crate::mixer::CompRenderTable;
use crate::types::*;

use super::{HandlerCtx, HandlerState};

/// Owns every `Arc` a [`HandlerCtx`] borrows, plus the [`HandlerState`]
/// the handlers mutate. `HandlerCtx` holds references rather than the
/// `Arc`s themselves, so it cannot be stored alongside them — it is
/// rebuilt per call in [`Self::with_ctx`] instead.
pub struct EngineHandlerHarness {
    shared: Arc<SharedState>,
    tracks: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: Arc<RwLock<MasterBus>>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<RwLock<Vec<MidiClip>>>,
    plugins: Arc<RwLock<PluginMap>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    latency_comp: Arc<arc_swap::ArcSwap<crate::latency::LatencyComp>>,
    automation: Arc<arc_swap::ArcSwap<AutomationSnapshot>>,
    monitor_prod: Arc<Mutex<ringbuf::HeapProd<f32>>>,
    event_tx: Sender<AudioEvent>,
    /// Keeps `event_tx` connected, and lets a test read the echoes a
    /// handler emitted ([`EngineHandlerHarness::drain_events`]).
    event_rx: Receiver<AudioEvent>,
    cmd_tx_retry: Sender<AudioCommand>,
    /// Held only so `cmd_tx_retry` never reports disconnected; nothing
    /// drains it.
    _cmd_rx_retry: Receiver<AudioCommand>,
    state: HandlerState,
}

impl Default for EngineHandlerHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineHandlerHarness {
    /// A harness at the engine's cold-start state: no tracks, no clips,
    /// no plugins, every allocator at 1, nothing published.
    ///
    /// Unlike the real thread this adds no default track — a handler
    /// test says what it needs.
    pub fn new() -> Self {
        let (event_tx, event_rx) = crossbeam_channel::unbounded::<AudioEvent>();
        let (cmd_tx_retry, _cmd_rx_retry) = crossbeam_channel::unbounded::<AudioCommand>();
        let (live_midi_tx, _) = crossbeam_channel::unbounded();
        let (live_control_tx, _) = crossbeam_channel::unbounded();
        let (clock_tx, _) = crossbeam_channel::unbounded();

        // The monitor ring is never driven here; one frame of capacity
        // keeps the allocation trivial.
        let (prod, _cons) = ringbuf::HeapRb::<f32>::new(1).split();

        Self {
            shared: Arc::new(SharedState::default()),
            tracks: Arc::new(RwLock::new(IndexMap::new())),
            busses: Arc::new(RwLock::new(IndexMap::new())),
            master: Arc::new(RwLock::new(MasterBus::new())),
            clips: Arc::new(RwLock::new(Vec::new())),
            midi_clips: Arc::new(RwLock::new(Vec::new())),
            plugins: Arc::new(RwLock::new(IndexMap::new())),
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
            latency_comp: Arc::new(arc_swap::ArcSwap::from_pointee(
                crate::latency::LatencyComp::empty(),
            )),
            automation: Arc::new(arc_swap::ArcSwap::from_pointee(
                AutomationSnapshot::default(),
            )),
            monitor_prod: Arc::new(Mutex::new(prod)),
            event_tx,
            event_rx,
            cmd_tx_retry,
            _cmd_rx_retry,
            state: HandlerState::new(48_000, live_midi_tx, live_control_tx, clock_tx),
        }
    }

    /// Build the borrowed [`HandlerCtx`] and hand it, with the mutable
    /// state, to a real handler.
    fn with_ctx<R>(&mut self, f: impl FnOnce(&HandlerCtx, &mut HandlerState) -> R) -> R {
        let ctx = HandlerCtx {
            shared: &self.shared,
            tracks: &self.tracks,
            busses: &self.busses,
            master: &self.master,
            clips: &self.clips,
            midi_clips: &self.midi_clips,
            plugins: &self.plugins,
            tempo_map: &self.tempo_map,
            latency_comp: &self.latency_comp,
            automation: &self.automation,
            monitor_prod: &self.monitor_prod,
            event_tx: &self.event_tx,
            cmd_tx_retry: &self.cmd_tx_retry,
            sample_rate: 48_000,
            buf_frames: 128,
            quantum: 128,
        };
        f(&ctx, &mut self.state)
    }

    /// Put `group` into the authoritative take-group store and publish
    /// the resulting comp table — the state a finished cycle-record pass
    /// leaves behind (`transport::finalize_loop_record_pass`), or a
    /// project load via `RestoreTakeGroups`.
    pub fn seed_take_group(&mut self, group: TakeGroup) {
        self.state.take_groups.insert(group.id, group);
        self.with_ctx(|ctx, state| takes::publish_take_comp(ctx, state));
    }

    /// Set the take-group id allocator, as capturing `n` groups would.
    pub fn set_next_take_group_id(&mut self, id: TakeGroupId) {
        self.state.next_take_group_id = id;
    }

    /// What `AudioCommand::SetProjectDir` does: reserve the clip ids of
    /// the dir's WAVs (code review STATE-08), then point the engine there.
    pub fn set_project_dir(&mut self, dir: std::path::PathBuf) {
        crate::engine::clips::reserve_clip_ids_in_project_dir(&mut self.state, &dir);
        self.state.project_dir = Some(dir);
    }

    /// Run the real `AudioCommand::ImportClip` handler: queues a decode on
    /// the import worker, which lands the clip asynchronously.
    pub fn import_clip(&mut self, track_id: TrackId, path: String, start_sample: u64) {
        self.with_ctx(|ctx, state| {
            crate::engine::clips::handle_import_clip(ctx, state, track_id, path, start_sample)
        });
    }

    /// Run the real `AudioCommand::ClearAll` handler.
    pub fn clear_all(&mut self) {
        self.with_ctx(tracks::handle_clear_all);
    }

    /// Ids currently in the engine's authoritative take-group store,
    /// ascending.
    pub fn take_group_ids(&self) -> Vec<TakeGroupId> {
        let mut ids: Vec<TakeGroupId> = self.state.take_groups.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// The take-group id allocator's current value.
    pub fn next_take_group_id(&self) -> TakeGroupId {
        self.state.next_take_group_id
    }

    /// The comp table the audio thread and the offline bounce actually
    /// read — `SharedState::take_comp`, not the control-thread store.
    pub fn published_comp_table(&self) -> Arc<CompRenderTable> {
        self.shared.take_comp.load_full()
    }

    /// The callback-visible state the handlers publish into — for a test
    /// that inspects the retire queue (code review MIX-04) or the
    /// contention counters (ARCH-02 A2-1).
    pub fn shared(&self) -> &SharedState {
        &self.shared
    }

    /// Run the real `AudioCommand::RemoveTrack` handler.
    pub fn remove_track(&mut self, track_id: TrackId) {
        self.with_ctx(|ctx, state| tracks::handle_remove_track(ctx, state, track_id));
    }

    /// Run the real `SetTrackFrozenSource` handler.
    pub fn set_track_frozen_source(&mut self, track_id: TrackId, source: Option<FrozenSource>) {
        self.with_ctx(|ctx, _| tracks::handle_set_track_frozen_source(ctx, track_id, source));
    }

    /// Insert `track` into the live track table, as `AddTrack` would.
    pub fn push_track(&mut self, track: Track) {
        self.tracks.write().insert(track.id, track);
    }

    /// The frozen source the callback would read for `track_id`.
    pub fn frozen_source(&self, track_id: TrackId) -> Option<Arc<FrozenSource>> {
        self.tracks.read().get(&track_id)?.frozen_source.load_full()
    }

    /// Replay one command from a captured project-load command stream
    /// through the engine's **real** handler; reports whether this harness
    /// understood it.
    ///
    /// Deliberately partial, and that is the point. A project load emits
    /// the whole command stream — tracks, plugins, tempo, timeline clips —
    /// and running all of it headlessly would drag CLAP into a take-lane
    /// test. Only the take-lane restore is dispatched here, so a test can
    /// hand over the entire captured stream and end up with an engine
    /// whose take state was built **by the load alone**, from cold, with no
    /// capture anywhere in its history. Anything the comp still renders
    /// then, it renders because the load put it there.
    pub fn replay_take_lane_command(&mut self, cmd: &AudioCommand) -> bool {
        match cmd {
            AudioCommand::RestoreTakeGroups { groups } => {
                let groups = groups.clone();
                self.with_ctx(|ctx, state| takes::handle_restore_take_groups(ctx, state, groups));
                true
            }
            AudioCommand::LoadTakeClipFromWav {
                clip_id,
                track_id,
                start_sample,
                path,
                name,
            } => {
                let (clip_id, track_id, start_sample) = (*clip_id, *track_id, *start_sample);
                let (path, name) = (path.clone(), name.clone());
                self.with_ctx(|ctx, state| {
                    crate::engine::clips::handle_load_take_clip_from_wav(
                        ctx,
                        state,
                        clip_id,
                        track_id,
                        start_sample,
                        path,
                        name,
                    )
                });
                true
            }
            _ => false,
        }
    }

    /// Run the real `AudioCommand::LoadClipFromWav` handler — the
    /// **timeline** clip load, which shares `submit_clip_load`'s worker
    /// with the take-clip load beside it.
    ///
    /// Here so a test can check that the take-clip park (ba todo #1403)
    /// stays out of the ordinary clip path's way: the park's interlock in
    /// the worker is deliberately unconditional, so an ordinary load that
    /// happened to reuse a stale claim's id would vanish.
    pub fn load_clip_from_wav(
        &mut self,
        clip_id: ClipId,
        track_id: TrackId,
        start_sample: u64,
        path: std::path::PathBuf,
        name: String,
    ) {
        self.with_ctx(|ctx, state| {
            crate::engine::clips::handle_load_clip_from_wav(
                ctx,
                state,
                clip_id,
                track_id,
                start_sample,
                path,
                name,
                0,
                0,
            )
        });
    }

    /// Block until the engine's clip list holds `expected` clips, or
    /// `timeout` elapses; reports whether it got there.
    ///
    /// Clip loading is handed to a worker (`ImportQueue`) so the control
    /// thread never blocks on an mmap, which means "the command was
    /// dispatched" and "the clip is in the list" are two different moments.
    /// A test that renders without waiting would race the worker and read
    /// silence for reasons that have nothing to do with what it asserts.
    pub fn wait_for_clips(&self, expected: usize, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if self.clips.read().len() >= expected {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Move the engine's audio clips out of the harness, for handing to a
    /// renderer.
    ///
    /// Draining rather than copying because [`AudioClip`] is deliberately
    /// not `Clone` (a mapped source is shared through an `Arc`, an in-RAM
    /// one would be duplicated wholesale). A test that needs to render the
    /// same restored state twice — live and bounced — rebuilds the harness
    /// twice, which is also the more honest reload.
    pub fn take_clips(&mut self) -> Vec<AudioClip> {
        std::mem::take(&mut *self.clips.write())
    }

    // -- take removal (ba todo #1397) ------------------------------------

    /// Push a recorded take's clip into the shared clip list, where
    /// `roll_audio_pass` puts it as a cycle-record pass rolls. A take group
    /// on its own proves nothing about audibility: the clip is what plays.
    pub fn push_clip(&mut self, clip: AudioClip) {
        self.clips.write().push(clip);
    }

    /// Clip ids in the shared clip list — the render's actual input —
    /// ascending.
    pub fn clip_ids(&self) -> Vec<ClipId> {
        let mut ids: Vec<ClipId> = self.clips.read().iter().map(|c| c.id).collect();
        ids.sort_unstable();
        ids
    }

    /// The clip-id allocator's current value — the reservation `#1393` and
    /// the clip loads both raise.
    pub fn next_clip_id(&self) -> ClipId {
        self.state.next_clip_id
    }

    /// Clip ids whose *recording* is currently parked out of the render
    /// because their take was removed, ascending.
    ///
    /// A removal that raced the take's still-in-flight load leaves a claim
    /// rather than a recording (ba todo #1403); the claim is deliberately
    /// not reported here, so this stays the answer to "what audio did the
    /// removal take out of the render", and an id shows up the moment the
    /// worker delivers into the park.
    pub fn parked_clip_ids(&self) -> Vec<ClipId> {
        self.state.take_clip_park.held_ids()
    }

    /// A group in the authoritative store, for asserting on its takes,
    /// comp and active take.
    pub fn take_group(&self, group_id: TakeGroupId) -> Option<&TakeGroup> {
        self.state.take_groups.get(&group_id)
    }

    /// Run the real `AudioCommand::SetTakeComp` handler.
    pub fn set_take_comp(&mut self, group_id: TakeGroupId, segments: Vec<CompSegment>) {
        self.with_ctx(|ctx, state| takes::handle_set_take_comp(ctx, state, group_id, segments));
    }

    /// Run the real `AudioCommand::SetActiveTake` handler.
    pub fn set_active_take(&mut self, group_id: TakeGroupId, take_id: Option<TakeId>) {
        self.with_ctx(|ctx, state| takes::handle_set_active_take(ctx, state, group_id, take_id));
    }

    /// Run the real `AudioCommand::RemoveTake` handler.
    pub fn remove_take(&mut self, group_id: TakeGroupId, take_id: TakeId) {
        self.with_ctx(|ctx, state| takes::handle_remove_take(ctx, state, group_id, take_id));
    }

    /// Run the real `AudioCommand::RemoveTakeGroup` handler.
    pub fn remove_take_group(&mut self, group_id: TakeGroupId) {
        self.with_ctx(|ctx, state| takes::handle_remove_take_group(ctx, state, group_id));
    }

    /// Run the real `AudioCommand::RestoreTakeGroups` handler — the whole
    /// undo/redo path for a take-lane edit.
    pub fn restore_take_groups(&mut self, groups: Vec<TakeGroup>) {
        self.with_ctx(|ctx, state| takes::handle_restore_take_groups(ctx, state, groups));
    }

    // -- MIDI note edits (code review VIEW-02 / CTL-02) ------------------

    /// Push a MIDI clip into the shared MIDI clip list.
    pub fn push_midi_clip(&mut self, clip: MidiClip) {
        self.midi_clips.write().push(clip);
    }

    /// The engine's notes for `clip_id`, in stored order.
    pub fn midi_notes(&self, clip_id: ClipId) -> Vec<MidiNote> {
        self.midi_clips
            .read()
            .iter()
            .find(|c| c.id == clip_id)
            .map(|c| c.notes.clone())
            .unwrap_or_default()
    }

    /// Run the real `MoveMidiNote` / `ResizeMidiNote` /
    /// `SetMidiNoteVelocity` handler for `cmd`; any other command is
    /// ignored and returns `false`.
    pub fn replay_midi_note_command(&mut self, cmd: &AudioCommand) -> bool {
        use crate::engine::midi;
        match *cmd {
            AudioCommand::MoveMidiNote { clip_id, note_index, new_start_tick, new_note } => {
                self.with_ctx(|ctx, _| {
                    midi::handle_move_midi_note(ctx, clip_id, note_index, new_start_tick, new_note)
                });
            }
            AudioCommand::ResizeMidiNote { clip_id, note_index, new_duration_ticks } => {
                self.with_ctx(|ctx, _| {
                    midi::handle_resize_midi_note(ctx, clip_id, note_index, new_duration_ticks)
                });
            }
            AudioCommand::SetMidiNoteVelocity { clip_id, note_index, velocity } => {
                self.with_ctx(|ctx, _| {
                    midi::handle_set_midi_note_velocity(ctx, clip_id, note_index, velocity)
                });
            }
            _ => return false,
        }
        true
    }

    /// Every echo the handlers have emitted since the last drain.
    pub fn drain_events(&mut self) -> Vec<AudioEvent> {
        self.event_rx.try_iter().collect()
    }

    /// Run the real `AudioCommand::Play` handler.
    pub fn play(&mut self) {
        self.with_ctx(|ctx, state| transport::handle_play(ctx, state));
    }

    /// Run the real `AudioCommand::Record { precount_bars }` handler.
    pub fn record(&mut self, precount_bars: u8) {
        self.with_ctx(|ctx, state| transport::handle_record(ctx, state, precount_bars));
    }

    /// Run the real `AudioCommand::Stop` handler.
    pub fn stop(&mut self) {
        self.with_ctx(|ctx, state| transport::handle_stop(ctx, state));
    }

    /// The transport flag the audio callback reads.
    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Whether the mixer is in its count-in branch (a `Record` with a
    /// precount arms it).
    pub fn count_in_active(&self) -> bool {
        self.shared
            .count_in_active
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Hold the real offline-render gate over this engine, as an export /
    /// bounce / freeze / stem worker does for the length of its render.
    pub fn hold_offline_render(&self) -> OfflineRenderGuard {
        OfflineRenderGuard::mark(&self.shared)
    }

    /// Render one block of `track_id` through the **real** `render_block`,
    /// from the engine's own clip list and published comp table, and return
    /// the left channel.
    ///
    /// This is the only way to ask what a take-lane command actually did to
    /// what the user hears: the clip phase skips governed clips, the comp
    /// phase renders the resolved spans, and a take that has fallen out of
    /// both shows up here as silence — or, if a removal forgot to park its
    /// recording, as a raw pass playing at full gain.
    pub fn render_track(&self, track_id: TrackId, playhead: u64, frames: usize) -> Vec<f32> {
        let table = self.published_comp_table();
        let clips = self.clips.read();
        let out = crate::mixer::render_take_comp_borrowed_for_test(
            vec![Track::new(track_id, "harness".into())],
            &clips,
            &table,
            playhead,
            frames,
            48_000,
            false,
        );
        out.chunks(2).map(|frame| frame[0]).collect()
    }
}
