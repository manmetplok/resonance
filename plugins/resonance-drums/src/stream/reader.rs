//! The disk-reader pool: a few threads that fill every registered
//! sampler's rings from disk (see the [module docs](super)).
//!
//! One process-wide pool ([`ReaderPool::global`]) serves every drum
//! instance; tests may build their own, shut it down to see what the
//! audio thread does with a dead reader, or build a **stepped** one
//! ([`ReaderPool::stepped`]) with no threads at all, whose passes the
//! test runs itself ([`ReaderPool::pump`]) — a reader as deterministic as
//! the render.
//!
//! # Lifetime
//!
//! A sampler registers its [`StreamSet`] and holds the [`Registration`].
//! The pool's threads are spawned by the first registration and exit —
//! joined by whoever drops the last one, which is never the audio thread
//! — once no set is registered; the next registration spawns them again.
//!
//! # Polling
//!
//! The audio thread never wakes a reader (that would be a syscall on the
//! audio thread), so the readers poll — but only the rings marked open
//! ([`StreamSet::open`](super::StreamSet)), and only quickly while any is
//! open: a pass that found work is followed by another at once, an idle
//! one sleeps [`ACTIVE_POLL`] while streams are open and parks for up to
//! [`IDLE_POLL`] when none is (woken early by a registration). A new
//! stream waits at most that long to be noticed, which its voice's head
//! (≥ 32 k frames, ≈ 0.68 s) covers many times over.
//!
//! # Scheduling
//!
//! A pass reads one chunk at a time, always for the ring whose voice
//! runs out first: its **deadline** is the frames the voice has left of
//! its head (published by the audio thread) plus what its ring already
//! buffers. A ring whose voice is more than a ring's length from its tail
//! gets one chunk ([`READ_CHUNK`]) and no more until it comes within that
//! length — a fresh claim has its whole head as lead, and a voice choked
//! or stolen on its head should not have cost a ring's worth of reads.
//!
//! The deadline is **time**, in output frames, not take frames: a voice
//! tuned up (E8) reads its take faster than the clock, so its head and
//! its buffered frames last it less — the audio thread publishes the
//! head it has left divided by its rate, and the buffered frames are
//! divided by the rate the ring was claimed at. At pitch both are the
//! frame counts they always were.
//!
//! # Faults
//!
//! Every reader step on a ring runs under `catch_unwind`: a panic (a
//! reader bug, or a decoder fed something it did not expect) fails that
//! ring's stream — its voice fades out where its delivered frames end —
//! and the thread carries on with the others.

use std::fs::File;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;
use resonance_common::TailScratch;

use super::{
    Pace, Ring, StreamSet, TailSource, READ_CHUNK, RING_FRAMES, WPOS_FAILED, WPOS_FRAMES,
};

/// Sleep between passes that found nothing to do while streams are open.
pub const ACTIVE_POLL: Duration = Duration::from_micros(500);
/// Park between passes with no stream open in any registered set.
pub const IDLE_POLL: Duration = Duration::from_millis(10);

/// Reader threads in the process-wide pool: a quarter of the cores,
/// from 2 to 4.
///
/// Resampled tails are what costs. Measured in release (2026-10-01,
/// `tests/streaming.rs`, `reader_throughput_at_saturation`): the 64-voice
/// saturation pattern with 44.1 kHz files read for a 48 kHz host takes
/// one reader thread 1.0–1.3 s per 3 s of audio — only 2.3–3x real time
/// — where 48 kHz files (no resampling) take 65–70 ms (≈ 45x); measured
/// on a 16-core machine under load. Two threads left 5–6x headroom at 64
/// voices, and E15's 128 voices would halve that; four give 9–12x (5–6x
/// at 128). Reads of one ring never run on two threads at
/// once, so more threads only help with more voices — which is the case
/// that needs them.
pub fn global_readers() -> usize {
    std::thread::available_parallelism().map_or(2, |n| (n.get() / 4).clamp(2, 4))
}

