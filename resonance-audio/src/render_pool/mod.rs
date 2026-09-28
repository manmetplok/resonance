//! The render worker pool: runs a block's independent jobs on several
//! threads and joins before the block continues
//! (realtime-multithreading.md §4.3).
//!
//! # Protocol
//!
//! The rendering thread (the *caller*: the PipeWire data thread live, a
//! render worker offline) is itself a worker. [`RenderPool::run`]:
//!
//! 1. stores the job closure and count, then opens an **epoch** (odd
//!    value) and unparks as many parked workers as there are jobs for;
//! 2. runs its caller-only jobs, then claims shared jobs with one
//!    `fetch_add` each, exactly like a worker;
//! 3. spins until every job is done (the *join*), then closes the epoch
//!    (even value) and waits until no worker is still inside it.
//!
//! Step 3's second wait is what makes lending a stack closure to other
//! threads sound: a worker registers in `active` *before* it re-checks the
//! epoch and only then reads the closure, and the caller stores the closed
//! epoch *before* it reads `active` (all `SeqCst`). Either the worker sees
//! the epoch closed and never touches the closure, or the caller sees the
//! worker registered and waits for it to leave.
//!
//! A pool that never got its workers, or whose workers could not match the
//! caller's realtime priority, runs every job on the caller — today's
//! serial engine, never silence.
//!
//! # Idle behaviour
//!
//! After a block a worker spins on the epoch for [`spin window`] (the next
//! block usually arrives within it at small quanta), then parks. Parking is
//! `std::thread::park`, whose wake token makes an unpark that races ahead
//! of the park harmless.
//!
//! Everything [`RenderPool::run`] does is allocation-free and lock-free:
//! atomics, `unpark` (a futex wake) and spinning.
//!
//! [`spin window`]: PoolConfig::spin

pub mod sched;

use std::cell::UnsafeCell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::{JoinHandle, Thread};
use std::time::{Duration, Instant};

use crate::bypass::FxDryScratch;
use crate::limits::{MAX_MIDI_EVENTS_PER_BUFFER, MAX_PLUGIN_OUTPUT_PORTS};
use crate::types::PendingNoteEvent;

use sched::Sched;

/// What a worker owns for the jobs it runs: the per-worker half of the
/// render scratch (realtime-multithreading.md §4.2).
pub(crate) struct WorkerScratch {
    port_scratch: Vec<(Vec<f32>, Vec<f32>)>,
    note_event_buf: Vec<PendingNoteEvent>,
    fx_dry: FxDryScratch,
}

impl WorkerScratch {
    /// Pre-allocated and pre-faulted for blocks of up to `frames`.
    pub(crate) fn new(frames: usize) -> Self {
        let frames = frames.max(1);
        let mut port_scratch: Vec<(Vec<f32>, Vec<f32>)> = (0..MAX_PLUGIN_OUTPUT_PORTS)
            .map(|_| (vec![0.0; frames], vec![0.0; frames]))
            .collect();
        for (l, r) in port_scratch.iter_mut() {
            crate::prefault::prefault_f32(l);
            crate::prefault::prefault_f32(r);
        }
        Self {
            port_scratch,
            note_event_buf: Vec::with_capacity(MAX_MIDI_EVENTS_PER_BUFFER),
            fx_dry: FxDryScratch::new(frames),
        }
    }

    pub(crate) fn bufs(&mut self) -> WorkerBufs<'_> {
        WorkerBufs {
            port_scratch: &mut self.port_scratch,
            note_event_buf: &mut self.note_event_buf,
            fx_dry: &mut self.fx_dry,
        }
    }
}

/// A job's view of the scratch of the thread running it.
pub(crate) struct WorkerBufs<'a> {
    pub(crate) port_scratch: &'a mut [(Vec<f32>, Vec<f32>)],
    pub(crate) note_event_buf: &'a mut Vec<PendingNoteEvent>,
    pub(crate) fx_dry: &'a mut FxDryScratch,
}

