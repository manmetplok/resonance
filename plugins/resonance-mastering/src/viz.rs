//! Lock-free visualization state shared between the audio thread and
//! the editor thread.
//!
//! The aggregate scalar snapshot uses [`arc_swap::ArcSwap`] so the audio
//! thread can publish a consistent copy of every meter in a single swap,
//! avoiding torn reads across 12 independent atomics. The two history
//! rings (LUFS-momentary trace and true-peak trace) follow the compressor
//! pattern — uncontended `parking_lot::Mutex` guarding a `[f32; N]`.
//!
//! The spectrum curve is fetched directly from the metering crate's
//! [`SpectrumHandle`], which is itself wait-free.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use resonance_metering::{MeterSnapshot, SpectrumHandle};

use crate::assistant::Assistant;

/// How many LUFS-momentary history samples to keep. 512 at ~17 Hz
/// (block pushes, configurable via the feed rate) ≈ 30 s trace.
pub const LUFS_HISTORY_LEN: usize = 512;
/// How many true-peak hold samples to keep. ~5 s at 17 Hz.
pub const TP_HISTORY_LEN: usize = 84;

/// Fixed-length ring buffer for scalar history traces. Used by the
/// editor to draw rolling meter-vs-time plots. Generic over the length
/// so the LUFS and TP traces share one implementation.
pub struct HistoryRing<const N: usize> {
    samples: [f32; N],
    write_pos: usize,
}

impl<const N: usize> HistoryRing<N> {
    /// Build a fresh ring pre-filled with `initial`. LUFS traces want
    /// `-inf` so an empty ring renders as silence; TP traces want the
    /// floor in dBTP (−120 dB) for the same reason.
    pub fn new(initial: f32) -> Self {
        Self {
            samples: [initial; N],
            write_pos: 0,
        }
    }

    pub fn push(&mut self, v: f32) {
        self.samples[self.write_pos] = v;
        self.write_pos = (self.write_pos + 1) % N;
    }

    /// Iterate the ring in chronological order (oldest sample first).
    pub fn iter_chrono(&self) -> impl Iterator<Item = f32> + '_ {
        let start = self.write_pos;
        (0..N).map(move |i| self.samples[(start + i) % N])
    }
}

/// Alias for the LUFS history ring (initialised to −∞).
pub type LufsHistoryRing = HistoryRing<LUFS_HISTORY_LEN>;
/// Alias for the true-peak history ring (initialised to −120 dBTP).
pub type TpHistoryRing = HistoryRing<TP_HISTORY_LEN>;

/// All visualization state shared with the editor.
pub struct MasteringViz {
    pub snapshot: ArcSwap<MeterSnapshot>,
    pub spectrum: parking_lot::RwLock<Option<SpectrumHandle>>,
    pub lufs_history: Mutex<LufsHistoryRing>,
    pub tp_history: Mutex<TpHistoryRing>,
    pub assistant: Assistant,
    /// Live gain-reduction in dB for the glue compressor (0 = no
    /// reduction, positive = attenuation). Published once per block
    /// from the audio thread as a bit-punned f32.
    glue_gr_db: AtomicU32,
    /// Live gain-reduction in dB for the brick-wall limiter.
    limiter_gr_db: AtomicU32,
}

impl MasteringViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            snapshot: ArcSwap::from_pointee(MeterSnapshot::default()),
            spectrum: parking_lot::RwLock::new(None),
            lufs_history: Mutex::new(LufsHistoryRing::new(f32::NEG_INFINITY)),
            tp_history: Mutex::new(TpHistoryRing::new(-120.0)),
            // Placeholder sample rate — the plugin's `initialize()`
            // calls `set_sample_rate` before the first audio block.
            assistant: Assistant::new(48_000.0),
            glue_gr_db: AtomicU32::new(0.0_f32.to_bits()),
            limiter_gr_db: AtomicU32::new(0.0_f32.to_bits()),
        })
    }

    /// Install the spectrum handle once the metering core has been built.
    pub fn set_spectrum_handle(&self, handle: SpectrumHandle) {
        *self.spectrum.write() = Some(handle);
    }

    /// Read the current scalar snapshot. Wait-free.
    pub fn load_snapshot(&self) -> Arc<MeterSnapshot> {
        self.snapshot.load_full()
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
}

