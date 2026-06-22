//! Lock-free visualization state shared between the audio thread and
//! the editor thread.
//!
//! The aggregate scalar snapshot uses [`arc_swap::ArcSwap`] so the audio
//! thread can publish a consistent copy of every meter in a single swap,
//! avoiding torn reads across 12 independent atomics. The two history
//! rings (LUFS-momentary trace and true-peak trace) use a wait-free SPSC
//! pattern — `[AtomicU32; N]` for the f32 samples plus an `AtomicUsize`
//! write index. The audio thread is the sole producer; the editor reads
//! at its own cadence and tolerates the one-frame skew inherent in the
//! unsynchronised hand-off.
//!
//! The spectrum curve is fetched directly from the metering crate's
//! [`SpectrumHandle`], which is itself wait-free.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

use resonance_metering::{AtomicMeterSnapshot, MeterSnapshot, SpectrumHandle};

use crate::assistant::Assistant;

/// How many LUFS-momentary history samples to keep. 512 at ~17 Hz
/// (block pushes, configurable via the feed rate) ≈ 30 s trace.
pub const LUFS_HISTORY_LEN: usize = 512;
/// How many true-peak hold samples to keep. ~5 s at 17 Hz.
pub const TP_HISTORY_LEN: usize = 84;

/// Wait-free SPSC ring of f32 samples. The audio thread writes via
/// [`push`](Self::push); the editor thread iterates via
/// [`iter_chrono`](Self::iter_chrono). Each sample is a single aligned
/// `AtomicU32` load/store so values are never torn; reads may straddle
/// a single producer update which is acceptable for a meter trace.
pub struct HistoryRing<const N: usize> {
    samples: [AtomicU32; N],
    write_pos: AtomicUsize,
}

impl<const N: usize> HistoryRing<N> {
    /// Build a fresh ring pre-filled with `initial`. LUFS traces want
    /// `-inf` so an empty ring renders as silence; TP traces want the
    /// floor in dBTP (−120 dB) for the same reason.
    pub fn new(initial: f32) -> Self {
        let bits = initial.to_bits();
        Self {
            samples: std::array::from_fn(|_| AtomicU32::new(bits)),
            write_pos: AtomicUsize::new(0),
        }
    }

    /// Wait-free producer. Audio-thread safe; no allocation, no locks.
    pub fn push(&self, v: f32) {
        let pos = self.write_pos.load(Ordering::Relaxed);
        self.samples[pos].store(v.to_bits(), Ordering::Relaxed);
        let next = if pos + 1 == N { 0 } else { pos + 1 };
        // Release so consumer's Acquire on write_pos observes the sample store.
        self.write_pos.store(next, Ordering::Release);
    }

    /// Iterate the ring in chronological order (oldest sample first).
    /// Wait-free consumer.
    pub fn iter_chrono(&self) -> impl Iterator<Item = f32> + '_ {
        let start = self.write_pos.load(Ordering::Acquire);
        (0..N).map(move |i| {
            let idx = (start + i) % N;
            f32::from_bits(self.samples[idx].load(Ordering::Relaxed))
        })
    }
}

/// Alias for the LUFS history ring (initialised to −∞).
pub type LufsHistoryRing = HistoryRing<LUFS_HISTORY_LEN>;
/// Alias for the true-peak history ring (initialised to −120 dBTP).
pub type TpHistoryRing = HistoryRing<TP_HISTORY_LEN>;

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
}
