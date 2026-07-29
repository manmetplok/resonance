//! Shared audio-thread → editor metering state (ba todo #1079), plus
//! the packed grain-snapshot array and coarse buffer peaks feeding the
//! redesigned editor's hero grain-cloud view (ba todo #1135, design
//! doc #264 req-1).
//!
//! Mirrors `plugins/resonance-delay/src/viz.rs`: a pre-allocated set of
//! relaxed atomics the audio thread stores into at block rate and the
//! editor reads each frame — lock-free, allocation-free on both sides.
//! Only cheap always-available meters are published; per-onset lists
//! (e.g. `psola_recent_onsets`) allocate and stay off this path.
//!
//! # Grain-snapshot slot encoding (ba todo #1135)
//!
//! Each of the [`GRAIN_SLOTS`] slots is one `AtomicU64` holding one
//! sounding grain, written with a single relaxed store so a slot can
//! never tear. Bit layout (LSB first):
//!
//! | bits    | field      | encoding                                        |
//! |---------|------------|-------------------------------------------------|
//! | 0..16   | position   | unsigned, 0.25 ms units behind the write head (0 … 16383.75 ms) |
//! | 16..28  | pitch      | signed two's complement, 1/16 semitone units (±128 st, covers ±24 st + shimmer accumulation) |
//! | 28..38  | size       | unsigned, 1 ms units (0 … 1023 ms)              |
//! | 38..46  | level      | unsigned, 1/255 units (0 … 1)                   |
//! | 46      | active     | 1 = slot holds a grain; 0 = the whole word is 0 |
//! | 47      | reversed   | grain plays backwards                           |
//! | 48      | voiced     | PSOLA voice (period-sliver rendering)           |
//! | 49..52  | generation | feedback generation: 0 = first pass, 1+ = recirculation ghosts |
//! | 52..64  | reserved   | 0                                               |
//!
//! Editors never touch the layout: [`GranularViz::read_grains`] decodes
//! into plain [`GrainSnapshot`] structs.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

/// Fixed number of grain-snapshot slots (matches the audible engine's
/// pool size, `resonance_dsp::MAX_GRAINS`).
pub const GRAIN_SLOTS: usize = 64;

/// Number of coarse buffer-peak bins covering the whole source ring.
pub const PEAK_BINS: usize = 128;

/// Plain decoded form of one grain-snapshot slot (ba todo #1135).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GrainSnapshot {
    /// Grain read position behind the write head, milliseconds.
    pub position_ms: f32,
    /// Pitch offset in semitones (includes recirculation transposition
    /// for shimmer ghosts).
    pub pitch_semitones: f32,
    /// Grain length, milliseconds.
    pub size_ms: f32,
    /// Current enveloped level, `0..=1`.
    pub level: f32,
    /// Grain plays in reverse.
    pub reversed: bool,
    /// PSOLA voice (Pitch-Sync scheduler) rather than a cloud grain.
    pub voiced: bool,
    /// Feedback generation: 0 = first pass, `1..=7` = recirculation
    /// ghosts (each one delay further back, dimmer).
    pub generation: u8,
}

const ACTIVE_BIT: u64 = 1 << 46;

/// Pack one grain snapshot into its slot word (see the module docs for
/// the exact bit layout). Values outside the encodable ranges clamp.
pub fn pack_grain(g: &GrainSnapshot) -> u64 {
    let pos = ((g.position_ms.max(0.0) * 4.0).round() as u64).min(0xFFFF);
    let pitch = (((g.pitch_semitones * 16.0).round() as i64).clamp(-2048, 2047) as u64) & 0xFFF;
    let size = (g.size_ms.max(0.0).round() as u64).min(0x3FF);
    let level = ((g.level.clamp(0.0, 1.0) * 255.0).round() as u64).min(255);
    ACTIVE_BIT
        | pos
        | (pitch << 16)
        | (size << 28)
        | (level << 38)
        | ((g.reversed as u64) << 47)
        | ((g.voiced as u64) << 48)
        | ((g.generation.min(7) as u64) << 49)
}

