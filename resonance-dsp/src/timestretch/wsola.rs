//! WSOLA time-stretching (waveform-similarity overlap-add).
//!
//! Preserves attacks on drums / percussive loops by matching frame transitions.

use super::{Fifo, Ola, FRAME, SYNTH_HOP, WSOLA_SEARCH};
use crate::window::hann_window;

/// WSOLA (waveform-similarity overlap-add) stretcher state.
pub struct Wsola {
    window: Vec<f32>,
    ola: Ola,
    /// Fractional nominal analysis position into the input FIFO.
    analysis_pos: f64,
    /// The "natural continuation" the next frame should resemble (the
    /// overlap region that would follow the previous chosen frame at the
    /// unscaled rate). Empty until the first frame is placed.
    target: Vec<f32>,
    frame_buf: Vec<f32>,
}

impl Wsola {
    pub fn new() -> Self {
        Self {
            window: hann_window(FRAME),
            ola: Ola::new(),
            analysis_pos: 0.0,
            target: Vec::new(),
            frame_buf: vec![0.0; FRAME],
        }
    }

    pub fn reset(&mut self) {
        self.ola.reset();
        self.analysis_pos = 0.0;
        self.target.clear();
    }

    pub fn run(&mut self, input: &mut Fifo, out: &mut Fifo, factor: f32) {
        let factor = factor as f64;
        let analysis_hop = (SYNTH_HOP as f64 / factor).max(1.0);
        let overlap = FRAME - SYNTH_HOP;

        loop {
            let nominal = self.analysis_pos.round() as usize;
            // The first frame is placed at δ=0 (no continuation to match
            // yet) and needs only a full frame of input; later frames
            // search ±WSOLA_SEARCH and so need that much extra lookahead.
            let delta = if self.target.is_empty() {
                if nominal + FRAME > input.len() {
                    break;
                }
                0
            } else {
                if nominal < WSOLA_SEARCH || nominal + WSOLA_SEARCH + FRAME > input.len() {
                    break;
                }
                best_offset(input, nominal, overlap, &self.target)
            };

            let start = (nominal as isize + delta).max(0) as usize;
            if start + FRAME > input.len() {
                break;
            }

            for j in 0..FRAME {
                self.frame_buf[j] = input.get(start + j);
            }
            let frame = std::mem::take(&mut self.frame_buf);
            self.ola.add_raw_frame(&frame, &self.window, SYNTH_HOP as f64);
            self.frame_buf = frame;

            // Natural continuation the next frame should resemble: what
            // follows the chosen frame after one synthesis hop. Always in
            // range given the loop's lookahead guard.
            let tgt_start = start + SYNTH_HOP;
            self.target.clear();
            for j in 0..overlap {
                let idx = tgt_start + j;
                self.target
                    .push(if idx < input.len() { input.get(idx) } else { 0.0 });
            }

            self.analysis_pos += analysis_hop;
        }

        // Drop input behind the search window, then emit settled output
        // (future frames start at or after `ola.pos`).
        let safe_consume = (self.analysis_pos.floor() as usize)
            .saturating_sub(WSOLA_SEARCH)
            .min(input.len());
        if safe_consume > 0 {
            input.consume(safe_consume);
            self.analysis_pos -= safe_consume as f64;
        }
        let settled = self.ola.pos().floor() as usize;
        self.ola.drain_below(settled, out);
    }
}

/// Find the offset `δ ∈ [-WSOLA_SEARCH, WSOLA_SEARCH]` that maximises the
/// normalised cross-correlation between `input[nominal+δ .. +overlap]` and
/// `target`. Returns `δ` (may be negative).
fn best_offset(input: &Fifo, nominal: usize, overlap: usize, target: &[f32]) -> isize {
    let len = overlap.min(target.len());
    if len == 0 {
        return 0;
    }
    let lo = -(WSOLA_SEARCH as isize).min(nominal as isize);
    let hi = WSOLA_SEARCH as isize;
    let mut best_delta = 0isize;
    let mut best_score = f32::NEG_INFINITY;
    let mut delta = lo;
    while delta <= hi {
        let start = nominal as isize + delta;
        if start < 0 || start as usize + len > input.len() {
            delta += 1;
            continue;
        }
        let start = start as usize;
        let mut dot = 0.0f32;
        let mut energy = 0.0f32;
        // Indexes both the FIFO (via `get`) and `target`; a zip would be
        // less clear than the shared index here.
        #[allow(clippy::needless_range_loop)]
        for j in 0..len {
            let s = input.get(start + j);
            dot += s * target[j];
            energy += s * s;
        }
        let score = dot / (energy.sqrt() + 1e-9);
        if score > best_score {
            best_score = score;
            best_delta = delta;
        }
        delta += 1;
    }
    best_delta
}
