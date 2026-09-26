//! [`MixAudioHarness`]: the whole audio callback over owned state.
//!
//! Mirrors the engine's `make_mixer` closure — the same locks, the same
//! pre-allocated scratch, the same monitor ring and live-MIDI channels —
//! so `tests/mixer/mix_audio_parity.rs` can drive every branch of
//! [`mix_audio`] (reference monitor, count-in, stopped-monitor, playing,
//! loop seam, lock contention, audition overlay) without an audio device,
//! a CLAP plugin or the engine thread.

use std::sync::Arc;

use indexmap::IndexMap;
use parking_lot::RwLock;
use ringbuf::traits::{Producer, Split};

use crate::clap_host::PluginMap;
use crate::engine::reference::ABMeters;
use crate::engine::{AutomationSnapshot, SharedState};
use crate::midi_hardware::LiveMidiEvent;
use crate::types::*;

use super::super::callback::{CallbackInputs, CallbackScratch};
use super::super::midi_stash::MidiStash;
use super::super::mix_audio;
use super::super::monitor::MonitorDrain;
use super::super::{MAX_MIDI_EVENTS_PER_BUFFER, MAX_PLUGIN_OUTPUT_PORTS};

/// One [`mix_audio`] call over a [`MixAudioHarness`]'s fields.
///
/// A macro rather than a method because the two call sites need the
/// inputs and the scratch borrowed from *disjoint fields* of the same
/// `&mut self` — which two accessor methods could not express, and which
/// is exactly the split [`CallbackInputs`] / [`CallbackScratch`] exist
/// for on the engine side too.
macro_rules! run_callback {
    ($h:ident) => {
        mix_audio(
            CallbackInputs {
                channels: $h.channels,
                shared: &*$h.shared,
                tracks: &$h.tracks,
                busses: &$h.busses,
                master: &$h.master,
                clips: &$h.clips,
                midi_clips: &$h.midi_clips,
                plugins: &$h.plugins,
                tempo_map: &$h.tempo_map,
                latency_comp: &$h.latency_comp,
                automation: &$h.automation,
                sample_rate: $h.sample_rate,
                live_midi_rx: &$h.live_midi_rx,
                live_midi_fwd: &$h.live_fwd_tx,
                buf_frames: $h.buf_frames,
                quantum: $h.quantum,
            },
            &mut CallbackScratch {
                data: &mut $h.data,
                track_buf_l: &mut $h.track_buf_l,
                track_buf_r: &mut $h.track_buf_r,
                bus_bufs: &mut $h.bus_bufs,
                port_scratch: &mut $h.port_scratch,
                note_event_buf: &mut $h.note_buf,
                midi_stash: &mut $h.midi_stash,
                monitor_cons: &mut $h.monitor_cons,
                monitor_temp: &mut $h.monitor_temp,
                monitor_drain: &mut $h.monitor_drain,
                ab_meters: &mut $h.ab_meters,
                sidechain: &mut $h.sidechain,
                fx_dry: &mut $h.fx_dry,
                continuity: &mut $h.continuity,
            },
        )
    };
}

