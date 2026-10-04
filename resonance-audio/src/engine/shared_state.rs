//! [`SharedState`]: everything the engine control thread and the audio
//! callback share — the transport atomics, the render graph slot, the
//! wait-free snapshot tables and the retire queue. Split out of
//! `engine/mod.rs` so that file re-exports and nothing else (ARCH2-12).

use indexmap::IndexMap;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, AtomicU8};
use std::sync::Arc;

use crate::clap_host::PluginMap;
use crate::types::*;

use super::{audition, bounce, count_in_arm, internal, loop_range, reference, render_graph, retire};

/// Shared state between the engine control thread and the audio callback.
/// `pub` (not `pub(crate)`) only so `test_support` can re-export it for
/// integration tests; the `engine` module itself stays private.
pub struct SharedState {
    /// Current playhead position in sample frames.
    pub playhead: AtomicU64,
    /// Whether playback is active.
    pub playing: AtomicBool,
    /// Whether recording is active.
    pub recording: AtomicBool,
    /// Whether any track is monitoring input.
    pub monitoring: AtomicBool,
    /// Master volume as linear gain (AtomicU32 bit-punned f32).
    pub master_volume_bits: AtomicU32,
    /// Master volume the previous audio block ended on (bit-punned
    /// f32). Audio thread only; the master pass ramps from this to
    /// `master_volume_bits` per sample to avoid zipper noise.
    pub master_last_volume_bits: AtomicU32,
    /// Master peak level L (AtomicU32 bit-punned f32), for VU meters.
    pub master_peak_l_bits: AtomicU32,
    /// Master peak level R (AtomicU32 bit-punned f32), for VU meters.
    pub master_peak_r_bits: AtomicU32,
    /// When bypassed, the mixer skips the master FX chain (everything in
    /// `MasterBus::plugin_ids`); fader + peak metering are unaffected.
    /// Toggling it crossfades over a few milliseconds, so a mastering
    /// chain can be A/B-ed mid-playback without a click.
    pub master_fx_bypass: crate::bypass::BypassFade,
    /// Count of whole input frames (at the capture device's rate) the
    /// capture callbacks discarded because the recording ring was full,
    /// since the current record take began. The RT producers
    /// (`platform`, `input_pipewire`) `fetch_add`; the engine control
    /// thread reads it alongside the recording drain and raises
    /// `AudioEvent::RecordingOverflow` once per take
    /// (`RecordingState::poll_overflow`); a new take zeroes it
    /// (`RecordingState::begin_overflow_episode`).
    pub recording_overflow: AtomicU64,
    /// Channel count of the currently-active input stream, or 0 when
    /// no stream is open. Used by the mix callback to de-interleave
    /// per-track monitor audio from a multi-channel input device.
    pub input_channels: AtomicU16,
    /// The loop (cycle) range, one published value so a block can never
    /// see half of a move (code review RT-13). Read with
    /// [`SharedState::loop_range`], written with
    /// [`SharedState::set_loop_range`].
    pub(crate) loop_range: arc_swap::ArcSwap<loop_range::LoopRange>,
    /// True while a record-with-count-in is in flight. The mixer uses
    /// this to pick its count-in branch (hold the playhead, skip
    /// track/clip rendering, render metronome ticks and monitoring).
    /// The engine control thread clears it after opening the
    /// recording stream so normal playback can resume on the next
    /// buffer.
    pub count_in_active: AtomicBool,
    /// Count-in frames remaining before the last click fires. When
    /// this hits zero while `count_in_active` is still set, the mixer
    /// stops emitting metronome ticks but keeps holding the playhead
    /// until the control thread has opened the recording stream.
    pub count_in_remaining: AtomicU64,
    /// Total count-in frames at the moment count-in was armed. Used
    /// with `count_in_remaining` to derive elapsed frames for beat
    /// alignment inside the mixer's count-in branch.
    pub count_in_total: AtomicU64,
    /// Count-in → record hand-off (code review RT-08), one of
    /// [`count_in_arm`]'s states. The engine thread opens the recording
    /// session when the count-in starts and sets `ARMED`; the audio
    /// thread, in the block whose frames finish the count-in, moves it
    /// `ARMED → FIRING`, starts the playhead at the exact frame the
    /// count-in ends, sets `recording`, leaves the count-in branch and
    /// publishes `FIRED`. Stop / Pause settle it first
    /// (`transport::settle_count_in_arm`), so a take is either fully
    /// started or never started.
    pub count_in_record_arm: AtomicU8,
    /// How many offline renders are running right now (ba todo #1218).
    ///
    /// Bounce / export / freeze / stem export / mix measurement all run on
    /// worker threads and all drive the SAME live CLAP plugin instances
    /// through `render_chunk`, resetting them at the start of a render.
    /// Two of them in flight at once interleave `process()` / `reset()`
    /// calls on those shared instances and corrupt both outputs, exactly
    /// as rendering during playback would.
    ///
    /// Every offline renderer publishes itself here for the duration of
    /// its render via `bounce::OfflineRenderGuard`. The *measurement*
    /// command additionally requires the count to be zero before it starts
    /// (`OfflineRenderGuard::try_acquire_exclusive`) and reports a busy
    /// error otherwise — it is a read-only query, so refusing costs the
    /// caller nothing and it must never disturb a render that is producing
    /// a file. The file-producing renderers keep their existing behaviour
    /// (the app serialises them through its export/freeze modals, and its
    /// bounce / freeze / export START paths refuse while an offline
    /// measurement is live — `Resonance::offline_measure_in_progress` —
    /// so the exclusion holds in both directions); making the renderers
    /// refuse each other engine-side is a behaviour change for another
    /// todo.
    ///
    /// It is also the gate between an offline render and the *live*
    /// callback (code review MIX-02 / ENG-05): while it is non-zero the
    /// audio callback outputs silence and holds the transport instead of
    /// touching a plugin, and Play / Record / realtime bounce / MIDI-clock
    /// start refuse — see [`Self::offline_render_active`].
    pub offline_render_count: AtomicU32,
    /// Whether the audio callback is inside a block right now, and how
    /// many it has run: raising the gate above waits on it, so a callback
    /// that passed the gate an instant before cannot still be processing
    /// the live plugins once the render starts (code review RT-09).
    pub callback_activity: bounce::CallbackActivity,
    /// External-instrument round-trip offsets per track
    /// (`latency_offset_samples`, positive = the hardware return
    /// arrives that late). Published by the engine control thread
    /// whenever the comp table is refreshed and read by the offline
    /// bounce/export threads so their latency comp folds the same
    /// offsets the live mixer compensates (doc #260 finding #4).
    pub external_offsets: arc_swap::ArcSwap<std::collections::HashMap<TrackId, i64>>,
    /// Capture-side I/O latency in samples at the engine rate — the
    /// graph-reported time a sample takes from the capture device to
    /// the native input stream (`pw_time.delay`). Published every graph
    /// cycle by the input process callback; 0 when no native input
    /// stream is running (doc #260 finding #13).
    pub capture_latency_samples: AtomicU64,
    /// Playback-side I/O latency in samples at the engine rate — the
    /// graph-reported time the next output sample takes from the native
    /// output stream to the playback device. Published every graph
    /// cycle by the output process callback; 0 on the cpal fallback
    /// (which cannot report it — part of why it is a fallback).
    pub playback_latency_samples: AtomicU64,
    /// Latched true by the output backend's error callback when the
    /// output stream has died under a live engine — the sink vanished
    /// (USB interface unplugged) or the audio server restarted. The
    /// engine *thread* survives this, so the app's engine-death check
    /// (`AudioEngine::is_disconnected`) never fires: the transport
    /// appears to run and edits still ack while nothing is audible and
    /// recording captures nothing. The app polls this flag instead
    /// (via [`AudioEngine::output_stream_lost`]) to raise a persistent
    /// banner. The native PipeWire backend clears it again when a
    /// healthy `Streaming` state change follows the error (the graph
    /// revived the stream); the cpal fallback's error callback reports
    /// no recovery transition, so on that backend it stays set until
    /// the app restarts.
    pub output_stream_lost: AtomicBool,
    /// True from the moment a recording session arms its flags until
    /// the input callback pushes the session's first frames. That push
    /// latches the take's aligned start position into
    /// `recording_start_latch` (see [`SharedState::latch_recording_start`]),
    /// eliminating the stream-open / engine-cadence variance that used
    /// to land inside takes (doc #260 finding #2).
    pub recording_start_pending: AtomicBool,
    /// The raw playhead latched at the session's first captured frame.
    /// The engine loop turns it into the take's aligned start (minus
    /// measured I/O latency for performer sessions), so recorded audio
    /// lands where the performer heard the mix.
    pub recording_start_latch: AtomicU64,
    /// Master-chain latency in samples (0 while master FX are
    /// bypassed), published by the engine thread's comp refresh. The
    /// audio callback reads it to latency-match the reference A/B
    /// monitor against the PDC-delayed, master-processed mix
    /// (doc #260 finding #19).
    pub master_latency_samples: AtomicU64,
    /// Smoothed per-cycle DSP load as a fraction of the cycle budget
    /// (AtomicU32 bit-punned f32), published every mix call by
    /// [`crate::cycle_load::CycleLoadMeter`]. ~0.01 means the mixer
    /// uses 1% of its realtime budget; ≥1.0 means it outran a cycle.
    pub dsp_load_ema_bits: AtomicU32,
    /// Highest single-cycle DSP load in the current report window
    /// (bit-punned f32). Reset each time the meter emits a summary.
    pub dsp_load_peak_bits: AtomicU32,
    /// Lifetime count of cycles whose mix call outran the cycle budget
    /// — each one is a guaranteed audible xrun regardless of what the
    /// graph reports.
    pub dsp_overrun_cycles: AtomicU64,
    /// Lifetime count of cycles where a monitoring mix read fewer
    /// monitor frames than it needed (the input stream's push for the
    /// cycle hadn't landed) — a quantum of dropped live input each,
    /// audible as a click/stutter in the monitored signal only.
    /// Counted by `mix_audio`, folded into the load meter's report.
    pub monitor_shortfall_cycles: AtomicU64,
    /// The load meter's report hand-off to the engine loop, which
    /// formats and prints it — never the audio thread.
    pub cycle_report: crate::cycle_load::CycleReportSlot,
    /// The callback's one-shot oversize-buffer warning, logged by the
    /// engine loop — never the audio thread (code review ARCH-05 A5-2).
    pub oversize_buffer: crate::cycle_load::OversizeBufferLatch,
    /// The live render pool's status, for the engine loop to report
    /// (`AudioEvent::RenderThreads`). Set when an output stream is built;
    /// engine side only.
    pub(crate) render_pool: parking_lot::Mutex<Option<crate::render_pool::PoolMonitor>>,
    /// The cpal output / input streams' error callbacks, counted on the
    /// audio thread and logged by the engine loop (code review FU-H6b).
    pub output_stream_errors: crate::stream_errors::StreamErrorLatch,
    pub input_stream_errors: crate::stream_errors::StreamErrorLatch,
    /// Plugins an offline render's reset took down: active before, and
    /// neither the stop/start cycle nor a full reactivation brought them
    /// back (FU-M8b). Pushed by the bounce workers, drained and reported
    /// by the engine loop — each instance once, since a plugin already
    /// inactive at the next render is not pushed again.
    pub plugins_dead_after_reset: parking_lot::Mutex<Vec<crate::types::PluginInstanceId>>,
    /// Replaced snapshots kept alive until the engine loop's sweep finds
    /// no reader pinning them (code review MIX-04 / ARCH-02 A2-2). Every
    /// `ArcSwap` the callback reads is published through
    /// `retire::publish`; the audio thread never touches this queue.
    pub retired: retire::Retired,
    /// The immutable render graph (code review ARCH-02 A2-4): built and
    /// published by the engine thread, `load()`ed once per block by the
    /// callback and once per chunk by the offline renderers. Holds every
    /// project map the renderers read: MIDI clips (B-1), busses and the
    /// master chain (B-2), tracks (B-3), plugin instances (B-4) and audio
    /// clips (B-5).
    pub graph: render_graph::RenderGraphSlot,
    /// Worker results waiting for the engine thread to apply them (code
    /// review ARCH-02 B-5): a finished clip load, a pitch analysis, a
    /// bounced clip, an offline render's retune caches. Workers post; only
    /// the engine loop drains — see [`internal`]. The audio thread never
    /// touches it.
    pub(crate) inbox: internal::EngineInbox,
    /// Latched true while any chain latency exceeds `MAX_COMP_LATENCY`
    /// (the comp clamp is engaging and alignment for that chain is
    /// degraded). Used to emit the warning once per engagement instead
    /// of on every comp refresh (doc #260 finding #20).
    pub comp_clamp_engaged: AtomicBool,
    /// Reference A/B monitor snapshot. Published by the control thread
    /// (`reference::ReferencePlayer::publish`) and read lock-free by the
    /// audio callback to replace the post-master output with the active
    /// reference's PCM. Never consulted by any offline/realtime bounce
    /// path, so exports always render the processed mix.
    pub reference: reference::ReferenceMonitor,
    /// Latest processed-mix loudness/peak/range snapshot, published by the
    /// audio callback's mix metering tap each block and read lock-free by
    /// the control thread to answer `PollABMeters`. Holds its last value
    /// while the mix isn't playing (e.g. while auditioning a reference).
    pub mix_meter: resonance_metering::AtomicMeterSnapshot,
    /// Latest active-reference loudness/peak/range snapshot, published by
    /// the audio callback's reference metering tap while a reference is
    /// auditioned (post loudness-match/trim gain). Forwarded to the UI only
    /// when a reference is active; see `reference::handle_poll_ab_meters`.
    pub ref_meter: resonance_metering::AtomicMeterSnapshot,
    /// Lock-free snapshot of the engine's aux-send table, published by
    /// the control thread on every send add/remove/clear and read once
    /// per block by the live mixer and the offline bounce renderer. The
    /// authoritative table lives on the control thread
    /// (`HandlerState::aux_sends`); this is the audio-thread-visible copy
    /// so the render path needs no lock. Empty until the first send is
    /// created, so projects without sends pay nothing.
    pub aux_sends: arc_swap::ArcSwap<Vec<AuxSend>>,

