//! Core drum sampler engine: sample loading, voice management, and audio rendering.

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};

use crate::drum_map::{self, NUM_PADS, PAD_MAPPINGS};
use crate::kit::{
    LoadedMicBank, LoadedPad, SampleData, VelocityLayer, MAIN_PORT_INDEX, OVERHEAD_PORT_INDEX,
};
use crate::kit_loader::KitLoadProgress;
use crate::level::db_to_gain;
use crate::params::{DrumParams, MicSlot, MIC_SLOTS};
use crate::stream::reader::ReaderPool;
use crate::stream::{
    wait_until, AudioStreams, Pace, RenderMode, Ring, StreamSet, HOST_RENDER_OFFLINE,
    HOST_RENDER_REALTIME, HOST_RENDER_UNKNOWN, NO_RING,
};
use crate::voice::{
    fade_frames, Voice, VoiceDestination, VoiceState, MAX_VOICES, RELEASE_FADE_MS,
    STEAL_FADE_MS, SWAP_FADE_MS, TAIL_SLOTS,
};

use super::janitor;
use super::voice_pick::{
    map_relative, pick_rr, pick_rr_random, pick_velocity_layer, RoundRobinMode, MAX_LAYERS,
    NO_LAST_TAKE,
};

/// The global settings a hit is started with, snapshotted once per
/// block from the params (see
/// [`DrumSampler::update_global_settings`]). Held on the sampler so
/// `note_on` stays a two-argument audio-thread call and so a headless
/// caller that never sets them gets exactly the plugin's historical
/// behaviour.
#[derive(Clone, Copy, Debug)]
pub struct GlobalSettings {
    /// Ceiling on simultaneously active voices.
    pub max_voices: usize,
    /// Velocity curve, -1 (hard) … 0 (linear) … +1 (soft).
    pub velocity_curve: f32,
    /// How a layer's takes are walked.
    pub round_robin: RoundRobinMode,
    /// Where hits sound (E11). A headless sampler never handed params
    /// routes [`OutputMode::Multi`], as the sampler always did; the
    /// plugin's own default (`output_mode`) is Stereo.
    pub output_mode: OutputMode,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            max_voices: MAX_VOICES,
            velocity_curve: 0.0,
            round_robin: RoundRobinMode::Cycle,
            output_mode: OutputMode::Multi,
        }
    }
}

/// How the sampler spreads a kit over its output ports (E11, D5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    /// Every pad and mic sums to Main (port 0).
    Stereo,
    /// Each pad's close mics play on its output port; every overhead
    /// take on the Overhead port.
    Multi,
}

impl OutputMode {
    /// Read the mode off its parameter value.
    pub fn from_param(value: i32) -> Self {
        if value == crate::params::OUTPUT_MODE_MULTI {
            Self::Multi
        } else {
            Self::Stereo
        }
    }
}

/// A [`PadSettings`] field that defers to what the kit itself says
/// ([`LoadedPad`]): what a headless sampler that is never handed params
/// does, as the sampler always did.
pub const FROM_KIT: u8 = u8::MAX;

/// One pad's trigger settings, snapshotted once per block from its
/// params with the [`GlobalSettings`]: they decide how a hit on the pad
/// is *started*, so block rate is the right granularity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadSettings {
    /// Choke group (E12): 0 = none, 1..=8, or [`FROM_KIT`].
    pub choke: u8,
    /// The port the pad's close mics play on in Multi (E11): a port
    /// index, or [`FROM_KIT`] for the pad's own group.
    pub port: u8,
    /// Playback rate from `pad_N_tune` (E8): `2^(st/12)`, exactly 1.0 at
    /// 0 st (the integer path).
    pub rate: f32,
    /// Sample start (E8), in take frames at the host rate.
    pub start_frames: u32,
    /// Hold and decay (E8), in output frames; `decay_frames == 0` is Off.
    pub hold_frames: u32,
    pub decay_frames: u32,
}

impl Default for PadSettings {
    fn default() -> Self {
        Self {
            choke: FROM_KIT,
            port: FROM_KIT,
            rate: 1.0,
            start_frames: 0,
            hold_frames: 0,
            decay_frames: 0,
        }
    }
}

impl PadSettings {
    /// This pad's settings from its params, at `sample_rate`.
    pub fn from_params(pad: &crate::params::PadParams, sample_rate: f32) -> Self {
        use crate::params::{DECAY_OFF_MS, MAX_TUNE_ST};
        let frames = |ms: f32| (ms.max(0.0) * sample_rate / 1000.0).round() as u32;
        let tune = pad.tune.value().clamp(-MAX_TUNE_ST, MAX_TUNE_ST);
        let decay_ms = pad.decay.value();
        Self {
            choke: pad.choke.value().clamp(0, crate::params::MAX_CHOKE_GROUP) as u8,
            port: pad
                .output
                .value()
                .clamp(0, crate::kit::NUM_OUTPUT_PORTS as i32 - 1) as u8,
            // Exactly 1.0 at 0 st: not `exp2(0.0)`'s word for it.
            rate: if tune == 0.0 {
                1.0
            } else {
                (tune / 12.0).exp2()
            },
            start_frames: frames(pad.start.value()),
            hold_frames: frames(pad.hold.value()),
            decay_frames: if decay_ms >= DECAY_OFF_MS {
                0
            } else {
                frames(decay_ms).max(1)
            },
        }
    }

    /// The port a hit on `pad` plays its close mics on in Multi.
    #[inline]
    fn close_port(&self, pad: &LoadedPad) -> u8 {
        match self.port {
            FROM_KIT => pad.output_group.index() as u8,
            port => port,
        }
    }

    /// The choke group a hit on `pad` joins.
    #[inline]
    fn choke_group(&self, pad: &LoadedPad) -> Option<u8> {
        match self.choke {
            FROM_KIT => pad.choke_group,
            0 => None,
            group => Some(group),
        }
    }
}

/// A note-on at a frame offset inside the block, for
/// [`DrumSampler::render_block`].
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    /// Frame within the block the hit starts at.
    pub frame: usize,
    pub note: u8,
    pub velocity: f32,
}

/// One stereo output buffer pair for a single plugin output port. Callers
/// build a slice of these (one per port) and hand it to `render_frame`.
pub struct PortBuffers<'a> {
    pub left: &'a mut [f32],
    pub right: &'a mut [f32],
}

/// How many retired kits can be fading out at once. Each kit swap parks
/// the outgoing kit here until the voices still reading it have faded
/// ([`SWAP_FADE_MS`]); a second swap inside that window takes the next
/// slot instead of cutting the first kit's voices. Only a fifth swap
/// within one fade — four full kit loads finishing inside 5 ms — finds
/// every slot taken and cuts the oldest.
const RETIRED_KITS: usize = 4;

/// The rate the sampler assumes until told otherwise
/// ([`DrumSampler::set_sample_rate`]).
const DEFAULT_SAMPLE_RATE: f32 = 48_000.0;

/// Audio time the offline detector ([`RenderMode::Auto`] with nothing
/// declared) measures its render speed over, in seconds.
const OFFLINE_WINDOW_SECS: f64 = 0.25;
/// Faster than this many times real time over a whole window, the window
/// counts as offline. A device-paced callback cannot sustain it.
const OFFLINE_ENTER_RATIO: f64 = 2.0;
/// A block that arrives no faster than this many times real time after
/// the one before — a live callback, a pause, playback resuming after a
/// bounce — makes the detector live at once, before the block renders.
const LIVE_BLOCK_RATIO: f64 = 1.5;
/// Offline windows in a row before the detector waits more than
/// [`AUTO_WAIT_PER_BLOCK`] a block.
const SUSTAINED_WINDOWS: u32 = 3;
/// The most a block waits for the disk reader in all when the host
/// declared offline rendering ([`RenderMode::Offline`]), before the
/// missing frames are given up as an underrun.
pub const OFFLINE_WAIT_PER_BLOCK: Duration = Duration::from_secs(10);
/// The most a block waits under [`RenderMode::Auto`] with nothing
/// declared, once the timing says offline: a wrong verdict costs a live
/// block no more than this.
pub const AUTO_WAIT_PER_BLOCK: Duration = Duration::from_millis(2);
/// The same, once [`SUSTAINED_WINDOWS`] windows in a row were offline.
pub const AUTO_SUSTAINED_WAIT_PER_BLOCK: Duration = Duration::from_millis(50);
/// After a long wait ran out, this much audio renders without waiting —
/// a dead reader must not slow a bounce to a crawl.
const OFFLINE_WAIT_HOLDOFF_SECS: f32 = 1.0;