/// A reader's view of one ring: the stream it is filling.
#[derive(Default)]
pub struct ReaderSide {
    source: Option<Arc<TailSource>>,
    file: Option<File>,
    /// The generation served; 0: none.
    gen: u32,
    /// Take frame of ring frame 0.
    start: u64,
    /// Ring frames published so far (what `wpos` holds for `gen`).
    next: u64,
}

impl ReaderSide {
    fn reset(&mut self) {
        self.source = None;
        self.file = None;
        self.gen = 0;
        self.start = 0;
        self.next = 0;
    }
}

struct PoolShared {
    sets: Mutex<Vec<Weak<StreamSet>>>,
    /// The threads are to exit (no set left, or the pool was shut down).
    stop: AtomicBool,
}

struct Threads {
    handles: Vec<JoinHandle<()>>,
    /// Live [`Registration`]s.
    registered: usize,
    /// [`ReaderPool::shutdown`] was called: never spawn again.
    killed: bool,
}

/// See the module docs.
pub struct ReaderPool {
    shared: Arc<PoolShared>,
    threads: Mutex<Threads>,
    /// Threads to run while any set is registered; 0 for a stepped pool.
    count: usize,
    /// A stepped pool's pass state.
    stepped: Mutex<Pass>,
}

/// A set's registration with a pool: it is served while this lives.
/// Drop it off the audio thread: dropping the last one joins the pool's
/// threads.
pub struct Registration {
    pool: Arc<ReaderPool>,
    set: Weak<StreamSet>,
}

impl ReaderPool {
    /// A pool of `threads` reader threads (at least one), spawned once a
    /// set registers.
    pub fn new(threads: usize) -> Arc<Self> {
        Self::with_threads(threads.max(1))
    }

    /// A pool with no threads: nothing is read until the caller runs
    /// [`pump`](Self::pump). Test hook.
    #[doc(hidden)]
    pub fn stepped() -> Arc<Self> {
        Self::with_threads(0)
    }

    fn with_threads(count: usize) -> Arc<Self> {
        Arc::new(Self {
            shared: Arc::new(PoolShared {
                sets: Mutex::new(Vec::new()),
                stop: AtomicBool::new(false),
            }),
            threads: Mutex::new(Threads {
                handles: Vec::new(),
                registered: 0,
                killed: false,
            }),
            count,
            stepped: Mutex::new(Pass::default()),
        })
    }

    /// The pool every sampler uses unless told otherwise.
    pub fn global() -> &'static Arc<ReaderPool> {
        static POOL: OnceLock<Arc<ReaderPool>> = OnceLock::new();
        POOL.get_or_init(|| ReaderPool::new(global_readers()))
    }

    /// Serve `set` for as long as the returned registration lives,
    /// spawning the threads if none are running.
    pub fn register(self: &Arc<Self>, set: &Arc<StreamSet>) -> Registration {
        let mut threads = self.threads.lock();
        threads.registered += 1;
        {
            let mut sets = self.shared.sets.lock();
            sets.retain(|s| s.strong_count() > 0);
            sets.push(Arc::downgrade(set));
        }
        if !threads.killed && threads.handles.is_empty() {
            self.shared.stop.store(false, Ordering::Release);
            for n in 0..self.count {
                let shared = self.shared.clone();
                if let Ok(handle) = std::thread::Builder::new()
                    .name(format!("resonance-drums-stream-{n}"))
                    .spawn(move || run(&shared))
                {
                    threads.handles.push(handle);
                }
            }
        }
        for handle in &threads.handles {
            handle.thread().unpark();
        }
        Registration {
            pool: self.clone(),
            set: Arc::downgrade(set),
        }
    }

    /// Reader threads running now.
    pub fn running_threads(&self) -> usize {
        self.threads.lock().handles.len()
    }

    /// Run reader passes on the calling thread until there is nothing
    /// left to do; returns whether anything was. For a stepped pool
    /// ([`stepped`](Self::stepped)). Test hook.
    #[doc(hidden)]
    pub fn pump(&self) -> bool {
        let mut pass = self.stepped.lock();
        let mut any = false;
        while run_pass(&self.shared, &mut pass, usize::MAX).0 {
            any = true;
        }
        any
    }

    /// One pass on the calling thread that reads at most one chunk: the
    /// most urgent. For a stepped pool. Test hook.
    #[doc(hidden)]
    pub fn step(&self) -> bool {
        run_pass(&self.shared, &mut self.stepped.lock(), 1).0
    }

    /// Stop every reader thread and wait for them, for good. The rings
    /// they served are left as they are: a test hook for a reader that
    /// dies.
    pub fn shutdown(&self) {
        let mut threads = self.threads.lock();
        threads.killed = true;
        stop_and_join(&self.shared, &mut threads);
    }
}

