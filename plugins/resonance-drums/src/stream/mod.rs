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
//! - **Rings** are preallocated with the sampler ([`StreamSet`],
//!   [`NUM_RINGS`] of them, [`RING_FRAMES`] stereo frames each). A voice
//!   holds a ring by index ([`crate::voice::Voice::ring`]); moving a
//!   stolen voice to a tail slot moves the index with it.
//! - **The audio thread** claims a ring at note-on and hands it the
//!   take's [`TailSource`] (an `Arc` clone — an atomic increment, never an
//!   allocation; the kit still holds the take, so nothing the audio
//!   thread drops is ever a last reference). It reads ring frames the
//!   reader has published, and lets a ring go once no voice reads it
//!   ([`AudioStreams::sweep`]) — which makes it claimable again at once,
//!   without waiting for the reader to notice. It never waits for the
//!   reader, never locks, never allocates, and never makes a syscall that
//!   can block.
//! - **The reader** threads poll the rings (no wake-ups from the audio
//!   thread: a futex wake is a syscall), take requests, open the file,
//!   fill each ring as far as its voice has made room, and drop streams
//!   the audio thread has let go.
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
//!   with the source pointer last (release).
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
//! ([`RenderMode`]): told outright, or — since the host has no CLAP
//! `render` extension yet — by measuring that audio advances much faster
//! than the wall clock. Offline, a missing frame is waited for (bounded),
//! which CLAP permits for offline processing; live, never.

pub mod reader;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

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
/// head (≥ 32 k frames) is the lead: a ring is requested at note-on and
/// filled while the head plays.
pub const RING_FRAMES: usize = 16_384;

/// Rings per sampler: one for every main voice and tail slot, plus room
/// for rings whose voices have ended but which the reader has not handed
/// back yet — 96 more, so even a reader held off the CPU for a while
/// (they are not realtime threads) does not run a fast pattern out of
/// rings. A claim takes the lowest free ring, so the spare ones are
/// rarely touched, and an untouched ring costs no resident memory
/// ([`zeroed_atomics`]).
pub const NUM_RINGS: usize = MAX_VOICES + TAIL_SLOTS + 96;

/// [`Voice::ring`] of a voice that streams nothing.
pub const NO_RING: u8 = u8::MAX;

const _: () = assert!(NUM_RINGS < NO_RING as usize);
const _: () = assert!(NUM_RINGS <= 64 * RING_WORDS);

