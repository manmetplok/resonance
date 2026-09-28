//! Audio-thread → editor meter levels. Bit-punned atomics, wait-free on
//! both sides; a reader that straddles a block boundary sees one stale
//! value, which a meter can afford.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub struct ColorViz {
    /// Decaying input peak in dBFS.
    input_db: AtomicU32,
    /// Decaying output peak in dBFS.
    output_db: AtomicU32,
    /// The gain auto-gain is applying to the wet path, in dB (0 when off).
    auto_gain_db: AtomicU32,
}

impl ColorViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            input_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            output_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            auto_gain_db: AtomicU32::new(0.0f32.to_bits()),
        })
    }

    /// Publish one block's levels. Called from the audio thread.
    pub fn store(&self, input_db: f32, output_db: f32, auto_gain_db: f32) {
        self.input_db.store(input_db.to_bits(), Ordering::Relaxed);
        self.output_db.store(output_db.to_bits(), Ordering::Relaxed);
        self.auto_gain_db.store(auto_gain_db.to_bits(), Ordering::Relaxed);
    }

    pub fn input_db(&self) -> f32 {
        f32::from_bits(self.input_db.load(Ordering::Relaxed))
    }

    pub fn output_db(&self) -> f32 {
        f32::from_bits(self.output_db.load(Ordering::Relaxed))
    }

    pub fn auto_gain_db(&self) -> f32 {
        f32::from_bits(self.auto_gain_db.load(Ordering::Relaxed))
    }
}
