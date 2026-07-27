//! Shared audio-thread → editor metering state (ba todo #1079).
//!
//! Mirrors `plugins/resonance-delay/src/viz.rs`: a pre-allocated set of
//! relaxed atomics the audio thread stores into at block rate and the
//! editor reads each frame — lock-free, allocation-free on both sides.
//! Only cheap always-available meters are published; per-onset lists
//! (e.g. `psola_recent_onsets`) allocate and stay off this path.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub struct GranularViz {
    /// Effective delay-tap position, milliseconds (the Repitch glide /
    /// Fade commit readout, ba todo #1076).
    delay_ms: AtomicU32,
    /// Host tempo (0 when the host provides none).
    bpm: AtomicU32,
    /// Tracked fundamental, Hz (0 until the first voiced lock).
    period_hz: AtomicU32,
    /// 1 while the pitch-synchronous Voice/Mono scheduler is engaged.
    engaged: AtomicU32,
    /// Currently sounding async-cloud grains.
    active_grains: AtomicU32,
    /// Currently sounding PSOLA voices.
    psola_voices: AtomicU32,
}

impl GranularViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            delay_ms: AtomicU32::new(0.0f32.to_bits()),
            bpm: AtomicU32::new(0.0f32.to_bits()),
            period_hz: AtomicU32::new(0.0f32.to_bits()),
            engaged: AtomicU32::new(0),
            active_grains: AtomicU32::new(0),
            psola_voices: AtomicU32::new(0),
        })
    }

    /// Block-rate store from the audio thread (relaxed, allocation-free).
    pub fn store_block(
        &self,
        delay_ms: f32,
        bpm: f32,
        period_hz: f32,
        engaged: bool,
        active_grains: usize,
        psola_voices: usize,
    ) {
        self.delay_ms.store(delay_ms.to_bits(), Ordering::Relaxed);
        self.bpm.store(bpm.to_bits(), Ordering::Relaxed);
        self.period_hz.store(period_hz.to_bits(), Ordering::Relaxed);
        self.engaged.store(engaged as u32, Ordering::Relaxed);
        self.active_grains
            .store(active_grains.min(u32::MAX as usize) as u32, Ordering::Relaxed);
        self.psola_voices
            .store(psola_voices.min(u32::MAX as usize) as u32, Ordering::Relaxed);
    }

    pub fn read_delay_ms(&self) -> f32 {
        f32::from_bits(self.delay_ms.load(Ordering::Relaxed))
    }

    pub fn read_bpm(&self) -> f32 {
        f32::from_bits(self.bpm.load(Ordering::Relaxed))
    }

    pub fn read_period_hz(&self) -> f32 {
        f32::from_bits(self.period_hz.load(Ordering::Relaxed))
    }

    pub fn read_engaged(&self) -> bool {
        self.engaged.load(Ordering::Relaxed) != 0
    }

    pub fn read_active_grains(&self) -> u32 {
        self.active_grains.load(Ordering::Relaxed)
    }

    pub fn read_psola_voices(&self) -> u32 {
        self.psola_voices.load(Ordering::Relaxed)
    }
}