    /// External sidechain (key) routes, audio-thread-visible copy of the
    /// control thread's table. Empty until the first route is created, so
    /// projects that never sidechain pay nothing (the render path's only
    /// cost is an `is_empty` check per plugin).
    pub sidechain_routes: arc_swap::ArcSwap<Vec<SidechainRoute>>,

    /// Lock-free snapshot of the take-comp playback plan (epic #15, doc
    /// #165). Published by the control thread whenever a take group is
    /// captured, comped, or has its active take changed, and read once per
    /// block by the live mixer and the offline bounce so a comped /
    /// active-take selection plays — and bounces — as one part. The
    /// authoritative `TakeGroup`s live on the control thread
    /// (`HandlerState::take_groups`); this is the flattened,
    /// audio-thread-visible view. Empty until the first take is recorded,
    /// so projects without take lanes pay nothing.
    pub take_comp: arc_swap::ArcSwap<crate::mixer::CompRenderTable>,

    // -- Audition preview (doc #175) --
    /// Decoded preview source, published wait-free by the engine thread and
    /// read by the audio callback. `None` when no preview is loaded. See
    /// [`audition`].
    pub audition_source: arc_swap::ArcSwapOption<audition::AuditionSource>,
    /// The preview's run state as one word: bit 0 = playing, the rest a
    /// run generation the engine thread bumps on every start and stop.
    /// One word so the audio callback's natural-finish latch is a
    /// compare-exchange that fails, rather than stopping the new preview,
    /// when a restart landed during its block (code review RT-11). Read it
    /// through [`Self::audition_playing`].
    pub audition_ctl: AtomicU64,
    /// Where the current run starts, in source frames (bit-punned `f64`),
    /// stored by the engine thread before it bumps the generation.
    pub audition_start_bits: AtomicU64,
    /// Audition playhead in source frames, stored as bit-punned `f64` (it can
    /// be fractional under sync-to-tempo varispeed). The audio callback
    /// advances it; the engine thread reads it for `AuditionPosition`
    /// events and seeds it on a start for that report. The callback only
    /// continues from it when [`Self::audition_pos_gen`] says it belongs
    /// to the current run — otherwise it restarts from
    /// `audition_start_bits` — so a block that loaded the old position
    /// can no longer carry it into a new run (RT-11).
    pub audition_pos_bits: AtomicU64,
    /// The run generation `audition_pos_bits` was last advanced for.
    /// Audio callback only.
    pub audition_pos_gen: AtomicU64,
    /// Loop the preview when it reaches the end (vs. stopping).
    pub audition_loop: AtomicBool,
    /// Sync-to-tempo (varispeed) enabled for the preview.
    pub audition_sync: AtomicBool,
    /// Playback ratio (source frames per output frame) as bit-punned `f32`,
    /// computed by the engine thread; `1.0` is natural speed.
    pub audition_ratio_bits: AtomicU32,
    /// Latched by the audio callback when a non-looping preview reaches its
    /// end, as the finished run's generation + 1 (0 = nothing latched);
    /// consumed by the engine thread to emit `AuditionStopped` once, and
    /// only for the run that is still current (RT-11).
    pub audition_finished: AtomicU64,
}

