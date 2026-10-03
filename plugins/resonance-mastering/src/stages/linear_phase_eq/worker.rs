//! Off-audio-thread FIR design for the linear-phase filters (FU-M2a).
//!
//! A band change used to design the FIR (per-bin biquad magnitudes, an
//! IFFT, the window) and FFT it for the convolver inside `process`. Now
//! the audio thread posts the new bands to a [`DesignWorker`] through a
//! lock-free request slot and keeps streaming; the worker designs the
//! filter's *spectrum* and parks it in one of two result slots (double
//! buffered, so a write never waits on a read).
//!
//! Determinism: the moment a new filter takes effect never depends on
//! the worker. [`StereoFir`] stages the change on the first hop boundary
//! at least [`StereoFir::min_lead`] samples (half a hop) after the
//! request — a pure function of the sample count, so it is the same live
//! and offline. If the worker's spectrum for those bands is ready by then
//! it is copied in; otherwise the same design runs inline, on an
//! identically built designer, so the output is bit-identical either way
//! and an offline bounce renders the same as a live pass.
//!
//! The lead is what keeps the inline design rare (DSP2-15): landing on
//! the very next boundary gave the worker anything from one sample to a
//! hop, so a request made just before a boundary was always designed on
//! the audio thread (~4097 bins × bands × 2 `sin` plus two 8192-point
//! FFTs at 48 kHz, more at higher rates). Half a hop (≈ 43 ms at any
//! rate) is far more than a design takes, so the inline path now only
//! runs when the worker thread was starved of CPU for that long. It is
//! kept as the last resort rather than deferring again, because a
//! deferral would make the landing time depend on thread scheduling.
//!
//! Real-time safety: the audio thread only copies into / out of slots
//! it claims with a single compare-and-swap (it never waits: a slot the
//! other side holds is simply skipped, which at worst forces the inline
//! fallback) and wakes the worker with `Thread::unpark`. No allocation,
//! no lock. The worker's client list sits behind a mutex that only the
//! worker and construction (off the audio thread) touch.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{JoinHandle, Thread};
use std::time::Duration;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use super::band::BandConfig;
use super::convolver::{FirGeometry, OverlapSaveConvolver};
use super::design::{FirDesigner, FirPart};
use super::NUM_BANDS;

/// Designs a filter all the way to the convolver's frequency-domain
/// partition: [`FirDesigner`]'s symmetric FIR, zero-padded and FFT'd.
pub struct SpectrumDesigner {
    designer: FirDesigner,
    fft: Arc<dyn Fft<f32> + Send + Sync>,
    fft_scratch: Vec<Complex<f32>>,
    spectrum: Vec<Complex<f32>>,
}

impl SpectrumDesigner {
    pub fn new(geometry: FirGeometry) -> Self {
        Self::with_part(geometry, FirPart::Direct)
    }

    /// A designer for one of the two M/S filters (see [`FirPart`]).
    pub fn with_part(geometry: FirGeometry, part: FirPart) -> Self {
        let fft = FftPlanner::<f32>::new().plan_fft_forward(geometry.fft_size);
        Self {
            designer: FirDesigner::with_part(geometry, part),
            fft_scratch: vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()],
            fft,
            spectrum: vec![Complex::new(0.0, 0.0); geometry.fft_size],
        }
    }

    /// Spectrum of the FIR for `bands`. Allocation-free.
    pub fn design(&mut self, bands: &[BandConfig], sample_rate: f32) -> &[Complex<f32>] {
        let h = self.designer.design(bands, sample_rate);
        for (i, c) in self.spectrum.iter_mut().enumerate() {
            *c = Complex::new(h.get(i).copied().unwrap_or(0.0), 0.0);
        }
        self.fft
            .process_with_scratch(&mut self.spectrum, &mut self.fft_scratch);
        &self.spectrum
    }
}

const FREE: u8 = 0;
const BUSY: u8 = 1;
const POSTED: u8 = 2;

/// Single-value mailbox claimed by compare-and-swap: whichever side
/// flips the state to `BUSY` owns the value until it releases it. Neither
/// side ever waits — a failed claim just returns.
struct Slot<T> {
    state: AtomicU8,
    value: UnsafeCell<T>,
}

