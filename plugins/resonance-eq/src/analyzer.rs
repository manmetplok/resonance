//! Live spectrum analyzer used behind the EQ response curve.
//!
//! Two independent analyzer channels are fed by the audio thread: one taps
//! the input block (pre-EQ) and the other taps the output block (post-EQ).
//! Each channel accumulates samples into a 2048-wide ring buffer and runs
//! a Hann-windowed FFT every 1024 samples (50% overlap, ~46 updates/sec at
//! 48 kHz). Each FFT emits a magnitude-in-dB vector that is merged into a
//! peak-hold-with-decay curve and published through a `parking_lot::Mutex`
//! into the shared state the editor reads at ~60 Hz.
//!
//! The math is deliberately kept per-FFT cheap (one complex FFT + one
//! magnitude+smoothing pass over NUM_BINS). The Mutex hold time is
//! micro-seconds — this is fine on the audio thread because the editor
//! side only locks on its own repaint tick and neither path does any I/O.

use std::sync::Arc;

use parking_lot::Mutex;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

pub const FFT_SIZE: usize = 2048;
pub const HOP_SIZE: usize = 1024;
pub const NUM_BINS: usize = FFT_SIZE / 2;

/// Floor value for smoothed bin magnitudes. Anything quieter is pinned here
/// so the display doesn't dip below the plot area.
pub const FLOOR_DB: f32 = -96.0;

/// Peak-hold decay rate in dB per second. Roughly Pro-Q 3 feel — a snappy
/// attack followed by a smooth glide downward.
const DECAY_DB_PER_SEC: f32 = 26.0;

// ---------------------------------------------------------------------------
// Shared snapshot (audio → UI).
// ---------------------------------------------------------------------------

/// One channel's published spectrum, refreshed each time the audio thread
/// finishes an FFT. Bins are linear in frequency, `bin[i]` corresponds to
/// `i * sample_rate / FFT_SIZE` Hz.
#[derive(Clone)]
pub struct SpectrumSnapshot {
    pub magnitudes_db: Vec<f32>,
    pub sample_rate: f32,
}

impl SpectrumSnapshot {
    pub fn silent(sample_rate: f32) -> Self {
        Self {
            magnitudes_db: vec![FLOOR_DB; NUM_BINS],
            sample_rate,
        }
    }
}

/// Shared between the audio thread and the editor thread. The audio thread
/// writes, the editor reads. Each channel gets its own mutex so the
/// pre-chain write doesn't block the post-chain write (or vice-versa).
pub struct AnalyzerState {
    pub pre: Mutex<SpectrumSnapshot>,
    pub post: Mutex<SpectrumSnapshot>,
}

impl AnalyzerState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            pre: Mutex::new(SpectrumSnapshot::silent(48_000.0)),
            post: Mutex::new(SpectrumSnapshot::silent(48_000.0)),
        })
    }
}

// ---------------------------------------------------------------------------
// Audio-thread FFT processor.
// ---------------------------------------------------------------------------

/// Streaming single-channel spectrum analyzer. Owned by the audio thread.
pub struct AnalyzerChannel {
    /// Ring buffer of the most recent FFT_SIZE mono samples.
    ring: Vec<f32>,
    write_pos: usize,
    /// Number of new samples accumulated since the last FFT. Once this hits
    /// HOP_SIZE we run another FFT.
    samples_since_fft: usize,
    /// Hann window coefficients of length FFT_SIZE.
    window: Vec<f32>,
    /// Scratch for the FFT input/output (rustfft operates in place).
    scratch: Vec<Complex<f32>>,
    /// Held magnitudes in dB per bin with peak-and-decay smoothing.
    held_db: Vec<f32>,
    /// Decay applied per FFT frame. Computed from DECAY_DB_PER_SEC and the
    /// current sample rate.
    decay_db_per_frame: f32,
    fft: Arc<dyn Fft<f32> + Send + Sync>,
    sample_rate: f32,
}

