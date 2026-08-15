//! Editor-facing state shared with the audio thread (ba todo #1314).
//!
//! The gate computed `key_connected` from the first block it ever
//! processed and documented it as UI-facing — "surfaced so the editor
//! can tell the user which detector is actually running" — but it
//! reached nothing except a test. Which detector is running is the one
//! thing about this plugin a user cannot work out by looking: a gate
//! keyed from its own input and a gate keyed from a silent sidechain
//! look identical and behave completely differently.
//!
//! Same shape as `resonance-compressor`'s viz: every cell is an atomic,
//! floats are bit-punned into `AtomicU32`, and the audio thread only
//! ever stores — it never allocates, locks, or reads back. The editor
//! reads at frame rate and tolerates seeing a straddled pair of cells,
//! since these are status readouts and not a signal path.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;

use crate::dsp::GateState;

/// Which signal the detector is actually reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectorSource {
    /// No key connected: the gate reads the signal it is gating.
    SelfInput,
    /// The host has connected a key; the detector reads that instead.
    ExternalKey,
}

impl DetectorSource {
    /// Short label for the editor's status chip.
    pub fn label(self) -> &'static str {
        match self {
            DetectorSource::SelfInput => "INPUT",
            DetectorSource::ExternalKey => "EXTERNAL KEY",
        }
    }
}

pub struct GateViz {
    /// Whether the host connected a key to the last processed block.
    key_connected: AtomicBool,
    /// [`GateState::code`] of the state the gate ended that block in.
    state: AtomicU8,
    /// Peak gain reduction across that block, dB (positive = reducing).
    gr_db: AtomicU32,
    /// Peak detector level across that block, dBFS — post key
    /// substitution and post key high-pass, i.e. the number the
    /// threshold is compared against.
    detector_db: AtomicU32,
}

impl GateViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            key_connected: AtomicBool::new(false),
            state: AtomicU8::new(GateState::Closed.code()),
            gr_db: AtomicU32::new(0.0f32.to_bits()),
            detector_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
        })
    }

    /// Record which detector the block ran off. Stored separately from
    /// [`GateViz::store_block`] because the plugin knows it even on the
    /// paths that return before the DSP runs.
    pub fn store_key_connected(&self, connected: bool) {
        self.key_connected.store(connected, Ordering::Relaxed);
    }

    /// Record the end-of-block detector state. Called once per process
    /// call from the audio thread; wait-free.
    pub fn store_block(&self, state: GateState, gr_db: f32, detector_db: f32) {
        self.state.store(state.code(), Ordering::Relaxed);
        self.gr_db.store(gr_db.to_bits(), Ordering::Relaxed);
        self.detector_db
            .store(detector_db.to_bits(), Ordering::Relaxed);
    }

    /// Back to the state a freshly constructed plugin reports, for
    /// `reset()`: nothing has been processed, so nothing is claimed.
    pub fn clear(&self) {
        self.key_connected.store(false, Ordering::Relaxed);
        self.store_block(GateState::Closed, 0.0, f32::NEG_INFINITY);
    }

    pub fn key_connected(&self) -> bool {
        self.key_connected.load(Ordering::Relaxed)
    }

    pub fn detector_source(&self) -> DetectorSource {
        if self.key_connected() {
            DetectorSource::ExternalKey
        } else {
            DetectorSource::SelfInput
        }
    }

    pub fn state(&self) -> GateState {
        GateState::from_code(self.state.load(Ordering::Relaxed))
    }

    pub fn gr_db(&self) -> f32 {
        f32::from_bits(self.gr_db.load(Ordering::Relaxed))
    }

    pub fn detector_db(&self) -> f32 {
        f32::from_bits(self.detector_db.load(Ordering::Relaxed))
    }
}
