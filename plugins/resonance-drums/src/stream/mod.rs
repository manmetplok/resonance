//! Disk streaming of long takes (drums-plugin-rework.md §7 E14, slice
//! K6b).
//!
//! A take longer than the preload keeps only its first frames — its
//! **head** — in memory ([`crate::kit::SampleData`]); the shared sample
//! cache shares those heads between kits and instances (E5). A voice
//! plays its head from memory, and its **tail** from a per-voice ring
//! that a [`reader::ReaderPool`] thread fills ahead of it, straight from
//! the WAV file ([`resonance_common::WavTail`], bit-identical to a full
//! decode, resampling included).
//!
//! # Threads and ownership
//!
//! - **Rings** are created with the sampler ([`StreamSet`], [`NUM_RINGS`]
//!   of them, [`RING_FRAMES`] stereo frames each), but their sample
//!   storage is allocated by a reader the first time the ring is served
//!   (see `Ring::data`): an untouched ring costs a few hundred bytes, and
//!   [`StreamSet::ring_bytes`] says what the used ones hold. A voice
//!   holds a ring by index ([`crate::voice::Voice::ring`]); moving a
//!   stolen voice to a tail slot moves the index with it.
//! - **The audio thread** claims a ring at note-on and hands it the
//!   take's [`TailSource`] (an `Arc` clone — an atomic increment, never an
//!   allocation; the kit still holds the take, so nothing the audio
//!   thread drops is ever a last reference). It reads ring frames the
//!   reader has published, publishes how far its voice still is from the
//!   tail (the reader's deadline), and lets a ring go once no voice reads
//!   it ([`AudioStreams::sweep`]) — which makes it claimable again at
//!   once, without waiting for the reader to notice. It never waits for the
//!   reader, never locks, never allocates, and never makes a syscall that
//!   can block.
//! - **The reader** threads ([`reader::ReaderPool`]) poll the rings that
//!   are open (no wake-ups from the audio thread: a futex wake is a
//!   syscall), take requests, open the file, fill the rings in deadline
//!   order (see [`reader`]) as far as each voice needs, and drop streams
//!   the audio thread has let go.
//!   They exist only while some sampler is registered.
//!
//! # The ring protocol
//!
//! Every claim gives the ring a new **generation**, and everything the
//! reader publishes is tagged with the generation it was read for:
//!
//! - The audio thread claims a ring whose previous request the reader has
//!   taken (its request slot is empty): it resets `read`, sets `wpos` to
//!   (generation, 0 frames), makes the generation the ring's `active_gen`,
//!   and posts the request — the source, its generation and its start —
//!   with the source pointer last (release). Then it marks the ring open
//!   for the readers.
//! - The reader takes the request (generation and pointer read
//!   consistently, see [`Ring::take_request`]), and serves it while
//!   `active_gen` still names it. It writes frames `write..` only where
//!   `frame < read + RING_FRAMES`, then publishes `wpos = (generation,
//!   write)` by compare-and-swap from what it last published — which fails
//!   once the ring has moved on, and the reader drops the stream.
//! - The audio thread reads frames below `write` only when `wpos` carries
//!   the ring's current generation (acquire), and publishes how far it got
//!   as `read`. Letting a ring go is `active_gen = 0`.
//!
//! A stale reader can still be writing frames of an old generation into
//! a ring that has been claimed again; that is harmless. One reader
//! thread serves a ring at a time (its `reader` lock), and it switches to
//! the new generation only after finishing with the old one, so every
//! frame the new generation publishes was written after the last stale
//! write, and the audio thread reads no frame before it is published.
//! Samples are stored as `AtomicU32` bit patterns, so the shared buffer
//! needs no `unsafe`.
//!
//! # Underruns
//!
//! A frame the reader has not delivered yet plays as silence, and the
//! voice moves on in time: the reader skips ahead to where the voice is
//! when it catches up. Each voice-span with missing frames counts once in
//! the underrun counter ([`crate::KitBridge::stream_underruns`]).
//!
//! # Offline rendering
//!
//! A bounce renders as fast as the CPU allows, far faster than a reader
//! can stream at a guaranteed pace, and an underrun there would be a gap
//! in the export. So the sampler tells offline from live rendering
//! ([`RenderMode`]), and offline a missing frame is waited for (bounded),
//! which CLAP permits for offline processing; live, never.
//!
//! What it goes by, first match wins:
//!
//! 1. a mode set on the sampler itself
//!    ([`crate::dsp::DrumSampler::set_render_mode`]: tests and headless
//!    callers);
//! 2. the mode the **host** declared, through
//!    [`crate::KitBridge::host_render_mode`] (one of the `HOST_RENDER_*`
//!    values below, read once per block): offline waits for as long as a
//!    read can take, real time never waits;
//! 3. while the host has declared nothing, a timing heuristic (see
//!    `DrumSampler::update_render_timing`), which waits a couple of
//!    milliseconds a block at most until it has seen offline rendering
//!    sustained, and drops back to live on the first block that arrives
//!    at a live pace.