/// A job: run job `index` with the running thread's scratch.
pub(crate) type Job<'f> = dyn Fn(usize, &mut WorkerBufs<'_>) + Sync + 'f;

/// How to build a pool.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Worker threads besides the caller. 0 = a serial pool.
    pub workers: usize,
    /// Longest block a job renders, for the workers' scratch.
    pub max_frames: usize,
    /// Whether workers copy the caller's scheduling class (live: yes,
    /// and a worker that cannot match a realtime caller disables the
    /// pool) or stay at normal priority (offline renders).
    pub follow_caller_sched: bool,
    /// How long an idle worker spins on the epoch before parking.
    pub spin: Duration,
    /// Thread name prefix.
    pub name: &'static str,
    /// Test hook: every worker's scheduling change fails with `EPERM`, as
    /// on a system whose `RLIMIT_RTPRIO` forbids realtime.
    pub deny_sched: bool,
}

impl PoolConfig {
    /// The live callback's pool, over `threads` total render threads (the
    /// audio thread included; 1 = serial): `RESONANCE_RENDER_THREADS` if
    /// set, else `configured` (the user's setting), else physical cores −
    /// 1 workers plus the audio thread. SMT siblings are left alone: two
    /// hot DSP threads on one core mostly contend. Allocates (reads
    /// sysfs); engine side.
    pub fn live(max_frames: usize, configured: Option<usize>) -> Self {
        let threads = env_threads()
            .or(configured)
            .unwrap_or_else(sched::physical_cores);
        let spin_us = std::env::var("RESONANCE_RENDER_SPIN_US")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_SPIN_US);
        Self {
            workers: threads.max(1) - 1,
            max_frames,
            follow_caller_sched: true,
            spin: Duration::from_micros(spin_us),
            name: "resonance-render",
            deny_sched: false,
        }
    }

    /// An offline renderer's pool (bounce, stems, freeze, measure —
    /// realtime-multithreading.md §4.7): [`configured_threads`] threads
    /// at normal priority. Its own pool, because an offline render can run
    /// while the live callback owns the realtime one.
    pub fn offline(max_frames: usize) -> Self {
        Self {
            workers: configured_threads() - 1,
            max_frames,
            follow_caller_sched: false,
            spin: Duration::from_micros(DEFAULT_SPIN_US),
            name: "resonance-offline",
            deny_sched: false,
        }
    }
}

/// Total render threads (the rendering thread included) the engine was
/// configured with; 0 until [`configure_threads`] runs.
static CONFIGURED_THREADS: AtomicUsize = AtomicUsize::new(0);

/// Record the engine's render thread count, for every pool built later —
/// the offline renderers' included. Engine startup.
pub fn configure_threads(threads: usize) {
    CONFIGURED_THREADS.store(threads.max(1), Ordering::Relaxed);
}

/// The render thread count pools use when none is given: the engine's
/// (see [`configure_threads`]), else `RESONANCE_RENDER_THREADS`, else 1.
/// A process with no engine — a hermetic test — renders serially unless
/// asked.
pub fn configured_threads() -> usize {
    if let Some(n) = THREADS_OVERRIDE.with(|o| o.get()) {
        return n;
    }
    match CONFIGURED_THREADS.load(Ordering::Relaxed) {
        0 => env_threads().unwrap_or(1),
        n => n,
    }
}