/// The most frames one reader pass fetches for one ring, so one long
/// read never holds up the others.
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
    /// The file's length when it was decoded; a file of another length
    /// is a different file, and is not read.
    pub file_len: u64,
    pub tail: WavTail,
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
    /// The generation a reader is serving (0: none), and its stream's
    /// length in ring frames: what the readers' lock-free scans go by.
    pub(crate) reader_gen: AtomicU32,
    pub(crate) reader_end: AtomicU64,
    /// `RING_FRAMES * 2` sample slots, as `f32` bits.
    pub(crate) data: Box<[AtomicU32]>,
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
            reader_gen: AtomicU32::new(0),
            reader_end: AtomicU64::new(0),
            data: zeroed_atomics(RING_FRAMES * 2),
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
    /// what [`published`](Self::published) said.
    #[inline]
    pub(crate) fn sample(&self, frame: u64, stride: usize, ch: usize) -> f32 {
        let slot = (frame as usize % RING_FRAMES) * stride + ch;
        f32::from_bits(self.data[slot].load(Ordering::Relaxed))
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

/// `n` atomics, all zero, from a zeroed allocation: the pages are only
/// committed when a reader first writes them, so a sampler that never
/// streams costs no resident memory for its rings.
fn zeroed_atomics(n: usize) -> Box<[AtomicU32]> {
    let zeroed: Box<[u32]> = vec![0u32; n].into_boxed_slice();
    let ptr = Box::into_raw(zeroed) as *mut [AtomicU32];
    // SAFETY: `AtomicU32` has the same size, alignment and bit validity
    // as `u32` (documented), so the boxed slice is reinterpreted as is.
    unsafe { Box::from_raw(ptr) }
}

/// One sampler's rings, shared with the reader pool, plus the test hooks
/// that slow or stop the reader for it.
pub struct StreamSet {
    pub(crate) rings: Box<[Ring]>,
    /// Test hook: while set, the reader leaves this set alone entirely —
    /// a stalled disk, or a reader that never comes back.
    pub(crate) paused: AtomicBool,
    /// Test hook: extra latency before each read, in microseconds — a
    /// cold page cache.
    pub(crate) read_latency_us: AtomicU32,
}

impl StreamSet {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            rings: (0..NUM_RINGS).map(|_| Ring::new()).collect(),
            paused: AtomicBool::new(false),
            read_latency_us: AtomicU32::new(0),
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

    /// Rings in use: claimed, holding a request the reader has not
    /// taken, or still being served by a reader.
    pub fn rings_in_use(&self) -> usize {
        self.rings
            .iter()
            .filter(|r| {
                r.active_gen.load(Ordering::Acquire) != 0
                    || !r.req.load(Ordering::Acquire).is_null()
                    || r.reader_gen.load(Ordering::Acquire) != 0
            })
            .count()
    }
}

const RING_WORDS: usize = 3;

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

/// How the sampler is being rendered, which decides what a missing tail
/// frame does (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// Tell from the block timing (the default).
    Auto,
    /// Live: never wait; a missing frame is an underrun.
    Realtime,
    /// Offline: wait (bounded) for a missing frame.
    Offline,
}

/// The audio thread's handle on its [`StreamSet`]: which rings it has
/// claimed, and the counters it publishes.
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
}

impl AudioStreams {
    pub fn new(set: Arc<StreamSet>) -> Self {
        Self {
            set,
            claimed: RingBits::default(),
            gens: [0; NUM_RINGS],
            underruns: Arc::new(AtomicU64::new(0)),
            ring_misses: 0,
        }
    }

    /// Audio thread: claim a free ring and request `source`'s frames from
    /// take frame `start` on. [`NO_RING`] when every ring is taken (the
    /// voice then plays its head only, and that counts as an underrun).
    ///
    /// `offline_wait`: offline only — wait, for at most this long (which
    /// is charged for it), for the reader to take a pending request and
    /// so free a ring, rather than give up at once.
    pub fn claim(
        &mut self,
        source: &Arc<TailSource>,
        start: usize,
        offline_wait: Option<&mut std::time::Duration>,
    ) -> u8 {
        if let Some(ring) = self.try_claim(source, start) {
            return ring;
        }
        if let Some(budget) = offline_wait {
            let began = std::time::Instant::now();
            let mut spins = 0u32;
            while began.elapsed() < *budget {
                if spins < 64 {
                    spins += 1;
                    std::hint::spin_loop();
                } else {
                    std::thread::sleep(std::time::Duration::from_micros(50));
                }
                if let Some(ring) = self.try_claim(source, start) {
                    *budget = budget.saturating_sub(began.elapsed());
                    return ring;
                }
            }
            *budget = std::time::Duration::ZERO;
        }
        self.underruns.fetch_add(1, Ordering::Relaxed);
        self.ring_misses += 1;
        NO_RING
    }

    fn try_claim(&mut self, source: &Arc<TailSource>, start: usize) -> Option<u8> {
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
            ring.wpos.store((gen as u64) << 32, Ordering::Relaxed);
            ring.active_gen.store(gen, Ordering::Release);
            ring.req_gen.store(gen, Ordering::Relaxed);
            ring.req_start.store(start as u64, Ordering::Relaxed);
            let raw = Arc::into_raw(Arc::clone(source)) as *mut TailSource;
            // Publishes everything above to the reader that takes it.
            ring.req.store(raw, Ordering::Release);
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