pub mod reader;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use parking_lot::Mutex;
use resonance_common::WavTail;

use crate::voice::{Voice, MAX_VOICES, TAIL_SLOTS};

/// The preload choices, in frames: how much of each take stays in
/// memory. 32 k is ≈ 0.68 s at 48 kHz, 64 k ≈ 1.4 s.
pub const PRELOAD_CHOICES: [u32; 3] = [32_768, 65_536, 131_072];

/// The default preload, in frames: 32 k (≈ 0.68 s at 48 kHz).
///
/// The spec's first sketch said 64 k; measured, that misses E14's memory
/// goal. The default Drummica setup (2,835 takes, 44.1 kHz 24-bit, about
/// half of them stereo, resampled to 48 kHz) holds 2,921 MiB whole,
/// 1,074 MiB at 64 k — nearly every take is longer than that, and a
/// head is `f32` at the host rate, so it cannot shrink — and 529 MiB at
/// 32 k (2026-10-01, `tests/streaming.rs`,
/// `drummica_default_setup_memory_and_bit_identity`). 0.68 s of head is
/// still hundreds of host blocks of lead for the reader's first read.
pub const DEFAULT_PRELOAD: u32 = 32_768;

/// The plugin-state key the preload is saved under (frames).
pub const PRELOAD_STATE_KEY: &str = "stream_preload";

/// [`crate::KitBridge::host_render_mode`]: the host has not said how it
/// renders (no CLAP `render` extension, or not called yet). The sampler
/// tells from the block timing.
pub const HOST_RENDER_UNKNOWN: u8 = 0;
/// The host renders in real time (`CLAP_RENDER_REALTIME`): a missing
/// tail frame is never waited for, whatever the timing looks like.
pub const HOST_RENDER_REALTIME: u8 = 1;
/// The host renders offline (a bounce, `CLAP_RENDER_OFFLINE`): a missing
/// tail frame is waited for, up to seconds a block.
pub const HOST_RENDER_OFFLINE: u8 = 2;

/// The preload a state value names: one of [`PRELOAD_CHOICES`], or 0
/// (streaming off). `None` for a missing or unknown value.
pub fn preload_from_state(value: Option<&serde_json::Value>) -> Option<u32> {
    let frames = u32::try_from(value?.as_u64()?).ok()?;
    (frames == 0 || PRELOAD_CHOICES.contains(&frames)).then_some(frames)
}

/// Set the preload (one of [`PRELOAD_CHOICES`], or 0 for none) and, if
/// it changed, reload the kit so its takes are split at the new size.
/// Returns whether a reload started. Never call it from the audio thread.
pub fn set_preload(bridge: &crate::KitBridge, frames: u32) -> bool {
    if frames != 0 && !PRELOAD_CHOICES.contains(&frames) {
        return false;
    }
    if bridge.stream_preload.swap(frames, Ordering::Relaxed) == frames {
        return false;
    }
    crate::reload::reload_kit(bridge)
}

