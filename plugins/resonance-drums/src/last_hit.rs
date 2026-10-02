//! What the sampler last played, published for the editor (K5).
//!
//! The editor's pad grid lights a cell on each hit, its inspector draws
//! the take a pad *last played* (not a fixed one), and its status bar
//! reads "Snare v98 → layer 5/7 · take 2/3". All three come from here:
//! on every hit `DrumSampler::note_on` packs the hit into one `u64` and
//! stores it twice — into the pad's slot and into `latest` — with a plain
//! atomic store each. Nothing on the audio side allocates, locks or
//! waits ([`LastHits::publish`]).
//!
//! Each hit carries a sequence number (wrapping, never zero), so a reader
//! can tell a new hit from the same value stored again — two hits on a
//! one-layer, one-take pad at the same velocity would otherwise look
//! identical, and the cell would not light the second time.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::drum_map::NUM_PADS;

/// One hit as the sampler played it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastHit {
    /// Which pad slot fired.
    pub pad: usize,
    /// The hit's MIDI velocity (1..=127) as it struck the pad: after
    /// velocity humanize, before the velocity curve shaped it into a
    /// layer pick.
    pub velocity: u8,
    /// Zero-based velocity layer of the pad's reference bank that played,
    /// and how many layers that bank has.
    pub layer: usize,
    pub layers: usize,
    /// Zero-based round-robin take that played, and how many the layer
    /// holds.
    pub take: usize,
    pub takes: usize,
    /// The hit's sequence number: wraps, never zero. Differs from the
    /// previous hit's, so a reader sees every new hit as new.
    pub seq: u16,
}

impl LastHit {
    /// `"layer 5/7 · take 2/3"`: one-based.
    pub fn cell_text(&self) -> String {
        format!(
            "layer {}/{} · take {}/{}",
            self.layer + 1,
            self.layers,
            self.take + 1,
            self.takes
        )
    }
}

/// Pack `hit` into one word. Every field saturates at its width (8 bits,
/// 16 for `seq`); `seq` 0 is reserved for "no hit yet".
pub fn pack(hit: &LastHit) -> u64 {
    let b = |v: usize| v.min(u8::MAX as usize) as u64;
    b(hit.takes)
        | b(hit.take) << 8
        | b(hit.layers) << 16
        | b(hit.layer) << 24
        | (hit.velocity as u64) << 32
        | b(hit.pad) << 40
        | (hit.seq.max(1) as u64) << 48
}

/// Unpack a published word. `None` for "no hit yet".
pub fn unpack(raw: u64) -> Option<LastHit> {
    let seq = (raw >> 48) as u16;
    if seq == 0 {
        return None;
    }
    let byte = |shift: u32| ((raw >> shift) & 0xff) as usize;
    let takes = byte(0).max(1);
    let layers = byte(16).max(1);
    Some(LastHit {
        pad: byte(40).min(NUM_PADS - 1),
        velocity: byte(32) as u8,
        layer: byte(24).min(layers - 1),
        layers,
        take: byte(8).min(takes - 1),
        takes,
        seq,
    })
}

/// The shared slots: one per pad, and the most recent hit on any pad.
#[derive(Debug)]
pub struct LastHits {
    pads: [AtomicU64; NUM_PADS],
    latest: AtomicU64,
}

impl Default for LastHits {
    fn default() -> Self {
        Self {
            pads: std::array::from_fn(|_| AtomicU64::new(0)),
            latest: AtomicU64::new(0),
        }
    }
}

impl LastHits {
    /// Publish `hit`. Audio-thread safe: two relaxed stores.
    pub fn publish(&self, hit: &LastHit) {
        let packed = pack(hit);
        if let Some(slot) = self.pads.get(hit.pad) {
            slot.store(packed, Ordering::Relaxed);
        }
        self.latest.store(packed, Ordering::Relaxed);
    }

    /// The last hit on `pad`, if it has been hit.
    pub fn pad(&self, pad: usize) -> Option<LastHit> {
        unpack(self.pads.get(pad)?.load(Ordering::Relaxed))
    }

    /// The most recent hit on any pad.
    pub fn latest(&self) -> Option<LastHit> {
        unpack(self.latest.load(Ordering::Relaxed))
    }
}

/// Velocity 0..1 → the MIDI step it stands for (1..=127).
pub fn midi_velocity(velocity: f32) -> u8 {
    (velocity.clamp(0.0, 1.0) * 127.0).round().clamp(1.0, 127.0) as u8
}