/// What [`RenderMode::Auto`] reads the time from.
enum RenderClock {
    /// The wall clock, since the sampler was built.
    Wall(Instant),
    /// Nanoseconds the caller sets (a test hook): timing tests that do
    /// not depend on how loaded the machine is.
    Manual(Arc<AtomicU64>),
}

impl RenderClock {
    /// Now, as time since the clock's start. One vDSO clock read for the
    /// wall clock — no syscall.
    fn now(&self) -> Duration {
        match self {
            Self::Wall(epoch) => epoch.elapsed(),
            Self::Manual(nanos) => Duration::from_nanos(nanos.load(Ordering::Relaxed)),
        }
    }
}

pub struct DrumSampler {
    pub pads: Vec<LoadedPad>,
    pub voices: Vec<Voice>,
    /// Stolen voices fading out (E1): when a hit needs a slot and every
    /// one is busy, the victim is copied here and faded over
    /// [`STEAL_FADE_MS`] while the hit takes its slot clean. Rendered with
    /// `voices`, but not counted against polyphony and never a steal
    /// candidate.
    tails: Vec<Voice>,
    voice_counter: u64,
    /// Monotonic round-robin counter per (pad, layer). Advanced on each
    /// note_on; indexed modulo the layer's RR count to pick the next take.
    rr_counters: [[u32; MAX_LAYERS]; NUM_PADS],
    /// The take that last fired per (pad, layer), or [`NO_LAST_TAKE`].
    /// Recorded in both round-robin modes so switching to Random never
    /// repeats whatever Cycle just played.
    rr_last: [[u16; MAX_LAYERS]; NUM_PADS],
    /// Xorshift state for the Random round-robin mode. Seeded to a fixed
    /// constant so a render is reproducible: bouncing the same project
    /// twice gives the same takes.
    rr_rng: u32,
    /// Global trigger settings, refreshed once per block from the params.
    globals: GlobalSettings,
    /// Per-pad trigger settings, refreshed with `globals`.
    pad_settings: [PadSettings; NUM_PADS],
    /// Shared display state for the editor: packed `rr_index | (n_rrs << 16)`.
    /// Written after each `note_on`; `None` when running headless / in tests.
    last_rr: Option<Arc<[AtomicU32; NUM_PADS]>>,
    /// Shared OUT meter for the editor: this block's peak across every
    /// output port as `f32::to_bits`, `[left, right]`. Written at the end
    /// of `render_block`; `None` when running headless / in tests.
    out_peak: Option<Arc<[AtomicU32; 2]>>,
    /// Kit load progress, told each time a kit is taken from the mailbox
    /// (one atomic add) so `kit_load_progress` reaches 1.0 only once the
    /// kit is really in place. `None` headless / in tests.
    load_progress: Option<Arc<KitLoadProgress>>,
    /// Receives new kit versions from the loader thread; `try_recv` at the
    /// top of each process block swaps in a freshly loaded kit without
    /// blocking. The audio thread is not the only receiver: a loader
    /// takes a stale kit back out of the slot (E3), and `initialize`
    /// drains it while the plugin is inactive. Both do so under
    /// `KitBridge::kit_handoff`, and either way the kit is freed off the
    /// audio thread.
    kit_receiver: Receiver<Vec<LoadedPad>>,
    /// Ships old kits to the janitor thread on swap so the large heap free
    /// happens off the audio thread. Bounded ([`janitor::JANITOR_DEPTH`]),
    /// so a send never allocates; a full channel leaves the kit parked in
    /// its retired slot until a later block. The sampler is the sole owner
    /// of this sender; when the sampler drops, the janitor's channel
    /// disconnects and the janitor thread exits cleanly.
    janitor_sender: Sender<Vec<LoadedPad>>,
    /// Previous kits' pads, kept alive while voices that were still
    /// sounding at swap time fade out against them (a voice's
    /// `retired_slot` says which). Each is shipped to the janitor once
    /// the last voice reading it ends — or later, if the janitor's
    /// channel is full then. Every outgoing kit is parked here first,
    /// even one no voice reads, so a kit is never dropped on the audio
    /// thread for want of room in the janitor's channel.
    retired_pads: [Option<Vec<LoadedPad>>; RETIRED_KITS],
    /// Swap number each `retired_pads` slot was filled at, so a swap that
    /// finds every slot taken retires the oldest.
    retired_stamp: [u64; RETIRED_KITS],
    swap_count: u64,
    /// The host rate the fades below were computed at.
    sample_rate: f32,
    /// [`RELEASE_FADE_MS`] in frames at `sample_rate`: choke groups and
    /// host chokes.
    release_frames: u32,
    /// [`SWAP_FADE_MS`] in frames at `sample_rate`: kit swaps.
    swap_frames: u32,
    /// [`STEAL_FADE_MS`] in frames at `sample_rate`: stolen voices.
    steal_frames: u32,
    /// Last block's master gain snapshot (linear, from the dB param).
    /// Used to interpolate from the previous block's value to the
    /// current one across the block so automation tweaks don't click.
    /// Initialized to 1.0 so the first block starts at unity gain.
    prev_master_volume: f32,
    /// Last block's per-pad parameter snapshots, mirroring the master
    /// volume ramp: each pad's volume / pan / per-mic trims is linearly
    /// interpolated (in gain) from the previous block's value to the
    /// current one across the block so automation jumps don't click.
    /// (`mute` folds into the volume snapshot, so mute toggles ramp
    /// too.) Seeded from the first block's snapshot (`pad_prev_valid`)
    /// so the plugin doesn't ramp from arbitrary defaults on startup.
    prev_pad_volume: [f32; NUM_PADS],
    prev_pad_pan: [f32; NUM_PADS],
    /// Per-mic trim gains, indexed by [`MicSlot`].
    prev_pad_trim: [[f32; MIC_SLOTS]; NUM_PADS],
    pad_prev_valid: bool,
    /// This block's per-pad parameter snapshots, taken by
    /// [`DrumSampler::begin_block`] and ramped toward from the `prev_*`
    /// ones by every [`DrumSampler::render_span`] of the block.
    cur_pad_volume: [f32; NUM_PADS],
    cur_pad_pan: [f32; NUM_PADS],
    cur_pad_trim: [[f32; MIC_SLOTS]; NUM_PADS],
    /// `1 / frames` for the block in progress (0 for an empty block).
    block_inv_frames: f32,
    /// True when `begin_block` found nothing to render: spans are no-ops
    /// and `end_block` only publishes the (silent) OUT meter.
    block_idle: bool,
    /// Disk streaming (E14): this sampler's tail rings and its underrun
    /// counter. See [`crate::stream`].
    streams: AudioStreams,
    /// Where the bytes of ring storage the streams hold are published
    /// (the bridge's `stream_ring_bytes`), once a block.
    ring_bytes_out: Option<Arc<AtomicU64>>,
    /// How missing tail frames are treated (see [`RenderMode`]).
    render_mode: RenderMode,
    /// What the host declared ([`crate::KitBridge::host_render_mode`]):
    /// read once a block, under [`RenderMode::Auto`].
    host_render_mode: Option<Arc<AtomicU8>>,
    /// This block renders offline: a missing tail frame is waited for.
    offline: bool,
    /// The most the block in progress may wait for the reader, and what
    /// is left of it.
    block_wait: Duration,
    offline_wait_left: Duration,
    /// Frames left to render without waiting, after a long wait ran out.
    offline_holdoff: u64,
    /// [`RenderMode::Auto`]'s measurement, on `clock`: the window's
    /// start, the audio frames rendered in it, the time it spent waiting
    /// for the reader (not rendering), the offline windows in a row, and
    /// the previous block's start and length.
    clock: RenderClock,
    timing_start: Option<Duration>,
    timing_frames: u64,
    timing_waited: Duration,
    fast_windows: u32,
    last_block_start: Option<Duration>,
    last_block_frames: usize,
}

impl DrumSampler {
    pub fn new(kit_receiver: Receiver<Vec<LoadedPad>>) -> Self {
        Self::with_janitor(kit_receiver, janitor::spawn())
    }

    /// [`new`](Self::new) with its tails streamed by `pool` instead of the
    /// process-wide reader pool. Test hook: a test's own pool can be shut
    /// down mid-render.
    #[doc(hidden)]
    pub fn with_reader_pool(
        kit_receiver: Receiver<Vec<LoadedPad>>,
        pool: &Arc<ReaderPool>,
    ) -> Self {
        Self::with_janitor_and_pool(kit_receiver, janitor::spawn(), pool)
    }

    /// [`new`](Self::new) with the caller's janitor channel instead of a
    /// janitor thread. Test hook: a channel nobody drains is how a test
    /// fills it up to see what the sampler does then.
    #[doc(hidden)]
    pub fn with_janitor(
        kit_receiver: Receiver<Vec<LoadedPad>>,
        janitor_sender: Sender<Vec<LoadedPad>>,
    ) -> Self {
        Self::with_janitor_and_pool(kit_receiver, janitor_sender, ReaderPool::global())
    }