/// Decode one slot word; `None` when the slot is inactive.
pub fn unpack_grain(bits: u64) -> Option<GrainSnapshot> {
    if bits & ACTIVE_BIT == 0 {
        return None;
    }
    let pitch_raw = ((bits >> 16) & 0xFFF) as i64;
    let pitch = if pitch_raw >= 2048 { pitch_raw - 4096 } else { pitch_raw };
    Some(GrainSnapshot {
        position_ms: (bits & 0xFFFF) as f32 * 0.25,
        pitch_semitones: pitch as f32 / 16.0,
        size_ms: ((bits >> 28) & 0x3FF) as f32,
        level: ((bits >> 38) & 0xFF) as f32 / 255.0,
        reversed: bits & (1 << 47) != 0,
        voiced: bits & (1 << 48) != 0,
        generation: ((bits >> 49) & 0x7) as u8,
    })
}

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
    /// Packed grain-snapshot slots (ba todo #1135; layout in the module
    /// docs). One relaxed `u64` store per slot — tear-free.
    grains: [AtomicU64; GRAIN_SLOTS],
    /// Coarse absolute-peak bins over the source ring (f32 bits).
    peaks: [AtomicU32; PEAK_BINS],
    /// Peak bin currently containing the write head.
    peak_head: AtomicU32,
    /// Milliseconds of buffer covered by one peak bin (f32 bits).
    peak_bin_ms: AtomicU32,
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
            grains: std::array::from_fn(|_| AtomicU64::new(0)),
            peaks: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            peak_head: AtomicU32::new(0),
            peak_bin_ms: AtomicU32::new(0.0f32.to_bits()),
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

    /// Audio-thread store of one grain-snapshot slot (relaxed, single
    /// `u64` store — allocation-free, tear-free; ba todo #1135).
    #[inline]
    pub fn store_grain(&self, slot: usize, snapshot: &GrainSnapshot) {
        if let Some(s) = self.grains.get(slot) {
            s.store(pack_grain(snapshot), Ordering::Relaxed);
        }
    }

    /// Audio-thread clear of every slot from `from` up — inactive slots
    /// must read as inactive (active bit 0).
    #[inline]
    pub fn clear_grains_from(&self, from: usize) {
        for s in self.grains.iter().skip(from) {
            s.store(0, Ordering::Relaxed);
        }
    }

    /// Audio-thread store of the coarse buffer-peak bins: `bins` are
    /// absolute peaks indexed by fixed ring position, `head_bin` is the
    /// bin the write head is in and `bin_ms` the buffer time per bin.
    pub fn store_peaks(&self, bins: &[f32; PEAK_BINS], head_bin: usize, bin_ms: f32) {
        for (slot, &v) in self.peaks.iter().zip(bins.iter()) {
            slot.store(v.to_bits(), Ordering::Relaxed);
        }
        self.peak_head
            .store((head_bin % PEAK_BINS) as u32, Ordering::Relaxed);
        self.peak_bin_ms.store(bin_ms.to_bits(), Ordering::Relaxed);
    }

    /// Editor-side decode of the grain snapshot (ba todo #1135): fills
    /// `out` with the currently sounding grains and returns the count.
    /// Callers should reuse one buffer across frames (no allocation).
    pub fn read_grains(&self, out: &mut [GrainSnapshot; GRAIN_SLOTS]) -> usize {
        let mut n = 0;
        for slot in &self.grains {
            if let Some(g) = unpack_grain(slot.load(Ordering::Relaxed)) {
                out[n] = g;
                n += 1;
            }
        }
        n
    }

    /// Editor-side read of the coarse buffer peaks, ordered oldest →
    /// newest: `out[PEAK_BINS - 1]` is the bin containing the write
    /// head, `out[0]` the furthest-back (oldest) content. Returns the
    /// buffer time per bin in milliseconds (0 before the first block).
    pub fn read_peaks(&self, out: &mut [f32; PEAK_BINS]) -> f32 {
        let head = self.peak_head.load(Ordering::Relaxed) as usize % PEAK_BINS;
        for (i, slot) in out.iter_mut().enumerate() {
            let bin = (head + 1 + i) % PEAK_BINS;
            *slot = f32::from_bits(self.peaks[bin].load(Ordering::Relaxed));
        }
        f32::from_bits(self.peak_bin_ms.load(Ordering::Relaxed))
    }
}