fn stop_and_join(shared: &PoolShared, threads: &mut Threads) {
    shared.stop.store(true, Ordering::Release);
    for handle in &threads.handles {
        handle.thread().unpark();
    }
    for handle in threads.handles.drain(..) {
        let _ = handle.join();
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut threads = self.pool.threads.lock();
        self.pool
            .shared
            .sets
            .lock()
            .retain(|s| !s.ptr_eq(&self.set) && s.strong_count() > 0);
        threads.registered -= 1;
        if threads.registered == 0 {
            stop_and_join(&self.pool.shared, &mut threads);
        }
    }
}

/// One reader's working state, kept across passes.
#[derive(Default)]
struct Pass {
    sets: Vec<Arc<StreamSet>>,
    scratch: TailScratch,
    buf: Vec<f32>,
    /// (deadline, set, ring) of the rings that want a read.
    wanting: Vec<(u64, usize, usize)>,
}

fn run(shared: &PoolShared) {
    let mut pass = Pass::default();
    while !shared.stop.load(Ordering::Acquire) {
        // Each ring's steps are guarded on their own (see `guarded`); this
        // is the last line, for a fault outside them.
        let (worked, open) =
            catch_unwind(AssertUnwindSafe(|| run_pass(shared, &mut pass, usize::MAX)))
            .unwrap_or_else(|_| {
                pass = Pass::default();
                (false, true)
            });
        if !worked {
            if open {
                std::thread::sleep(ACTIVE_POLL);
            } else {
                std::thread::park_timeout(IDLE_POLL);
            }
        }
    }
}

/// One pass over every registered set: take requests and drop let-go
/// streams, then fill the rings that want it, earliest deadline first
/// (at most `max_reads` chunks), taking care of requests and
/// cancellations again after every read, so a slow disk never holds up a
/// ring being handed back. Returns (did anything, is any stream open).
fn run_pass(shared: &PoolShared, pass: &mut Pass, max_reads: usize) -> (bool, bool) {
    let Pass {
        sets,
        scratch,
        buf,
        wanting,
    } = pass;
    sets.clear();
    sets.extend(shared.sets.lock().iter().filter_map(Weak::upgrade));
    if buf.len() < READ_CHUNK * 2 {
        buf.resize(READ_CHUNK * 2, 0.0);
    }
    let (mut worked, open) = admin_all(sets);
    let mut reads = 0;
    while reads < max_reads && !shared.stop.load(Ordering::Acquire) {
        wanting.clear();
        for (si, set) in sets.iter().enumerate() {
            if set.paused.load(Ordering::Acquire) {
                continue;
            }
            for_each_open(set, |ri| {
                if let Some(deadline) = wants_fill(&set.rings[ri]) {
                    wanting.push((deadline, si, ri));
                }
            });
        }
        wanting.sort_unstable();
        let mut filled = false;
        for &(_, si, ri) in wanting.iter() {
            let set = &sets[si];
            let ring = &set.rings[ri];
            // Another reader thread is on it: the next one.
            let Some(mut side) = ring.reader.try_lock() else {
                continue;
            };
            // Nothing to read after all (the voice moved on, or the file
            // is done with): the next one.
            if guarded(ring, &mut side, |side| fill(set, ring, side, scratch, buf)) {
                filled = true;
                break;
            }
        }
        if !filled {
            break;
        }
        reads += 1;
        worked = true;
        admin_all(sets);
    }
    // A set dropped by its sampler goes here, off the audio thread.
    sets.clear();
    (worked, open)
}