// SAFETY: `value` is only accessed by the side that won the CAS into
// `BUSY` (Acquire), and released with a Release store, so accesses never
// overlap and are properly ordered.
unsafe impl<T: Send> Sync for Slot<T> {}

impl<T> Slot<T> {
    fn new(value: T) -> Self {
        Self {
            state: AtomicU8::new(FREE),
            value: UnsafeCell::new(value),
        }
    }

    /// Overwrite the slot (free or holding an unread value) and post it.
    fn try_post(&self, write: impl FnOnce(&mut T)) -> bool {
        let cur = self.state.load(Ordering::Relaxed);
        if cur == BUSY
            || self
                .state
                .compare_exchange(cur, BUSY, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
        {
            return false;
        }
        // SAFETY: we hold the slot (see `impl Sync`).
        write(unsafe { &mut *self.value.get() });
        self.state.store(POSTED, Ordering::Release);
        true
    }

    /// Consume a posted value.
    fn try_take<R>(&self, read: impl FnOnce(&T) -> R) -> Option<R> {
        self.state
            .compare_exchange(POSTED, BUSY, Ordering::Acquire, Ordering::Relaxed)
            .ok()?;
        // SAFETY: we hold the slot (see `impl Sync`).
        let r = read(unsafe { &*self.value.get() });
        self.state.store(FREE, Ordering::Release);
        Some(r)
    }
}

#[derive(Clone, Copy)]
struct Request {
    generation: u64,
    bands: [BandConfig; NUM_BANDS],
}

struct Designed {
    generation: u64,
    spectrum: Vec<Complex<f32>>,
}

/// One filter's side of the worker: its request slot, two result slots,
/// and the worker-owned designer.
struct ClientShared {
    sample_rate: f32,
    request: Slot<Request>,
    results: [Slot<Designed>; 2],
    /// Result slot the worker tries first (alternates).
    next_result: AtomicUsize,
    /// Only ever locked by the worker thread.
    designer: Mutex<SpectrumDesigner>,
}

impl ClientShared {
    /// Worker side: design a posted request, if any. True if it did.
    fn serve(&self) -> bool {
        let Some(req) = self.request.try_take(|r| *r) else {
            return false;
        };
        let mut designer = self.designer.lock().unwrap_or_else(|e| e.into_inner());
        let spectrum = designer.design(&req.bands, self.sample_rate);
        let first = self.next_result.load(Ordering::Relaxed);
        for k in 0..2 {
            let i = (first + k) % 2;
            let posted = self.results[i].try_post(|d| {
                d.generation = req.generation;
                d.spectrum.copy_from_slice(spectrum);
            });
            if posted {
                self.next_result.store(1 - i, Ordering::Relaxed);
                break;
            }
        }
        true
    }
}

struct WorkerInner {
    clients: Mutex<Vec<Arc<ClientShared>>>,
    shutdown: AtomicBool,
}

/// Background FIR designer shared by any number of filters (the
/// mastering chain runs one for its two EQs and three crossovers).
/// Dropping the last handle stops and joins the thread, so drop it off
/// the audio thread — the plugin builds and drops its chain in
/// `initialize`/teardown.
pub struct DesignWorker {
    inner: Arc<WorkerInner>,
    thread: Option<JoinHandle<()>>,
}

impl DesignWorker {
    pub fn spawn() -> Arc<Self> {
        let inner = Arc::new(WorkerInner {
            clients: Mutex::new(Vec::new()),
            shutdown: AtomicBool::new(false),
        });
        let worker_inner = Arc::clone(&inner);
        let thread = std::thread::Builder::new()
            .name("mastering-fir-design".into())
            .spawn(move || run(&worker_inner))
            .expect("spawn mastering FIR design worker");
        Arc::new(Self {
            inner,
            thread: Some(thread),
        })
    }

