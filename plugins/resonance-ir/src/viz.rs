//! Lock-free visualisation state shared between the audio thread and
//! the editor thread. Mirrors the amp plugin's `viz.rs`: per-scalar
//! atomics for the peak meters (cheap wait-free reads), and a
//! `parking_lot::Mutex<Option<IrSnapshot>>` for the one-shot waveform
//! and magnitude-response arrays that the loader thread publishes
//! whenever a new IR finishes loading.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

/// Number of points in the decimated waveform drawn by the editor.
/// Fixed size so the editor can snapshot into a stack array without
/// allocating.
pub const WAVEFORM_POINTS: usize = 512;

/// Number of points in the precomputed log-spaced magnitude response.
/// 256 is enough for a smooth EQ curve across the audible range.
pub const RESPONSE_POINTS: usize = 256;

/// Precomputed visualisation data for the currently loaded IR. Built
/// once on the loader thread when a new IR arrives, then handed over
/// through `IrViz::snapshot` for the editor to read.
#[derive(Clone)]
pub struct IrSnapshot {
    /// Left-channel envelope decimated to `WAVEFORM_POINTS`. Each entry
    /// is the max absolute sample over the corresponding source window.
    pub wave_left: [f32; WAVEFORM_POINTS],
    /// Right-channel envelope. For mono IRs this equals `wave_left`.
    pub wave_right: [f32; WAVEFORM_POINTS],
    /// Number of populated points in the waveform. Short IRs don't
    /// fill the full `WAVEFORM_POINTS` — the editor clips at this.
    pub wave_len: usize,
    /// Log-smoothed magnitude response in dBFS, one entry per
    /// geometrically-spaced frequency bin from `response_min_hz` to
    /// `response_max_hz`.
    pub response_db: [f32; RESPONSE_POINTS],
    /// Lowest frequency in the response plot (Hz).
    pub response_min_hz: f32,
    /// Highest frequency in the response plot (Hz).
    pub response_max_hz: f32,
}

impl IrSnapshot {
    pub fn empty() -> Self {
        Self {
            wave_left: [0.0; WAVEFORM_POINTS],
            wave_right: [0.0; WAVEFORM_POINTS],
            wave_len: 0,
            response_db: [f32::NEG_INFINITY; RESPONSE_POINTS],
            response_min_hz: 20.0,
            response_max_hz: 20_000.0,
        }
    }
}

pub struct IrViz {
    in_l_db: AtomicU32,
    in_r_db: AtomicU32,
    out_l_db: AtomicU32,
    out_r_db: AtomicU32,

    /// Latest IR snapshot. `None` until the first IR finishes loading.
    snapshot: Mutex<Option<IrSnapshot>>,
}

impl IrViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            in_l_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            in_r_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            out_l_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            out_r_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            snapshot: Mutex::new(None),
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

    pub fn store_snapshot(&self, snap: IrSnapshot) {
        *self.snapshot.lock() = Some(snap);
    }

    pub fn clear_snapshot(&self) {
        *self.snapshot.lock() = None;
    }

    /// Snapshot the current IR viz into a fresh clone. The editor
    /// calls this once per frame. Returns `None` until the first IR
    /// has loaded.
    pub fn snapshot(&self) -> Option<IrSnapshot> {
        self.snapshot.lock().clone()
    }
}