/// A take is only split when its tail would be at least this long; a
/// slightly-longer-than-the-head take stays whole rather than costing a
/// ring and a file read for a few frames.
pub const MIN_STREAMED_TAIL: usize = 8_192;

/// Frames one ring holds (stereo; a mono take uses half of each frame's
/// room). ≈ 0.34 s at 48 kHz, ≈ 128 host blocks of 128 frames — the
/// reader's lead once a voice is on its tail. Before that, the voice's
/// head (≥ 32 k frames) is the lead: a ring is requested at note-on, gets
/// one chunk ([`READ_CHUNK`]) at once, and is filled the rest of the way
/// once its voice is within a ring's length of its tail — so a voice
/// choked or stolen on its head costs one read, not a ring's worth.
pub const RING_FRAMES: usize = 16_384;

/// Bytes of sample storage a ring holds once it has been used.
pub const RING_BYTES: usize = RING_FRAMES * 2 * std::mem::size_of::<f32>();

/// Rings beyond one per main voice and tail slot: room for rings whose
/// voices have ended but which the reader has not handed back yet, so
/// even a reader held off the CPU for a while (they are not realtime
/// threads) does not run a fast pattern out of rings. A claim takes the
/// lowest free ring, so the spare ones are rarely touched — and a ring
/// never served holds no sample storage.
pub const SPARE_RINGS: usize = 96;

/// The rings a sampler with `voices` main voices and `tails` tail slots
/// has.
pub const fn rings_for(voices: usize, tails: usize) -> usize {
    voices + tails + SPARE_RINGS
}

/// Rings per sampler (see [`rings_for`]).
pub const NUM_RINGS: usize = rings_for(MAX_VOICES, TAIL_SLOTS);

/// [`Voice::ring`] of a voice that streams nothing.
pub const NO_RING: u8 = u8::MAX;

/// 64-bit words in a set of ring indices.
const RING_WORDS: usize = NUM_RINGS.div_ceil(64);

const _: () = assert!(NUM_RINGS < NO_RING as usize);
// E15 raises the cap to 128 voices + 16 tail slots: still indexable by a
// `u8` below `NO_RING`, in four words.
const _: () = assert!(rings_for(128, 16) < NO_RING as usize);
const _: () = assert!(rings_for(128, 16).div_ceil(64) == 4);

/// The most frames one reader pass fetches for one ring, so one long
/// read never holds up the others. Also all a ring gets while its voice
/// is still more than a ring's length from its tail.
pub(crate) const READ_CHUNK: usize = 4_096;

/// `wpos`: the generation is the high 32 bits; below it, this bit says
/// the stream failed (the file is gone or changed — the voice will not
/// get its tail), and the rest is the frames written.
pub(crate) const WPOS_FAILED: u64 = 1 << 31;
pub(crate) const WPOS_FRAMES: u64 = WPOS_FAILED - 1;

/// Where a streamed take's tail lives: the file it was decoded from, and
/// how to read any of its frames at the decode rate.
pub struct TailSource {
    pub path: PathBuf,
    /// The file's length and modification time when it was decoded; a
    /// file that differs in either is a different file, and is not read.
    pub file_len: u64,
    pub modified: Option<SystemTime>,
    pub tail: WavTail,
}

impl TailSource {
    /// Whether `meta` describes the file this tail was split from.
    pub fn same_file(&self, meta: &std::fs::Metadata) -> bool {
        meta.len() == self.file_len && meta.modified().ok() == self.modified
    }
}