impl SharedState {
    /// [`RenderGraphSlot::edit_midi_clips`], retiring the replaced graph
    /// onto this state's queue. Engine thread.
    pub fn edit_midi_clips<R>(&self, f: impl FnOnce(&mut Vec<Arc<MidiClip>>) -> R) -> R {
        self.graph.edit_midi_clips(&self.retired, f)
    }

    /// [`RenderGraphSlot::edit_midi_clip`], retiring the replaced graph
    /// onto this state's queue. Engine thread.
    pub fn edit_midi_clip<R>(
        &self,
        clip_id: ClipId,
        f: impl FnOnce(&mut MidiClip) -> R,
    ) -> Option<R> {
        self.graph.edit_midi_clip(&self.retired, clip_id, f)
    }

    /// [`RenderGraphSlot::edit_busses`], retiring the replaced graph onto
    /// this state's queue. Engine thread.
    pub fn edit_busses<R>(&self, f: impl FnOnce(&mut IndexMap<BusId, Arc<Bus>>) -> R) -> R {
        self.graph.edit_busses(&self.retired, f)
    }

    /// [`RenderGraphSlot::edit_bus`], retiring the replaced graph onto
    /// this state's queue. Engine thread.
    pub fn edit_bus<R>(&self, bus_id: BusId, f: impl FnOnce(&mut Bus) -> R) -> Option<R> {
        self.graph.edit_bus(&self.retired, bus_id, f)
    }

