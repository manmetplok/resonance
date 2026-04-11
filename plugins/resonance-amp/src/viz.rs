//! Lock-free visualisation state shared between the audio thread and
//! the editor thread. Mirrors the reverb plugin's `viz.rs` patterns:
//! per-scalar atomics for cheap/wait-free reads, and a short
//! `parking_lot::Mutex` around fixed-size ring buffers for trace data.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

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
pub struct ScopeHistory {
    pub input: [f32; SCOPE_LEN],
    pub output: [f32; SCOPE_LEN],
    pub write_pos: usize,
}

impl ScopeHistory {
    fn new() -> Self {
        Self {
            input: [0.0; SCOPE_LEN],
            output: [0.0; SCOPE_LEN],
            write_pos: 0,
        }
    }

    pub fn push(&mut self, input: f32, output: f32) {
        self.input[self.write_pos] = input;
        self.output[self.write_pos] = output;
        self.write_pos = (self.write_pos + 1) % SCOPE_LEN;
    }

    /// Block-wise push: write two source slices (equal length) into the
    /// ring, handling wrap once via two `copy_from_slice` calls. Much
    /// cheaper than calling `push` per sample because the audio thread
    /// only holds the mutex for the duration of at most two memcpys.
    pub fn push_slice(&mut self, input: &[f32], output: &[f32]) {
        debug_assert_eq!(input.len(), output.len());
        let mut n = input.len();
        // If the caller hands us more than SCOPE_LEN in one shot, only
        // the tail is retained — the earlier samples would be overwritten
        // anyway on the next wrap.
        let (mut src_in, mut src_out) = if n > SCOPE_LEN {
            let skip = n - SCOPE_LEN;
            n = SCOPE_LEN;
            (&input[skip..], &output[skip..])
        } else {
            (input, output)
        };

        let mut pos = self.write_pos;
        let first = (SCOPE_LEN - pos).min(n);
        self.input[pos..pos + first].copy_from_slice(&src_in[..first]);
        self.output[pos..pos + first].copy_from_slice(&src_out[..first]);
        pos = (pos + first) & (SCOPE_LEN - 1);
        src_in = &src_in[first..];
        src_out = &src_out[first..];

        let remaining = n - first;
        if remaining > 0 {
            self.input[pos..pos + remaining].copy_from_slice(&src_in[..remaining]);
            self.output[pos..pos + remaining].copy_from_slice(&src_out[..remaining]);
            pos = (pos + remaining) & (SCOPE_LEN - 1);
        }
        self.write_pos = pos;
    }

    /// Iterate the ring in chronological order (oldest first).
    pub fn iter_chrono(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        let start = self.write_pos;
        (0..SCOPE_LEN).map(move |i| {
            let idx = (start + i) % SCOPE_LEN;
            (self.input[idx], self.output[idx])
        })
    }
}

pub struct AmpViz {
    // Peak meters (dBFS).
    in_l_db: AtomicU32,
    in_r_db: AtomicU32,
    out_l_db: AtomicU32,
    out_r_db: AtomicU32,

    // Tuner state.
    /// Detected pitch in Hz; 0.0 means "no pitch".
    tuner_hz: AtomicU32,
    /// 0.0..1.0 confidence.
    tuner_confidence: AtomicU32,

    /// Rolling per-block scope history (both traces).
    pub scope: Mutex<ScopeHistory>,

    /// Static nonlinear transfer curve: `y = model(x)` sampled on
    /// `x ∈ [-1, 1]`. Recomputed on the loader thread each time a new
    /// model is installed; `None` until the first model has loaded.
    pub transfer_curve: Mutex<Option<[f32; CURVE_POINTS]>>,
}

impl AmpViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            in_l_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            in_r_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            out_l_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            out_r_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            tuner_hz: AtomicU32::new(0.0f32.to_bits()),
            tuner_confidence: AtomicU32::new(0.0f32.to_bits()),
            scope: Mutex::new(ScopeHistory::new()),
            transfer_curve: Mutex::new(None),
        })
    }

    pub fn store_peaks(&self, in_l_db: f32, in_r_db: f32, out_l_db: f32, out_r_db: f32) {
        self.in_l_db.store(in_l_db.to_bits(), Ordering::Relaxed);
        self.in_r_db.store(in_r_db.to_bits(), Ordering::Relaxed);
        self.out_l_db.store(out_l_db.to_bits(), Ordering::Relaxed);
        self.out_r_db.store(out_r_db.to_bits(), Ordering::Relaxed);
    }

    pub fn read_in_peaks_db(&self) -> (f32, f32) {
        (
            f32::from_bits(self.in_l_db.load(Ordering::Relaxed)),
            f32::from_bits(self.in_r_db.load(Ordering::Relaxed)),
        )
    }

    pub fn read_out_peaks_db(&self) -> (f32, f32) {
        (
            f32::from_bits(self.out_l_db.load(Ordering::Relaxed)),
            f32::from_bits(self.out_r_db.load(Ordering::Relaxed)),
        )
    }

    pub fn store_tuner(&self, hz: f32, confidence: f32) {
        self.tuner_hz.store(hz.to_bits(), Ordering::Relaxed);
        self.tuner_confidence
            .store(confidence.to_bits(), Ordering::Relaxed);
    }

    /// Clear the tuner (used when no model is loaded or the model is
    /// mid-swap — nothing to show).
    pub fn clear_tuner(&self) {
        self.tuner_hz.store(0.0f32.to_bits(), Ordering::Relaxed);
        self.tuner_confidence
            .store(0.0f32.to_bits(), Ordering::Relaxed);
    }

    pub fn read_tuner(&self) -> (f32, f32) {
        (
            f32::from_bits(self.tuner_hz.load(Ordering::Relaxed)),
            f32::from_bits(self.tuner_confidence.load(Ordering::Relaxed)),
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