/// One voice's stream: see the module docs for the protocol.
pub struct Ring {
    /// The generation the audio thread wants served; 0: none (let go).
    pub(crate) active_gen: AtomicU32,
    /// A request the reader has not taken yet, as `Arc::into_raw`; null
    /// once taken. Written by the audio thread only while null.
    pub(crate) req: AtomicPtr<TailSource>,
    /// The request's generation, and the take frame its ring frame 0 is.
    pub(crate) req_gen: AtomicU32,
    pub(crate) req_start: AtomicU64,
    /// Published by the reader: (generation << 32) | failed | frames.
    pub(crate) wpos: AtomicU64,
    /// Ring frames the audio thread is done with.
    pub(crate) read: AtomicU64,
    /// Published by the audio thread: **output** frames its voice has
    /// left to play of its head before it needs ring frame 0 (0 once on
    /// its tail). Plus what is buffered (at `rate_q16`), the reader's
    /// deadline for this ring.
    pub(crate) head_left: AtomicU64,
    /// The voice's playback rate in 16.16 fixed point (E8): take frames
    /// it reads per output frame, `1 << 16` at pitch. The reader turns
    /// the frames a ring buffers into the time they last by it, so a
    /// voice pitched up an octave is served as the one that runs out
    /// twice as soon as it is. Set at the claim.
    pub(crate) rate_q16: AtomicU32,
    /// The generation a reader is serving (0: none), and its stream's
    /// length in ring frames: what the readers' lock-free scans go by.
    pub(crate) reader_gen: AtomicU32,
    pub(crate) reader_end: AtomicU64,
    /// `RING_FRAMES * 2` sample slots, as `f32` bits — allocated by the
    /// reader that first serves the ring, before it publishes a frame.
    /// The audio thread only ever `get`s it (one atomic load, never a
    /// wait).
    data: OnceLock<Box<[AtomicU32]>>,
    /// The reader's side of the ring. Only reader threads lock it (with
    /// `try_lock`, as the claim that one thread serves it at a time).
    pub(crate) reader: Mutex<reader::ReaderSide>,
}

impl Ring {
    fn new() -> Self {
        Self {
            active_gen: AtomicU32::new(0),
            req: AtomicPtr::new(std::ptr::null_mut()),
            req_gen: AtomicU32::new(0),
            req_start: AtomicU64::new(0),
            wpos: AtomicU64::new(0),
            read: AtomicU64::new(0),
            head_left: AtomicU64::new(0),
            rate_q16: AtomicU32::new(Pace::UNITY_Q16),
            reader_gen: AtomicU32::new(0),
            reader_end: AtomicU64::new(0),
            data: OnceLock::new(),
            reader: Mutex::new(reader::ReaderSide::default()),
        }
    }

    /// Audio thread: frames published for the generation the ring is
    /// claimed under, and whether the stream failed.
    #[inline]
    pub(crate) fn published(&self) -> (u64, bool) {
        let gen = self.active_gen.load(Ordering::Relaxed);
        let wpos = self.wpos.load(Ordering::Acquire);
        if (wpos >> 32) as u32 != gen {
            return (0, false);
        }
        (wpos & WPOS_FRAMES, wpos & WPOS_FAILED != 0)
    }

    /// Sample `ch` (0 or 1) of ring frame `frame`, which must be below
    /// what [`published`](Self::published) said (and so is stored).
    #[inline]
    pub(crate) fn sample(&self, frame: u64, stride: usize, ch: usize) -> f32 {
        let Some(data) = self.data.get() else {
            return 0.0;
        };
        let slot = (frame as usize % RING_FRAMES) * stride + ch;
        f32::from_bits(data[slot].load(Ordering::Relaxed))
    }

    /// Reader: the ring's sample storage, allocated on first use, and
    /// whether this call allocated it. Callers hold the `reader` lock.
    pub(crate) fn data_or_alloc(&self) -> (&[AtomicU32], bool) {
        let mut fresh = false;
        let data = self.data.get_or_init(|| {
            fresh = true;
            (0..RING_FRAMES * 2).map(|_| AtomicU32::new(0)).collect()
        });
        (data, fresh)
    }

    /// Whether the ring holds sample storage (it has been served).
    pub fn has_storage(&self) -> bool {
        self.data.get().is_some()
    }