/// Run one reader step on a ring, catching a panic: the ring's stream is
/// then failed and dropped (its voice fades out where its frames end),
/// and the reader carries on. Returns what the step did (a panic counts
/// as work).
fn guarded(ring: &Ring, side: &mut ReaderSide, step: impl FnOnce(&mut ReaderSide) -> bool) -> bool {
    match catch_unwind(AssertUnwindSafe(|| step(side))) {
        Ok(worked) => worked,
        Err(_) => {
            let gen = match side.gen {
                0 => ring.active_gen.load(Ordering::Acquire),
                gen => gen,
            };
            if gen != 0 {
                ring.fail(gen);
            }
            side.reset();
            ring.reader_gen.store(0, Ordering::Release);
            true
        }
    }
}

/// Call `f` with the index of every ring of `set` marked open.
fn for_each_open(set: &StreamSet, mut f: impl FnMut(usize)) {
    for (w, word) in set.open.iter().enumerate() {
        let mut bits = word.load(Ordering::Acquire);
        while bits != 0 {
            let bit = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            f(w * 64 + bit);
        }
    }
}

/// Take every pending request and drop every stream let go of, across
/// `sets`, and unmark the rings that are idle again. Returns (did
/// anything, is any stream open).
fn admin_all(sets: &[Arc<StreamSet>]) -> (bool, bool) {
    let mut worked = false;
    let mut open = false;
    for set in sets {
        if set.paused.load(Ordering::Acquire) {
            open |= set.rings_open() > 0;
            continue;
        }
        for_each_open(set, |ri| {
            let ring = &set.rings[ri];
            let active = ring.active_gen.load(Ordering::Acquire);
            let served = ring.reader_gen.load(Ordering::Acquire);
            let pending = !ring.req.load(Ordering::Acquire).is_null();
            if pending || (served != 0 && served != active) {
                if let Some(mut side) = ring.reader.try_lock() {
                    worked |= guarded(ring, &mut side, |side| admin(set, ring, side));
                }
            }
            set.close_if_idle(ri);
        });
        open |= set.rings_open() > 0;
    }
    (worked, open)
}

/// How far (in ring frames) a stream may be filled now, its voice having
/// read up to `read`: one chunk while the voice is more than a ring's
/// length from its tail, a whole ring ahead of the voice after.
fn fill_limit(ring: &Ring, read: u64) -> u64 {
    if ring.head_left.load(Ordering::Acquire) > RING_FRAMES as u64 {
        READ_CHUNK as u64
    } else {
        read + RING_FRAMES as u64
    }
}

/// The deadline of a served ring that has room for a worthwhile read (or
/// its last one) — frames until its voice runs out: what is left of its
/// head plus what the ring buffers; `None` when it wants no read. Lock-free.
fn wants_fill(ring: &Ring) -> Option<u64> {
    let gen = ring.reader_gen.load(Ordering::Acquire);
    if gen == 0 || ring.active_gen.load(Ordering::Acquire) != gen {
        return None;
    }
    let wpos = ring.wpos.load(Ordering::Acquire);
    if (wpos >> 32) as u32 != gen || wpos & WPOS_FAILED != 0 {
        return None;
    }
    let read = ring.read.load(Ordering::Acquire);
    let next = (wpos & WPOS_FRAMES).max(read);
    let end = ring.reader_end.load(Ordering::Acquire);
    if next >= end {
        return None;
    }
    let room = fill_limit(ring, read).saturating_sub(next);
    let n = room.min(end - next).min(READ_CHUNK as u64);
    if n == 0 || (n < READ_CHUNK as u64 / 4 && n < end - next) {
        return None;
    }
    // In output frames: what the ring buffers lasts a pitched voice
    // (E8) its length over its rate.
    let rate_q16 = ring.rate_q16.load(Ordering::Acquire);
    Some(ring.head_left.load(Ordering::Acquire) + Pace::frames_to_time(next - read, rate_q16))
}