thread_local! {
    static THREADS_OVERRIDE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Test hook: pools built on the calling thread use `threads` (`None`
/// restores [`configured_threads`]' usual answer). Thread-scoped, so
/// tests running side by side in one binary cannot change each other's
/// renders.
pub fn override_threads_on_this_thread(threads: Option<usize>) {
    THREADS_OVERRIDE.with(|o| o.set(threads.map(|n| n.max(1))));
}

fn env_threads() -> Option<usize> {
    std::env::var("RESONANCE_RENDER_THREADS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .map(|n| n.max(1))
}

/// Default idle spin before a worker parks: long enough to cover the
/// scheduling jitter between back-to-back small-quantum blocks' job
/// batches, short enough that an idle engine does not burn cores.
pub const DEFAULT_SPIN_US: u64 = 50;

/// Why a pool renders with fewer threads than it was built with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolHealth {
    /// Every worker is usable (or the pool has none).
    Ok,
    /// A worker could not take the caller's realtime class, so the pool
    /// renders serially. The OS error code is kept for the report.
    RealtimeDenied { errno: i32 },
}

/// A snapshot of a pool for reporting (engine side).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolStatus {
    /// Workers the pool was built with.
    pub workers: usize,
    /// Threads jobs actually run on: workers + caller, or 1 when the pool
    /// fell back to serial.
    pub effective_threads: usize,
    /// The class workers were asked to match: the caller's, once it has
    /// rendered through the pool.
    pub sched: Option<Sched>,
    pub health: PoolHealth,
}

/// See [`RenderPool::monitor`].
#[derive(Clone)]
pub(crate) struct PoolMonitor {
    shared: Arc<Shared>,
    workers: usize,
}

impl PoolMonitor {
    pub(crate) fn status(&self) -> PoolStatus {
        let errno = self.shared.sched_errno.load(Ordering::Relaxed);
        let health = if errno == 0 {
            PoolHealth::Ok
        } else {
            PoolHealth::RealtimeDenied {
                errno: (errno - 1) as i32,
            }
        };
        PoolStatus {
            workers: self.workers,
            effective_threads: if self.workers > 0 && health == PoolHealth::Ok {
                self.workers + 1
            } else {
                1
            },
            sched: Sched::unpack(self.shared.sched_want.load(Ordering::Relaxed)),
            health,
        }
    }
}

/// What one [`RenderPool::run`] measured.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RunStats {
    /// Nanoseconds the caller spun at the join with no job left to claim.
    pub(crate) join_wait_ns: u64,
    /// Threads the jobs could have run on this time (caller included).
    pub(crate) threads: usize,
}

struct Task {
    job: *const Job<'static>,
    n: usize,
    /// When set, shared claim `k` runs job `perm[k]` (test hook).
    permute: bool,
}

struct Shared {
    /// Odd = a run is open. Written only by the caller.
    epoch: AtomicU64,
    next: AtomicUsize,
    done: AtomicUsize,
    /// Workers inside the open epoch (see the module docs).
    active: AtomicUsize,
    shutdown: AtomicBool,
    task: UnsafeCell<Task>,
    shuffle: UnsafeCell<Shuffle>,
    parked: Box<[AtomicBool]>,
    /// Workers past their startup (see `worker_main`).
    ready: AtomicUsize,
    /// Jobs each worker has run, one cache line each (see
    /// [`RenderPool::min_worker_jobs`]).
    jobs_run: Box<[PaddedCounter]>,
    /// The workers' handles, for a plugin's sub-task request to wake
    /// parked ones (it reaches the pool only through [`CURRENT_POOL`]).
    threads: OnceLock<Box<[Thread]>>,
    /// Plugin sub-tasks (CLAP `thread-pool`) — see [`exec_plugin_tasks`].
    sub: SubTasks,
    /// Packed [`Sched`] the workers should run at; 0 until the caller has
    /// published its own.
    sched_want: AtomicU64,
    /// First worker scheduling failure, as `errno + 1`; 0 = none.
    sched_errno: AtomicU64,
    follow_caller_sched: bool,
    deny_sched: bool,
    spin: Duration,
}

#[repr(align(64))]
struct PaddedCounter(AtomicU64);

// SAFETY: `task` and `shuffle` are written only by the caller while the
// epoch is closed and no worker is registered, and read by workers only
// while registered in an open epoch (module docs); `sub.task` likewise,
// under the sub-task epoch and the `sub.busy` claim. Everything else is
// atomic.
unsafe impl Sync for Shared {}
unsafe impl Send for Shared {}

/// See the module docs.
pub(crate) struct RenderPool {
    shared: Arc<Shared>,
    threads: Vec<Thread>,
    handles: Vec<JoinHandle<()>>,
    caller_sched_published: AtomicBool,
}