    /// Reader: mark generation `gen`'s stream failed, keeping the frames
    /// it published. A no-op once the ring has moved on.
    pub(crate) fn fail(&self, gen: u32) {
        let mut cur = self.wpos.load(Ordering::Acquire);
        while (cur >> 32) as u32 == gen && cur & WPOS_FAILED == 0 {
            match self.wpos.compare_exchange_weak(
                cur,
                cur | WPOS_FAILED,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(now) => cur = now,
            }
        }
    }

    /// Whether anyone still has business with the ring: claimed, holding
    /// a request the reader has not taken, or still being served.
    fn in_use(&self) -> bool {
        self.active_gen.load(Ordering::Acquire) != 0
            || !self.req.load(Ordering::Acquire).is_null()
            || self.reader_gen.load(Ordering::Acquire) != 0
    }

    /// Reader: take the pending request, if any, as (source, generation,
    /// start). The generation is read before and after the pointer: the
    /// audio thread writes it only while the slot is empty, so once the
    /// pointer is seen and the generation has not moved, the two belong
    /// together. Callers hold the ring's `reader` lock, so the exchange
    /// is never contended.
    pub(crate) fn take_request(&self) -> Option<(Arc<TailSource>, u32, u64)> {
        loop {
            let gen = self.req_gen.load(Ordering::Acquire);
            let ptr = self.req.load(Ordering::Acquire);
            if ptr.is_null() {
                return None;
            }
            if self.req_gen.load(Ordering::Acquire) != gen {
                continue;
            }
            let start = self.req_start.load(Ordering::Relaxed);
            if self
                .req
                .compare_exchange(
                    ptr,
                    std::ptr::null_mut(),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                // SAFETY: a non-null `req` is an `Arc::into_raw` of the
                // audio thread's clone, and the exchange took sole
                // ownership of it.
                return Some((unsafe { Arc::from_raw(ptr) }, gen, start));
            }
        }
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        let ptr = self.req.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !ptr.is_null() {
            // SAFETY: as in `take_request`.
            drop(unsafe { Arc::from_raw(ptr) });
        }
    }
}

/// One sampler's rings, shared with the reader pool, plus the test hooks
/// that slow, stop or break the reader for it.
pub struct StreamSet {
    pub(crate) rings: Box<[Ring]>,
    /// Rings the readers must look at: set by the audio thread when it
    /// claims one (an atomic `or`, no syscall), cleared by a reader once
    /// the ring is wholly idle again — so a scan skips every ring nobody
    /// uses.
    pub(crate) open: [AtomicU64; RING_WORDS],
    /// Rings holding sample storage.
    allocated: AtomicU32,
    /// Test hook: while set, the reader leaves this set alone entirely —
    /// a stalled disk, or a reader that never comes back.
    pub(crate) paused: AtomicBool,
    /// Test hook: extra latency before each read, in microseconds — a
    /// cold page cache.
    pub(crate) read_latency_us: AtomicU32,
    /// Test hook: this many of the next reads panic.
    pub(crate) panic_reads: AtomicU32,
    /// Times the audio thread waited for a tail frame (offline only).
    pub(crate) waits: AtomicU64,
}