/// Drop a stream the audio thread has let go of, and take a pending
/// request (see the protocol in the module docs). Returns whether it did
/// anything.
fn admin(set: &StreamSet, ring: &Ring, side: &mut ReaderSide) -> bool {
    let mut worked = false;
    if side.gen != 0 && ring.active_gen.load(Ordering::Acquire) != side.gen {
        side.reset();
        ring.reader_gen.store(0, Ordering::Release);
        worked = true;
    }
    let Some((source, gen, start)) = ring.take_request() else {
        return worked;
    };
    side.reset();
    ring.reader_gen.store(0, Ordering::Release);
    if ring.active_gen.load(Ordering::Acquire) != gen {
        // Let go of before it was even taken: just drop it (here, off the
        // audio thread).
        return true;
    }
    // The ring's storage, allocated the first time it is served — here,
    // before any frame is published, never on the audio thread.
    if ring.data_or_alloc().1 {
        set.note_allocated();
    }
    let file = File::open(&source.path)
        .ok()
        .filter(|f| f.metadata().is_ok_and(|m| source.same_file(&m)));
    if file.is_none() {
        ring.fail(gen);
    }
    ring.reader_end.store(
        source.tail.frames().saturating_sub(start),
        Ordering::Release,
    );
    side.source = Some(source);
    side.file = file;
    side.gen = gen;
    side.start = start;
    side.next = 0;
    ring.reader_gen.store(gen, Ordering::Release);
    true
}

/// Read the next chunk of a served stream into its ring, if the voice
/// has made room for one. Returns whether it did anything.
fn fill(
    set: &StreamSet,
    ring: &Ring,
    side: &mut ReaderSide,
    scratch: &mut TailScratch,
    buf: &mut [f32],
) -> bool {
    if side.gen == 0 {
        return false;
    }
    if ring.active_gen.load(Ordering::Acquire) != side.gen {
        side.reset();
        ring.reader_gen.store(0, Ordering::Release);
        return true;
    }
    let (Some(source), Some(file)) = (side.source.as_ref(), side.file.as_ref()) else {
        return false;
    };
    let tagged = (side.gen as u64) << 32;
    let total = source.tail.frames().saturating_sub(side.start);
    let read = ring.read.load(Ordering::Acquire);
    // Behind the voice (it underran): skip to where it is.
    let next = side.next.max(read);
    if next >= total {
        // Delivered in full: the file is done with.
        side.file = None;
        return false;
    }
    let room = fill_limit(ring, read).saturating_sub(next);
    let n = room.min(total - next).min(READ_CHUNK as u64) as usize;
    if n == 0 {
        return false;
    }
    if set
        .panic_reads
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
        .is_ok()
    {
        panic!("resonance-drums stream: test hook, a reader fault");
    }
    let latency = set.read_latency_us.load(Ordering::Relaxed);
    if latency > 0 {
        std::thread::sleep(Duration::from_micros(latency as u64));
    }
    let stride = source.tail.channels();
    let out = &mut buf[..n * stride];
    if source
        .tail
        .read(file, side.start + next, out, scratch)
        .is_err()
    {
        side.file = None;
        ring.fail(side.gen);
        return true;
    }
    let (data, _) = ring.data_or_alloc();
    for (j, frame) in out.chunks_exact(stride).enumerate() {
        let slot = ((next as usize + j) % RING_FRAMES) * stride;
        for (ch, &s) in frame.iter().enumerate() {
            data[slot + ch].store(s.to_bits(), Ordering::Relaxed);
        }
    }
    let published = next + n as u64;
    if ring
        .wpos
        .compare_exchange(
            tagged | side.next,
            tagged | published,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        // The ring has moved on to another generation.
        side.reset();
        ring.reader_gen.store(0, Ordering::Release);
        return true;
    }
    side.next = published;
    true
}