/// Owned stand-in for everything the engine hands the audio callback, so
/// [`mix_audio`] — the whole callback, not just the render core — can be
/// driven from an integration test.
///
/// Mirrors the engine's `make_mixer` closure: the same locks, the same
/// pre-allocated scratch, the same monitor ring and live-MIDI channels.
/// The test drives branch selection through [`Self::shared`] (transport,
/// count-in, loop, monitoring flags) exactly as the engine control thread
/// would, then hashes [`Self::render`]'s output and the side effects the
/// callback publishes back into shared state.
#[doc(hidden)]
pub struct MixAudioHarness {
    /// `Arc` rather than inline so a test can hand a clone to a second
    /// thread and reposition the playhead *while* a block renders — the
    /// engine control thread's Seek / Stop race against the callback
    /// (`tests/engine/playhead_seek_race.rs`).
    shared: Arc<SharedState>,
    tracks: RwLock<IndexMap<TrackId, Track>>,
    busses: RwLock<IndexMap<BusId, Bus>>,
    master: RwLock<MasterBus>,
    clips: RwLock<Vec<AudioClip>>,
    midi_clips: RwLock<Vec<MidiClip>>,
    plugins: RwLock<PluginMap>,
    tempo_map: arc_swap::ArcSwap<TempoMap>,
    latency_comp: arc_swap::ArcSwap<crate::latency::LatencyComp>,
    automation: arc_swap::ArcSwap<AutomationSnapshot>,
    sample_rate: u32,
    channels: usize,
    buf_frames: usize,
    quantum: usize,
    data: Vec<f32>,
    track_buf_l: Vec<f32>,
    track_buf_r: Vec<f32>,
    bus_bufs: Vec<(Vec<f32>, Vec<f32>)>,
    port_scratch: Vec<(Vec<f32>, Vec<f32>)>,
    note_buf: Vec<PendingNoteEvent>,
    midi_stash: MidiStash,
    monitor_prod: ringbuf::HeapProd<f32>,
    monitor_cons: ringbuf::HeapCons<f32>,
    monitor_temp: Vec<f32>,
    monitor_drain: MonitorDrain,
    ab_meters: ABMeters,
    sidechain: SidechainTaps,
    fx_dry: crate::bypass::FxDryScratch,
    continuity: crate::mixer::TransportContinuity,
    live_midi_tx: crossbeam_channel::Sender<LiveMidiEvent>,
    live_midi_rx: crossbeam_channel::Receiver<LiveMidiEvent>,
    live_fwd_tx: crossbeam_channel::Sender<LiveMidiEvent>,
    live_fwd_rx: crossbeam_channel::Receiver<LiveMidiEvent>,
}

