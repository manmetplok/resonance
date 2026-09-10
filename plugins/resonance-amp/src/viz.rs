//! Lock-free visualisation state shared between the audio thread and
//! the editor thread.
//!
//! Every cell delegates to the shared bit-punned atomics in
//! `resonance-metering` — cheap, wait-free, no tearing for a single
//! scalar or L/R pair. The rolling scope history used to be a
//! `parking_lot::Mutex`-guarded ring the audio thread only entered via
//! `try_lock` (dropping a whole block's worth of trace under editor
//! contention); it is now two of the shared [`AtomicHistoryRing`]s, so the
//! audio thread never takes a lock at all. The static transfer curve stays
//! a plain `Mutex<Option<...>>`: it is written once per model load from
//! the *loader* thread, never the audio thread, so there is no RT-safety
//! reason to touch it.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use parking_lot::Mutex;
use resonance_metering::{AtomicF32, AtomicF32Pair, AtomicHistoryRing};

/// Rolling scope history length in samples. At 48 kHz this covers about
/// 43 ms — enough to draw several full periods of a low-E guitar note
/// (82 Hz → ~12 ms per cycle). The audio thread pushes one entry per
/// processed sample; the view reads the whole buffer once per frame.
pub const SCOPE_LEN: usize = 2048;

/// Resolution of the static transfer curve (input-amplitude samples).
pub const CURVE_POINTS: usize = 128;

/// Two rolling buffers of RMS-per-block values — one for the pre-gain
/// input and one for the post-model output — so the scope view can
/// draw a dual-trace oscilloscope.
///
/// Backed by two [`AtomicHistoryRing`]s rather than one struct-of-arrays
/// ring: the shared type pushes one f32 at a time, and the two channels
/// are always pushed together in lockstep (see [`Self::push_slice`]), so
/// their cursors never drift apart and [`Self::iter_chrono`] still yields
/// synchronized (input, output) pairs.
pub struct ScopeHistory {
    input: AtomicHistoryRing<SCOPE_LEN>,
    output: AtomicHistoryRing<SCOPE_LEN>,
}

impl ScopeHistory {
    fn new() -> Self {
        Self {
            input: AtomicHistoryRing::new(0.0),
            output: AtomicHistoryRing::new(0.0),
        }
    }

    /// Push a block's worth of paired dry/wet samples. Wait-free and
    /// allocation-free — safe to call from the audio thread every block,
    /// unconditionally (no `try_lock` to fall back on, because there is no
    /// lock).
    pub fn push_slice(&self, input: &[f32], output: &[f32]) {
        debug_assert_eq!(input.len(), output.len());
        for (&i, &o) in input.iter().zip(output.iter()) {
            self.input.push(i);
            self.output.push(o);
        }
    }

    /// Iterate the ring in chronological order (oldest first). Wait-free
    /// consumer; a read pass may straddle one producer push per channel,
    /// which a scope trace tolerates.
    pub fn iter_chrono(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.input.iter_chrono().zip(self.output.iter_chrono())
    }
}

pub struct AmpViz {
    // Peak meters (dBFS).
    in_db: AtomicF32Pair,
    out_db: AtomicF32Pair,

    // Tuner state.
    /// Detected pitch in Hz; 0.0 means "no pitch".
    tuner_hz: AtomicF32,
    /// 0.0..1.0 confidence.
    tuner_confidence: AtomicF32,

    /// Native sample rate of the loaded NAM model in Hz; 0.0 = unknown.
    model_sample_rate: AtomicF32,
    /// Engine sample rate in Hz (set at `initialize`); 0.0 = unknown.
    engine_sample_rate: AtomicF32,

    /// Rolling per-block scope history (both traces). Lock-free — see
    /// [`ScopeHistory`].
    pub scope: ScopeHistory,

    /// Static nonlinear transfer curve: `y = model(x)` sampled on
    /// `x ∈ [-1, 1]`. Recomputed on the loader thread each time a new
    /// model is installed; `None` until the first model has loaded.
    pub transfer_curve: Mutex<Option<[f32; CURVE_POINTS]>>,
}

impl AmpViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            in_db: AtomicF32Pair::new(f32::NEG_INFINITY),
            out_db: AtomicF32Pair::new(f32::NEG_INFINITY),
            tuner_hz: AtomicF32::new(0.0),
            tuner_confidence: AtomicF32::new(0.0),
            model_sample_rate: AtomicF32::new(0.0),
            engine_sample_rate: AtomicF32::new(0.0),
            scope: ScopeHistory::new(),
            transfer_curve: Mutex::new(None),
        })
    }

    pub fn store_peaks(&self, in_l_db: f32, in_r_db: f32, out_l_db: f32, out_r_db: f32) {
        self.in_db.store(in_l_db, in_r_db);
        self.out_db.store(out_l_db, out_r_db);
    }

    pub fn read_in_peaks_db(&self) -> (f32, f32) {
        self.in_db.load()
    }

    pub fn read_out_peaks_db(&self) -> (f32, f32) {
        self.out_db.load()
    }

    pub fn store_tuner(&self, hz: f32, confidence: f32) {
        self.tuner_hz.store(hz, Ordering::Relaxed);
        self.tuner_confidence.store(confidence, Ordering::Relaxed);
    }

    /// Clear the tuner (used when no model is loaded or the model is
    /// mid-swap — nothing to show).
    pub fn clear_tuner(&self) {
        self.tuner_hz.store(0.0, Ordering::Relaxed);
        self.tuner_confidence.store(0.0, Ordering::Relaxed);
    }

    pub fn read_tuner(&self) -> (f32, f32) {
        (
            self.tuner_hz.load(Ordering::Relaxed),
            self.tuner_confidence.load(Ordering::Relaxed),
        )
    }

    pub fn store_model_sample_rate(&self, hz: f32) {
        self.model_sample_rate.store(hz, Ordering::Relaxed);
    }

    pub fn store_engine_sample_rate(&self, hz: f32) {
        self.engine_sample_rate.store(hz, Ordering::Relaxed);
    }

    /// `(model_hz, engine_hz)`; 0.0 means unknown/not set yet.
    pub fn read_sample_rates(&self) -> (f32, f32) {
        (
            self.model_sample_rate.load(Ordering::Relaxed),
            self.engine_sample_rate.load(Ordering::Relaxed),
        )
    }

    pub fn store_transfer_curve(&self, curve: [f32; CURVE_POINTS]) {
        *self.transfer_curve.lock() = Some(curve);
    }

    /// Snapshot the current transfer curve, if any, into a fresh array.
    /// The editor calls this once per frame.
    pub fn snapshot_transfer_curve(&self) -> Option<[f32; CURVE_POINTS]> {
        *self.transfer_curve.lock()
    }
}
