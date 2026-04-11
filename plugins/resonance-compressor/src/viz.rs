//! Shared visualization state between the audio thread and the editor.
//!
//! The audio thread writes instantaneous input/output/GR values as atomic
//! floats (cheap, lock-free) and pushes the per-block gain-reduction value
//! into a small ring buffer (mutex-guarded) so the editor can render a
//! rolling history trace. The history ring is sized for ~2 seconds of
//! history at 60 Hz refresh which is plenty for visually tracking
//! compression events.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

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
    pub history: Mutex<GrHistory>,
}

pub struct GrHistory {
    pub samples: [f32; HISTORY_LEN],
    pub write_pos: usize,
}

impl GrHistory {
    /// Iterate the ring in chronological order (oldest first).
    pub fn iter_chrono(&self) -> impl Iterator<Item = f32> + '_ {
        let start = self.write_pos;
        (0..HISTORY_LEN).map(move |i| self.samples[(start + i) % HISTORY_LEN])
    }
}

impl CompressorViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            input_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            output_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            gr_db: AtomicU32::new(0.0f32.to_bits()),
            history: Mutex::new(GrHistory {
                samples: [0.0; HISTORY_LEN],
                write_pos: 0,
            }),
        })
    }

    pub fn store_levels(&self, input_db: f32, output_db: f32, gr_db: f32) {
        self.input_db.store(input_db.to_bits(), Ordering::Relaxed);
        self.output_db.store(output_db.to_bits(), Ordering::Relaxed);
        self.gr_db.store(gr_db.to_bits(), Ordering::Relaxed);
    }

    pub fn push_gr(&self, gr_db: f32) {
        let mut h = self.history.lock();
        let pos = h.write_pos;
        h.samples[pos] = gr_db;
        h.write_pos = (pos + 1) % HISTORY_LEN;
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