#[doc(hidden)]
impl MixAudioHarness {
    /// Build the callback state for a project. `native_drain` selects the
    /// monitor-drain policy (`true` = native PipeWire backend).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tracks: Vec<Track>,
        busses: Vec<Bus>,
        clips: Vec<AudioClip>,
        midi_clips: Vec<MidiClip>,
        aux_sends: Vec<AuxSend>,
        tempo_map: TempoMap,
        frames: usize,
        channels: usize,
        sample_rate: u32,
        native_drain: bool,
    ) -> Self {
        let tracks: IndexMap<TrackId, Track> = tracks.into_iter().map(|t| (t.id, t)).collect();
        let busses: IndexMap<BusId, Bus> = busses.into_iter().map(|b| (b.id, b)).collect();
        let bus_count = busses.len().max(1);
        let shared = Arc::new(SharedState::default());
        shared.aux_sends.store(Arc::new(aux_sends));
        // Room for a few blocks of the widest input we drive, matching the
        // engine's ring sizing policy.
        let ring = ringbuf::HeapRb::<f32>::new(frames * MAX_MONITOR_CHANNELS * 4);
        let (monitor_prod, monitor_cons) = ring.split();
        let mut ab_meters = ABMeters::new(sample_rate as f32);
        ab_meters.reserve(frames);
        let (live_midi_tx, live_midi_rx) = crossbeam_channel::unbounded();
        let (live_fwd_tx, live_fwd_rx) = crossbeam_channel::unbounded();
        Self {
            shared,
            tracks: RwLock::new(tracks),
            busses: RwLock::new(busses),
            master: RwLock::new(MasterBus::default()),
            clips: RwLock::new(clips),
            midi_clips: RwLock::new(midi_clips),
            plugins: RwLock::new(IndexMap::new()),
            tempo_map: arc_swap::ArcSwap::from_pointee(tempo_map),
            latency_comp: arc_swap::ArcSwap::from_pointee(crate::latency::LatencyComp::empty()),
            automation: arc_swap::ArcSwap::from_pointee(AutomationSnapshot::default()),
            sample_rate,
            channels,
            buf_frames: frames,
            quantum: frames,
            data: vec![0.0; frames * channels],
            track_buf_l: vec![0.0; frames],
            track_buf_r: vec![0.0; frames],
            bus_bufs: (0..bus_count)
                .map(|_| (vec![0.0; frames], vec![0.0; frames]))
                .collect(),
            port_scratch: (0..MAX_PLUGIN_OUTPUT_PORTS)
                .map(|_| (vec![0.0; frames], vec![0.0; frames]))
                .collect(),
            note_buf: Vec::with_capacity(MAX_MIDI_EVENTS_PER_BUFFER),
            midi_stash: MidiStash::new(),
            monitor_prod,
            monitor_cons,
            monitor_temp: vec![0.0; frames * MAX_MONITOR_CHANNELS],
            monitor_drain: MonitorDrain::new(native_drain),
            ab_meters,
            sidechain: SidechainTaps::new(frames),
            fx_dry: crate::bypass::FxDryScratch::new(frames),
            continuity: crate::mixer::TransportContinuity::default(),
            live_midi_tx,
            live_midi_rx,
            live_fwd_tx,
            live_fwd_rx,
        }
    }

    /// The callback's view of engine state — transport, loop, count-in,
    /// monitoring and metering flags.
    pub fn shared(&self) -> &SharedState {
        &self.shared
    }

    /// A second handle on the callback's shared state, for a thread that
    /// plays the engine control thread against the rendering one (seek
    /// / stop / MIDI-clock reposition while a block is in flight), or
    /// for holding the real `OfflineRenderGuard` over the harness.
    pub fn shared_arc(&self) -> Arc<SharedState> {
        Arc::clone(&self.shared)
    }

    /// The track table the callback reads, for a test that mutes / solos
    /// / re-routes between blocks as the engine control thread would.
    pub fn tracks(&self) -> &RwLock<IndexMap<TrackId, Track>> {
        &self.tracks
    }

    /// The bus table the callback reads.
    pub fn busses(&self) -> &RwLock<IndexMap<BusId, Bus>> {
        &self.busses
    }

    /// The plugin instances the callback drives — empty until a test
    /// inserts a hand-rolled CLAP instance (`__instance_from_raw_for_test`).
    pub fn plugins(&self) -> &RwLock<PluginMap> {
        &self.plugins
    }

    /// Publish a new automation snapshot (as the engine thread does on a
    /// lane edit): the replaced one goes to `shared.retired`.
    pub fn set_automation(&self, snapshot: AutomationSnapshot) {
        crate::engine::retire::publish(
            &self.automation,
            std::sync::Arc::new(snapshot),
            &self.shared.retired,
        );
    }

    /// Publish a new plugin-delay-compensation table.
    pub fn set_latency_comp(&self, comp: crate::latency::LatencyComp) {
        crate::engine::retire::publish(
            &self.latency_comp,
            std::sync::Arc::new(comp),
            &self.shared.retired,
        );
    }

    /// Publish a new tempo map (metronome flag included).
    pub fn set_tempo_map(&self, map: TempoMap) {
        crate::engine::retire::publish(
            &self.tempo_map,
            std::sync::Arc::new(map),
            &self.shared.retired,
        );
    }

    /// Push interleaved capture frames into the monitor ring, as the
    /// input stream's callback would.
    pub fn push_monitor(&mut self, samples: &[f32]) -> usize {
        self.monitor_prod.push_slice(samples)
    }

    /// Arm the A/B reference monitor with `pcm` (interleaved stereo) and
    /// switch the monitored source to it.
    pub fn enable_reference(&self, pcm: Vec<f32>) {
        use crate::engine::reference::{
            handle_reference_analyzed, register_reference, ReferencePlayer,
        };
        let mut player = ReferencePlayer::new();
        let id = register_reference(&mut player, ReferenceId(1), std::path::PathBuf::from("parity.wav"));
        handle_reference_analyzed(&mut player, id, std::sync::Arc::new(pcm), -14.0);
        player.active_id = Some(id);
        player.ab_source = ABSource::Reference;
        player.publish(&self.shared, true);
    }

    /// Switch the monitored A/B source back to the mix, as the A/B toggle
    /// does (the reference branch then no longer takes the block).
    pub fn disable_reference(&self) {
        use crate::engine::reference::ReferencePlayer;
        ReferencePlayer::new().publish(&self.shared, true);
    }

    /// Load an audition preview source and start it (the overlay branch).
    pub fn start_audition(&self, samples: Vec<f32>, looping: bool) {
        use std::sync::atomic::Ordering;
        let source = crate::engine::AuditionSource::from_samples(samples, self.sample_rate);
        crate::engine::retire::publish_opt(
            &self.shared.audition_source,
            Some(std::sync::Arc::new(source)),
            &self.shared.retired,
        );
        self.shared.audition_pos_bits.store(0f64.to_bits(), Ordering::Relaxed);
        self.shared.audition_loop.store(looping, Ordering::Relaxed);
        // Release pairs with the overlay's Acquire gate, matching
        // `start_audition_in_place`.
        self.shared.audition_playing.store(true, Ordering::Release);
    }

    /// Queue a live hardware-MIDI event for the callback's pickup pass.
    pub fn send_live_midi(&self, ev: LiveMidiEvent) {
        let _ = self.live_midi_tx.send(ev);
    }

    /// Live-MIDI events the callback forwarded to the engine thread.
    pub fn drain_forwarded_midi(&self) -> usize {
        self.live_fwd_rx.try_iter().count()
    }

    /// Run one audio callback and return the interleaved output.
    pub fn render(&mut self) -> &[f32] {
        run_callback!(self);
        &self.data
    }

    /// Make every later callback's host buffer `frames` long — larger than
    /// the pre-allocated scratch is the `BufferSize::Default` fallback
    /// shape the callback clamps (and reports) rather than renders.
    pub fn set_host_buffer_frames(&mut self, frames: usize) {
        self.data = vec![0.0; frames * self.channels];
    }

    /// Run one audio callback while a UI edit "holds" the clips write
    /// lock, so the arrangement render takes its lock-contended branch
    /// (silence out, playhead advanced, `render_skip_cycles` bumped).
    pub fn render_lock_contended(&mut self) -> &[f32] {
        let _held = self.clips.write();
        run_callback!(self);
        &self.data
    }

    /// Run one audio callback while `map` is *read*-held on a worker
    /// thread with a writer queued behind it — the offline-render / load-
    /// worker shape from code review ARCH-02: parking_lot's task-fair
    /// policy fails the callback's `try_read` on that one map only. The
    /// writer is released after the block.
    pub fn render_with_queued_writer(&mut self, map: crate::cycle_load::StateMap) -> &[f32] {
        use crate::cycle_load::StateMap;
        macro_rules! with_map {
            ($lock:expr) => {{
                let lock = &$lock;
                let reader = lock.read();
                std::thread::scope(|s| {
                    s.spawn(|| {
                        let _w = lock.write();
                    });
                    // A queued writer is exactly what makes `try_read`
                    // fail, so this is the deterministic "writer parked"
                    // signal, not a sleep.
                    while lock.try_read().is_some() {
                        std::thread::yield_now();
                    }
                    run_callback!(self);
                    drop(reader);
                });
            }};
        }
        match map {
            StateMap::Tracks => with_map!(self.tracks),
            StateMap::Busses => with_map!(self.busses),
            StateMap::Master => with_map!(self.master),
            StateMap::Clips => with_map!(self.clips),
            StateMap::MidiClips => with_map!(self.midi_clips),
            StateMap::Plugins => with_map!(self.plugins),
        }
        &self.data
    }

    /// Every scalar the callback publishes back into shared state, in a
    /// fixed order, so a parity test can fold them into its hash: playhead,
    /// master peaks, the two stutter counters, the audition playhead, and
    /// the reference cursor.
    pub fn side_effects(&self) -> [u64; 7] {
        use std::sync::atomic::Ordering;
        [
            self.shared.playhead.load(Ordering::Relaxed),
            self.shared.master_peak_l_bits.load(Ordering::Relaxed) as u64,
            self.shared.master_peak_r_bits.load(Ordering::Relaxed) as u64,
            self.shared.monitor_shortfall_cycles.load(Ordering::Relaxed),
            self.shared.render_skip_cycles.load(Ordering::Relaxed),
            self.shared.audition_pos_bits.load(Ordering::Relaxed),
            self.shared.reference.cursor_for_test(),
        ]
    }

    /// Post-fader peak levels every track accumulated since the last
    /// call — the monitor / render passes' VU writes. Reads *and clears*,
    /// exactly like the engine thread's meter poll, so consecutive calls
    /// report per-block peaks.
    pub fn take_track_peaks(&self) -> Vec<(f32, f32)> {
        self.tracks
            .read()
            .values()
            .map(|t| (t.swap_peak_l(), t.swap_peak_r()))
            .collect()
    }

    /// The gain each track's ramp ended the last block on, so a parity
    /// hash also covers the ramp state carried between blocks.
    pub fn track_last_gains(&self) -> Vec<(f32, f32)> {
        self.tracks.read().values().map(|t| t.last_gains()).collect()
    }
}

/// Monitor scratch width for the harness: wide enough for the
/// multi-channel interleaved inputs the parity fixtures drive.
const MAX_MONITOR_CHANNELS: usize = 8;
