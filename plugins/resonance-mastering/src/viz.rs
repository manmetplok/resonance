//! Lock-free visualization state shared between the audio thread and
//! the editor thread.
//!
//! The aggregate scalar snapshot uses [`arc_swap::ArcSwap`] so the audio
//! thread can publish a consistent copy of every meter in a single swap,
//! avoiding torn reads across 12 independent atomics. The two history
//! rings (LUFS-momentary trace and true-peak trace) are the shared
//! [`resonance_metering::AtomicHistoryRing`] — a wait-free SPSC pattern:
//! `[AtomicU32; N]` for the f32 samples plus an atomic cursor. The audio
//! thread is the sole producer; the editor reads at its own cadence and
//! tolerates the one-frame skew inherent in the unsynchronised hand-off.
//! (The shared ring's cursor counts total pushes and only grows, rather
//! than wrapping at `N` the way this module's own ring used to — a purely
//! internal bookkeeping difference; `new`/`push`/`iter_chrono` behave the
//! same.)
//!
//! The spectrum curve is fetched directly from the metering crate's
//! [`SpectrumHandle`], which is itself wait-free.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use resonance_metering::{AtomicHistoryRing, AtomicMeterSnapshot, MeterSnapshot, SpectrumHandle};

use crate::assistant::Assistant;
use crate::stages::multiband::NUM_BANDS;

/// How many LUFS-momentary history samples to keep. 512 at ~17 Hz
/// (block pushes, configurable via the feed rate) ≈ 30 s trace.
pub const LUFS_HISTORY_LEN: usize = 512;
/// How many true-peak hold samples to keep. ~5 s at 17 Hz.
pub const TP_HISTORY_LEN: usize = 84;

/// Alias for the LUFS history ring (initialised to −∞).
pub type LufsHistoryRing = AtomicHistoryRing<LUFS_HISTORY_LEN>;
/// Alias for the true-peak history ring (initialised to −120 dBTP).
pub type TpHistoryRing = AtomicHistoryRing<TP_HISTORY_LEN>;

/// All visualization state shared with the editor.
pub struct MasteringViz {
    pub snapshot: AtomicMeterSnapshot,
    pub spectrum: parking_lot::RwLock<Option<SpectrumHandle>>,
    pub lufs_history: LufsHistoryRing,
    pub tp_history: TpHistoryRing,
    pub assistant: Assistant,
    /// Live gain-reduction in dB for the glue compressor (0 = no
    /// reduction, positive = attenuation). Published once per block
    /// from the audio thread as a bit-punned f32.
    glue_gr_db: AtomicU32,
    /// Live gain-reduction in dB for the brick-wall limiter.
    limiter_gr_db: AtomicU32,
    /// Live gain-reduction in dB for each multiband band, low to high.
    /// Each band compressor already tracks this for its own meter, so
    /// publishing it costs the audio thread four relaxed stores per
    /// block and no extra measurement.
    band_gr_db: [AtomicU32; NUM_BANDS],
}

impl MasteringViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            snapshot: AtomicMeterSnapshot::new(),
            spectrum: parking_lot::RwLock::new(None),
            lufs_history: LufsHistoryRing::new(f32::NEG_INFINITY),
            tp_history: TpHistoryRing::new(-120.0),
            // Placeholder sample rate — the plugin's `initialize()`
            // calls `set_sample_rate` before the first audio block.
            assistant: Assistant::new(48_000.0),
            glue_gr_db: AtomicU32::new(0.0_f32.to_bits()),
            limiter_gr_db: AtomicU32::new(0.0_f32.to_bits()),
            band_gr_db: std::array::from_fn(|_| AtomicU32::new(0.0_f32.to_bits())),
        })
    }

    /// Install the spectrum handle once the metering core has been built.
    pub fn set_spectrum_handle(&self, handle: SpectrumHandle) {
        *self.spectrum.write() = Some(handle);
    }

    /// Read the current scalar snapshot. Wait-free.
    pub fn load_snapshot(&self) -> MeterSnapshot {
        self.snapshot.load()
    }

    /// Publish the current glue-compressor and limiter GR values (dB,
    /// non-negative). Called from the audio thread once per block.
    pub fn store_gr(&self, glue_db: f32, limiter_db: f32) {
        self.glue_gr_db.store(glue_db.to_bits(), Ordering::Relaxed);
        self.limiter_gr_db
            .store(limiter_db.to_bits(), Ordering::Relaxed);
    }

    /// Read the glue-compressor GR (dB).
    pub fn glue_gr_db(&self) -> f32 {
        f32::from_bits(self.glue_gr_db.load(Ordering::Relaxed))
    }

    /// Read the limiter GR (dB).
    pub fn limiter_gr_db(&self) -> f32 {
        f32::from_bits(self.limiter_gr_db.load(Ordering::Relaxed))
    }

    /// Publish the four multiband band GR values (dB, non-negative),
    /// low band first. Called from the audio thread once per block.
    pub fn store_band_gr(&self, band_db: [f32; NUM_BANDS]) {
        for (slot, db) in self.band_gr_db.iter().zip(band_db) {
            slot.store(db.to_bits(), Ordering::Relaxed);
        }
    }

    /// Read the multiband band GR values (dB), low band first.
    pub fn band_gr_db(&self) -> [f32; NUM_BANDS] {
        std::array::from_fn(|i| f32::from_bits(self.band_gr_db[i].load(Ordering::Relaxed)))
    }
}
