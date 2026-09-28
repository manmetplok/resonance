//! De-harsh stage: `resonance_dsp`'s [`ResonanceSuppressor`] between the
//! corrective EQ and the glue compressor
//! (`docs/design/deharsh-resonance-suppressor.md`).
//!
//! # Latency, and where it is spent
//!
//! The suppressor delays by one STFT frame ([`DeharshStage::latency`],
//! 2048 samples at 48 kHz), and the chain reports that on top of its
//! other stages in every state (a plugin cannot move its latency without
//! a restart, F2). Where the delay *sits* has two modes:
//!
//! * **Inline.** The stage outputs the suppressor, so everything after
//!   it runs on the delayed signal. Off is the suppressor's own
//!   bit-exact delay tap, and on/off crossfade over 10 ms.
//! * **Tail.** The stage is a wire and the delay is a plain delay line at
//!   the very end of the chain. Every other stage then runs on exactly
//!   the samples, at exactly the stream positions, it did before this
//!   stage existed. That includes the FIR hop grid, the dither RNG and
//!   the block timing of automation. So a project that never engages the
//!   stage renders as the pre-W12 chain delayed by exactly the latency,
//!   bit for bit. Inline cannot give that: delaying the downstream input
//!   moves every one of those alignments.
//!
//! The mode is picked on the first block after a reset: inline if
//! `dh_on`, tail otherwise. The first time the stage is switched on in
//! tail mode, it hands over to inline once and stays there:
//!
//! 1. The downstream input crossfades from the undelayed signal to the
//!    suppressor's (delayed) output over [`RAMP_MS`]. That is a splice,
//!    so for a moment the downstream stages see audio one frame "early".
//! 2. The tail keeps delaying until the splice has come out of the
//!    downstream stages (their latency) plus half a frame. It then
//!    crossfades from its delayed path to the direct one. Both paths
//!    carry the same timeline there, so the output timeline never jumps,
//!    and the switch happens halfway between the splice's appearance on
//!    the direct path and on the delayed one, so neither is heard.
//!
//! After that the stage is inline until the next reset. Switching it off
//! and on again is then a plain crossfade.

use resonance_dsp::{DelayLine, ResonanceSuppressor, SuppressorConfig};

/// Stage-input and tail crossfade length of the tail → inline handover.
pub const RAMP_MS: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Timing {
    /// Not decided yet (first block after a reset).
    Unprimed,
    /// Stage is a wire; the latency is a delay at the chain's end.
    Tail,
    /// Tail → inline handover in progress.
    Handover {
        /// Stage output weight of the suppressor path, 0 → 1.
        stage: f32,
        /// Samples until the tail starts its crossfade.
        countdown: usize,
        /// Tail output weight of the direct path, 0 → 1.
        tail: f32,
    },
    /// Stage outputs the suppressor; the tail is a wire.
    Inline,
}

pub struct DeharshStage {
    sup: ResonanceSuppressor,
    latency: usize,
    /// Latency of the stages after this one; the handover waits it out.
    downstream_latency: usize,
    timing: Timing,
    tail_l: DelayLine,
    tail_r: DelayLine,
    /// The suppressor runs on a copy: in tail mode its output is only
    /// kept warm, not used.
    scratch_l: Vec<f32>,
    scratch_r: Vec<f32>,
    ramp_step: f32,
}

