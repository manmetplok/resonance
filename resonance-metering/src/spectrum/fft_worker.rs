//! Background-thread FFT worker.
//!
//! Owns the `rustfft` plan and its scratch buffers, drains the SPSC ring
//! the audio thread writes to, runs a Hann-windowed FFT with 50 % overlap,
//! and publishes a 1/6-octave snapshot via [`arc_swap::ArcSwap`]. Runs
//! until its `done` flag is set and the parent thread joins it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use super::octave::{OctaveTable, NUM_OCTAVE_BINS};
use super::ring::SpscRing;
use super::{SpectrumSnapshot, FFT_SIZE, HOP_SIZE};

/// Floor value used for both the raw magnitude and the octave-smoothed
/// output when the signal is silent.
pub const FLOOR_DB: f32 = -96.0;
/// Peak-hold decay rate. Matches the EQ's existing analyzer feel.
const DECAY_DB_PER_SEC: f32 = 26.0;

pub struct FftWorker {
    sample_rate: f32,
    ring: Arc<SpscRing>,
    snapshot: Arc<ArcSwap<SpectrumSnapshot>>,
    done: Arc<AtomicBool>,

    fft: Arc<dyn Fft<f32> + Send + Sync>,
    window: Vec<f32>,
    /// Rolling history — always holds the last FFT_SIZE samples after a
    /// run completes, so a 50 % overlap FFT is cheap.
    history: Vec<f32>,
    /// Number of new samples since the last FFT.
    samples_since_fft: usize,

    /// Scratch buffers for the FFT (reused across runs to avoid alloc).
    complex_scratch: Vec<Complex<f32>>,
    mag_db: Vec<f32>,

    /// Peak-hold-with-decay buffer at 1/6-octave resolution.
    held_db: [f32; NUM_OCTAVE_BINS],
    octave_table: OctaveTable,
}

impl FftWorker {
    pub fn new(
        sample_rate: f32,
        ring: Arc<SpscRing>,
        snapshot: Arc<ArcSwap<SpectrumSnapshot>>,
        done: Arc<AtomicBool>,
    ) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        Self {
            sample_rate,
            ring,
            snapshot,
            done,
            fft,
            window: hann_window(FFT_SIZE),
            history: vec![0.0; FFT_SIZE],
            samples_since_fft: 0,
            complex_scratch: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            mag_db: vec![FLOOR_DB; FFT_SIZE / 2],
            held_db: [FLOOR_DB; NUM_OCTAVE_BINS],
            octave_table: OctaveTable::new(),
        }
    }

    /// Worker entry point — loops until `done` is set.
    pub fn run(mut self) {
        let mut drain_scratch = vec![0.0_f32; HOP_SIZE];
        while !self.done.load(Ordering::Acquire) {
            let did_work = self.try_process_one(&mut drain_scratch);
            if !did_work {
                // Nothing to do yet — yield.
                std::thread::park_timeout(Duration::from_millis(16));
            }
        }
    }

    /// Try to drain and run one FFT frame. Returns true if work was done.
    fn try_process_one(&mut self, drain: &mut [f32]) -> bool {
        // Pull samples from the ring. Only try for chunks up to HOP_SIZE
        // at a time so large bursts still yield between FFTs.
        let n = self.ring.pop_into(drain);
        if n == 0 {
            return false;
        }
        // Shift history left by `n` and append new samples at the tail.
        // Because n <= HOP_SIZE << FFT_SIZE, a plain copy_within is cheap.
        if n < FFT_SIZE {
            self.history.copy_within(n..FFT_SIZE, 0);
            self.history[FFT_SIZE - n..FFT_SIZE].copy_from_slice(&drain[..n]);
        } else {
            // Rare: the consumer fell far behind. Just take the tail.
            let src_start = n - FFT_SIZE;
            self.history
                .copy_from_slice(&drain[src_start..src_start + FFT_SIZE]);
        }
        self.samples_since_fft += n;

        // Run one FFT per HOP_SIZE new samples (50 % overlap for FFT_SIZE
        // = 2 * HOP_SIZE).
        let mut ran = false;
        while self.samples_since_fft >= HOP_SIZE {
            self.samples_since_fft -= HOP_SIZE;
            self.run_fft();
            ran = true;
        }
        ran
    }

    fn run_fft(&mut self) {
        // Apply Hann window and copy into the complex scratch.
        for i in 0..FFT_SIZE {
            let w = self.history[i] * self.window[i];
            self.complex_scratch[i] = Complex::new(w, 0.0);
        }
        self.fft.process(&mut self.complex_scratch);

        // Single-sided magnitude in dB. Hann coherent gain correction:
        // window_sum ≈ FFT_SIZE / 2 → amplitude = 2*|X|/win_sum = 4*|X|/N.
        let norm = 4.0 / FFT_SIZE as f32;
        let half = FFT_SIZE / 2;
        for k in 0..half {
            let re = self.complex_scratch[k].re;
            let im = self.complex_scratch[k].im;
            let mag = (re * re + im * im).sqrt() * norm;
            self.mag_db[k] = 20.0 * mag.max(1e-10).log10();
        }

        // Aggregate to 1/6-octave bands, then apply peak-hold with decay.
        let frames_per_sec = self.sample_rate / HOP_SIZE as f32;
        let decay_per_frame = DECAY_DB_PER_SEC / frames_per_sec.max(1.0);

        let mut new_bands = [FLOOR_DB; NUM_OCTAVE_BINS];
        self.octave_table
            .aggregate(&self.mag_db, self.sample_rate, &mut new_bands, FLOOR_DB);
        for (i, held) in self.held_db.iter_mut().enumerate().take(NUM_OCTAVE_BINS) {
            let decayed = (*held - decay_per_frame).max(FLOOR_DB);
            *held = decayed.max(new_bands[i]);
        }

        // Publish. `magnitudes_db` is a fixed-size array (Copy), so
        // the `SpectrumSnapshot` is built inline and the only heap
        // allocation per frame is the `Arc` itself — no separate
        // `Vec` backing buffer to allocate as we did before.
        self.snapshot.store(Arc::new(SpectrumSnapshot {
            magnitudes_db: self.held_db,
            sample_rate: self.sample_rate,
        }));
    }
}

fn hann_window(len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| {
            let x = i as f32 / (len as f32 - 1.0);
            0.5 - 0.5 * (std::f32::consts::TAU * x).cos()
        })
        .collect()
}