impl StreamSet {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            rings: (0..NUM_RINGS).map(|_| Ring::new()).collect(),
            open: std::array::from_fn(|_| AtomicU64::new(0)),
            allocated: AtomicU32::new(0),
            paused: AtomicBool::new(false),
            read_latency_us: AtomicU32::new(0),
            panic_reads: AtomicU32::new(0),
            waits: AtomicU64::new(0),
        })
    }

    /// Test hook: stall (true) or resume the reader for this set.
    #[doc(hidden)]
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
    }

    /// Test hook: delay every read by `us` microseconds.
    #[doc(hidden)]
    pub fn set_read_latency_us(&self, us: u32) {
        self.read_latency_us.store(us, Ordering::Relaxed);
    }

    /// Test hook: make the next `n` reads panic (a reader bug).
    #[doc(hidden)]
    pub fn panic_next_reads(&self, n: u32) {
        self.panic_reads.store(n, Ordering::Release);
    }

    /// Times the sampler has waited for a tail frame (offline only).
    pub fn offline_waits(&self) -> u64 {
        self.waits.load(Ordering::Relaxed)
    }

    /// Rings in use: claimed, holding a request the reader has not
    /// taken, or still being served by a reader.
    pub fn rings_in_use(&self) -> usize {
        self.rings.iter().filter(|r| r.in_use()).count()
    }

    /// Rings marked open for the readers.
    pub fn rings_open(&self) -> usize {
        self.open
            .iter()
            .map(|w| w.load(Ordering::Acquire).count_ones() as usize)
            .sum()
    }

    /// Frames published for ring `ring`'s current stream (0 for a ring
    /// that is not claimed).
    pub fn published_frames(&self, ring: u8) -> u64 {
        self.rings.get(ring as usize).map_or(0, |r| r.published().0)
    }

    /// Frames published for the streams claimed now, across every ring.
    pub fn frames_buffered(&self) -> u64 {
        self.rings
            .iter()
            .filter(|r| r.active_gen.load(Ordering::Acquire) != 0)
            .map(|r| r.published().0)
            .sum()
    }

    /// Rings whose current stream failed (its file gone or changed).
    pub fn rings_failed(&self) -> usize {
        self.rings
            .iter()
            .filter(|r| {
                let gen = r.active_gen.load(Ordering::Acquire);
                let wpos = r.wpos.load(Ordering::Acquire);
                gen != 0 && (wpos >> 32) as u32 == gen && wpos & WPOS_FAILED != 0
            })
            .count()
    }

    /// Rings holding sample storage (ever served).
    pub fn rings_allocated(&self) -> usize {
        self.allocated.load(Ordering::Relaxed) as usize
    }

    /// Bytes of ring sample storage this set holds: only rings that have
    /// been served hold any.
    pub fn ring_bytes(&self) -> u64 {
        self.rings_allocated() as u64 * RING_BYTES as u64
    }

    /// Audio thread: mark ring `i` open for the readers.
    #[inline]
    fn mark_open(&self, i: usize) {
        self.open[i / 64].fetch_or(1 << (i % 64), Ordering::AcqRel);
    }

    /// Reader: unmark ring `i` if it is wholly idle. The ring is checked
    /// again after the bit is cleared, so a claim racing the clear (which
    /// sets the bit after posting its request) is never lost.
    pub(crate) fn close_if_idle(&self, i: usize) {
        let ring = &self.rings[i];
        if ring.in_use() {
            return;
        }
        let bit = 1u64 << (i % 64);
        self.open[i / 64].fetch_and(!bit, Ordering::AcqRel);
        if ring.in_use() {
            self.open[i / 64].fetch_or(bit, Ordering::AcqRel);
        }
    }

    /// Reader: count a ring that just got its storage.
    pub(crate) fn note_allocated(&self) {
        self.allocated.fetch_add(1, Ordering::Relaxed);
    }
}

/// A set of ring indices.
#[derive(Clone, Copy, Default)]
struct RingBits([u64; RING_WORDS]);

impl RingBits {
    #[inline]
    fn get(&self, i: usize) -> bool {
        self.0[i / 64] & (1 << (i % 64)) != 0
    }
    #[inline]
    fn set(&mut self, i: usize) {
        self.0[i / 64] |= 1 << (i % 64);
    }
    #[inline]
    fn clear(&mut self, i: usize) {
        self.0[i / 64] &= !(1 << (i % 64));
    }
}

/// How fast a claiming voice will need its ring (E8): see
/// [`AudioStreams::claim`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pace {
    /// Output frames until the voice reads ring frame 0.
    pub head_left: usize,
    /// Take frames read per output frame, 16.16 fixed point.
    pub rate_q16: u32,
}