    /// [`RenderGraphSlot::edit_master`], retiring the replaced graph onto
    /// this state's queue. Engine thread.
    pub fn edit_master<R>(&self, f: impl FnOnce(&mut MasterBus) -> R) -> R {
        self.graph.edit_master(&self.retired, f)
    }

    /// [`RenderGraphSlot::edit_tracks`], retiring the replaced graph onto
    /// this state's queue. Engine thread.
    pub fn edit_tracks<R>(&self, f: impl FnOnce(&mut TrackMap) -> R) -> R {
        self.graph.edit_tracks(&self.retired, f)
    }

    /// [`RenderGraphSlot::edit_track`], retiring the replaced graph onto
    /// this state's queue. Engine thread.
    pub fn edit_track<R>(&self, track_id: TrackId, f: impl FnOnce(&mut Track) -> R) -> Option<R> {
        self.graph.edit_track(&self.retired, track_id, f)
    }

    /// [`RenderGraphSlot::edit_tracks_and_busses`], retiring the replaced
    /// graph onto this state's queue. Engine thread.
    pub fn edit_tracks_and_busses<R>(
        &self,
        f: impl FnOnce(&mut TrackMap, &mut IndexMap<BusId, Arc<Bus>>) -> R,
    ) -> R {
        self.graph.edit_tracks_and_busses(&self.retired, f)
    }