impl AnalyzerChannel {
    pub fn new(planner: &mut FftPlanner<f32>, sample_rate: f32) -> Self {
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let window = hann_window(FFT_SIZE);
        let mut s = Self {
            ring: vec![0.0; FFT_SIZE],
            write_pos: 0,
            samples_since_fft: 0,
            window,
            scratch: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            held_db: vec![FLOOR_DB; NUM_BINS],
            decay_db_per_frame: 0.0,
            fft,
            sample_rate,
        };
        s.set_sample_rate(sample_rate);
        s
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sample_rate = sr;
        // One FFT runs per HOP_SIZE samples; convert DECAY_DB_PER_SEC into
        // per-frame decay.
        let frames_per_sec = sr / HOP_SIZE as f32;
        self.decay_db_per_frame = DECAY_DB_PER_SEC / frames_per_sec.max(1.0);
    }

    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.write_pos = 0;
        self.samples_since_fft = 0;
        self.held_db.fill(FLOOR_DB);
    }

    /// Feed a mono block of samples. Runs zero, one, or multiple FFTs
    /// depending on the block size relative to HOP_SIZE. Publishes the
    /// latest magnitudes to the given shared snapshot after each FFT.
    pub fn push(&mut self, samples: &[f32], shared: &Mutex<SpectrumSnapshot>) {
        for &s in samples {
            self.ring[self.write_pos] = s;
            self.write_pos = (self.write_pos + 1) % FFT_SIZE;
            self.samples_since_fft += 1;
            if self.samples_since_fft >= HOP_SIZE {
                self.samples_since_fft = 0;
                self.run_fft();
                self.publish(shared);
            }
        }
    }

    fn run_fft(&mut self) {
        // Copy the ring into the scratch in time order (oldest sample first)
        // while applying the Hann window.
        for i in 0..FFT_SIZE {
            let src = (self.write_pos + i) % FFT_SIZE;
            let windowed = self.ring[src] * self.window[i];
            self.scratch[i] = Complex::new(windowed, 0.0);
        }
        self.fft.process(&mut self.scratch);

        // Convert the first NUM_BINS (positive frequencies) to dB and merge
        // into the peak-hold buffer. The normalization factor accounts for
        // the Hann window coherent gain (~0.5) and the FFT length.
        //
        // Hann window sum ≈ FFT_SIZE / 2, so the single-sided amplitude is
        // `2 * |X[k]| / window_sum` = `4 * |X[k]| / FFT_SIZE`.
        let norm = 4.0 / FFT_SIZE as f32;
        for i in 0..NUM_BINS {
            let re = self.scratch[i].re;
            let im = self.scratch[i].im;
            let mag = (re * re + im * im).sqrt() * norm;
            let mag_db = 20.0 * mag.max(1e-10).log10();
            let decayed = (self.held_db[i] - self.decay_db_per_frame).max(FLOOR_DB);
            self.held_db[i] = decayed.max(mag_db);
        }
    }

    fn publish(&self, shared: &Mutex<SpectrumSnapshot>) {
        // `try_lock` so the audio thread never blocks on the editor's
        // ~60 Hz read. Skipping a single FFT publish costs at most one
        // frame of visual update (~22 ms at HOP_SIZE/48 kHz) — acceptable
        // for an analyzer overlay. Blocking the audio thread on a 1024-
        // float copy under contention is not.
        let Some(mut guard) = shared.try_lock() else {
            return;
        };
        guard.sample_rate = self.sample_rate;
        if guard.magnitudes_db.len() != NUM_BINS {
            guard.magnitudes_db.resize(NUM_BINS, FLOOR_DB);
        }
        guard.magnitudes_db.copy_from_slice(&self.held_db);
    }
}

fn hann_window(len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| {
            let x = (i as f32) / (len as f32 - 1.0);
            0.5 - 0.5 * (std::f32::consts::TAU * x).cos()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Stereo convenience wrapper (both taps together).
// ---------------------------------------------------------------------------

/// Owns the two analyzer channels (pre and post) and the mono-mix scratch
/// buffer used to feed them. Lives on the plugin struct; the plugin's
/// `process()` calls `feed_pre` before the DSP runs and `feed_post` after.
pub struct StereoAnalyzers {
    pre: AnalyzerChannel,
    post: AnalyzerChannel,
    mono_scratch: Vec<f32>,
}

impl StereoAnalyzers {
    pub fn new(sample_rate: f32, max_buffer_size: usize) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        Self {
            pre: AnalyzerChannel::new(&mut planner, sample_rate),
            post: AnalyzerChannel::new(&mut planner, sample_rate),
            mono_scratch: vec![0.0; max_buffer_size.max(HOP_SIZE)],
        }
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.pre.set_sample_rate(sr);
        self.post.set_sample_rate(sr);
    }

    pub fn reset(&mut self) {
        self.pre.reset();
        self.post.reset();
    }

    pub fn feed_pre(&mut self, left: &[f32], right: &[f32], shared: &AnalyzerState) {
        let n = Self::fill_mono(&mut self.mono_scratch, left, right);
        self.pre.push(&self.mono_scratch[..n], &shared.pre);
    }

    pub fn feed_post(&mut self, left: &[f32], right: &[f32], shared: &AnalyzerState) {
        let n = Self::fill_mono(&mut self.mono_scratch, left, right);
        self.post.push(&self.mono_scratch[..n], &shared.post);
    }

    fn fill_mono(mono_scratch: &mut [f32], left: &[f32], right: &[f32]) -> usize {
        let n = left.len().min(right.len()).min(mono_scratch.len());
        for i in 0..n {
            mono_scratch[i] = 0.5 * (left[i] + right[i]);
        }
        n
    }
}