impl Pace {
    /// `rate_q16` at pitch.
    pub const UNITY_Q16: u32 = 1 << 16;

    /// A voice at pitch, `head_left` frames from its tail.
    pub fn unity(head_left: usize) -> Self {
        Self {
            head_left,
            rate_q16: Self::UNITY_Q16,
        }
    }

    /// A voice reading `rate` take frames per output frame, `head` take
    /// frames from its tail.
    pub fn at_rate(head: usize, rate: f32) -> Self {
        if rate == 1.0 {
            return Self::unity(head);
        }
        let rate = rate.max(1.0 / 64.0);
        Self {
            head_left: (head as f32 / rate) as usize,
            rate_q16: ((rate * Self::UNITY_Q16 as f32) as u32).max(1),
        }
    }

    /// The output frames `frames` take frames last at `rate_q16`.
    #[inline]
    pub(crate) fn frames_to_time(frames: u64, rate_q16: u32) -> u64 {
        if rate_q16 == Self::UNITY_Q16 {
            frames
        } else {
            frames.saturating_mul(Self::UNITY_Q16 as u64) / rate_q16.max(1) as u64
        }
    }
}

/// How the sampler is being rendered, which decides what a missing tail
/// frame does (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// Go by what the host declared ([`crate::KitBridge::host_render_mode`]),
    /// or by the block timing while it has declared nothing (the
    /// default).
    Auto,
    /// Live: never wait; a missing frame is an underrun.
    Realtime,
    /// Offline: wait (bounded, generously) for a missing frame.
    Offline,
}

/// Spin briefly, then sleep in short steps, until `ready()` holds or
/// `budget` runs out; the time spent is charged to `budget` (zero once it
/// ran out). Returns whether `ready()` held. Offline only: sleeping is a
/// syscall.
pub(crate) fn wait_until(budget: &mut Duration, mut ready: impl FnMut() -> bool) -> bool {
    let began = Instant::now();
    let mut spins = 0u32;
    loop {
        if ready() {
            *budget = budget.saturating_sub(began.elapsed());
            return true;
        }
        if began.elapsed() >= *budget {
            *budget = Duration::ZERO;
            return false;
        }
        if spins < 64 {
            spins += 1;
            std::hint::spin_loop();
        } else {
            std::thread::sleep(Duration::from_micros(50));
        }
    }
}

/// The audio thread's handle on its [`StreamSet`]: which rings it has
/// claimed, and the counters it publishes. Holds the set's registration
/// with its reader pool: dropping it (off the audio thread — it may join
/// the pool's threads) unregisters the set.
pub struct AudioStreams {
    pub(crate) set: Arc<StreamSet>,
    claimed: RingBits,
    /// The last generation given to each ring.
    gens: [u32; NUM_RINGS],
    /// Voice-spans that had to play silence for frames the reader had not
    /// delivered, plus streamed voices that found no ring.
    pub(crate) underruns: Arc<AtomicU64>,
    /// Of those, the claims that found no free ring.
    pub(crate) ring_misses: u64,
    _registration: Option<reader::Registration>,
}

impl AudioStreams {
    /// Streams on `set`, which no pool serves (unless the caller
    /// registered it).
    pub fn new(set: Arc<StreamSet>) -> Self {
        Self::with_registration(set, None)
    }

    /// Streams on `set`, served for as long as `registration` lives.
    pub fn with_registration(set: Arc<StreamSet>, registration: Option<reader::Registration>) -> Self {
        Self {
            set,
            claimed: RingBits::default(),
            gens: [0; NUM_RINGS],
            underruns: Arc::new(AtomicU64::new(0)),
            ring_misses: 0,
            _registration: registration,
        }
    }

