//! Bounded worker pool for the engine's off-thread clip work (WAV
//! import and project-load mmap + waveform decimation).
//!
//! Why a pool and not a thread per request: the heavy step —
//! `ClipSource::open_wav_at_rate` (pre-touches every page of the mmap)
//! plus `compute_waveform_peaks` (an O(n) decimation across the whole
//! sample buffer) — is memory-bandwidth bound, so running more than a
//! handful at once only makes every one of them slower while the audio
//! callback competes for the same memory bandwidth. It used to run
//! on the engine control thread, which stalled the command queue for
//! hundreds of milliseconds on a project with many large clips.
//!
//! Why a queue and not a cap-and-drop: project load fires one
//! `LoadClipFromWav` per audio clip, so a hard rejection past the cap
//! meant any project with more than [`MAX_CONCURRENT_IMPORTS`] audio
//! clips silently lost the excess — the clips never reached the engine,
//! played back silent, and were then dropped from the bundle on the
//! next save. Requests now queue instead: nothing is ever discarded,
//! and concurrency is still bounded because at most
//! [`MAX_CONCURRENT_IMPORTS`] worker threads ever exist.
//!
//! [`ImportQueue::submit`] is what the engine thread calls. It pushes
//! onto an unbounded channel (never blocks) and lazily spawns a worker
//! if we are still below the cap, so the engine thread returns
//! immediately no matter how deep the backlog is. Workers park on
//! `recv()` when idle and exit when the queue (and therefore its
//! `Sender`) is dropped at engine shutdown, draining whatever is still
//! queued first.

use crossbeam_channel::{unbounded, Receiver, Sender};
use thiserror::Error;

use crate::types::{EngineError, EngineErrorKind};

/// Hard cap on concurrent clip decode/load workers. Requests past this
/// bound wait in the queue and run as workers free up; none are dropped.
pub const MAX_CONCURRENT_IMPORTS: usize = 4;

/// Failure spawning the pool's first worker thread. Transparent: Display
/// matches the wrapped `std::io::Error`'s text exactly (the historical
/// `e.to_string()`).
#[derive(Debug, Error)]
#[error(transparent)]
pub struct ImportQueueError(#[from] std::io::Error);

impl From<ImportQueueError> for EngineError {
    fn from(e: ImportQueueError) -> Self {
        EngineError::new(EngineErrorKind::Io, e.to_string())
    }
}

type ImportJob = Box<dyn FnOnce() + Send + 'static>;

/// Engine-thread-owned handle to the clip worker pool. Lives in
/// `HandlerState`; dropping it signals the workers to finish the
/// backlog and exit.
pub struct ImportQueue {
    tx: Sender<ImportJob>,
    /// Kept alive alongside the sender so a worker can be spawned lazily
    /// on any later `submit` — and so the channel never reports
    /// disconnected while the engine is up.
    rx: Receiver<ImportJob>,
    max_workers: usize,
    /// Workers spawned so far. Only ever grows, up to `max_workers`;
    /// workers are long-lived and park on `recv()` between jobs rather
    /// than exiting, so this is also the live worker count. That claim
    /// holds even for a panicking job: the worker loop contains each
    /// job's panic (`crate::supervise::run_supervised`), because a
    /// worker that unwound and died would never be replaced — after
    /// `max_workers` panics, imports would silently queue forever.
    workers: usize,
    /// Test-only: while `Some`, [`Self::submit`] parks each job here in
    /// submission order instead of running it, so a harness test can run
    /// them itself in whatever completion order it is pinning
    /// (`EngineHandlerHarness::hold_imports`).
    #[cfg(feature = "test-internals")]
    held: Option<Vec<ImportJob>>,
}

impl ImportQueue {
    /// Create a queue that will run at most `max_workers` jobs at once.
    /// A zero is clamped to one so a job can always make progress.
    pub fn new(max_workers: usize) -> Self {
        let (tx, rx) = unbounded();
        Self {
            tx,
            rx,
            max_workers: max_workers.max(1),
            workers: 0,
            #[cfg(feature = "test-internals")]
            held: None,
        }
    }

    /// Enqueue `job` for execution on a worker thread. Returns promptly
    /// — the send is to an unbounded channel and the (at most
    /// `max_workers`) spawn is cheap — so this is safe to call from the
    /// engine control thread.
    ///
    /// The only error case is failing to spawn the *first* worker, in
    /// which case nothing has been enqueued and the caller should report
    /// the failure. Once at least one worker exists a later spawn
    /// failure is benign: the job is queued and an existing worker will
    /// pick it up.
    pub fn submit<F: FnOnce() + Send + 'static>(&mut self, job: F) -> Result<(), ImportQueueError> {
        #[cfg(feature = "test-internals")]
        if let Some(held) = self.held.as_mut() {
            held.push(Box::new(job));
            return Ok(());
        }
        self.ensure_worker()?;
        // Unbounded channel with a live receiver held right here, so
        // this can neither block nor fail.
        let _ = self.tx.send(Box::new(job));
        Ok(())
    }

    /// Number of worker threads spawned so far (never above the cap).
    pub fn worker_count(&self) -> usize {
        self.workers
    }

    /// Test-only: park every later job instead of running it.
    #[cfg(feature = "test-internals")]
    pub(crate) fn hold(&mut self) {
        self.held.get_or_insert_with(Vec::new);
    }

    /// Test-only: the jobs parked since [`Self::hold`] (or the last
    /// call), in submission order. Holding stays on.
    #[cfg(feature = "test-internals")]
    pub(crate) fn take_held(&mut self) -> Vec<ImportJob> {
        self.held.as_mut().map(std::mem::take).unwrap_or_default()
    }

    fn ensure_worker(&mut self) -> Result<(), ImportQueueError> {
        if self.workers >= self.max_workers {
            return Ok(());
        }
        let rx = self.rx.clone();
        let name = format!("resonance-clip-import-{}", self.workers + 1);
        match std::thread::Builder::new().name(name).spawn(move || {
            // Ends when every `Sender` is gone (engine shutdown), after
            // the remaining queued jobs have been drained. A panicking
            // job must NOT end it: `workers` is never decremented, so a
            // worker lost to an unwind would permanently shrink the
            // pool and, after `max_workers` panics, strand every later
            // import in the queue. Contain the panic and keep serving.
            while let Ok(job) = rx.recv() {
                crate::supervise::run_supervised("clip-import", job, |message| {
                    tracing::error!("clip-import: {message}");
                });
            }
        }) {
            Ok(_handle) => {
                self.workers += 1;
                Ok(())
            }
            Err(e) if self.workers == 0 => Err(e.into()),
            // At least one worker is already running: it will pick the
            // job up, so this spawn failure costs throughput, not work.
            Err(_) => Ok(()),
        }
    }
}

impl Default for ImportQueue {
    fn default() -> Self {
        Self::new(MAX_CONCURRENT_IMPORTS)
    }
}
