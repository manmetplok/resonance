//! Lock-free, allocation-free holder for a [`MeterSnapshot`].
//!
//! The audio thread publishes a fresh snapshot every block via
//! [`AtomicMeterSnapshot::store`] (eleven `Relaxed` `AtomicU32` stores, no
//! allocation); a UI / control thread reads the latest via
//! [`AtomicMeterSnapshot::load`]. A reader may observe a *torn* snapshot
//! (different fields from adjacent blocks), but values change at meter
//! rates so the visual error is at most one frame of mixed readouts — the
//! same trade-off the meter history rings already accept. This replaces an
//! `ArcSwap<MeterSnapshot>` whose `store(Arc::new(...))` allocated on every
//! audio block.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::snapshot::MeterSnapshot;

/// Lock-free, allocation-free aggregate of every scalar meter readout.
/// Each field is a bit-punned [`AtomicU32`].
pub struct AtomicMeterSnapshot {
    momentary_lufs_bits: AtomicU32,
    short_term_lufs_bits: AtomicU32,
    integrated_lufs_bits: AtomicU32,
    true_peak_left_dbtp_bits: AtomicU32,
    true_peak_right_dbtp_bits: AtomicU32,
    true_peak_max_dbtp_bits: AtomicU32,
    correlation_bits: AtomicU32,
    crest_db_bits: AtomicU32,
    plr_db_bits: AtomicU32,
    psr_db_bits: AtomicU32,
    lra_lu_bits: AtomicU32,
}

impl AtomicMeterSnapshot {
    pub fn new() -> Self {
        Self::from_snapshot(&MeterSnapshot::default())
    }

    /// Seed every field from an existing snapshot.
    pub fn from_snapshot(s: &MeterSnapshot) -> Self {
        Self {
            momentary_lufs_bits: AtomicU32::new(s.momentary_lufs.to_bits()),
            short_term_lufs_bits: AtomicU32::new(s.short_term_lufs.to_bits()),
            integrated_lufs_bits: AtomicU32::new(s.integrated_lufs.to_bits()),
            true_peak_left_dbtp_bits: AtomicU32::new(s.true_peak_left_dbtp.to_bits()),
            true_peak_right_dbtp_bits: AtomicU32::new(s.true_peak_right_dbtp.to_bits()),
            true_peak_max_dbtp_bits: AtomicU32::new(s.true_peak_max_dbtp.to_bits()),
            correlation_bits: AtomicU32::new(s.correlation.to_bits()),
            crest_db_bits: AtomicU32::new(s.crest_db.to_bits()),
            plr_db_bits: AtomicU32::new(s.plr_db.to_bits()),
            psr_db_bits: AtomicU32::new(s.psr_db.to_bits()),
            lra_lu_bits: AtomicU32::new(s.lra_lu.to_bits()),
        }
    }

    /// Audio-thread store. Eleven `AtomicU32` Relaxed stores, no allocation.
    pub fn store(&self, s: &MeterSnapshot) {
        self.momentary_lufs_bits
            .store(s.momentary_lufs.to_bits(), Ordering::Relaxed);
        self.short_term_lufs_bits
            .store(s.short_term_lufs.to_bits(), Ordering::Relaxed);
        self.integrated_lufs_bits
            .store(s.integrated_lufs.to_bits(), Ordering::Relaxed);
        self.true_peak_left_dbtp_bits
            .store(s.true_peak_left_dbtp.to_bits(), Ordering::Relaxed);
        self.true_peak_right_dbtp_bits
            .store(s.true_peak_right_dbtp.to_bits(), Ordering::Relaxed);
        self.true_peak_max_dbtp_bits
            .store(s.true_peak_max_dbtp.to_bits(), Ordering::Relaxed);
        self.correlation_bits
            .store(s.correlation.to_bits(), Ordering::Relaxed);
        self.crest_db_bits
            .store(s.crest_db.to_bits(), Ordering::Relaxed);
        self.plr_db_bits.store(s.plr_db.to_bits(), Ordering::Relaxed);
        self.psr_db_bits.store(s.psr_db.to_bits(), Ordering::Relaxed);
        self.lra_lu_bits.store(s.lra_lu.to_bits(), Ordering::Relaxed);
    }

    /// Reader-thread load. Returns a struct whose fields are each
    /// individually fresh; cross-field tearing is possible but harmless at
    /// meter update rates.
    pub fn load(&self) -> MeterSnapshot {
        MeterSnapshot {
            momentary_lufs: f32::from_bits(self.momentary_lufs_bits.load(Ordering::Relaxed)),
            short_term_lufs: f32::from_bits(self.short_term_lufs_bits.load(Ordering::Relaxed)),
            integrated_lufs: f32::from_bits(self.integrated_lufs_bits.load(Ordering::Relaxed)),
            true_peak_left_dbtp: f32::from_bits(
                self.true_peak_left_dbtp_bits.load(Ordering::Relaxed),
            ),
            true_peak_right_dbtp: f32::from_bits(
                self.true_peak_right_dbtp_bits.load(Ordering::Relaxed),
            ),
            true_peak_max_dbtp: f32::from_bits(self.true_peak_max_dbtp_bits.load(Ordering::Relaxed)),
            correlation: f32::from_bits(self.correlation_bits.load(Ordering::Relaxed)),
            crest_db: f32::from_bits(self.crest_db_bits.load(Ordering::Relaxed)),
            plr_db: f32::from_bits(self.plr_db_bits.load(Ordering::Relaxed)),
            psr_db: f32::from_bits(self.psr_db_bits.load(Ordering::Relaxed)),
            lra_lu: f32::from_bits(self.lra_lu_bits.load(Ordering::Relaxed)),
        }
    }
}

impl Default for AtomicMeterSnapshot {
    fn default() -> Self {
        Self::new()
    }
}