    fn register(
        self: &Arc<Self>,
        geometry: FirGeometry,
        sample_rate: f32,
        part: FirPart,
    ) -> DesignClient {
        let shared = Arc::new(ClientShared {
            sample_rate,
            request: Slot::new(Request {
                generation: 0,
                bands: [BandConfig::off(); NUM_BANDS],
            }),
            results: std::array::from_fn(|_| {
                Slot::new(Designed {
                    generation: 0,
                    spectrum: vec![Complex::new(0.0, 0.0); geometry.fft_size],
                })
            }),
            next_result: AtomicUsize::new(0),
            designer: Mutex::new(SpectrumDesigner::with_part(geometry, part)),
        });
        self.inner
            .clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::clone(&shared));
        let waker = self
            .thread
            .as_ref()
            .map(|t| t.thread().clone())
            .expect("worker thread");
        DesignClient {
            shared,
            waker,
            _worker: Arc::clone(self),
        }
    }
}

impl Drop for DesignWorker {
    fn drop(&mut self) {
        self.inner.shutdown.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

fn run(inner: &WorkerInner) {
    while !inner.shutdown.load(Ordering::Acquire) {
        let mut served = false;
        {
            let clients = inner.clients.lock().unwrap_or_else(|e| e.into_inner());
            for c in clients.iter() {
                served |= c.serve();
            }
        }
        if !served {
            // Woken by `unpark` on every request; the timeout only
            // bounds a missed wake-up.
            std::thread::park_timeout(Duration::from_millis(50));
        }
    }
}

/// A filter's handle on the worker (audio-thread side).
struct DesignClient {
    shared: Arc<ClientShared>,
    waker: Thread,
    /// Keeps the worker alive as long as a client exists.
    _worker: Arc<DesignWorker>,
}

impl DesignClient {
    fn post(&self, req: Request) {
        if self.shared.request.try_post(|r| *r = req) {
            self.waker.unpark();
        }
    }

    /// Hand the designed spectrum for `generation` to `apply`, if the
    /// worker has it ready. Stale results met on the way are discarded.
    fn take(&self, generation: u64, mut apply: impl FnMut(&[Complex<f32>])) -> bool {
        for slot in &self.shared.results {
            let hit = slot.try_take(|d| {
                if d.generation == generation {
                    apply(&d.spectrum);
                    true
                } else {
                    false
                }
            });
            if hit == Some(true) {
                return true;
            }
        }
        false
    }
}

/// A stereo pair of linear-phase FIR convolvers plus the plumbing that
/// designs their filter off the audio thread (see the module docs).
///
/// Changes are rate-limited to one per hop: [`Self::request`] refuses a
/// new design while one is waiting for its hop boundary.
pub struct StereoFir {
    sample_rate: f32,
    left: OverlapSaveConvolver,
    right: OverlapSaveConvolver,
    /// Audio-thread designer for the fallback (and construction).
    inline: SpectrumDesigner,
    client: Option<DesignClient>,
    generation: u64,
    /// Requested design waiting for its hop boundary.
    pending: Option<Request>,
    /// Samples the pending design must still wait before it may land.
    lead_left: usize,
    worker_designs: u64,
    inline_designs: u64,
}

impl StereoFir {
    /// A pair for `sample_rate`'s geometry, designed through `worker`
    /// (`None`: always inline), initially a pure delay.
    pub fn new(sample_rate: f32, worker: Option<&Arc<DesignWorker>>) -> Self {
        Self::with_part(sample_rate, worker, FirPart::Direct)
    }

    /// A pair designing one of the two M/S filters (see [`FirPart`]).
    /// Initially a pure delay, like [`Self::new`].
    pub fn with_part(
        sample_rate: f32,
        worker: Option<&Arc<DesignWorker>>,
        part: FirPart,
    ) -> Self {
        let geometry = FirGeometry::for_sample_rate(sample_rate);
        Self {
            sample_rate,
            left: OverlapSaveConvolver::with_geometry(geometry),
            right: OverlapSaveConvolver::with_geometry(geometry),
            inline: SpectrumDesigner::with_part(geometry, part),
            client: worker.map(|w| w.register(geometry, sample_rate, part)),
            generation: 0,
            pending: None,
            lead_left: 0,
            worker_designs: 0,
            inline_designs: 0,
        }
    }

    pub fn geometry(&self) -> FirGeometry {
        self.left.geometry()
    }

    /// Per-channel latency in samples.
    pub fn latency(&self) -> usize {
        self.left.latency()
    }

    /// The least time, in samples, between a [`Self::request`] and the
    /// hop boundary its design lands on: half a hop. Gives the worker
    /// that long to deliver before the inline fallback would run.
    pub fn min_lead(&self) -> usize {
        self.left.geometry().hop / 2
    }

    /// Clear the streaming state; keeps the filter. A pending design
    /// still lands on the next hop boundary.
    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }

    /// Set the filter for `bands` right now, designed inline: instant
    /// before any audio has run (construction), otherwise crossfaded in
    /// on the next iteration. Drops a pending request.
    pub fn design_now(&mut self, bands: &[BandConfig; NUM_BANDS]) {
        self.pending = None;
        let spectrum = self.inline.design(bands, self.sample_rate);
        self.left.crossfade_to_spectrum(spectrum);
        self.right.crossfade_to_spectrum(spectrum);
    }

    /// Stagger the two channels' FFT iterations by these offsets (DSP-16).
    /// Resets the streaming state, so set it before processing.
    pub fn set_phase_offsets(&mut self, [left, right]: [usize; 2]) {
        self.left.set_phase_offset(left);
        self.right.set_phase_offset(right);
    }

    /// Samples until each channel's next FFT iteration.
    pub fn iteration_countdowns(&self) -> [usize; 2] {
        [
            self.left.samples_until_iteration(),
            self.right.samples_until_iteration(),
        ]
    }

    /// True while a requested design waits for its hop boundary.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Designs taken from the worker vs. designed inline, since
    /// construction (diagnostics).
    pub fn design_counts(&self) -> (u64, u64) {
        (self.worker_designs, self.inline_designs)
    }

    /// Ask for a filter for `bands`, to crossfade in on the first hop
    /// boundary at least [`Self::min_lead`] samples away. Returns false
    /// (and does nothing) while an earlier request is still waiting — at
    /// most one change per hop.
    pub fn request(&mut self, bands: &[BandConfig; NUM_BANDS]) -> bool {
        if self.pending.is_some() {
            return false;
        }
        self.generation += 1;
        let req = Request {
            generation: self.generation,
            bands: *bands,
        };
        if let Some(client) = &self.client {
            client.post(req);
        }
        self.pending = Some(req);
        self.lead_left = self.min_lead();
        true
    }

    /// Process one stereo block in place, landing a pending design on
    /// the first hop boundary it reaches once its lead has run out.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let mut pos = 0;
        while pos < n {
            if self.pending.is_none() {
                self.left.process_in_place(&mut left[pos..n]);
                self.right.process_in_place(&mut right[pos..n]);
                return;
            }
            // Stop just short of the push that runs the next iteration.
            let until = self
                .left
                .samples_until_iteration()
                .min(self.right.samples_until_iteration());
            let pre = (until - 1).min(n - pos);
            self.left.process_in_place(&mut left[pos..pos + pre]);
            self.right.process_in_place(&mut right[pos..pos + pre]);
            pos += pre;
            self.lead_left = self.lead_left.saturating_sub(pre);
            if pos == n {
                return;
            }
            if self.lead_left == 0 {
                self.land_pending();
            } else {
                // Too soon after the request: let this boundary pass
                // (the push that runs its iteration) and wait for the
                // next one.
                self.left.process_in_place(&mut left[pos..pos + 1]);
                self.right.process_in_place(&mut right[pos..pos + 1]);
                pos += 1;
                self.lead_left = self.lead_left.saturating_sub(1);
            }
        }
    }

    fn land_pending(&mut self) {
        let Some(req) = self.pending.take() else {
            return;
        };
        let Self {
            left,
            right,
            client,
            ..
        } = self;
        let from_worker = client.as_ref().is_some_and(|c| {
            c.take(req.generation, |spectrum| {
                left.crossfade_to_spectrum(spectrum);
                right.crossfade_to_spectrum(spectrum);
            })
        });
        if from_worker {
            self.worker_designs += 1;
        } else {
            let spectrum = self.inline.design(&req.bands, self.sample_rate);
            self.left.crossfade_to_spectrum(spectrum);
            self.right.crossfade_to_spectrum(spectrum);
            self.inline_designs += 1;
        }
    }
}
