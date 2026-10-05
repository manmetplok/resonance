//! Lock-free visualization state shared between the audio thread and
//! the editor thread.
//!
//! Every field delegates to the shared bit-punned atomics in
//! `resonance-metering` — cheap, wait-free, no tearing for a single
//! scalar. The rolling tail-RMS trace is the shared
//! [`AtomicHistoryRing`]: the audio thread never blocks, and the UI
//! reader can tolerate one straddled sample at frame boundaries (it's
//! only ever rendering a viz).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use resonance_metering::{AtomicF32, AtomicF32Array, AtomicF32Pair, AtomicHistoryRing};

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

/// Fixed-length lock-free ring for the wet-RMS history trace. The
/// writer pushes one sample per audio block; the reader snapshots the
/// whole ring in chronological order each viz frame.
pub struct TailHistory(AtomicHistoryRing<TAIL_HISTORY_LEN>);

impl TailHistory {
    fn new() -> Self {
        Self(AtomicHistoryRing::new(0.0))
    }

    /// Push one sample. Called from the audio thread once per block;
    /// wait-free.
    pub fn push(&self, v: f32) {
        self.0.push(v);
    }

    /// Snapshot the ring in chronological order (oldest first). Two
    /// adjacent samples can straddle a writer push; for a viz history
    /// this is acceptable.
    pub fn iter_chrono(&self) -> impl Iterator<Item = f32> + '_ {
        self.0.iter_chrono()
    }
}

/// Shared viz state for the reverb editor. Stored as `Arc<ReverbViz>`
/// on the plugin; the editor holds a clone.
pub struct ReverbViz {
    // Peak meters.
    in_db: AtomicF32Pair,
    out_db: AtomicF32Pair,

    /// Per-channel smoothed FDN energy for the tank view.
    channel_energies: AtomicF32Array<FDN_CHANNELS>,
    /// FDN delay lengths in ms for the tank labels.
    fdn_delay_ms: AtomicF32Array<FDN_CHANNELS>,

    /// ER tap times in ms (left, right), as bit-punned atomic f32.
    er_tap_ms_l: AtomicF32Array<ER_TAPS>,
    er_tap_ms_r: AtomicF32Array<ER_TAPS>,
    /// ER tap gains (absolute; polarity is cosmetic in the viz).
    er_tap_gain_l: AtomicF32Array<ER_TAPS>,
    er_tap_gain_r: AtomicF32Array<ER_TAPS>,

    /// Rolling history of wet RMS samples (one push per audio block).
    pub tail: TailHistory,

    /// Whether the host has connected a sidechain key this block — the
    /// ducker keys off it when true, off the dry input otherwise.
    key_connected: AtomicBool,
    /// The ducker's current gain reduction on the wet return, dB (>= 0).
    duck_gr_db: AtomicF32,
    /// The tempo-synced pre-delay (ms) and decay (s) in effect, NaN while
    /// the knob rules (sync off, or no tempo from the host).
    synced_predelay_ms: AtomicF32,
    synced_decay_s: AtomicF32,
}