impl RenderPool {
    /// Spawn the workers. Allocates; never call on the audio thread.
    pub(crate) fn new(config: PoolConfig) -> Self {
        let shared = Arc::new(Shared {
            epoch: AtomicU64::new(0),
            next: AtomicUsize::new(0),
            done: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
            task: UnsafeCell::new(Task {
                job: std::ptr::null::<fn(usize, &mut WorkerBufs<'_>)>() as *const Job<'static>,
                n: 0,
                permute: false,
            }),
            shuffle: UnsafeCell::new(Shuffle {
                seed: 0,
                perm: Vec::new(),
            }),
            parked: (0..config.workers)
                .map(|_| AtomicBool::new(false))
                .collect(),
            ready: AtomicUsize::new(0),
            jobs_run: (0..config.workers)
                .map(|_| PaddedCounter(AtomicU64::new(0)))
                .collect(),
            threads: OnceLock::new(),
            sub: SubTasks::new(),
            sched_want: AtomicU64::new(0),
            sched_errno: AtomicU64::new(0),
            follow_caller_sched: config.follow_caller_sched,
            deny_sched: config.deny_sched,
            spin: config.spin,
        });
        let mut threads = Vec::with_capacity(config.workers);
        let mut handles = Vec::with_capacity(config.workers);
        for index in 0..config.workers {
            let worker_shared = Arc::clone(&shared);
            let scratch = WorkerScratch::new(config.max_frames);
            let spawned = std::thread::Builder::new()
                .name(format!("{}-{index}", config.name))
                .spawn(move || worker_main(worker_shared, index, scratch));
            match spawned {
                Ok(handle) => {
                    threads.push(handle.thread().clone());
                    handles.push(handle);
                }
                // Fewer workers is only slower; the caller always runs.
                Err(_) => break,
            }
        }
        let _ = shared.threads.set(threads.clone().into_boxed_slice());
        // A returned pool's workers own their per-thread state (the
        // arc-swap node): nothing a job touches first can allocate.
        while shared.ready.load(Ordering::SeqCst) < threads.len() {
            std::thread::yield_now();
        }
        Self {
            shared,
            threads,
            handles,
            caller_sched_published: AtomicBool::new(false),
        }
    }

    /// A pool with no workers: every job runs on the caller.
    pub(crate) fn serial() -> Self {
        Self::new(PoolConfig {
            workers: 0,
            max_frames: 1,
            follow_caller_sched: false,
            spin: Duration::ZERO,
            name: "resonance-render",
            deny_sched: false,
        })
    }

    /// Threads a run currently spreads over, the caller included.
    pub(crate) fn effective_threads(&self) -> usize {
        if self.usable() {
            self.threads.len() + 1
        } else {
            1
        }
    }

    fn usable(&self) -> bool {
        !self.threads.is_empty() && self.shared.sched_errno.load(Ordering::Relaxed) == 0
    }

    pub(crate) fn status(&self) -> PoolStatus {
        self.monitor().status()
    }

    /// A read-only handle on this pool's status for another thread (the
    /// engine loop's reporting).
    pub(crate) fn monitor(&self) -> PoolMonitor {
        PoolMonitor {
            shared: Arc::clone(&self.shared),
            workers: self.threads.len(),
        }
    }

    /// The fewest jobs any worker has run since the pool was built (0 for
    /// a pool without workers). Lets a test prove every worker has served
    /// a block. Allocation-free.
    pub(crate) fn min_worker_jobs(&self) -> u64 {
        self.shared
            .jobs_run
            .iter()
            .take(self.threads.len())
            .map(|c| c.0.load(Ordering::Relaxed))
            .min()
            .unwrap_or(0)
    }

    /// Test hook: shuffle the order shared jobs are claimed in, from
    /// `seed`, for runs of up to `max_jobs` jobs (0 turns it off), so a
    /// correctness test never depends on scheduling luck. Allocates.
    pub(crate) fn set_claim_shuffle(&mut self, seed: u64, max_jobs: usize) {
        // `&mut self`: no run can be open, so no worker reads it.
        let shuffle = unsafe { &mut *self.shared.shuffle.get() };
        shuffle.seed = seed;
        shuffle.perm = Vec::with_capacity(if seed == 0 { 0 } else { max_jobs });
    }

    /// Run jobs `0..n` and return once all are done. Jobs
    /// `0..caller_only` run on the caller, before it claims shared ones
    /// (a chain that must stay on one thread); the rest go to whichever
    /// thread claims them first. `caller` is the calling thread's scratch.
    /// Allocation-free and lock-free.
    pub(crate) fn run(
        &self,
        caller: &mut WorkerBufs<'_>,
        caller_only: usize,
        n: usize,
        job: &Job<'_>,
    ) -> RunStats {
        let caller_only = caller_only.min(n);
        let shared_jobs = n - caller_only;
        // A plugin in one of these jobs may hand work back to the pool
        // (CLAP `thread-pool`): point this thread at it for the run.
        let _current = self.usable().then(|| CurrentPool::enter(&self.shared));
        if shared_jobs <= 1 || !self.usable() {
            for index in 0..n {
                job(index, caller);
            }
            return RunStats {
                join_wait_ns: 0,
                threads: 1,
            };
        }
        self.publish_caller_sched();

        let s = &*self.shared;
        // SAFETY: the epoch is closed and no worker is registered (the
        // previous run's close waited for `active == 0`), so nothing reads
        // `task` or `perm` now. The lifetime erasure is undone by the
        // close below, which outlives every worker's use of `job`.
        let permute = unsafe {
            let permute = (*s.shuffle.get()).shuffle(caller_only, n);
            *s.task.get() = Task {
                job: std::mem::transmute::<*const Job<'_>, *const Job<'static>>(job),
                n,
                permute,
            };
            permute
        };
        s.next.store(caller_only, Ordering::Relaxed);
        s.done.store(0, Ordering::Relaxed);
        let epoch = s.epoch.load(Ordering::Relaxed) + 1;
        s.epoch.store(epoch, Ordering::SeqCst);
        let _close = CloseOnDrop(s, epoch);

        // Wake only as many as can find a job; the caller takes one too.
        let mut to_wake = shared_jobs - 1;
        for (flag, thread) in s.parked.iter().zip(&self.threads) {
            if to_wake == 0 {
                break;
            }
            if flag.load(Ordering::SeqCst) {
                thread.unpark();
            }
            to_wake -= 1;
        }

        for index in 0..caller_only {
            job(index, caller);
            s.done.fetch_add(1, Ordering::Release);
        }
        loop {
            let claim = s.next.fetch_add(1, Ordering::AcqRel);
            if claim >= n {
                break;
            }
            // SAFETY: read-only while the epoch is open.
            let index = if permute {
                unsafe { (&*s.shuffle.get()).perm[claim] as usize }
            } else {
                claim
            };
            job(index, caller);
            s.done.fetch_add(1, Ordering::Release);
        }
        let wait_start = Instant::now();
        let mut spins = 0u32;
        while s.done.load(Ordering::Acquire) < n {
            // Waiting on a job whose plugin split its work: help with it.
            if !help_sub_tasks(s) {
                spin(&mut spins);
            }
        }
        RunStats {
            join_wait_ns: wait_start.elapsed().as_nanos() as u64,
            threads: self.threads.len() + 1,
        }
    }

    /// Hand the workers the caller's scheduling class, once. One syscall
    /// on the caller's first parallel run.
    fn publish_caller_sched(&self) {
        if !self.shared.follow_caller_sched
            || self.caller_sched_published.swap(true, Ordering::Relaxed)
        {
            return;
        }
        let want = sched::current();
        self.shared.sched_want.store(want.pack(), Ordering::Release);
    }
}

impl Drop for RenderPool {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::SeqCst);
        for thread in &self.threads {
            thread.unpark();
        }
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

/// One iteration of a busy-wait. Every [`YIELD_EVERY`]th one yields the
/// CPU: at equal `SCHED_FIFO` priority a spinning thread is never
/// preempted by a same-priority thread queued on its CPU, so a caller
/// spinning at the join could otherwise starve the very worker it waits
/// for, if the scheduler put them on one core. `sched_yield` hands the CPU
/// to exactly those threads, and returns at once when there are none.
#[inline]
fn spin(spins: &mut u32) {
    *spins = spins.wrapping_add(1);
    if *spins % YIELD_EVERY == 0 {
        std::thread::yield_now();
    } else {
        std::hint::spin_loop();
    }
}

const YIELD_EVERY: u32 = 64;

/// Closes the epoch and waits for every registered worker to leave it —
/// on the normal path and when a job panics on the caller.
struct CloseOnDrop<'a>(&'a Shared, u64);