    /// Audio thread: claim a free ring and request `source`'s frames from
    /// take frame `start` on. [`NO_RING`] when every ring is taken (the
    /// voice then plays its head only, and that counts as an underrun).
    ///
    /// `pace` is how fast the voice will get there: its first deadline
    /// (output frames until it needs ring frame 0) and its playback rate
    /// (E8: a pitched voice reads more, or fewer, take frames per output
    /// frame). [`Pace::unity`] for a voice that plays from frame 0 at
    /// pitch.
    ///
    /// `offline_wait`: offline only — wait, for at most this long (which
    /// is charged for it), for the reader to take a pending request and
    /// so free a ring, rather than give up at once.
    pub fn claim(
        &mut self,
        source: &Arc<TailSource>,
        start: usize,
        pace: Pace,
        offline_wait: Option<&mut Duration>,
    ) -> u8 {
        if let Some(ring) = self.try_claim(source, start, pace) {
            return ring;
        }
        if let Some(budget) = offline_wait.filter(|b| !b.is_zero()) {
            let mut got = None;
            wait_until(budget, || {
                got = self.try_claim(source, start, pace);
                got.is_some()
            });
            if let Some(ring) = got {
                return ring;
            }
        }
        self.underruns.fetch_add(1, Ordering::Relaxed);
        self.ring_misses += 1;
        NO_RING
    }

    fn try_claim(&mut self, source: &Arc<TailSource>, start: usize, pace: Pace) -> Option<u8> {
        for (i, ring) in self.set.rings.iter().enumerate() {
            // A ring no voice holds, whose last request the reader has
            // taken. (One it has not taken yet stays as it is: only the
            // reader may drop that `Arc`.)
            if self.claimed.get(i) || !ring.req.load(Ordering::Acquire).is_null() {
                continue;
            }
            let gen = match self.gens[i].wrapping_add(1) {
                0 => 1,
                g => g,
            };
            self.gens[i] = gen;
            ring.read.store(0, Ordering::Relaxed);
            ring.head_left.store(pace.head_left as u64, Ordering::Relaxed);
            ring.rate_q16.store(pace.rate_q16, Ordering::Relaxed);
            ring.wpos.store((gen as u64) << 32, Ordering::Relaxed);
            ring.active_gen.store(gen, Ordering::Release);
            ring.req_gen.store(gen, Ordering::Relaxed);
            ring.req_start.store(start as u64, Ordering::Relaxed);
            let raw = Arc::into_raw(Arc::clone(source)) as *mut TailSource;
            // Publishes everything above to the reader that takes it.
            ring.req.store(raw, Ordering::Release);
            self.set.mark_open(i);
            self.claimed.set(i);
            return Some(i as u8);
        }
        None
    }

    /// Audio thread: let go of every claimed ring no active voice reads
    /// any more. It can be claimed again at once; the reader drops the
    /// stream when it sees the generation gone.
    pub fn sweep<'a>(&mut self, voices: impl Iterator<Item = &'a Voice>) {
        let mut referenced = RingBits::default();
        for voice in voices {
            if voice.active && voice.ring != NO_RING {
                referenced.set(voice.ring as usize);
            }
        }
        for w in 0..RING_WORDS {
            let mut gone = self.claimed.0[w] & !referenced.0[w];
            while gone != 0 {
                let bit = gone.trailing_zeros() as usize;
                gone &= gone - 1;
                let i = w * 64 + bit;
                if let Some(ring) = self.set.rings.get(i) {
                    ring.active_gen.store(0, Ordering::Release);
                }
                self.claimed.clear(i);
            }
        }
    }

    /// The ring at `index`.
    #[inline]
    pub(crate) fn ring(&self, index: u8) -> Option<&Ring> {
        self.set.rings.get(index as usize)
    }

    /// The set (for the reader pool and test hooks).
    pub fn set(&self) -> &Arc<StreamSet> {
        &self.set
    }

    /// Claims that found no free ring (counted in the underruns too).
    pub fn ring_misses(&self) -> u64 {
        self.ring_misses
    }

    /// Rings this sampler has claimed and not yet let go.
    pub fn claimed_count(&self) -> usize {
        self.claimed.0.iter().map(|w| w.count_ones() as usize).sum()
    }
}