impl ReverbViz {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            in_db: AtomicF32Pair::new(f32::NEG_INFINITY),
            out_db: AtomicF32Pair::new(f32::NEG_INFINITY),
            channel_energies: AtomicF32Array::new(0.0),
            fdn_delay_ms: AtomicF32Array::new(0.0),
            er_tap_ms_l: AtomicF32Array::new(0.0),
            er_tap_ms_r: AtomicF32Array::new(0.0),
            er_tap_gain_l: AtomicF32Array::new(0.0),
            er_tap_gain_r: AtomicF32Array::new(0.0),
            tail: TailHistory::new(),
            key_connected: AtomicBool::new(false),
            duck_gr_db: AtomicF32::new(0.0),
            synced_predelay_ms: AtomicF32::new(f32::NAN),
            synced_decay_s: AtomicF32::new(f32::NAN),
        })
    }

    /// Publish the synced values in effect this block (`None` = the knob
    /// rules).
    pub fn store_synced(&self, predelay_ms: Option<f32>, decay_s: Option<f32>) {
        self.synced_predelay_ms
            .store(predelay_ms.unwrap_or(f32::NAN), Ordering::Relaxed);
        self.synced_decay_s
            .store(decay_s.unwrap_or(f32::NAN), Ordering::Relaxed);
    }

    /// The tempo-synced pre-delay in ms, while one is in effect.
    pub fn synced_predelay_ms(&self) -> Option<f32> {
        Some(self.synced_predelay_ms.load(Ordering::Relaxed)).filter(|v| !v.is_nan())
    }

    /// The tempo-synced decay (T60) in s, while one is in effect.
    pub fn synced_decay_s(&self) -> Option<f32> {
        Some(self.synced_decay_s.load(Ordering::Relaxed)).filter(|v| !v.is_nan())
    }

    pub fn store_peaks(&self, in_l_db: f32, in_r_db: f32, out_l_db: f32, out_r_db: f32) {
        self.in_db.store(in_l_db, in_r_db);
        self.out_db.store(out_l_db, out_r_db);
    }

    pub fn read_in_peaks_db(&self) -> (f32, f32) {
        self.in_db.load()
    }

    pub fn read_out_peaks_db(&self) -> (f32, f32) {
        self.out_db.load()
    }

    pub fn store_channel_energies(&self, energies: &[f32; FDN_CHANNELS]) {
        self.channel_energies.store(energies);
    }

    pub fn read_channel_energies(&self) -> [f32; FDN_CHANNELS] {
        self.channel_energies.load()
    }

    pub fn store_fdn_delay_ms(&self, ms: &[f32; FDN_CHANNELS]) {
        self.fdn_delay_ms.store(ms);
    }

    pub fn read_fdn_delay_ms(&self) -> [f32; FDN_CHANNELS] {
        self.fdn_delay_ms.load()
    }

    pub fn store_er_taps(&self, times: &[(f32, f32); ER_TAPS], gains: &[(f32, f32); ER_TAPS]) {
        for i in 0..ER_TAPS {
            self.er_tap_ms_l.store_at(i, times[i].0);
            self.er_tap_ms_r.store_at(i, times[i].1);
            // The viz renders height from |gain|; polarity is carried as
            // up/down direction in the lollipop plot, so we store absolute
            // values here.
            self.er_tap_gain_l.store_at(i, gains[i].0.abs());
            self.er_tap_gain_r.store_at(i, gains[i].1.abs());
        }
    }

    pub fn read_er_taps(&self) -> ErTapsSnapshot {
        let mut times = [(0.0f32, 0.0f32); ER_TAPS];
        let mut gains = [(0.0f32, 0.0f32); ER_TAPS];
        for i in 0..ER_TAPS {
            times[i] = (self.er_tap_ms_l.load_at(i), self.er_tap_ms_r.load_at(i));
            gains[i] = (self.er_tap_gain_l.load_at(i), self.er_tap_gain_r.load_at(i));
        }
        (times, gains)
    }

    pub fn push_tail_rms(&self, rms: f32) {
        self.tail.push(rms);
    }

    pub fn store_key_connected(&self, connected: bool) {
        self.key_connected.store(connected, Ordering::Relaxed);
    }

    /// True while a sidechain key is connected: the ducker's detector is
    /// the key, not the dry input.
    pub fn key_connected(&self) -> bool {
        self.key_connected.load(Ordering::Relaxed)
    }

    pub fn store_duck_gr_db(&self, gr_db: f32) {
        self.duck_gr_db.store(gr_db, Ordering::Relaxed);
    }

    /// The ducker's gain reduction on the wet return, dB (>= 0).
    pub fn duck_gr_db(&self) -> f32 {
        self.duck_gr_db.load(Ordering::Relaxed)
    }
}