    /// [`RenderGraphSlot::edit_clips`], retiring the replaced graph (and
    /// with it every clip the edit removed) onto this state's queue.
    /// Engine thread.
    pub fn edit_clips<R>(&self, f: impl FnOnce(&mut Vec<Arc<AudioClip>>) -> R) -> R {
        self.graph.edit_clips(&self.retired, f)
    }

    /// [`RenderGraphSlot::edit_clip`], retiring the replaced graph onto
    /// this state's queue. Engine thread.
    pub fn edit_clip<R>(&self, clip_id: ClipId, f: impl FnOnce(&mut AudioClip) -> R) -> Option<R> {
        self.graph.edit_clip(&self.retired, clip_id, f)
    }

    /// The published audio clip list (an `Arc` clone, like
    /// [`Self::tracks`]). Holding it keeps the listed clips' audio alive;
    /// it never blocks an edit.
    pub fn clips(&self) -> Arc<[Arc<AudioClip>]> {
        Arc::clone(&self.graph.load().clips)
    }

    /// The published track map (an `Arc` clone — keep it for as long as
    /// the caller reads, it never blocks an edit).
    pub fn tracks(&self) -> Arc<TrackMap> {
        Arc::clone(&self.graph.load().tracks)
    }

    /// [`RenderGraphSlot::edit_plugins`], retiring the replaced graph —
    /// and every slot the edit removed — onto this state's queue. Engine
    /// thread.
    pub fn edit_plugins<R>(&self, f: impl FnOnce(&mut PluginMap) -> R) -> R {
        self.graph.edit_plugins(&self.retired, f)
    }

