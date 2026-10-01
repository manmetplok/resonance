//! The disk-reader pool: a few threads that fill every registered
//! sampler's rings from disk (see the [module docs](super)).
//!
//! One process-wide pool ([`ReaderPool::global`]) serves every drum
//! instance; tests may build their own and shut it down to see what the
//! audio thread does with a dead reader.
//!
//! The readers poll: the audio thread never wakes them (that would be a
//! syscall on the audio thread). A pass that found work is followed by
//! another at once; an idle one sleeps [`ACTIVE_POLL`] while any ring is
//! in use and [`IDLE_POLL`] otherwise. The lead a reader needs is
//! covered by the voice's head (≥ 32 k frames) before the voice reaches
//! its tail, and by the ring ([`super::RING_FRAMES`]) after.

use std::fs::File;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;
use resonance_common::TailScratch;

use super::{Ring, StreamSet, TailSource, READ_CHUNK, RING_FRAMES, WPOS_FAILED, WPOS_FRAMES};

/// Sleep between passes that found nothing to do while streams are open.
pub const ACTIVE_POLL: Duration = Duration::from_micros(500);
/// Sleep between passes with no stream open anywhere.
pub const IDLE_POLL: Duration = Duration::from_millis(5);

/// Reader threads in the process-wide pool.
pub const GLOBAL_READERS: usize = 2;

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
    shutdown: AtomicBool,
}

/// See the module docs.
pub struct ReaderPool {
    shared: Arc<PoolShared>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl ReaderPool {
    /// A pool of `threads` reader threads (at least one).
    pub fn new(threads: usize) -> Arc<Self> {
        let shared = Arc::new(PoolShared {
            sets: Mutex::new(Vec::new()),
            shutdown: AtomicBool::new(false),
        });
        let handles = (0..threads.max(1))
            .filter_map(|n| {
                let shared = shared.clone();
                std::thread::Builder::new()
                    .name(format!("resonance-drums-stream-{n}"))
                    .spawn(move || run(&shared))
                    .ok()
            })
            .collect();
        Arc::new(Self {
            shared,
            threads: Mutex::new(handles),
        })
    }

    /// The pool every sampler uses unless told otherwise.
    pub fn global() -> &'static Arc<ReaderPool> {
        static POOL: OnceLock<Arc<ReaderPool>> = OnceLock::new();
        POOL.get_or_init(|| ReaderPool::new(GLOBAL_READERS))
    }

    /// Serve `set` from now on, until it is dropped.
    pub fn register(&self, set: &Arc<StreamSet>) {
        let mut sets = self.shared.sets.lock();
        sets.retain(|s| s.strong_count() > 0);
        sets.push(Arc::downgrade(set));
    }

    /// Stop every reader thread and wait for them. The rings they served
    /// are left as they are: a test hook for a reader that dies.
    pub fn shutdown(&self) {
        self.shared.shutdown.store(true, Ordering::Release);
        for handle in self.threads.lock().drain(..) {
            let _ = handle.join();
        }
    }
}

fn run(shared: &PoolShared) {
    let mut sets: Vec<Arc<StreamSet>> = Vec::new();
    let mut scratch = TailScratch::default();
    let mut buf: Vec<f32> = vec![0.0; READ_CHUNK * 2];
    let mut wanting: Vec<(u64, usize, usize)> = Vec::new();
    while !shared.shutdown.load(Ordering::Acquire) {
        sets.clear();
        sets.extend(shared.sets.lock().iter().filter_map(Weak::upgrade));
        let (mut worked, open) = admin_all(&sets);
        // Fill the neediest ring first — the one with the least read
        // ahead of its voice — then take care of requests and
        // cancellations again before the next read, so a slow disk
        // never holds up a ring being handed back.
        loop {
            if shared.shutdown.load(Ordering::Acquire) {
                break;
            }
            wanting.clear();
            for (si, set) in sets.iter().enumerate() {
                if set.paused.load(Ordering::Acquire) {
                    continue;
                }
                for (ri, ring) in set.rings.iter().enumerate() {
                    if let Some(ahead) = wants_fill(ring) {
                        wanting.push((ahead, si, ri));
                    }
                }
            }
            if wanting.is_empty() {
                break;
            }
            wanting.sort_unstable();
            let mut filled = false;
            for &(_, si, ri) in &wanting {
                let set = &sets[si];
                let ring = &set.rings[ri];
                // Another reader thread is on it: the next one.
                let Some(mut side) = ring.reader.try_lock() else {
                    continue;
                };
                filled = fill(set, ring, &mut side, &mut scratch, &mut buf);
                break;
            }
            if !filled {
                break;
            }
            worked = true;
            admin_all(&sets);
        }
        // A set dropped by its sampler goes here, off the audio thread.
        sets.clear();
        if !worked {
            std::thread::sleep(if open { ACTIVE_POLL } else { IDLE_POLL });
        }
    }
}

/// Take every pending request and drop every stream let go of, across
/// `sets`. Returns (did anything, is any stream open).
fn admin_all(sets: &[Arc<StreamSet>]) -> (bool, bool) {
    let mut worked = false;
    let mut open = false;
    for set in sets {
        if set.paused.load(Ordering::Acquire) {
            continue;
        }
        for ring in set.rings.iter() {
            let active = ring.active_gen.load(Ordering::Acquire);
            let served = ring.reader_gen.load(Ordering::Acquire);
            let pending = !ring.req.load(Ordering::Acquire).is_null();
            open |= active != 0 || served != 0 || pending;
            if pending || (served != 0 && served != active) {
                if let Some(mut side) = ring.reader.try_lock() {
                    worked |= admin(ring, &mut side);
                }
            }
        }
    }
    (worked, open)
}

/// Frames buffered ahead of the voice, for a served ring that has room
/// for a worthwhile read (or its last one); `None` otherwise. Lock-free.
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
    let room = (read + RING_FRAMES as u64).saturating_sub(next);
    let n = room.min(end - next).min(READ_CHUNK as u64);
    if n == 0 || (n < READ_CHUNK as u64 / 4 && n < end - next) {
        return None;
    }
    Some(next - read)
}

/// Drop a stream the audio thread has let go of, and take a pending
/// request (see the protocol in the module docs). Returns whether it did
/// anything.
fn admin(ring: &Ring, side: &mut ReaderSide) -> bool {
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
    let file = File::open(&source.path)
        .ok()
        .filter(|f| f.metadata().is_ok_and(|m| m.len() == source.file_len));
    let tagged = (gen as u64) << 32;
    if file.is_none() {
        let _ = ring.wpos.compare_exchange(
            tagged,
            tagged | WPOS_FAILED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
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
    let room = (read + RING_FRAMES as u64).saturating_sub(next);
    let n = room.min(total - next).min(READ_CHUNK as u64) as usize;
    if n == 0 {
        return false;
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
        let _ = ring.wpos.compare_exchange(
            tagged | side.next,
            tagged | side.next | WPOS_FAILED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        return true;
    }
    for (j, frame) in out.chunks_exact(stride).enumerate() {
        let slot = ((next as usize + j) % RING_FRAMES) * stride;
        for (ch, &s) in frame.iter().enumerate() {
            ring.data[slot + ch].store(s.to_bits(), Ordering::Relaxed);
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
