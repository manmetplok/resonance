//! Spectrum analyzer with a background FFT worker.
//!
//! The audio thread pushes mono samples through [`SpectrumAnalyzer::push_stereo`]
//! into a lock-free SPSC ring. A worker thread drains the ring, runs an
//! 8192-point Hann-windowed FFT with 50 % overlap, and publishes a 1/6-
//! octave [`SpectrumSnapshot`] through an [`arc_swap::ArcSwap`] that the
//! UI thread reads wait-free via [`SpectrumHandle::latest`].

pub mod fft_worker;
pub mod octave;
pub mod ring;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use arc_swap::ArcSwap;

pub use octave::NUM_OCTAVE_BINS;

/// FFT window length. 8192 samples at 48 kHz is ~170 ms — good frequency
/// resolution (~5.86 Hz/bin) without being painfully slow to update.
pub const FFT_SIZE: usize = 8192;
/// 50 % overlap.
pub const HOP_SIZE: usize = FFT_SIZE / 2;
/// SPSC ring capacity — at least 2× FFT_SIZE, plus slack for burst input.
/// Power of two.
pub const RING_CAPACITY: usize = 32_768;

/// Snapshot of the analyzer's held peak-with-decay 1/6-octave bars.
///
/// `magnitudes_db` is a fixed-size array, not a `Vec`, so each
/// `Arc::new(SpectrumSnapshot { … })` is a single heap allocation —
/// the 60×f32 band data lives inline next to the `Arc` refcount,
/// avoiding the separate `Vec` backing buffer that the previous
/// `Vec<f32>` field caused us to allocate on every published frame.
#[derive(Clone)]
pub struct SpectrumSnapshot {
    /// dB values, one per 1/6-octave band.
    pub magnitudes_db: [f32; NUM_OCTAVE_BINS],
    /// Sample rate the snapshot was computed at.
    pub sample_rate: f32,
}

impl SpectrumSnapshot {
    pub fn silent(sample_rate: f32) -> Self {
        Self {
            magnitudes_db: [fft_worker::FLOOR_DB; NUM_OCTAVE_BINS],
            sample_rate,
        }
    }
}

/// Wait-free handle the UI thread uses to read the latest snapshot.
#[derive(Clone)]
pub struct SpectrumHandle {
    snapshot: Arc<ArcSwap<SpectrumSnapshot>>,
}

impl SpectrumHandle {
    pub fn latest(&self) -> Arc<SpectrumSnapshot> {
        self.snapshot.load_full()
    }
}

/// Owns the SPSC ring producer and the background FFT worker thread.
/// Drop semantics: signals the worker to stop and joins it.
pub struct SpectrumAnalyzer {
    ring: Arc<ring::SpscRing>,
    handle: SpectrumHandle,
    done: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl SpectrumAnalyzer {
    /// Spawn the worker thread and return the analyzer.
    pub fn spawn(sample_rate: f32) -> Self {
        let ring = Arc::new(ring::SpscRing::new(RING_CAPACITY));
        let snapshot = Arc::new(ArcSwap::from(Arc::new(SpectrumSnapshot::silent(
            sample_rate,
        ))));
        let done = Arc::new(AtomicBool::new(false));
        let handle = SpectrumHandle {
            snapshot: snapshot.clone(),
        };

        let worker_ring = ring.clone();
        let worker_snapshot = snapshot.clone();
        let worker_done = done.clone();
        let worker = std::thread::Builder::new()
            .name("resonance-metering-spectrum".to_string())
            .spawn(move || {
                let w = fft_worker::FftWorker::new(
                    sample_rate,
                    worker_ring,
                    worker_snapshot,
                    worker_done,
                );
                w.run();
            })
            .ok();

        Self {
            ring,
            handle,
            done,
            worker,
        }
    }

    /// Clone a reader handle for the UI thread.
    pub fn handle(&self) -> SpectrumHandle {
        self.handle.clone()
    }

    /// Feed a stereo block. Downmixes to mono and pushes into the ring.
    ///
    /// Intentionally does **not** `unpark()` the worker — that could
    /// syscall on the audio thread. The worker polls the ring every
    /// 16 ms instead; see the latency rationale in
    /// [`fft_worker::FftWorker::run`].
    #[inline]
    pub fn push_stereo(&self, left: &[f32], right: &[f32]) {
        let n = left.len().min(right.len());
        for i in 0..n {
            let mono = 0.5 * (left[i] + right[i]);
            self.ring.push(mono);
        }
    }

    /// Clear any pending ring samples and publish a silent snapshot.
    /// Filter history inside the worker decays naturally on the next FFT.
    pub fn reset(&self) {
        self.ring.clear();
    }
}

impl Drop for SpectrumAnalyzer {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}