    fn with_janitor_and_pool(
        kit_receiver: Receiver<Vec<LoadedPad>>,
        janitor_sender: Sender<Vec<LoadedPad>>,
        pool: &Arc<ReaderPool>,
    ) -> Self {
        let set = StreamSet::new();
        let registration = pool.register(&set);
        Self {
            pads: Vec::new(),
            voices: (0..MAX_VOICES).map(|_| Voice::new()).collect(),
            tails: (0..TAIL_SLOTS).map(|_| Voice::new()).collect(),
            voice_counter: 0,
            rr_counters: [[0; MAX_LAYERS]; NUM_PADS],
            rr_last: [[NO_LAST_TAKE; MAX_LAYERS]; NUM_PADS],
            rr_rng: 0x9E37_79B9,
            globals: GlobalSettings::default(),
            pad_settings: [PadSettings::default(); NUM_PADS],
            last_rr: None,
            out_peak: None,
            load_progress: None,
            kit_receiver,
            janitor_sender,
            retired_pads: std::array::from_fn(|_| None),
            retired_stamp: [0; RETIRED_KITS],
            swap_count: 0,
            sample_rate: DEFAULT_SAMPLE_RATE,
            release_frames: fade_frames(RELEASE_FADE_MS, DEFAULT_SAMPLE_RATE),
            swap_frames: fade_frames(SWAP_FADE_MS, DEFAULT_SAMPLE_RATE),
            steal_frames: fade_frames(STEAL_FADE_MS, DEFAULT_SAMPLE_RATE),
            prev_master_volume: 1.0,
            prev_pad_volume: [1.0; NUM_PADS],
            prev_pad_pan: [0.0; NUM_PADS],
            prev_pad_trim: [[1.0; MIC_SLOTS]; NUM_PADS],
            pad_prev_valid: false,
            cur_pad_volume: [1.0; NUM_PADS],
            cur_pad_pan: [0.0; NUM_PADS],
            cur_pad_trim: [[1.0; MIC_SLOTS]; NUM_PADS],
            block_inv_frames: 0.0,
            block_idle: true,
            streams: AudioStreams::with_registration(set, Some(registration)),
            ring_bytes_out: None,
            render_mode: RenderMode::Auto,
            host_render_mode: None,
            offline: false,
            block_wait: Duration::ZERO,
            offline_wait_left: Duration::ZERO,
            offline_holdoff: 0,
            clock: RenderClock::Wall(Instant::now()),
            timing_start: None,
            timing_frames: 0,
            timing_waited: Duration::ZERO,
            fast_windows: 0,
            last_block_start: None,
            last_block_frames: 0,
        }
    }

    /// Publish stream underruns on `counter` (the bridge's) instead of
    /// the sampler's own.
    pub fn set_underrun_counter(&mut self, counter: Arc<AtomicU64>) {
        self.streams.underruns = counter;
    }

    /// Publish the bytes of ring storage this sampler's streams hold on
    /// `counter` (the bridge's), once a block.
    pub fn set_ring_bytes_counter(&mut self, counter: Arc<AtomicU64>) {
        self.ring_bytes_out = Some(counter);
    }

    /// Stream underruns so far (see [`crate::stream`]).
    pub fn stream_underruns(&self) -> u64 {
        self.streams.underruns.load(Ordering::Relaxed)
    }

    /// Of [`stream_underruns`](Self::stream_underruns), the hits whose
    /// take found no free ring.
    pub fn stream_ring_misses(&self) -> u64 {
        self.streams.ring_misses()
    }

    /// This sampler's tail rings (test hooks: stall or slow its reader).
    pub fn stream_set(&self) -> &Arc<StreamSet> {
        self.streams.set()
    }

    /// Rings the sampler holds for voices (or has not yet seen let go).
    pub fn stream_rings_claimed(&self) -> usize {
        self.streams.claimed_count()
    }

    /// Fix how the sampler is rendered: live, offline (a bounce), or
    /// [`RenderMode::Auto`] (the default) to go by what the host declared
    /// and, while it declared nothing, by the block timing. A fixed mode
    /// overrides the host's.
    pub fn set_render_mode(&mut self, mode: RenderMode) {
        self.render_mode = mode;
        self.offline = mode == RenderMode::Offline;
        self.restart_auto();
    }

    /// Read what the host declares from `mode` (the bridge's
    /// `host_render_mode`: one of the `stream::HOST_RENDER_*` values),
    /// once a block.
    pub fn set_host_render_mode(&mut self, mode: Arc<AtomicU8>) {
        self.host_render_mode = Some(mode);
    }

    /// Time [`RenderMode::Auto`]'s detector by `nanos` instead of the
    /// wall clock (test hook). Time spent waiting for the reader is then
    /// not taken off: the caller's clock does not run during a wait.
    #[doc(hidden)]
    pub fn set_manual_clock(&mut self, nanos: Arc<AtomicU64>) {
        self.clock = RenderClock::Manual(nanos);
        self.restart_auto();
    }

    /// Start the timing detector over, as live: a render after this
    /// (`reset`, `activate`) proves itself offline afresh.
    pub fn restart_render_timing(&mut self) {
        self.restart_auto();
        if self.render_mode == RenderMode::Auto {
            self.offline = false;
        }
    }

    fn restart_auto(&mut self) {
        self.timing_start = None;
        self.timing_frames = 0;
        self.timing_waited = Duration::ZERO;
        self.fast_windows = 0;
        self.last_block_start = None;
        self.last_block_frames = 0;
    }

    /// Whether the block in progress (or the last one) rendered offline.
    pub fn renders_offline(&self) -> bool {
        self.offline
    }

    /// The most the block in progress (or the last one) may wait for the
    /// reader in all: zero live.
    pub fn block_wait_budget(&self) -> Duration {
        self.block_wait
    }

    /// Times a block has waited for a tail frame (offline only).
    pub fn stream_offline_waits(&self) -> u64 {
        self.streams.set.offline_waits()
    }