impl DeharshStage {
    /// `max_buffer` bounds the block size [`Self::process_stage`] takes;
    /// `downstream_latency` is the latency of every stage after this one.
    pub fn new(sample_rate: f32, max_buffer: usize, downstream_latency: usize) -> Self {
        let mut sup = ResonanceSuppressor::new(sample_rate);
        let hop = sup.geometry().hop;
        // FFT stagger (DSP-16): run the frames at 3/4 of each hop. With
        // a 128-frame quantum at 48 kHz that is the odd callbacks, which
        // hold 4 of the 10 primary convolver slots and 4 of the 10 M/S
        // cross slots (the even ones hold 6 + 6). At a hop of 256 every
        // other callback runs a frame, so no phase avoids them all.
        sup.set_phase_offset(hop / 4 - 1);
        let latency = sup.latency();
        let max_buffer = max_buffer.max(1);
        Self {
            sup,
            latency,
            downstream_latency,
            timing: Timing::Unprimed,
            tail_l: DelayLine::new(latency + 1),
            tail_r: DelayLine::new(latency + 1),
            scratch_l: vec![0.0; max_buffer],
            scratch_r: vec![0.0; max_buffer],
            ramp_step: 1.0 / (RAMP_MS * 1e-3 * sample_rate).max(1.0),
        }
    }

    /// Constant latency in samples, whatever the config or timing mode.
    pub fn latency(&self) -> usize {
        self.latency
    }

    pub fn timing(&self) -> Timing {
        self.timing
    }

    /// Deepest current cut, dB (for meters).
    pub fn max_cut_db(&self) -> f32 {
        self.sup.max_cut_db()
    }

    pub fn reset(&mut self) {
        self.sup.reset();
        self.tail_l.clear();
        self.tail_r.clear();
        self.timing = Timing::Unprimed;
    }

    /// The stage proper, at its place in the chain. Blocks must be at
    /// most `max_buffer` frames.
    pub fn process_stage(&mut self, left: &mut [f32], right: &mut [f32], cfg: &SuppressorConfig) {
        let frames = left.len().min(right.len()).min(self.scratch_l.len());
        if self.timing == Timing::Unprimed {
            self.timing = if cfg.enabled { Timing::Inline } else { Timing::Tail };
        }
        if self.timing == Timing::Tail && cfg.enabled {
            self.timing = Timing::Handover {
                stage: 0.0,
                countdown: self.downstream_latency + self.latency / 2,
                tail: 0.0,
            };
        }

        let (sl, sr) = (&mut self.scratch_l[..frames], &mut self.scratch_r[..frames]);
        sl.copy_from_slice(&left[..frames]);
        sr.copy_from_slice(&right[..frames]);
        self.sup.process_stereo(sl, sr, cfg);

        match &mut self.timing {
            Timing::Tail | Timing::Unprimed => {}
            Timing::Inline => {
                left[..frames].copy_from_slice(sl);
                right[..frames].copy_from_slice(sr);
            }
            Timing::Handover { stage, .. } => {
                for i in 0..frames {
                    if *stage < 1.0 {
                        *stage = (*stage + self.ramp_step).min(1.0);
                    }
                    let g = *stage;
                    left[i] += g * (sl[i] - left[i]);
                    right[i] += g * (sr[i] - right[i]);
                }
            }
        }
    }

    /// The tail delay, at the very end of the chain (after dither).
    pub fn process_tail(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        let lat = self.latency;
        let step = self.ramp_step;
        match self.timing {
            Timing::Inline | Timing::Unprimed => {}
            Timing::Tail => {
                for i in 0..frames {
                    self.tail_l.push(left[i]);
                    self.tail_r.push(right[i]);
                    left[i] = self.tail_l.tap(lat);
                    right[i] = self.tail_r.tap(lat);
                }
            }
            Timing::Handover {
                stage,
                mut countdown,
                mut tail,
            } => {
                for i in 0..frames {
                    self.tail_l.push(left[i]);
                    self.tail_r.push(right[i]);
                    let (dl, dr) = (self.tail_l.tap(lat), self.tail_r.tap(lat));
                    if countdown > 0 {
                        countdown -= 1;
                    } else if tail < 1.0 {
                        tail = (tail + step).min(1.0);
                    }
                    left[i] = dl + tail * (left[i] - dl);
                    right[i] = dr + tail * (right[i] - dr);
                }
                self.timing = if tail >= 1.0 && stage >= 1.0 {
                    Timing::Inline
                } else {
                    Timing::Handover {
                        stage,
                        countdown,
                        tail,
                    }
                };
            }
        }
    }
}
