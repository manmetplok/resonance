//! Shared visualization state between the audio thread and the editor.
//!
//! Every cell is bit-punned atomic — instantaneous input/output/GR
//! values are scalar `AtomicU32`s, and the rolling gain-reduction
//! history is a fixed-length array of atomic samples plus an atomic
//! write index. The audio thread never blocks; the UI reader can
//! tolerate the (very rare) one-sample straddle at frame boundaries
//! since this is purely a viz trace.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

/// Number of samples kept in the GR history ring buffer.
pub const HISTORY_LEN: usize = 256;

/// How often the audio thread pushes a new history sample. One push every
/// `HISTORY_STEP_SAMPLES` at the runtime sample rate; the editor reads and
/// interpolates in its own time. At 48 kHz and 256 samples/step the ring
/// covers ~1.4 seconds at 187 Hz temporal resolution, which looks smooth at
/// 60 FPS.
pub const HISTORY_STEP_SAMPLES: u32 = 256;

pub struct CompressorViz {
    /// Most recent input peak in dBFS (`-inf` when silent).
    pub input_db: AtomicU32,
    /// Most recent output peak in dBFS.
    pub output_db: AtomicU32,
    /// Most recent gain reduction in dB (positive = reducing).
    pub gr_db: AtomicU32,
    /// Rolling history of GR samples, newest at `write_pos`.
    pub history: GrHistory,
}

pub struct GrHistory {
    samples: [AtomicU32; HISTORY_LEN],
    write_pos: AtomicUsize,
}

impl GrHistory {
    fn new() -> Self {
        Self {
            samples: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            write_pos: AtomicUsize::new(0),
        }
    }

    /// Push one sample. Called from the audio thread once per
    /// `HISTORY_STEP_SAMPLES`; wait-free.
    fn push(&self, v: f32) {
        let pos = self.write_pos.load(Ordering::Relaxed);
        self.samples[pos].store(v.to_bits(), Ordering::Relaxed);
        // Release so the consumer's Acquire on write_pos observes the sample store.
        self.write_pos
            .store((pos + 1) % HISTORY_LEN, Ordering::Release);
    }

    /// Iterate the ring in chronological order (oldest first).
    pub fn iter_chrono(&self) -> impl Iterator<Item = f32> + '_ {
        let start = self.write_pos.load(Ordering::Acquire);
        (0..HISTORY_LEN).map(move |i| {
            f32::from_bits(self.samples[(start + i) % HISTORY_LEN].load(Ordering::Relaxed))
        })
    }
}

impl CompressorViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            input_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            output_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            gr_db: AtomicU32::new(0.0f32.to_bits()),
            history: GrHistory::new(),
        })
    }

    pub fn store_levels(&self, input_db: f32, output_db: f32, gr_db: f32) {
        self.input_db.store(input_db.to_bits(), Ordering::Relaxed);
        self.output_db.store(output_db.to_bits(), Ordering::Relaxed);
        self.gr_db.store(gr_db.to_bits(), Ordering::Relaxed);
    }

    pub fn push_gr(&self, gr_db: f32) {
        self.history.push(gr_db);
    }

    pub fn read_input_db(&self) -> f32 {
        f32::from_bits(self.input_db.load(Ordering::Relaxed))
    }

    pub fn read_output_db(&self) -> f32 {
        f32::from_bits(self.output_db.load(Ordering::Relaxed))
    }

    pub fn read_gr_db(&self) -> f32 {
        f32::from_bits(self.gr_db.load(Ordering::Relaxed))
    }
}