    /// Set the host sample rate the fades are timed against, so a choke
    /// or swap fade lasts the same milliseconds at every rate. Called by
    /// [`load_defaults`](Self::load_defaults) (i.e. from `initialize`),
    /// never from the audio thread.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate > 0.0 && sample_rate.is_finite() {
            self.sample_rate = sample_rate;
            self.release_frames = fade_frames(RELEASE_FADE_MS, sample_rate);
            self.swap_frames = fade_frames(SWAP_FADE_MS, sample_rate);
            self.steal_frames = fade_frames(STEAL_FADE_MS, sample_rate);
        }
    }

    /// The host rate fades are timed against.
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// How many stolen voices are still fading out in tail slots.
    pub fn tail_voices_active(&self) -> usize {
        self.tails.iter().filter(|v| v.active).count()
    }

    /// The tail slots themselves, active or not.
    pub fn tail_voices(&self) -> &[Voice] {
        &self.tails
    }

    /// Refresh the global trigger settings from the params. Called once
    /// per block from `process()`, before any event is drained, so every
    /// hit in the block is started under the same settings.
    pub fn update_global_settings(&mut self, params: &DrumParams) {
        self.globals = GlobalSettings {
            max_voices: (params.polyphony.value().max(1) as usize).min(MAX_VOICES),
            velocity_curve: params.velocity_curve.value(),
            round_robin: RoundRobinMode::from_param(params.round_robin_mode.value()),
            output_mode: OutputMode::from_param(params.output_mode.value()),
        };
        for (settings, pad) in self.pad_settings.iter_mut().zip(params.pads.iter()) {
            *settings = PadSettings::from_params(pad, self.sample_rate);
        }
    }

    /// The settings hits are currently started with.
    pub fn global_settings(&self) -> GlobalSettings {
        self.globals
    }

    /// The settings a hit on pad `index` is currently started with.
    pub fn pad_settings(&self, index: usize) -> PadSettings {
        self.pad_settings
            .get(index)
            .copied()
            .unwrap_or_default()
    }

    /// Attach the shared last-RR display array so the editor can show
    /// per-pad round-robin indicators.
    pub fn set_last_rr(&mut self, last_rr: Arc<[AtomicU32; NUM_PADS]>) {
        self.last_rr = Some(last_rr);
    }

    /// Attach the shared OUT meter so the editor's status bar can show the
    /// plugin's real output level instead of a dead bar.
    pub fn set_out_peak(&mut self, out_peak: Arc<[AtomicU32; 2]>) {
        self.out_peak = Some(out_peak);
    }

    /// Attach the kit load progress the bridge publishes, so taking a kit
    /// from the mailbox marks the load complete.
    pub fn set_load_progress(&mut self, progress: Arc<KitLoadProgress>) {
        self.load_progress = Some(progress);
    }

    /// Bytes of decoded sample data this kit holds, counting every mic
    /// bank, velocity layer and round-robin take. Used for the status
    /// bar's memory readout, which is a measurement of the kit — not a
    /// guess at the process's RSS.
    pub fn total_sample_bytes(&self) -> usize {
        crate::sample_info::total_sample_bytes(&self.pads)
    }

    /// Load the embedded default samples as a single-bank fallback kit.
    /// Called once from `initialize()` so the plugin always boots with
    /// audible sound — even before a real Drummica kit is loaded from
    /// disk. The embedded fallback has no overhead bank, so it renders
    /// into the pad's assigned close-mic output port (or Main for Clap /
    /// Cowbell) with nothing on the Overhead port.
    pub fn load_defaults(&mut self, sample_rate: f32) {
        self.load_defaults_sourced(sample_rate);
    }

    /// [`load_defaults`](Self::load_defaults), returning per pad where
    /// its take came from: the shared cache (someone held it) or a fresh
    /// decode; `None` for a pad that got no take.
    pub fn load_defaults_sourced(
        &mut self,
        sample_rate: f32,
    ) -> Vec<Option<crate::kit_loader::cache::Source>> {
        self.set_sample_rate(sample_rate);
        self.pads.clear();
        let mut sources = Vec::with_capacity(PAD_MAPPINGS.len());

        for mapping in &PAD_MAPPINGS {
            // The embedded WAVs are mono, and stay mono (E5); the shared
            // cache gives every instance the same copy.
            match crate::kit_loader::build_fallback_pad_sourced(mapping, sample_rate) {
                Ok((pad, source)) => {
                    self.pads.push(pad);
                    sources.push(Some(source));
                }
                Err(e) => {
                    sources.push(None);
                    eprintln!("Failed to load sample for {}: {}", mapping.name, e);
                    self.pads.push(LoadedPad {
                        name: mapping.name.to_string(),
                        choke_group: mapping.choke_group,
                        output_group: mapping.output_group,
                        close_mics: Vec::new(),
                        overhead: None,
                    });
                }
            }
        }
        sources
    }

    /// Install `pads` as the live kit at once, with every voice silenced.
    /// For `initialize` only — the plugin is inactive, so nothing is
    /// sounding to fade, and the old kit is freed on the calling (main)
    /// thread. Never call this from the audio thread.
    pub fn install_kit(&mut self, pads: Vec<LoadedPad>) {
        self.reset();
        self.rr_counters = [[0; MAX_LAYERS]; NUM_PADS];
        self.rr_last = [[NO_LAST_TAKE; MAX_LAYERS]; NUM_PADS];
        self.pads = pads;
    }

    /// Audio-thread: check for a freshly loaded kit and swap it in if one is
    /// waiting. Called once per `process()` call from `lib.rs`. Voices that
    /// are still sounding fade out over [`SWAP_FADE_MS`] instead of being
    /// cut; the old `Vec<LoadedPad>` is parked in a `retired_pads` slot so
    /// those voices keep reading valid sample data until the fade ends,
    /// after which `end_block` hands it to the janitor thread so the
    /// heap free happens off-audio.
    ///
    /// A second swap while an earlier kit's voices are still fading parks
    /// its kit in another slot, so those voices fade too rather than
    /// being cut (E2). Nothing here allocates: the slots are fixed,
    /// moving a `Vec` into one is a pointer copy, and the janitor's
    /// channel is a preallocated ring.
    ///
    /// The outgoing kit always goes to a slot first. If every slot is
    /// taken and the janitor's channel is full too, the new kit is left
    /// in the mailbox for the next block rather than dropping either kit
    /// here.
    pub fn try_swap_kit(&mut self) {
        loop {
            let Some(slot) = self.free_retired_slot() else {
                return;
            };
            let Ok(new_pads) = self.kit_receiver.try_recv() else {
                return;
            };
            if let Some(progress) = &self.load_progress {
                progress.note_taken();
            }
            for voice in self.voices.iter_mut().chain(self.tails.iter_mut()) {
                // Voices of an earlier retired kit keep their own fade
                // against their own slot.
                if voice.active && !voice.retired {
                    voice.retired = true;
                    voice.retired_slot = slot as u8;
                    voice.force_fade(self.swap_frames);
                }
            }
            self.rr_counters = [[0; MAX_LAYERS]; NUM_PADS];
            self.rr_last = [[NO_LAST_TAKE; MAX_LAYERS]; NUM_PADS];
            let old_pads = std::mem::replace(&mut self.pads, new_pads);
            // Parked even when no voice reads it: `end_block` ships it to
            // the janitor this block, or a later one if the channel is
            // full.
            self.swap_count += 1;
            self.retired_pads[slot] = Some(old_pads);
            self.retired_stamp[slot] = self.swap_count;
        }
    }

    /// A retired-kit slot the next swap can park the outgoing kit in.
    ///
    /// With every slot taken, the oldest kit's voices are cut and the kit
    /// goes to the janitor — reachable only with `RETIRED_KITS + 1` swaps
    /// inside one fade. If the janitor's channel is full as well, the kit
    /// stays where it is (its voices already silenced) and there is no
    /// slot this block.
    fn free_retired_slot(&mut self) -> Option<usize> {
        if let Some(slot) = self.retired_pads.iter().position(Option::is_none) {
            return Some(slot);
        }
        // A kit no voice reads any more can go first.
        self.ship_idle_retired();
        if let Some(slot) = self.retired_pads.iter().position(Option::is_none) {
            return Some(slot);
        }
        let (oldest, _) = self
            .retired_stamp
            .iter()
            .enumerate()
            .min_by_key(|(_, stamp)| **stamp)
            .expect("RETIRED_KITS > 0");
        for voice in self.voices.iter_mut().chain(self.tails.iter_mut()) {
            if voice.retired && voice.retired_slot as usize == oldest {
                voice.active = false;
            }
        }
        let kit = self.retired_pads[oldest].take()?;
        match self.janitor_sender.try_send(kit) {
            Ok(()) => Some(oldest),
            Err(err) => {
                // Full: keep it parked and try again next block. (The
                // janitor cannot disconnect while the sampler holds the
                // sender; keeping the kit is right then too.)
                self.retired_pads[oldest] = Some(err.into_inner());
                None
            }
        }
    }

    /// Hand every retired kit no voice reads any more to the janitor. One
    /// that does not fit in the janitor's channel stays parked for a later
    /// block — never dropped here, on the audio thread.
    fn ship_idle_retired(&mut self) {
        for slot in 0..RETIRED_KITS {
            if self.retired_pads[slot].is_none() {
                continue;
            }
            let in_use = self
                .voices
                .iter()
                .chain(self.tails.iter())
                .any(|v| v.active && v.retired && v.retired_slot as usize == slot);
            if !in_use {
                if let Some(retired) = self.retired_pads[slot].take() {
                    if let Err(err) = self.janitor_sender.try_send(retired) {
                        self.retired_pads[slot] = Some(err.into_inner());
                        return;
                    }
                }
            }
        }
    }

    /// How many retired kits are parked, waiting for their voices to fade
    /// or for room in the janitor's channel.
    pub fn retired_kits_parked(&self) -> usize {
        self.retired_pads.iter().filter(|k| k.is_some()).count()
    }

    /// Move the sounding voice in main slot `idx` to a tail slot and fade
    /// it out there, so the slot can take a new hit without cutting the
    /// old one dead (E1). A struct copy — nothing allocates.
    ///
    /// With every tail busy the quietest one is reused — the one whose cut
    /// is least audible. Equally quiet tails (every victim of a burst on
    /// one frame starts its fade at the same gain) go by fewest fade
    /// frames left, then by least heard: a victim that had not rendered a
    /// frame before it was stolen goes before one that was sounding.
    /// (Comparing frames left alone picked tail 0 every time a burst of
    /// steals filled the tails at once, cutting a full-level voice.)
    fn steal_to_tail(&mut self, idx: usize) {
        let victim = self.voices[idx];
        self.voices[idx].active = false;
        let tail = match self.tails.iter().position(|t| !t.active) {
            Some(t) => t,
            None => self
                .tails
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    let left = |t: &Voice| t.release_len.saturating_sub(t.release_pos);
                    a.current_gain()
                        .total_cmp(&b.current_gain())
                        .then_with(|| left(a).cmp(&left(b)))
                        .then_with(|| a.position.cmp(&b.position))
                })
                .map(|(t, _)| t)
                .unwrap_or(0),
        };
        let mut victim = victim;
        victim.force_fade(self.steal_frames);
        self.tails[tail] = victim;
    }

    /// Trigger a note-on event. Allocates **one voice per loaded mic bank**
    /// for the matching pad — so a kick hit fires up to 3 voices (KickIn,
    /// KickOut, OH), a tom hit fires 2 (close + OH), and a cymbal hit on
    /// Drummica fires only 1 (the overhead). All voices for a hit share
    /// the same velocity layer, round-robin index, choke group, and age
    /// so they play in lockstep.
    ///
    /// The layer and take are picked on the reference bank (the first
    /// close bank, else the overhead). A bank of the same shape plays
    /// that very cell — the same strike, on another mic. A bank whose
    /// recording has fewer layers or takes there plays the one at the
    /// same relative position ([`map_relative`], E7) rather than going
    /// silent.
    ///
    /// Routing (E11): in [`OutputMode::Stereo`] every voice sums to Main;
    /// in [`OutputMode::Multi`] the close banks play on the pad's output
    /// port and the overhead bank on the Overhead port — for every pad,
    /// so a cymbal recorded on the overheads only plays on Overhead.
    ///
    /// The incoming velocity is shaped by the global velocity curve
    /// first, so the curve moves both which layer fires and how hard it
    /// is struck — the two things velocity means here. At the default
    /// (linear) the shaping is an exact identity.
    pub fn note_on(&mut self, note: u8, velocity: f32) {
        let velocity = crate::velocity::shape(velocity, self.globals.velocity_curve);
        let pad_index = match drum_map::pad_index_for_note(note) {
            Some(i) => i,
            None => return,
        };

        if pad_index >= self.pads.len() {
            return;
        }
        let pad = &self.pads[pad_index];

        // Choose a reference bank to drive the velocity layer / round-robin
        // selection. Prefer a close-mic bank (that's where the dynamics
        // tend to live); fall back to overhead. If neither exists the pad
        // is silent and note_on is a no-op.
        let reference_layers: &[VelocityLayer] = if let Some(first) = pad.close_mics.first() {
            &first.layers
        } else if let Some(oh) = &pad.overhead {
            &oh.layers
        } else {
            return;
        };
        if reference_layers.is_empty() {
            return;
        }
        let n_layers = reference_layers.len();
        let layer_index = pick_velocity_layer(velocity, n_layers);
        let layer = &reference_layers[layer_index];
        if layer.round_robins.is_empty() {
            return;
        }
        let counter_slot = layer_index.min(MAX_LAYERS - 1);
        let n_rrs = layer.round_robins.len();
        let rr_index = match self.globals.round_robin {
            RoundRobinMode::Cycle => {
                pick_rr(&mut self.rr_counters[pad_index][counter_slot], n_rrs)
            }
            RoundRobinMode::Random => pick_rr_random(
                &mut self.rr_rng,
                self.rr_last[pad_index][counter_slot],
                n_rrs,
            ),
        };
        // Recorded in both modes: Random must not repeat whatever fired
        // last, however it was chosen.
        self.rr_last[pad_index][counter_slot] = rr_index.min(NO_LAST_TAKE as usize) as u16;

        // Publish the last-played RR for the editor display: both which
        // take fired and how many the layer holds, so the pad can show
        // "take 2 of 3" rather than just "something played".
        if let Some(ref last_rr) = self.last_rr {
            last_rr[pad_index].store(
                crate::rr_display::pack(rr_index, n_rrs),
                Ordering::Relaxed,
            );
        }

        // Single-layer fallback pads bake dynamics into the MIDI velocity;
        // multi-layer kits have the velocity layer already shaped so we
        // use a flat trigger gain.
        let trigger_gain = if n_layers > 1 { 1.0 } else { velocity };
        let settings = self.pad_settings[pad_index];
        let choke_group = settings.choke_group(pad);
        let close_mic_count = pad.close_mics.len();
        // E11: in Stereo every bank sums to Main; in Multi the close mics
        // play on the pad's output port and the overhead on Overhead.
        let (output_port, oh_port) = match self.globals.output_mode {
            OutputMode::Stereo => (MAIN_PORT_INDEX as u8, MAIN_PORT_INDEX as u8),
            OutputMode::Multi => (settings.close_port(pad), OVERHEAD_PORT_INDEX as u8),
        };
        let has_overhead = pad.overhead.is_some();

        // Handle choke groups: release any active voices in the same choke group
        if let Some(group) = choke_group {
            janitor::choke_group(&mut self.voices, group, self.release_frames);
        }

        // Build the list of destinations we need to allocate a voice for.
        // Kick + snare: one CloseMic voice per bank (two, trimmed by
        // mic1/mic2). Tom + hat: one CloseMic voice (mic1). Cymbal: no
        // close mic. Plus an Overhead
        // voice if the pad has one loaded.
        let mut destinations: [Option<VoiceDestination>; 3] = [None, None, None];
        // The (layer, take) each destination's bank plays.
        let mut cells = [(0usize, 0usize); 3];
        // The ring each destination streams its take's tail through (E14).
        // Claimed now, while the pad is at hand: the reader starts filling
        // it at once, while the voice plays its head.
        let mut rings = [NO_RING; 3];
        let streams = &mut self.streams;
        // Offline, a hit may wait for the reader to take a pending
        // request, which frees its ring.
        let mut wait = (self.offline && self.offline_holdoff == 0)
            .then_some(&mut self.offline_wait_left);
        // Where each destination's voice starts in its take (E8's sample
        // start). A streamed take starts inside its head: a start past
        // the head is clamped to its last frame — out of reach of the
        // shipped preloads (≥ 32 k frames) at any rate up to 192 kHz,
        // whose 100 ms is 19.2 k frames.
        let mut starts = [0usize; 3];
        let start_frames = settings.start_frames as usize;
        let rate = settings.rate;
        let mut ring_for = |bank: &LoadedMicBank, (layer, rr): (usize, usize)| -> (u8, usize) {
            let take = bank.layers.get(layer).and_then(|l| l.round_robins.get(rr));
            match take.and_then(|t| t.tail().map(|tail| (tail, t.resident_frames()))) {
                Some((tail, head)) => {
                    let start = start_frames.min(head.saturating_sub(1));
                    let pace = Pace::at_rate(head - start, rate);
                    (streams.claim(tail, head, pace, wait.as_deref_mut()), start)
                }
                None => (NO_RING, start_frames),
            }
        };
        let cell_in = |bank: &LoadedMicBank| -> (usize, usize) {
            let layer = map_relative(layer_index, n_layers, bank.layers.len());
            let takes = bank
                .layers
                .get(layer)
                .map_or(0, |l| l.round_robins.len());
            (layer, map_relative(rr_index, n_rrs, takes))
        };
        let mut dest_count = 0;
        for bank_index in 0..close_mic_count.min(2) {
            cells[dest_count] = cell_in(&pad.close_mics[bank_index]);
            (rings[dest_count], starts[dest_count]) =
                ring_for(&pad.close_mics[bank_index], cells[dest_count]);
            destinations[dest_count] = Some(VoiceDestination::CloseMic {
                bank_index,
                output_port,
            });
            dest_count += 1;
        }
        if has_overhead && dest_count < destinations.len() {
            // Every pad's overhead take goes to the Overhead port in
            // Multi — the pads the library records with overheads only
            // (every cymbal, ride and china piece in Drummica) included,
            // which until E11 played on their own group port (Cymbals).
            // Stereo has a stereo kit on Main for whoever wants one port.
            if let Some(oh) = &pad.overhead {
                cells[dest_count] = cell_in(oh);
                (rings[dest_count], starts[dest_count]) = ring_for(oh, cells[dest_count]);
            }
            destinations[dest_count] = Some(VoiceDestination::Overhead {
                output_port: oh_port,
            });
            dest_count += 1;
        }

        // Allocate one voice per destination. All share pad, note, the
        // hit's cell (mapped onto each bank), choke group, and base gain.
        // Age is bumped together so voice stealing treats the set as a
        // single unit.
        self.voice_counter += 1;
        let shared_age = self.voice_counter;
        for (((dest_slot, &(bank_layer, bank_rr)), &ring), &start) in destinations
            .iter()
            .zip(&cells)
            .zip(&rings)
            .zip(&starts)
            .take(dest_count)
        {
            let Some(dest) = dest_slot else {
                continue;
            };
            let dest = *dest;
            let voice_idx =
                janitor::find_free_voice(&self.voices, pad_index, self.globals.max_voices);
            if self.voices[voice_idx].active {
                self.steal_to_tail(voice_idx);
            }
            let voice = &mut self.voices[voice_idx];
            voice.active = true;
            voice.pad_index = pad_index;
            voice.note = note;
            voice.base_gain = trigger_gain;
            voice.destination = dest;
            voice.layer_index = bank_layer;
            voice.rr_index = bank_rr;
            voice.position = start;
            voice.rate = settings.rate;
            voice.frac = 0.0;
            voice.env_pos = 0;
            voice.hold_frames = settings.hold_frames;
            voice.decay_frames = settings.decay_frames;
            voice.choke_group = choke_group;
            voice.retired = false;
            voice.state = VoiceState::Playing;
            voice.release_pos = 0;
            voice.age = shared_age;
            voice.ring = ring;
            voice.stream_lost = false;
        }
    }

    /// Drum samples are one-shots: musical NOTE_OFF is intentionally
    /// ignored so the sample plays through to its natural end regardless
    /// of how short the MIDI note is. Host-level CLAP choke events take
    /// the `choke_note` path instead.
    pub fn note_off(&mut self, _note: u8) {}

    /// Host-level "silence this note now" — used by the CLAP host when
    /// playback stops or a track is muted mid-hit. Fades the matching
    /// voices out rather than clicking them off.
    pub fn choke_note(&mut self, note: u8) {
        janitor::choke_note(&mut self.voices, note, self.release_frames);
    }

    /// Decide whether this block renders offline (see [`RenderMode`]) and
    /// set its wait budget.
    ///
    /// A fixed mode decides, then a mode the host declared: offline waits
    /// up to [`OFFLINE_WAIT_PER_BLOCK`], real time never. With nothing
    /// declared, the timing does (see `measure_block`): a block waits at
    /// most [`AUTO_WAIT_PER_BLOCK`] once the last window rendered more
    /// than [`OFFLINE_ENTER_RATIO`] times faster than the clock, and
    /// [`AUTO_SUSTAINED_WAIT_PER_BLOCK`] once [`SUSTAINED_WINDOWS`] did in
    /// a row.
    fn update_render_timing(&mut self, frames: usize) {
        // What the last block spent waiting for the reader.
        let waited = self.block_wait.saturating_sub(self.offline_wait_left);
        self.offline_holdoff = self.offline_holdoff.saturating_sub(frames as u64);
        let host = self
            .host_render_mode
            .as_ref()
            .map_or(HOST_RENDER_UNKNOWN, |m| m.load(Ordering::Relaxed));
        let mode = match (self.render_mode, host) {
            (RenderMode::Auto, HOST_RENDER_REALTIME) => RenderMode::Realtime,
            (RenderMode::Auto, HOST_RENDER_OFFLINE) => RenderMode::Offline,
            (mode, _) => mode,
        };
        self.block_wait = match mode {
            RenderMode::Realtime => {
                self.restart_auto();
                self.offline = false;
                Duration::ZERO
            }
            RenderMode::Offline => {
                self.restart_auto();
                self.offline = true;
                OFFLINE_WAIT_PER_BLOCK
            }
            RenderMode::Auto => {
                if frames > 0 {
                    self.measure_block(frames, waited);
                }
                self.offline = self.fast_windows > 0;
                if self.fast_windows >= SUSTAINED_WINDOWS {
                    AUTO_SUSTAINED_WAIT_PER_BLOCK
                } else if self.offline {
                    AUTO_WAIT_PER_BLOCK
                } else {
                    Duration::ZERO
                }
            }
        };
        self.offline_wait_left = self.block_wait;
    }

    /// The timing detector, at the start of a block of `frames`: the
    /// previous block took `waited` of waiting for the reader, which is
    /// not render time.
    ///
    /// A block that arrives no faster than [`LIVE_BLOCK_RATIO`] times real
    /// time after the previous one — a live callback, a pause, playback
    /// resuming after a bounce — makes the detector live at once and
    /// starts a new window, before this block renders. Otherwise the
    /// previous block counts toward the window, and a full window
    /// ([`OFFLINE_WINDOW_SECS`] of audio) is offline only when it rendered
    /// more than [`OFFLINE_ENTER_RATIO`] times faster than the clock; a
    /// window that did not ends the verdict. Every verdict is re-earned
    /// window by window.
    fn measure_block(&mut self, frames: usize, waited: Duration) {
        let now = self.clock.now();
        let waited = match self.clock {
            RenderClock::Wall(_) => waited,
            RenderClock::Manual(_) => Duration::ZERO,
        };
        let rate = self.sample_rate as f64;
        let live = self.last_block_start.is_none_or(|prev| {
            let gap = now.saturating_sub(prev).saturating_sub(waited).as_secs_f64();
            gap * LIVE_BLOCK_RATIO >= self.last_block_frames as f64 / rate
        });
        if live {
            self.fast_windows = 0;
            self.timing_start = Some(now);
            self.timing_frames = 0;
            self.timing_waited = Duration::ZERO;
        } else {
            self.timing_frames += self.last_block_frames as u64;
            self.timing_waited += waited;
            let audio = self.timing_frames as f64 / rate;
            if audio >= OFFLINE_WINDOW_SECS {
                let start = self.timing_start.unwrap_or(now);
                let wall = now
                    .saturating_sub(start)
                    .saturating_sub(self.timing_waited)
                    .as_secs_f64();
                self.fast_windows = if audio > OFFLINE_ENTER_RATIO * wall {
                    self.fast_windows.saturating_add(1)
                } else {
                    0
                };
                self.timing_start = Some(now);
                self.timing_frames = 0;
                self.timing_waited = Duration::ZERO;
            }
        }
        self.last_block_start = Some(now);
        self.last_block_frames = frames;
    }

    /// Render `frames` samples into each of the 7 output ports in
    /// `outputs`, starting each of `hits` at its own frame. Expects
    /// `outputs.len() >= NUM_OUTPUT_PORTS`.
    ///
    /// Voices started before the call (a `note_on` outside any block)
    /// sound from frame 0. Hit offsets follow `process()`'s contract: one
    /// past the block lands on its last frame, and one earlier than the
    /// hit before it lands at the frame already reached. It used to take
    /// no hits at all, so a caller with events inside the block could only
    /// start them at frame 0 (FU-G1); `process()` itself interleaves
    /// chokes and editor auditions too, so it drives
    /// [`begin_block`](Self::begin_block) / [`render_span`](Self::render_span)
    /// / [`end_block`](Self::end_block) directly.
    pub fn render_block(
        &mut self,
        outputs: &mut [PortBuffers<'_>],
        frames: usize,
        params: &DrumParams,
        hits: &[Hit],
    ) {
        self.begin_block(outputs, frames, params);
        let last_frame = frames.saturating_sub(1);
        let mut cursor = 0usize;
        for hit in hits {
            let at = hit.frame.min(last_frame).max(cursor);
            if at > cursor {
                self.render_span(outputs, cursor, at);
                cursor = at;
            }
            self.note_on(hit.note, hit.velocity);
        }
        self.render_span(outputs, cursor, frames);
        self.end_block(outputs, frames, params);
    }

    /// Start a block of `frames` frames: zero every port and snapshot
    /// the per-pad params the block ramps toward. Follow with one or
    /// more [`render_span`](Self::render_span) calls covering
    /// `0..frames` in order, then [`end_block`](Self::end_block).
    pub fn begin_block(
        &mut self,
        outputs: &mut [PortBuffers<'_>],
        frames: usize,
        params: &DrumParams,
    ) {
        // Zero every port for this block before we start summing voices.
        for port in outputs.iter_mut() {
            port.left[..frames].fill(0.0);
            port.right[..frames].fill(0.0);
        }

        self.update_render_timing(frames);

        self.block_idle = self.pads.is_empty() && self.retired_pads.iter().all(Option::is_none);
        if self.block_idle {
            return;
        }

        // Snapshot per-pad params once per block so the inner render loop
        // doesn't re-read atomics for every sample. Each param is then
        // linearly ramped from last block's snapshot across this block
        // (same declick scheme as the master volume in `end_block`).
        // Levels are dB params; the ramps run in gain.
        let mut pad_volume = [0.0f32; NUM_PADS];
        let mut pad_pan = [0.0f32; NUM_PADS];
        let mut pad_trim = [[1.0f32; MIC_SLOTS]; NUM_PADS];
        for (i, pad) in params.pads.iter().enumerate() {
            pad_volume[i] = if pad.mute.value() {
                0.0
            } else {
                db_to_gain(pad.volume.value())
            };
            pad_pan[i] = pad.pan.value();
            for (slot, trim) in pad.trims.iter().enumerate() {
                pad_trim[i][slot] = db_to_gain(trim.value());
            }
        }
        if !self.pad_prev_valid {
            // First block ever: start the ramps at the current values
            // so we don't sweep in from arbitrary defaults.
            self.prev_pad_volume = pad_volume;
            self.prev_pad_pan = pad_pan;
            self.prev_pad_trim = pad_trim;
            self.pad_prev_valid = true;
        }
        self.cur_pad_volume = pad_volume;
        self.cur_pad_pan = pad_pan;
        self.cur_pad_trim = pad_trim;
        self.block_inv_frames = if frames > 0 {
            1.0 / frames as f32
        } else {
            0.0
        };
    }

    /// Sum every active voice into frames `start..end` of the block begun
    /// by [`begin_block`](Self::begin_block). A voice started between two
    /// spans therefore sounds from the frame the second span starts at,
    /// which is how `process()` honours note-event offsets.
    pub fn render_span(&mut self, outputs: &mut [PortBuffers<'_>], start: usize, end: usize) {
        if self.block_idle || start >= end {
            return;
        }
        let inv_frames = self.block_inv_frames;
        let streams = &self.streams;
        let offline = self.offline;
        let offline_wait_left = &mut self.offline_wait_left;
        let offline_holdoff = &mut self.offline_holdoff;
        // Only a long wait running dry means a dead reader; a short Auto
        // budget runs dry as a matter of course.
        let long_waits = self.block_wait >= AUTO_SUSTAINED_WAIT_PER_BLOCK;
        let holdoff_frames = (OFFLINE_WAIT_HOLDOFF_SECS * self.sample_rate) as u64;
        let pad_volume = &self.cur_pad_volume;
        let pad_pan = &self.cur_pad_pan;
        let pad_trim = &self.cur_pad_trim;
        // A voice whose tail will not come fades out over this, ending
        // where its frames end.
        let cut_fade = self.release_frames as usize;

        for voice in self.voices.iter_mut().chain(self.tails.iter_mut()) {
            if !voice.active {
                continue;
            }
            let pad_index = voice.pad_index;
            // Voices that predate a kit swap fade out against the
            // retired kit's data; everything else reads the current one.
            let pad_source = if voice.retired {
                self.retired_pads
                    .get(voice.retired_slot as usize)
                    .and_then(|kit| kit.as_deref())
            } else {
                Some(self.pads.as_slice())
            };
            let Some(pad) = pad_source.and_then(|pads| pads.get(pad_index)) else {
                voice.active = false;
                continue;
            };

            // Resolve the voice's source bank from its destination tag.
            let bank: Option<&LoadedMicBank> = match voice.destination {
                VoiceDestination::CloseMic { bank_index, .. } => pad.close_mics.get(bank_index),
                VoiceDestination::Overhead { .. } => pad.overhead.as_ref(),
            };
            let Some(bank) = bank else {
                voice.active = false;
                continue;
            };
            if voice.layer_index >= bank.layers.len() {
                voice.active = false;
                continue;
            }
            let layer = &bank.layers[voice.layer_index];
            if voice.rr_index >= layer.round_robins.len() {
                voice.active = false;
                continue;
            }
            let sample: &SampleData = &layer.round_robins[voice.rr_index];
            // Mono takes are read onto both sides: the right channel's
            // index is the left's for a mono take (E5), which plays the
            // very floats a duplicated-stereo take held.
            let data = sample.samples();
            let stride = sample.channels();
            let right_offset = stride - 1;
            let resident = sample.resident_frames();
            // A streamed take (E14) is `total` long: frames past the
            // resident head come from the voice's ring, as far as the
            // reader has written it.
            let total = sample.frames();
            let ring: Option<&Ring> = if voice.ring == NO_RING {
                None
            } else {
                streams.ring(voice.ring)
            };
            let (mut written, failed) = ring.map_or((0, false), |r| r.published());
            let mut missing = false;
            // Where the voice's frames end: the take's end — or, when its
            // tail will not come (no ring, a failed stream), the end of
            // what it has. It fades out to end there rather than cut off
            // or play silence to the take's end.
            let mut end_at = if total <= resident {
                total
            } else if ring.is_none() {
                resident
            } else if failed {
                resident + written as usize
            } else {
                total
            };
            if end_at < total && !voice.stream_lost {
                voice.stream_lost = true;
                // A hit that found no ring was counted at its claim.
                if ring.is_some() {
                    streams.underruns.fetch_add(1, Ordering::Relaxed);
                }
            }

            // Which port does this voice sum into, and what's the
            // destination-specific gain multiplier? Computed at both
            // the previous and current block's param snapshots so the
            // inner loop can ramp between them.
            let (port_index, slot) = match voice.destination {
                VoiceDestination::CloseMic {
                    output_port,
                    bank_index,
                } => (output_port as usize, MicSlot::close(bank_index)),
                VoiceDestination::Overhead { output_port } => {
                    (output_port as usize, MicSlot::Overhead)
                }
            };
            let dest_gain0 = self.prev_pad_trim[pad_index][slot as usize];
            let dest_gain1 = pad_trim[pad_index][slot as usize];
            let vol0 = self.prev_pad_volume[pad_index];
            let vol1 = pad_volume[pad_index];
            let (pan_l0, pan_r0) =
                resonance_dsp::stereo_balance(self.prev_pad_pan[pad_index]);
            let (pan_l1, pan_r1) = resonance_dsp::stereo_balance(pad_pan[pad_index]);

            // Per-sample ramp increments, mirroring the master volume
            // ramp below: start at the previous block's value and step
            // toward the current one across the block. Pan and balance
            // ramp in gain space, which keeps the path continuous (and
            // linear in the pan position, since stereo_balance is
            // piecewise-linear). A span that starts mid-block picks the
            // ramps up where they stand at its first frame (exactly the
            // start values when it starts at 0).
            let vol_step = (vol1 - vol0) * inv_frames;
            let dest_step = (dest_gain1 - dest_gain0) * inv_frames;
            let pan_l_step = (pan_l1 - pan_l0) * inv_frames;
            let pan_r_step = (pan_r1 - pan_r0) * inv_frames;
            let at = start as f32;
            let mut vol = vol0 + vol_step * at;
            let mut dest_gain = dest_gain0 + dest_step * at;
            let mut pan_l = pan_l0 + pan_l_step * at;
            let mut pan_r = pan_r0 + pan_r_step * at;

            // Split-borrow the destination port's buffers so the inner
            // loop can write into both channels cheaply. A port the host
            // did not provide: the voice plays on unheard, in time.
            let mut port = outputs
                .get_mut(port_index)
                .map(|p| (&mut p.left[..end], &mut p.right[..end]));

            // E8: a voice at pitch plays on the integer path below, exactly
            // as it always did; a tuned one reads between frames.
            let unity = voice.rate == 1.0;
            let rate = voice.rate;
            let ahd = voice.decay_frames > 0;

            for frame in start..end {
                if voice.position >= end_at {
                    voice.active = false;
                    break;
                }
                if voice.release_done() || voice.ahd_done() {
                    voice.active = false;
                    break;
                }
                if end_at < total {
                    // Output frames until the voice's frames end.
                    let left_src = end_at - voice.position;
                    let left = if unity {
                        left_src
                    } else {
                        ((left_src as f32 / rate) as usize).max(1)
                    };
                    if left <= cut_fade {
                        voice.end_within(left);
                    }
                }

                let (sample_l, sample_r) = if !unity {
                    // Fractional playback, 4-point Hermite over frames
                    // p−1 … p+2, through the same head / ring path.
                    let pos = voice.position;
                    let need = (pos + 2).min(end_at - 1);
                    if let (Some(ring), true) = (ring, need >= resident) {
                        let at = (need - resident) as u64;
                        if at >= written
                            && offline
                            && *offline_holdoff == 0
                            && !offline_wait_left.is_zero()
                        {
                            streams.set.waits.fetch_add(1, Ordering::Relaxed);
                            // Keep frame p−1 from being overwritten while
                            // the reader catches up.
                            let keep = (pos.saturating_sub(1).max(resident) - resident) as u64;
                            let failed;
                            (written, failed) =
                                wait_for_frame(ring, at, keep, offline_wait_left);
                            if failed {
                                end_at = resident + written as usize;
                                if !voice.stream_lost {
                                    voice.stream_lost = true;
                                    streams.underruns.fetch_add(1, Ordering::Relaxed);
                                }
                                if pos >= end_at {
                                    voice.active = false;
                                    break;
                                }
                            } else if at >= written && offline_wait_left.is_zero() && long_waits {
                                *offline_holdoff = holdoff_frames;
                            }
                        }
                    }
                    let fetch = |i: usize| -> Option<(f32, f32)> {
                        if i >= end_at {
                            Some((0.0, 0.0))
                        } else if i < resident {
                            let idx = i * stride;
                            Some((data[idx], data[idx + right_offset]))
                        } else if let Some(ring) = ring {
                            let at = (i - resident) as u64;
                            (at < written).then(|| {
                                (ring.sample(at, stride, 0), ring.sample(at, stride, right_offset))
                            })
                        } else {
                            Some((0.0, 0.0))
                        }
                    };
                    let before = if pos == 0 {
                        Some((0.0, 0.0))
                    } else {
                        fetch(pos - 1)
                    };
                    match (before, fetch(pos), fetch(pos + 1), fetch(pos + 2)) {
                        (Some(a), Some(b), Some(c), Some(d)) => {
                            let t = voice.frac;
                            (hermite(a.0, b.0, c.0, d.0, t), hermite(a.1, b.1, c.1, d.1, t))
                        }
                        _ => {
                            // Not delivered yet: silence, and on in time.
                            missing = true;
                            (0.0, 0.0)
                        }
                    }
                } else if voice.position < resident {
                    let idx = voice.position * stride;
                    (data[idx], data[idx + right_offset])
                } else if let Some(ring) = ring {
                    let at = (voice.position - resident) as u64;
                    if at >= written
                        && offline
                        && *offline_holdoff == 0
                        && !offline_wait_left.is_zero()
                    {
                        streams.set.waits.fetch_add(1, Ordering::Relaxed);
                        let failed;
                        (written, failed) = wait_for_frame(ring, at, at, offline_wait_left);
                        if failed {
                            // Found out at the very frame it is missing:
                            // nothing left to fade over.
                            end_at = resident + written as usize;
                            if !voice.stream_lost {
                                voice.stream_lost = true;
                                streams.underruns.fetch_add(1, Ordering::Relaxed);
                            }
                            if voice.position >= end_at {
                                voice.active = false;
                                break;
                            }
                        } else if at >= written && offline_wait_left.is_zero() && long_waits {
                            // A long budget ran out: stop waiting a while.
                            *offline_holdoff = holdoff_frames;
                        }
                    }
                    if at < written {
                        (ring.sample(at, stride, 0), ring.sample(at, stride, right_offset))
                    } else {
                        // Not delivered yet: silence, and on in time.
                        missing = true;
                        (0.0, 0.0)
                    }
                } else {
                    // A streamed take that found no ring ends with its
                    // head (`end_at`); never reached.
                    voice.active = false;
                    break;
                };
                let env = voice.current_gain();
                // The AHD envelope only multiplies in when a decay is set:
                // Off leaves the gain bit-identical.
                let env = if ahd { env * voice.ahd_gain() } else { env };
                let gain = env * vol * dest_gain;

                if let Some((port_l, port_r)) = port.as_mut() {
                    port_l[frame] += sample_l * gain * pan_l;
                    port_r[frame] += sample_r * gain * pan_r;
                }

                vol += vol_step;
                dest_gain += dest_step;
                pan_l += pan_l_step;
                pan_r += pan_r_step;

                if unity {
                    voice.position += 1;
                } else {
                    let advance = voice.frac + rate;
                    let whole = advance as usize;
                    voice.frac = advance - whole as f32;
                    voice.position += whole;
                }
                voice.env_pos = voice.env_pos.saturating_add(1);
                if voice.state == VoiceState::Releasing {
                    voice.release_pos += 1;
                }
            }
            if let Some(ring) = ring {
                // Room for the reader: every ring frame before the
                // voice's position is done with (a tuned voice still
                // reads the frame before it). And its deadline: what is
                // left of the head, in output frames.
                let done = if unity {
                    voice.position
                } else {
                    voice.position.saturating_sub(1)
                };
                if done > resident {
                    ring.read.store((done - resident) as u64, Ordering::Release);
                }
                let head_left = resident.saturating_sub(voice.position);
                let head_left = if unity {
                    head_left
                } else {
                    (head_left as f32 / rate) as usize
                };
                ring.head_left.store(head_left as u64, Ordering::Relaxed);
            }
            if missing {
                streams.underruns.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Finish the block begun by [`begin_block`](Self::begin_block):
    /// retire a drained kit, roll the param snapshots forward, apply the
    /// master volume ramp and publish the OUT meter.
    pub fn end_block(
        &mut self,
        outputs: &mut [PortBuffers<'_>],
        frames: usize,
        params: &DrumParams,
    ) {
        // Let go of the rings of voices that ended this block (E14).
        self.streams
            .sweep(self.voices.iter().chain(self.tails.iter()));
        if let Some(out) = &self.ring_bytes_out {
            out.store(self.streams.set.ring_bytes(), Ordering::Relaxed);
        }
        if self.block_idle {
            // Nothing to render — the ports are silent, and the OUT meter
            // must say so rather than hold its last value.
            self.publish_out_peak(outputs, frames);
            return;
        }

        // Once the last fading pre-swap voice of a retired kit has
        // ended, its samples are unreferenced: hand them to the janitor.
        self.ship_idle_retired();

        // Next block ramps from this block's snapshots.
        self.prev_pad_volume = self.cur_pad_volume;
        self.prev_pad_pan = self.cur_pad_pan;
        self.prev_pad_trim = self.cur_pad_trim;

        // Apply master volume in-place over every port. Linearly
        // interpolate from the previous block's value to the current
        // one across the block so automation tweaks and user fader
        // moves don't click. With small block sizes (≤512 frames at
        // typical SR) per-sample lerp is essentially free.
        let master_vol = db_to_gain(params.master_volume.value());
        let prev = self.prev_master_volume;
        if (prev - 1.0).abs() > f32::EPSILON
            || (master_vol - 1.0).abs() > f32::EPSILON
            || (master_vol - prev).abs() > f32::EPSILON
        {
            let step = if frames > 0 {
                (master_vol - prev) / frames as f32
            } else {
                0.0
            };
            for port in outputs.iter_mut() {
                let mut g = prev;
                for s in port.left[..frames].iter_mut() {
                    *s *= g;
                    g += step;
                }
                let mut g = prev;
                for s in port.right[..frames].iter_mut() {
                    *s *= g;
                    g += step;
                }
            }
        }
        self.prev_master_volume = master_vol;

        self.publish_out_peak(outputs, frames);
    }

    /// Publish this block's peak level across every output port for the
    /// editor's OUT meter. Written every block (including silent ones) so
    /// the meter falls back to −∞ instead of freezing at the last hit.
    fn publish_out_peak(&self, outputs: &[PortBuffers<'_>], frames: usize) {
        let Some(ref out_peak) = self.out_peak else {
            return;
        };
        let mut peak_l = 0.0f32;
        let mut peak_r = 0.0f32;
        for port in outputs.iter() {
            for s in port.left[..frames].iter() {
                peak_l = peak_l.max(s.abs());
            }
            for s in port.right[..frames].iter() {
                peak_r = peak_r.max(s.abs());
            }
        }
        out_peak[0].store(peak_l.to_bits(), Ordering::Relaxed);
        out_peak[1].store(peak_r.to_bits(), Ordering::Relaxed);
    }

    /// Kill all active voices immediately — CLAP `reset()`.
    ///
    /// The one hard cut left, on purpose. CLAP calls `reset` when the
    /// plugin is *not* processing ("clear all buffers … and kill all
    /// voices"), and the host calls it so the next render starts clean:
    /// the bounce renderer resets every plugin before a pass so no tail
    /// from live playback bleeds into the export
    /// (`resonance-audio/src/engine/bounce/render.rs`, `reset_plugins`).
    /// A fade here would play out at the start of that next render —
    /// exactly the leftover `reset` exists to remove. Nothing is audible
    /// between the cut and the next block, so there is no click to fade.
    pub fn reset(&mut self) {
        janitor::reset_all(&mut self.voices);
        janitor::reset_all(&mut self.tails);
        self.streams
            .sweep(self.voices.iter().chain(self.tails.iter()));
        // A render after a reset (a bounce) is measured afresh.
        self.restart_render_timing();
    }
}

/// 4-point, 3rd-order Hermite (Catmull-Rom) interpolation at `t` (0..1)
/// between `x0` and `x1`, with `xm1` before and `x2` after (E8).
#[inline]
fn hermite(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * t + c2) * t + c1) * t + x0
}

/// Offline only: wait for the reader to deliver ring frame `at`, for at
/// most what is left of `budget` (which is charged for the wait). Returns
/// the ring's `write` — past `at` unless the wait ran out or the stream
/// failed — and whether it failed. Ring frames from `keep` (≤ `at`) on
/// are still to be read: a tuned voice (E8) interpolates over the frame
/// before its position as well.
fn wait_for_frame(ring: &Ring, at: u64, keep: u64, budget: &mut Duration) -> (u64, bool) {
    // The reader only writes where the voice has made room, and serves
    // the neediest ring first: this one, on its tail.
    ring.read.store(keep.min(at), Ordering::Release);
    ring.head_left.store(0, Ordering::Relaxed);
    let mut published = (0, false);
    wait_until(budget, || {
        published = ring.published();
        published.0 > at || published.1
    });
    published
}