impl Drop for CloseOnDrop<'_> {
    fn drop(&mut self) {
        let s = self.0;
        s.epoch.store(self.1 + 1, Ordering::SeqCst);
        let mut spins = 0u32;
        while s.active.load(Ordering::SeqCst) != 0 {
            spin(&mut spins);
        }
    }
}

/// The claim-order test hook (see [`RenderPool::set_claim_shuffle`]).
struct Shuffle {
    /// xorshift64* state; 0 = off.
    seed: u64,
    /// Claim slot → job index, for the open run.
    perm: Vec<u32>,
}

impl Shuffle {
    /// Shuffle the shared claims `first..n` for the next run. `false`
    /// (claim in order) when the hook is off or `n` exceeds the capacity
    /// reserved for it, so this never allocates.
    fn shuffle(&mut self, first: usize, n: usize) -> bool {
        if self.seed == 0 || n > self.perm.capacity() {
            return false;
        }
        self.perm.clear();
        self.perm.extend((0..n).map(|i| i as u32));
        for i in (first + 1..n).rev() {
            let j = first + (self.next() % (i - first + 1) as u64) as usize;
            self.perm.swap(i, j);
        }
        true
    }

    fn next(&mut self) -> u64 {
        let mut x = self.seed;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.seed = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

fn worker_main(shared: Arc<Shared>, index: usize, mut scratch: WorkerScratch) {
    crate::clap_host::thread_check::mark_audio_thread();
    resonance_dsp::flush_denormals();
    // At spawn, not inside the first job: that job can come arbitrarily
    // late (under load the caller and the other workers may claim every
    // job for many blocks), and it would allocate mid-block.
    crate::rt_prep::claim_arc_swap_node();
    let s = &*shared;
    s.ready.fetch_add(1, Ordering::SeqCst);
    // A plugin running on this worker may split its work into sub-tasks.
    let _current = CurrentPool::enter(s);
    let mut seen = 0u64;
    let mut applied_sched = 0u64;
    loop {
        // Wait for an epoch we haven't served: spin, then park.
        let epoch = 'wait: loop {
            let spin_start = Instant::now();
            let mut spins = 0u32;
            loop {
                if s.shutdown.load(Ordering::Acquire) {
                    return;
                }
                let epoch = s.epoch.load(Ordering::Acquire);
                if epoch & 1 == 1 && epoch != seen {
                    break 'wait epoch;
                }
                if help_sub_tasks(s) {
                    continue 'wait;
                }
                if spins % YIELD_EVERY != YIELD_EVERY - 1 || spin_start.elapsed() < s.spin {
                    spin(&mut spins);
                    continue;
                }
                // Park. The flag goes up before the re-check, and the
                // caller stores the epoch before it reads the flag, so a
                // wake cannot be missed; an early unpark leaves a token
                // that makes `park` return at once.
                s.parked[index].store(true, Ordering::SeqCst);
                let epoch = s.epoch.load(Ordering::SeqCst);
                let sub_open = s.sub.epoch.load(Ordering::SeqCst) & 1 == 1;
                if !(epoch & 1 == 1 && epoch != seen)
                    && !sub_open
                    && !s.shutdown.load(Ordering::SeqCst)
                {
                    std::thread::park();
                }
                s.parked[index].store(false, Ordering::Relaxed);
                continue 'wait;
            }
        };
        seen = epoch;

        // Match the caller's scheduling class before serving it. This can
        // take a while, so it skips this epoch; the caller never waits
        // for a worker that did not register.
        let want = s.sched_want.load(Ordering::Acquire);
        if want != applied_sched {
            applied_sched = want;
            if let Some(sched) = Sched::unpack(want) {
                let applied = if s.deny_sched {
                    Err(std::io::Error::from_raw_os_error(1)) // EPERM
                } else {
                    sched::apply(sched)
                };
                if let Err(err) = applied {
                    let errno = err.raw_os_error().unwrap_or(0).max(0) as u64;
                    let _ = s.sched_errno.compare_exchange(
                        0,
                        errno + 1,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    );
                }
            }
            continue;
        }

        s.active.fetch_add(1, Ordering::SeqCst);
        if s.epoch.load(Ordering::SeqCst) != epoch {
            s.active.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        // Plugins may leave MXCSR changed; re-assert FTZ/DAZ per run.
        resonance_dsp::flush_denormals();
        // SAFETY: registered in the open epoch `epoch` (module docs).
        let task = unsafe { &*s.task.get() };
        let job = unsafe { &*task.job };
        let mut bufs = scratch.bufs();
        loop {
            let claim = s.next.fetch_add(1, Ordering::AcqRel);
            if claim >= task.n {
                break;
            }
            let job_index = if task.permute {
                unsafe { (&*s.shuffle.get()).perm[claim] as usize }
            } else {
                claim
            };
            // A panicking job must still count as done, or the join
            // would spin forever. The render path does not panic; this
            // is the backstop.
            let _ = catch_unwind(AssertUnwindSafe(|| job(job_index, &mut bufs)));
            s.done.fetch_add(1, Ordering::Release);
            s.jobs_run[index].0.fetch_add(1, Ordering::Relaxed);
        }
        s.active.fetch_sub(1, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Plugin sub-tasks (CLAP `thread-pool`, realtime-multithreading.md §5 P4)
// ---------------------------------------------------------------------------

thread_local! {
    /// The pool the calling thread renders for: set on every worker, and
    /// on a caller for the length of a run.
    static CURRENT_POOL: Cell<*const Shared> = const { Cell::new(std::ptr::null()) };
}

/// Scoped [`CURRENT_POOL`] entry; restores the previous value on drop.
struct CurrentPool(*const Shared);

impl CurrentPool {
    fn enter(shared: &Shared) -> Self {
        Self(CURRENT_POOL.with(|c| c.replace(shared)))
    }
}

impl Drop for CurrentPool {
    fn drop(&mut self) {
        CURRENT_POOL.with(|c| c.set(self.0));
    }
}

/// A plugin sub-task batch: `run(i)` for `i in 0..n`.
type SubJob<'f> = dyn Fn(u32) + Sync + 'f;

struct SubTask {
    job: *const SubJob<'static>,
    n: usize,
}

/// One plugin's sub-task batch at a time, on the same epoch / claim /
/// join protocol as the main runs (module docs), claimed first through
/// `busy` so two plugins asking at once cannot share it.
struct SubTasks {
    busy: AtomicBool,
    epoch: AtomicU64,
    next: AtomicUsize,
    done: AtomicUsize,
    active: AtomicUsize,
    task: UnsafeCell<SubTask>,
}

impl SubTasks {
    fn new() -> Self {
        Self {
            busy: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            next: AtomicUsize::new(0),
            done: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            task: UnsafeCell::new(SubTask {
                job: std::ptr::null::<fn(u32)>() as *const SubJob<'static>,
                n: 0,
            }),
        }
    }
}

/// Run a plugin's `thread-pool` request: `task(i)` for every `i` in
/// `0..n`, returning once all are done. Called from inside a plugin's
/// `process()`.
///
/// On a thread rendering for a pool, the tasks are shared out: the
/// calling thread runs them too, and idle workers — plus a caller waiting
/// at its join — pick them up. With no pool (a serial render), or while
/// another plugin's batch holds the pool's sub-task channel, the calling
/// thread runs them all itself: CLAP only requires that the host executed
/// every task. Allocation-free and lock-free.
pub(crate) fn exec_plugin_tasks(n: u32, task: &SubJob<'_>) {
    let n = n as usize;
    let current = CURRENT_POOL.with(|c| c.get());
    // SAFETY: a set pointer is the `Shared` of a pool this thread is
    // rendering for right now, kept alive by the run or the worker.
    let Some(s) = (unsafe { current.as_ref() }) else {
        return (0..n).for_each(|i| task(i as u32));
    };
    let sub = &s.sub;
    if n <= 1
        || sub
            .busy
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
    {
        return (0..n).for_each(|i| task(i as u32));
    }
    // SAFETY: `busy` is ours and the previous batch's close waited for
    // `active == 0`, so nobody reads `task`; the close below outlives
    // every helper's use of the erased borrow.
    unsafe {
        *sub.task.get() = SubTask {
            job: std::mem::transmute::<*const SubJob<'_>, *const SubJob<'static>>(task),
            n,
        };
    }
    sub.next.store(0, Ordering::Relaxed);
    sub.done.store(0, Ordering::Relaxed);
    let epoch = sub.epoch.load(Ordering::Relaxed) + 1;
    sub.epoch.store(epoch, Ordering::SeqCst);
    let _close = CloseSubOnDrop(sub, epoch);

    if let Some(threads) = s.threads.get() {
        let mut to_wake = n - 1;
        for (flag, thread) in s.parked.iter().zip(threads.iter()) {
            if to_wake == 0 {
                break;
            }
            if flag.load(Ordering::SeqCst) {
                thread.unpark();
                to_wake -= 1;
            }
        }
    }
    loop {
        let claim = sub.next.fetch_add(1, Ordering::AcqRel);
        if claim >= n {
            break;
        }
        task(claim as u32);
        sub.done.fetch_add(1, Ordering::Release);
    }
    let mut spins = 0u32;
    while sub.done.load(Ordering::Acquire) < n {
        spin(&mut spins);
    }
}

/// Closes a sub-task batch, waits for its helpers to leave, and frees the
/// channel — on the normal path and when a task panics on the requester.
struct CloseSubOnDrop<'a>(&'a SubTasks, u64);

impl Drop for CloseSubOnDrop<'_> {
    fn drop(&mut self) {
        let sub = self.0;
        sub.epoch.store(self.1 + 1, Ordering::SeqCst);
        let mut spins = 0u32;
        while sub.active.load(Ordering::SeqCst) != 0 {
            spin(&mut spins);
        }
        sub.busy.store(false, Ordering::Release);
    }
}

/// Help with an open sub-task batch, if there is one. Returns whether
/// this thread took part.
fn help_sub_tasks(s: &Shared) -> bool {
    let sub = &s.sub;
    let epoch = sub.epoch.load(Ordering::Acquire);
    if epoch & 1 == 0 {
        return false;
    }
    sub.active.fetch_add(1, Ordering::SeqCst);
    if sub.epoch.load(Ordering::SeqCst) != epoch {
        sub.active.fetch_sub(1, Ordering::SeqCst);
        return false;
    }
    // SAFETY: registered in the open sub-task epoch.
    let task = unsafe { &*sub.task.get() };
    let job = unsafe { &*task.job };
    loop {
        let claim = sub.next.fetch_add(1, Ordering::AcqRel);
        if claim >= task.n {
            break;
        }
        let _ = catch_unwind(AssertUnwindSafe(|| job(claim as u32)));
        sub.done.fetch_add(1, Ordering::Release);
    }
    sub.active.fetch_sub(1, Ordering::SeqCst);
    true
}
