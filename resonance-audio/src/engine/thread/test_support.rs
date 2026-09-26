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
    automation::AutomationSnapshot, busses, master, plugins, takes, tracks, transport,
    OfflineRenderGuard, SharedState,
};
use crate::midi_clock::MidiClockEvent;
use crate::midi_hardware::{LiveControlEvent, LiveMidiEvent};
use crate::mixer::CompRenderTable;
use crate::types::*;

use super::{engine_thread, EngineThreadParams, HandlerCtx, HandlerState};

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
    /// Test-only convenience counter for [`Self::import_audio_to_pool`]'s
    /// `Vec<String>` shorthand (D-7a moved asset-id allocation to the app,
    /// so the engine's own `HandlerState` has nowhere left to keep one).
    /// Never reset, so repeated calls on one harness never reissue an id.
    next_test_asset_id: AssetId,
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
    /// Same as the real thread's cold start since FU-D4a (it no longer
    /// adds a default track unprompted either) — this harness just gets
    /// there by rebuilding [`HandlerState::new`] directly rather than
    /// running [`engine_thread`](super::engine_thread) end to end. A
    /// handler test says what it needs; [`Self::startup_events`] is the
    /// one that actually runs the real thread's startup.
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
            next_test_asset_id: 1,
        }
    }

    /// Spawn the REAL [`engine_thread`] — not this harness's piecemeal
    /// per-handler dispatch — with fake channels and no audio device,
    /// queue it a `ShutDown` before it starts, and return every event it
    /// emitted before exiting.
    ///
    /// The one hermetic way to observe what runs before the command loop
    /// ever reads a command: `engine_thread`'s startup section used to
    /// unprompted-insert a default track there (FU-D4a), a step no
    /// per-handler harness call could ever exercise, since that code
    /// doesn't live in any handler. `ShutDown` is queued on the command
    /// channel before the thread is spawned (an unbounded channel, so
    /// order of send-vs-spawn doesn't matter), so the loop's first
    /// `recv_timeout` sees it and the thread exits right after the
    /// one-time startup section runs; a real hang here would be a
    /// correctness bug in `engine_thread` itself, worth `join`'s panic
    /// rather than a silently-skipped test.
    pub fn startup_events() -> Vec<AudioEvent> {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<AudioCommand>();
        let cmd_tx_retry = cmd_tx.clone();
        let (event_tx, event_rx) = crossbeam_channel::unbounded::<AudioEvent>();
        let (live_midi_tx, _live_midi_rx) = crossbeam_channel::bounded::<LiveMidiEvent>(8);
        let (_live_midi_fwd_tx, live_midi_fwd_rx) = crossbeam_channel::bounded::<LiveMidiEvent>(8);
        let (live_control_tx, live_control_rx) = crossbeam_channel::bounded::<LiveControlEvent>(8);
        let (clock_tx, clock_rx) = crossbeam_channel::bounded::<MidiClockEvent>(8);
        let (prod, _cons) = ringbuf::HeapRb::<f32>::new(1).split();

        let _ = cmd_tx.send(AudioCommand::ShutDown);

        let params = EngineThreadParams {
            cmd_rx,
            cmd_tx_retry,
            event_tx,
            shared: Arc::new(SharedState::default()),
            tracks_arc: Arc::new(RwLock::new(IndexMap::new())),
            busses_arc: Arc::new(RwLock::new(IndexMap::new())),
            master_arc: Arc::new(RwLock::new(MasterBus::new())),
            clips_arc: Arc::new(RwLock::new(Vec::new())),
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
            plugins_arc: Arc::new(RwLock::new(IndexMap::new())),
            latency_comp: Arc::new(arc_swap::ArcSwap::from_pointee(
                crate::latency::LatencyComp::empty(),
            )),
            automation: Arc::new(arc_swap::ArcSwap::from_pointee(
                AutomationSnapshot::default(),
            )),
            monitor_prod: Arc::new(Mutex::new(prod)),
            live_midi_tx,
            live_midi_fwd_rx,
            live_control_tx,
            live_control_rx,
            clock_tx,
            clock_rx,
            sample_rate: 48_000,
            buf_frames: 128,
            quantum: 128,
        };

        let handle = std::thread::Builder::new()
            .name("fu-d4a-engine-startup-test".into())
            .spawn(move || engine_thread(params))
            .expect("spawn engine_thread for FU-D4a startup test");
        handle
            .join()
            .expect("engine_thread panicked in FU-D4a startup test");

        event_rx.try_iter().collect()
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

    /// What the `SetProjectDir` handler itself does (FU-M12b): start the
    /// reservation scan on its worker and return; the reservation lands
    /// at the next clip-id allocation (or engine-loop poll).
    pub fn set_project_dir_async(&mut self, dir: std::path::PathBuf) {
        crate::engine::clips::start_clip_id_scan(&mut self.state, &dir);
        self.state.project_dir = Some(dir);
    }

    /// Run the real `AudioCommand::ImportAudioToPool` handler: spawns the
    /// pool-import worker, whose events arrive asynchronously.
    ///
    /// D-7a: the real app allocates each file's asset id before sending the
    /// command; this shorthand plays that part for a test, handing out
    /// [`Self::next_test_asset_id`] in order so repeated calls on one
    /// harness never collide. Use [`Self::import_audio_to_pool_with_ids`]
    /// when a test needs to name specific ids (e.g. to provoke a
    /// collision).
    pub fn import_audio_to_pool(&mut self, paths: Vec<String>) {
        let files = paths
            .into_iter()
            .map(|path| {
                let asset_id = self.next_test_asset_id;
                self.next_test_asset_id += 1;
                PoolImportFile { asset_id, path }
            })
            .collect();
        self.import_audio_to_pool_with_ids(files);
    }

    /// [`Self::import_audio_to_pool`] with the caller naming each file's
    /// asset id explicitly.
    pub fn import_audio_to_pool_with_ids(&mut self, files: Vec<PoolImportFile>) {
        self.with_ctx(|ctx, state| {
            crate::engine::import_pool::handle_import_audio_to_pool(ctx, state, files)
        });
    }

    /// Run the real `AudioCommand::SetBpm` handler.
    pub fn set_bpm(&mut self, bpm: f32) {
        self.with_ctx(|ctx, _| transport::handle_set_bpm(ctx, bpm));
    }

    /// The flat tempo of the published tempo map.
    pub fn published_bpm(&self) -> f32 {
        self.tempo_map.load().bpm
    }

    /// Run the real `AudioCommand::PersistClipWavs` handler (FU-V5b).
    pub fn persist_clip_wavs(&mut self) {
        self.with_ctx(|ctx, state| crate::engine::clips::handle_persist_clip_wavs(ctx, state));
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
        self.with_ctx(|ctx, state| {
            tracks::handle_set_track_frozen_source(ctx, state, track_id, source)
        });
    }

    /// Land freeze-cache conversions (FU-A4c): wait for every one in
    /// flight with `wait` (what an offline render does), else only the
    /// finished ones (the engine loop's poll).
    pub fn settle_frozen_conversions(&mut self, wait: bool) {
        self.with_ctx(|ctx, state| tracks::settle_frozen_conversions(ctx, state, wait));
    }

    /// Insert `track` into the live track table, as `AddTrack` would.
    pub fn push_track(&mut self, track: Track) {
        self.tracks.write().insert(track.id, track);
    }

    /// Run the real `MovePluginInMaster` handler (ARCH-05/C-1's
    /// `EngineError::not_found` guard: an instance id absent from the
    /// master chain).
    pub fn move_plugin_in_master(&mut self, instance_id: PluginInstanceId, to_index: usize) {
        self.with_ctx(|ctx, _state| {
            master::handle_move_plugin_in_master(ctx, instance_id, to_index)
        });
    }

    /// Run the real `AddTrack` handler (ARCH-04 D-4's
    /// `EngineErrorKind::Internal` guard: `id` already live refuses the
    /// add rather than replacing the track). The app allocates every
    /// track id now, so `id` is mandatory — no more `id_hint`.
    pub fn add_track(&mut self, id: TrackId, name: Option<String>) -> Vec<AudioEvent> {
        self.with_ctx(|ctx, _state| tracks::handle_add_track(ctx, id, name));
        self.drain_events()
    }

    /// The live track ids, in insertion order. Used to confirm a refused
    /// duplicate-id `AddTrack` left the registry exactly as it was.
    pub fn test_track_ids(&self) -> Vec<TrackId> {
        self.tracks.read().keys().copied().collect()
    }

    /// A live track's name, if it exists. Used to confirm a refused
    /// duplicate-id `AddTrack` did not rename the track it collided with.
    pub fn test_track_name(&self, id: TrackId) -> Option<String> {
        self.tracks.read().get(&id).map(|t| t.name.clone())
    }

    /// Run the real `AddBus` handler (ARCH-05/C-1's `EngineError::busy`
    /// guard: past `MAX_BUSSES`, the engine refuses rather than adding;
    /// ARCH-04 D-3's `EngineErrorKind::Internal` guard: `id` already live
    /// refuses the add rather than replacing the bus). The app allocates
    /// every bus id now, so `id` is mandatory — no more `id_hint`.
    pub fn add_bus(&mut self, id: BusId, name: Option<String>) -> Vec<AudioEvent> {
        self.with_ctx(|ctx, _state| busses::handle_add_bus(ctx, id, name));
        self.drain_events()
    }

    /// Run the real `AddAuxSend` handler (ARCH-04 D-2's
    /// `EngineErrorKind::Internal` guard: `id` already live refuses the
    /// add rather than editing the send it names).
    #[allow(clippy::too_many_arguments)]
    pub fn add_aux_send(
        &mut self,
        id: SendId,
        source: SendSource,
        dest: BusId,
        level_db: f32,
        pre_fader: bool,
        enabled: bool,
    ) -> Vec<AudioEvent> {
        self.with_ctx(|ctx, state| {
            busses::handle_add_aux_send(ctx, state, id, source, dest, level_db, pre_fader, enabled)
        });
        self.drain_events()
    }

    /// Run the real `SetAuxSend` handler — the edit-only twin of
    /// [`Self::add_aux_send`] (ARCH-04 D-2): a quiet no-op if `id` names
    /// no live send.
    #[allow(clippy::too_many_arguments)]
    pub fn set_aux_send(
        &mut self,
        id: SendId,
        source: SendSource,
        dest: BusId,
        level_db: f32,
        pre_fader: bool,
        enabled: bool,
    ) -> Vec<AudioEvent> {
        self.with_ctx(|ctx, state| {
            busses::handle_set_aux_send(ctx, state, id, source, dest, level_db, pre_fader, enabled)
        });
        self.drain_events()
    }

    /// The live aux-send ids, in insertion order. Used to confirm a
    /// refused duplicate-id add left the graph exactly as it was.
    pub fn aux_send_ids(&self) -> Vec<SendId> {
        self.state.aux_sends.keys().copied().collect()
    }

    /// The live bus ids, in insertion order. Used to confirm a refused
    /// duplicate-id `AddBus` left the registry exactly as it was.
    pub fn test_bus_ids(&self) -> Vec<BusId> {
        self.busses.read().keys().copied().collect()
    }

    /// A live bus's name, if it exists. Used to confirm a refused
    /// duplicate-id `AddBus` did not rename the bus it collided with.
    pub fn test_bus_name(&self, id: BusId) -> Option<String> {
        self.busses.read().get(&id).map(|b| b.name.clone())
    }

    /// Run the real `AddPlugin` handler (ARCH-04 D-1's
    /// `EngineErrorKind::Internal` guard: `id` already live in
    /// `ctx.plugins` refuses the add rather than replacing the instance).
    pub fn add_plugin(
        &mut self,
        track_id: TrackId,
        clap_file_path: String,
        clap_plugin_id: String,
        id: PluginInstanceId,
    ) {
        self.with_ctx(|ctx, state| {
            plugins::handle_add_plugin(ctx, state, track_id, clap_file_path, clap_plugin_id, id)
        });
    }

    /// Run the real `AddPluginToBus` handler — the bus twin of
    /// [`Self::add_plugin`], for the ARCH-04 D-1 guard that the
    /// duplicate-id refusal isn't a track-only special case.
    pub fn add_plugin_to_bus(
        &mut self,
        bus_id: BusId,
        clap_file_path: String,
        clap_plugin_id: String,
        id: PluginInstanceId,
    ) -> Vec<AudioEvent> {
        self.with_ctx(|ctx, state| {
            busses::handle_add_plugin_to_bus(
                ctx,
                state,
                bus_id,
                clap_file_path,
                clap_plugin_id,
                id,
            )
        });
        self.drain_events()
    }

    /// The plugin ids on `track_id`'s chain, in order — what `push_plugin`
    /// has appended. Used to confirm a refused duplicate-id add left the
    /// chain exactly as it was.
    pub fn track_plugin_ids(&self, track_id: TrackId) -> Vec<PluginInstanceId> {
        self.tracks
            .read()
            .get(&track_id)
            .map(|t| t.plugins().as_ref().clone())
            .unwrap_or_default()
    }

    /// How many live CLAP instances `ctx.plugins` holds, across every
    /// track/bus/master chain. Used to confirm a refused duplicate-id add
    /// did not insert (or replace) an instance.
    pub fn plugin_instance_count(&self) -> usize {
        self.plugins.read().len()
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

    /// Block until every job submitted to the clip-import pool so far
    /// (`LoadClipFromWav`, take-clip loads) has finished,
    /// so a test can assert that a wrong publish did *not* happen without
    /// a grace sleep that a slow machine outruns (FU-A5a).
    ///
    /// How: park one barrier job per worker. The queue is FIFO, so a
    /// worker only reaches a barrier after every earlier job has been
    /// dequeued, and a worker sitting in the barrier has finished its
    /// previous job — once every worker is in one, nothing earlier is
    /// still running. Panics after `timeout` (a hang guard, not pacing).
    pub fn settle_imports(&mut self, timeout: std::time::Duration) {
        use std::sync::{Condvar, Mutex as StdMutex};
        // (workers arrived, released)
        let gate = Arc::new((StdMutex::new((0usize, false)), Condvar::new()));
        for _ in 0..crate::engine::MAX_CONCURRENT_IMPORTS {
            let gate = Arc::clone(&gate);
            self.state
                .imports
                .submit(move || {
                    let (lock, cvar) = &*gate;
                    let mut g = lock.lock().unwrap();
                    g.0 += 1;
                    cvar.notify_all();
                    while !g.1 {
                        g = cvar.wait(g).unwrap();
                    }
                })
                .expect("spawn a clip-import worker");
        }
        // Every submit tops the pool up, so this is every worker there is.
        let workers = self.state.imports.worker_count();
        let (lock, cvar) = &*gate;
        let (mut g, wait) = cvar
            .wait_timeout_while(lock.lock().unwrap(), timeout, |g| g.0 < workers)
            .unwrap();
        // Release the workers either way, so a timeout doesn't strand them.
        g.1 = true;
        cvar.notify_all();
        assert!(
            !wait.timed_out(),
            "clip-import jobs still running after {timeout:?}"
        );
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

    /// Run the real `AudioCommand::CreateMidiClip` handler (a GUI-drawn
    /// clip). `clip_id` is mandatory since D-7c: the app allocates it and
    /// the engine only checks it against the live clip lists, refusing a
    /// collision with `EngineErrorKind::Internal` rather than inventing
    /// or reusing an id.
    pub fn create_midi_clip(
        &mut self,
        clip_id: ClipId,
        track_id: TrackId,
        start_sample: u64,
        duration_ticks: u64,
    ) -> Vec<AudioEvent> {
        self.with_ctx(|ctx, _state| {
            crate::engine::midi::handle_create_midi_clip(
                ctx,
                clip_id,
                track_id,
                start_sample,
                duration_ticks,
                "drawn".into(),
            )
        });
        self.drain_events()
    }

    /// A live MIDI clip's name, if it exists. Used to confirm a refused
    /// duplicate-id `CreateMidiClip` did not rename/replace the clip it
    /// collided with.
    pub fn midi_clip_name(&self, clip_id: ClipId) -> Option<String> {
        self.shared
            .graph
            .load()
            .midi_clip(clip_id)
            .map(|c| c.name.clone())
    }

    /// Run the real `AudioCommand::LoadMidiClipDirect` handler: a clip
    /// whose id the app chose (a derived clip, a replayed one).
    pub fn load_midi_clip_direct(&mut self, clip_id: ClipId, track_id: TrackId) {
        self.with_ctx(|ctx, state| {
            crate::engine::midi::handle_load_midi_clip_direct(
                ctx,
                state,
                clip_id,
                track_id,
                0,
                1920,
                Vec::new(),
                "direct".into(),
                0,
                0,
            )
        });
    }

    /// Ids in the shared MIDI clip list, ascending.
    pub fn midi_clip_ids(&self) -> Vec<ClipId> {
        let graph = self.shared.graph.load();
        let mut ids: Vec<ClipId> = graph.midi_clips.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        ids
    }

    /// Push a MIDI clip into the shared MIDI clip list (publishes a new
    /// render graph, as a clip handler does).
    pub fn push_midi_clip(&mut self, clip: MidiClip) {
        self.shared
            .edit_midi_clips(|clips| clips.push(Arc::new(clip)));
    }

    /// The engine's notes for `clip_id`, in stored order.
    pub fn midi_notes(&self, clip_id: ClipId) -> Vec<MidiNote> {
        self.shared
            .graph
            .load()
            .midi_clip(clip_id)
            .map(|c| c.notes.clone())
            .unwrap_or_default()
    }

    /// The render graph the audio callback would load right now.
    pub fn render_graph(&self) -> Arc<crate::engine::RenderGraph> {
        self.shared.graph.load_full()
    }

    /// Run the real `AudioCommand::AddMidiNote` handler.
    pub fn add_midi_note(&mut self, clip_id: ClipId, note: MidiNote) {
        self.with_ctx(|ctx, _| crate::engine::midi::handle_add_midi_note(ctx, clip_id, note));
    }

    /// Run the real `AudioCommand::SetMidiClipNotes` handler (the bulk
    /// replace the control API's note writes use).
    pub fn set_midi_clip_notes(&mut self, clip_id: ClipId, notes: Vec<MidiNote>) {
        self.with_ctx(|ctx, _| {
            crate::engine::midi::handle_set_midi_clip_notes(ctx, clip_id, notes)
        });
    }

    /// Run the real `AudioCommand::DeleteMidiClip` handler.
    pub fn delete_midi_clip(&mut self, clip_id: ClipId) {
        self.with_ctx(|ctx, _| crate::engine::midi::handle_delete_midi_clip(ctx, clip_id));
    }

    /// A second handle on the engine's shared state, for a thread that
    /// plays the audio callback's reader against the handlers.
    pub fn shared_arc(&self) -> Arc<SharedState> {
        Arc::clone(&self.shared)
    }

    /// Run the engine loop's retire sweep, as its 16 ms tick does.
    /// Returns how many replaced snapshots it dropped.
    pub fn sweep_retired(&self) -> usize {
        self.shared.retired.sweep()
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

    /// Run the real live-MIDI bookkeeping for `event` (record-into-clip
    /// when the track is armed and the transport rolls).
    pub fn live_midi_event(&mut self, event: crate::midi_hardware::LiveMidiEvent) {
        self.with_ctx(|ctx, state| crate::engine::midi::handle_live_midi_event(ctx, state, event));
    }

    /// Move the playhead the handlers read, as the audio callback does.
    pub fn set_playhead(&self, sample: u64) {
        self.shared
            .playhead
            .store(sample, std::sync::atomic::Ordering::SeqCst);
    }

    /// Every echo the handlers have emitted since the last drain.
    pub fn drain_events(&mut self) -> Vec<AudioEvent> {
        self.event_rx.try_iter().collect()
    }

    /// Drop the harness and return every event still to come, once every
    /// worker a handler spawned (a pool-import batch, an import job) has
    /// exited: each holds a clone of the event sender, so the channel
    /// disconnects exactly when the last of them is done. The explicit
    /// replacement for "drain, sleep a grace period, drain again" — a
    /// late event can't arrive after this returns.
    ///
    /// Panics if the workers are still running after `timeout`: a safety
    /// net against a wedged worker hanging the suite, not a pacing knob,
    /// so pass something far above the work's worst case.
    pub fn finish_and_drain_events(self, timeout: std::time::Duration) -> Vec<AudioEvent> {
        let event_rx = self.event_rx.clone();
        drop(self);
        let deadline = std::time::Instant::now() + timeout;
        let mut events = Vec::new();
        loop {
            match event_rx.recv_deadline(deadline) {
                Ok(ev) => events.push(ev),
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return events,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    panic!("engine workers still running after {timeout:?}; events so far: {events:?}")
                }
            }
        }
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

    /// Run the real `AudioCommand::MeasureMix` spawn path: one offline
    /// (`Render`) measurement of the master, on its worker thread.
    pub fn measure_master(&mut self, measure_id: u64) {
        self.with_ctx(|ctx, _| {
            crate::engine::bounce::measure_mix_spawn(
                measure_id,
                vec![StemSource::Master],
                None,
                MeasureSource::Render,
                Arc::clone(ctx.shared),
                Arc::clone(ctx.tracks),
                Arc::clone(ctx.busses),
                Arc::clone(ctx.master),
                Arc::clone(ctx.clips),
                Arc::clone(ctx.plugins),
                Arc::clone(ctx.tempo_map),
                ctx.sample_rate,
                ctx.event_tx.clone(),
            )
        });
    }

    /// The engine's clip list, so a test can hold its lock to park a
    /// worker that reads it.
    pub fn clips_lock(&self) -> Arc<RwLock<Vec<AudioClip>>> {
        Arc::clone(&self.clips)
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