    /// The published plugin map (an `Arc` clone, like [`Self::tracks`]).
    /// Holding it pins the slots it lists; drop it before the engine
    /// loop's next sweep is expected to free a removed one.
    pub fn plugins(&self) -> Arc<PluginMap> {
        Arc::clone(&self.graph.load().plugins)
    }

    /// Whether an offline renderer (export, stem export, bounce in place,
    /// freeze, offline measurement) currently owns the live plugin
    /// instances — the one gate the audio callback and the transport
    /// handlers both honour (code review MIX-02 / ENG-05). A single
    /// acquire load, safe on the audio thread.
    #[inline]
    pub fn offline_render_active(&self) -> bool {
        self.offline_render_count
            .load(std::sync::atomic::Ordering::Acquire)
            > 0
    }

    /// Called by the capture callbacks on every recording push: on the
    /// *first* push of a session (armed via `recording_start_pending`)
    /// latch the raw playhead at that instant into
    /// `recording_start_latch`. That is the first moment capture data
    /// actually flows — after the stream-open delay, independent of the
    /// engine thread's wake cadence — so the stream-open gap can never
    /// land inside a take (doc #260 finding #2). The engine loop then
    /// derives the take's aligned start from it (subtracting the
    /// measured capture+playback latency for performer sessions).
    /// Subsequent pushes are a single relaxed load + failed CAS.
    pub fn latch_recording_start(&self) {
        if self
            .recording_start_pending
            .compare_exchange(
                true,
                false,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_ok()
        {
            let playhead = self.playhead.load(std::sync::atomic::Ordering::Relaxed);
            self.recording_start_latch
                .store(playhead, std::sync::atomic::Ordering::Release);
        }
    }
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            playhead: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            recording: AtomicBool::new(false),
            monitoring: AtomicBool::new(false),
            master_volume_bits: AtomicU32::new(1.0f32.to_bits()),
            master_last_volume_bits: AtomicU32::new(1.0f32.to_bits()),
            master_peak_l_bits: AtomicU32::new(0),
            master_peak_r_bits: AtomicU32::new(0),
            master_fx_bypass: crate::bypass::BypassFade::new(),
            recording_overflow: AtomicU64::new(0),
            input_channels: AtomicU16::new(0),
            loop_range: arc_swap::ArcSwap::from_pointee(loop_range::LoopRange::default()),
            count_in_active: AtomicBool::new(false),
            count_in_remaining: AtomicU64::new(0),
            count_in_total: AtomicU64::new(0),
            count_in_record_arm: AtomicU8::new(count_in_arm::IDLE),
            offline_render_count: AtomicU32::new(0),
            callback_activity: bounce::CallbackActivity::default(),
            external_offsets: arc_swap::ArcSwap::from_pointee(std::collections::HashMap::new()),
            dsp_load_ema_bits: AtomicU32::new(0),
            dsp_load_peak_bits: AtomicU32::new(0),
            dsp_overrun_cycles: AtomicU64::new(0),
            monitor_shortfall_cycles: AtomicU64::new(0),
            cycle_report: crate::cycle_load::CycleReportSlot::default(),
            oversize_buffer: crate::cycle_load::OversizeBufferLatch::default(),
            render_pool: parking_lot::Mutex::new(None),
            output_stream_errors: Default::default(),
            input_stream_errors: Default::default(),
            plugins_dead_after_reset: parking_lot::Mutex::new(Vec::new()),
            retired: retire::Retired::new(),
            graph: render_graph::RenderGraphSlot::new(),
            inbox: internal::EngineInbox::default(),
            comp_clamp_engaged: AtomicBool::new(false),
            master_latency_samples: AtomicU64::new(0),
            capture_latency_samples: AtomicU64::new(0),
            playback_latency_samples: AtomicU64::new(0),
            output_stream_lost: AtomicBool::new(false),
            recording_start_pending: AtomicBool::new(false),
            recording_start_latch: AtomicU64::new(0),
            reference: reference::ReferenceMonitor::default(),
            mix_meter: resonance_metering::AtomicMeterSnapshot::new(),
            ref_meter: resonance_metering::AtomicMeterSnapshot::new(),
            aux_sends: arc_swap::ArcSwap::from_pointee(Vec::new()),
            sidechain_routes: arc_swap::ArcSwap::from_pointee(Vec::new()),
            take_comp: arc_swap::ArcSwap::from_pointee(crate::mixer::CompRenderTable::default()),
            audition_source: arc_swap::ArcSwapOption::empty(),
            audition_ctl: AtomicU64::new(0),
            audition_start_bits: AtomicU64::new(0),
            audition_pos_bits: AtomicU64::new(0),
            audition_pos_gen: AtomicU64::new(0),
            audition_loop: AtomicBool::new(false),
            audition_sync: AtomicBool::new(false),
            audition_ratio_bits: AtomicU32::new(1.0f32.to_bits()),
            audition_finished: AtomicU64::new(0),
        }
    }
}
