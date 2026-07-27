//! Overlap-add accumulator for fractional-hop frames.
//!
//! This is shared by both the phase-vocoder and WSOLA stretchers.

use super::{Fifo, WEIGHT_EPS};

/// Fractional-hop overlap-add accumulator. Frames are added at rising
/// synthesis positions; each output sample is the windowed-frame sum
/// normalised by the accumulated window weight, so any (even fractional)
/// hop sequence reconstructs unity gain on stationary input.
pub struct Ola {
    signal: Vec<f32>,
    weight: Vec<f32>,
    /// Absolute synthesis index of `signal[0]` / `weight[0]`.
    base: usize,
    /// Fractional absolute position where the next frame is added.
    pos: f64,
    /// Absolute index up to which output has already been emitted.
    emitted: usize,
}

impl Ola {
    pub fn new() -> Self {
        Self {
            signal: Vec::new(),
            weight: Vec::new(),
            base: 0,
            pos: 0.0,
            emitted: 0,
        }
    }

    pub fn reset(&mut self) {
        self.signal.clear();
        self.weight.clear();
        self.base = 0;
        self.pos = 0.0;
        self.emitted = 0;
    }

    /// Get the current synthesis position (used to determine settled output).
    pub fn pos(&self) -> f64 {
        self.pos
    }

    /// Add `frame` (already windowed) weighted by `window`, at the
    /// current synthesis position, then advance the position by `hop`.
    pub fn add_frame(&mut self, frame: &[f32], window: &[f32], hop: f64) {
        let start = self.pos.round() as usize;
        let rel = start - self.base;
        let end = rel + frame.len();
        if end > self.signal.len() {
            self.signal.resize(end, 0.0);
            self.weight.resize(end, 0.0);
        }
        for j in 0..frame.len() {
            self.signal[rel + j] += frame[j] * window[j];
            self.weight[rel + j] += window[j] * window[j];
        }
        self.pos += hop;
    }

    /// Emit every sample below `up_to` (absolute index) that will receive
    /// no further contributions, normalising by accumulated weight.
    pub fn drain_below(&mut self, up_to: usize, out: &mut Fifo) {
        if up_to <= self.emitted {
            return;
        }
        let from_rel = self.emitted - self.base;
        let to_rel = (up_to - self.base).min(self.signal.len());
        let mut tmp = Vec::with_capacity(to_rel.saturating_sub(from_rel));
        for k in from_rel..to_rel {
            let w = self.weight[k];
            tmp.push(if w > WEIGHT_EPS { self.signal[k] / w } else { 0.0 });
        }
        out.push(&tmp);
        let drained = to_rel - from_rel;
        self.emitted += drained;
        // Compact the consumed prefix.
        if from_rel + drained > 0 {
            self.signal.drain(..to_rel);
            self.weight.drain(..to_rel);
            self.base += to_rel;
        }
    }
}
