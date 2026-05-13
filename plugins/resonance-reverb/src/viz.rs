//! Lock-free visualization state shared between the audio thread and
//! the editor thread.
//!
//! Per-sample values (input/output peaks, per-FDN-channel energies) are
//! stored as bit-punned atomic f32s — cheap, wait-free, no tearing for
//! a single scalar. The rolling tail-RMS trace follows the compressor's
//! pattern: a short `parking_lot::Mutex` around a fixed-size ring buffer,
//! written by the audio thread once per block and read once per frame
//! by the UI.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

/// Length of the rolling wet-RMS history shown behind the analytic
/// decay polygon. At ~1 push/block (e.g. ~350 Hz at 48 k / 128-frame
/// blocks) this is ~0.7 s of visible history, which is all the impulse
/// view needs — the analytic polygon carries the full decay shape.
pub const TAIL_HISTORY_LEN: usize = 256;

/// Number of FDN channels the tank view shows. Must match
/// `dsp::CHANNELS`.
pub const FDN_CHANNELS: usize = 8;

/// Number of early-reflection taps the impulse view shows. Must match
/// `dsp::ER_TAPS`.
pub const ER_TAPS: usize = 12;

/// `(times, gains)` per stereo channel. `times` and `gains` are each
/// `[(L, R); ER_TAPS]` — one (left, right) pair per tap.
pub type ErTapsSnapshot = ([(f32, f32); ER_TAPS], [(f32, f32); ER_TAPS]);

/// Fixed-length ring for the wet-RMS history trace.
pub struct TailHistory {
    pub samples: [f32; TAIL_HISTORY_LEN],
    pub write_pos: usize,
}

impl TailHistory {
    fn new() -> Self {
        Self {
            samples: [0.0; TAIL_HISTORY_LEN],
            write_pos: 0,
        }
    }

    pub fn push(&mut self, v: f32) {
        self.samples[self.write_pos] = v;
        self.write_pos = (self.write_pos + 1) % TAIL_HISTORY_LEN;
    }

    /// Iterate the ring in chronological order (oldest first).
    pub fn iter_chrono(&self) -> impl Iterator<Item = f32> + '_ {
        let start = self.write_pos;
        (0..TAIL_HISTORY_LEN).map(move |i| self.samples[(start + i) % TAIL_HISTORY_LEN])
    }
}

/// Shared viz state for the reverb editor. Stored as `Arc<ReverbViz>`
/// on the plugin; the editor holds a clone.
pub struct ReverbViz {
    // Peak meters.
    in_l_db: AtomicU32,
    in_r_db: AtomicU32,
    out_l_db: AtomicU32,
    out_r_db: AtomicU32,

    /// Per-channel smoothed FDN energy for the tank view.
    channel_energies: [AtomicU32; FDN_CHANNELS],
    /// FDN delay lengths in ms for the tank labels.
    fdn_delay_ms: [AtomicU32; FDN_CHANNELS],

    /// ER tap times in ms (left, right), as bit-punned atomic f32.
    er_tap_ms_l: [AtomicU32; ER_TAPS],
    er_tap_ms_r: [AtomicU32; ER_TAPS],
    /// ER tap gains (absolute; polarity is cosmetic in the viz).
    er_tap_gain_l: [AtomicU32; ER_TAPS],
    er_tap_gain_r: [AtomicU32; ER_TAPS],

    /// Rolling history of wet RMS samples (one push per audio block).
    pub tail: Mutex<TailHistory>,
}

impl ReverbViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            in_l_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            in_r_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            out_l_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            out_r_db: AtomicU32::new(f32::NEG_INFINITY.to_bits()),
            channel_energies: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            fdn_delay_ms: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            er_tap_ms_l: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            er_tap_ms_r: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            er_tap_gain_l: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            er_tap_gain_r: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            tail: Mutex::new(TailHistory::new()),
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

    pub fn store_channel_energies(&self, energies: &[f32; FDN_CHANNELS]) {
        for (slot, &v) in self.channel_energies.iter().zip(energies.iter()) {
            slot.store(v.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn read_channel_energies(&self) -> [f32; FDN_CHANNELS] {
        let mut out = [0.0f32; FDN_CHANNELS];
        for (i, slot) in self.channel_energies.iter().enumerate() {
            out[i] = f32::from_bits(slot.load(Ordering::Relaxed));
        }
        out
    }

    pub fn store_fdn_delay_ms(&self, ms: &[f32; FDN_CHANNELS]) {
        for (slot, &v) in self.fdn_delay_ms.iter().zip(ms.iter()) {
            slot.store(v.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn read_fdn_delay_ms(&self) -> [f32; FDN_CHANNELS] {
        let mut out = [0.0f32; FDN_CHANNELS];
        for (i, slot) in self.fdn_delay_ms.iter().enumerate() {
            out[i] = f32::from_bits(slot.load(Ordering::Relaxed));
        }
        out
    }

    pub fn store_er_taps(&self, times: &[(f32, f32); ER_TAPS], gains: &[(f32, f32); ER_TAPS]) {
        for i in 0..ER_TAPS {
            self.er_tap_ms_l[i].store(times[i].0.to_bits(), Ordering::Relaxed);
            self.er_tap_ms_r[i].store(times[i].1.to_bits(), Ordering::Relaxed);
            // The viz renders height from |gain|; polarity is carried as
            // up/down direction in the lollipop plot, so we store absolute
            // values here.
            self.er_tap_gain_l[i].store(gains[i].0.abs().to_bits(), Ordering::Relaxed);
            self.er_tap_gain_r[i].store(gains[i].1.abs().to_bits(), Ordering::Relaxed);
        }
    }

    pub fn read_er_taps(&self) -> ErTapsSnapshot {
        let mut times = [(0.0f32, 0.0f32); ER_TAPS];
        let mut gains = [(0.0f32, 0.0f32); ER_TAPS];
        for i in 0..ER_TAPS {
            times[i] = (
                f32::from_bits(self.er_tap_ms_l[i].load(Ordering::Relaxed)),
                f32::from_bits(self.er_tap_ms_r[i].load(Ordering::Relaxed)),
            );
            gains[i] = (
                f32::from_bits(self.er_tap_gain_l[i].load(Ordering::Relaxed)),
                f32::from_bits(self.er_tap_gain_r[i].load(Ordering::Relaxed)),
            );
        }
        (times, gains)
    }

    pub fn push_tail_rms(&self, rms: f32) {
        self.tail.lock().push(rms);
    }
}
