//! Phase-vocoder time-stretching (STFT, per-bin phase propagation).
//!
//! Best for sustained / harmonic material.

use std::sync::Arc;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use super::{Fifo, Ola, FRAME, SYNTH_HOP};
use crate::window::hann_window;

/// Phase-vocoder stretcher state.
pub struct PhaseVocoder {
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    frame_buf: Vec<f32>,
    /// Per-bin analysis magnitude and phase for the current frame.
    mag: Vec<f32>,
    anal_phase: Vec<f32>,
    /// Previous analysis phase per bin.
    last_phase: Vec<f32>,
    /// Accumulated (per-bin) synthesis phase.
    sum_phase: Vec<f32>,
    /// Spectral-peak bins of the current frame (for identity phase
    /// locking), reused across frames.
    peaks: Vec<usize>,
    ola: Ola,
    /// Fractional start of the next analysis frame, relative to the input
    /// FIFO front. Fractional so the average analysis hop is exact (no
    /// per-frame rounding drift in the output length).
    cursor: f64,
}

impl PhaseVocoder {
    pub fn new() -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FRAME);
        let ifft = planner.plan_fft_inverse(FRAME);
        let scratch_len = fft
            .get_inplace_scratch_len()
            .max(ifft.get_inplace_scratch_len());
        Self {
            fft,
            ifft,
            window: hann_window(FRAME),
            spectrum: vec![Complex::new(0.0, 0.0); FRAME],
            scratch: vec![Complex::new(0.0, 0.0); scratch_len],
            frame_buf: vec![0.0; FRAME],
            mag: vec![0.0; FRAME / 2 + 1],
            anal_phase: vec![0.0; FRAME / 2 + 1],
            last_phase: vec![0.0; FRAME / 2 + 1],
            sum_phase: vec![0.0; FRAME / 2 + 1],
            peaks: Vec::new(),
            ola: Ola::new(),
            cursor: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.last_phase.iter_mut().for_each(|p| *p = 0.0);
        self.sum_phase.iter_mut().for_each(|p| *p = 0.0);
        self.peaks.clear();
        self.ola.reset();
        self.cursor = 0.0;
    }

    pub fn run(&mut self, input: &mut Fifo, out: &mut Fifo, factor: f32) {
        let factor = factor as f64;
        let analysis_hop = (SYNTH_HOP as f64 / factor).max(1.0);
        let synth_hop = SYNTH_HOP as f64;
        let bins = FRAME / 2 + 1;

        // Consume whole frames while the input FIFO holds one.
        loop {
            let start = self.cursor.round() as usize;
            if start + FRAME > input.len() {
                break;
            }

            // Windowed analysis frame → spectrum.
            for j in 0..FRAME {
                self.spectrum[j] = Complex::new(input.get(start + j) * self.window[j], 0.0);
            }
            self.fft
                .process_with_scratch(&mut self.spectrum, &mut self.scratch);

            // Pass 1: magnitude/phase + standard per-bin phase propagation
            // (each bin's instantaneous frequency drives its accumulator).
            for b in 0..bins {
                let re = self.spectrum[b].re;
                let im = self.spectrum[b].im;
                self.mag[b] = (re * re + im * im).sqrt();
                let phase = im.atan2(re);
                self.anal_phase[b] = phase;

                let omega = std::f32::consts::TAU * b as f32 / FRAME as f32;
                let expected = omega * analysis_hop as f32;
                let delta = princ_arg(phase - self.last_phase[b] - expected);
                let true_freq = omega + delta / analysis_hop as f32;
                self.last_phase[b] = phase;
                self.sum_phase[b] = princ_arg(self.sum_phase[b] + true_freq * synth_hop as f32);
            }

            // Pass 2: identity phase locking (Laroche & Dolson). Lock each
            // bin's synthesis phase to its nearest spectral peak, keeping
            // the within-frame phase *relationships* around every peak.
            // This preserves vertical coherence so the windowed sinusoids
            // reconstruct at full amplitude under non-unity stretch — the
            // "phase-locked" requirement; a plain per-bin vocoder loses
            // gain badly here.
            find_peaks(&self.mag, &mut self.peaks);
            if self.peaks.is_empty() {
                for b in 0..bins {
                    self.spectrum[b] = Complex::from_polar(self.mag[b], self.sum_phase[b]);
                }
            } else {
                let mut pk = 0usize;
                for b in 0..bins {
                    // Advance to the nearest peak at or after `b` if it is
                    // closer than the current one.
                    while pk + 1 < self.peaks.len()
                        && self.peaks[pk + 1].abs_diff(b) <= self.peaks[pk].abs_diff(b)
                    {
                        pk += 1;
                    }
                    let p = self.peaks[pk];
                    let locked =
                        princ_arg(self.sum_phase[p] + (self.anal_phase[b] - self.anal_phase[p]));
                    self.spectrum[b] = Complex::from_polar(self.mag[b], locked);
                }
            }
            // Hermitian-symmetric upper half for a real inverse transform.
            for b in 1..bins - 1 {
                self.spectrum[FRAME - b] = self.spectrum[b].conj();
            }

            self.ifft
                .process_with_scratch(&mut self.spectrum, &mut self.scratch);
            let norm = 1.0 / FRAME as f32;
            for j in 0..FRAME {
                self.frame_buf[j] = self.spectrum[j].re * norm;
            }

            let frame = std::mem::take(&mut self.frame_buf);
            self.ola.add_frame(&frame, &self.window, synth_hop);
            self.frame_buf = frame;

            self.cursor += analysis_hop;
        }

        // Drop the input the analysis window has fully passed (everything
        // before the next frame's start) and emit settled output: future
        // frames start at or after `ola.pos`, so lower samples are final.
        let consume = (self.cursor.floor() as usize).min(input.len());
        if consume > 0 {
            input.consume(consume);
            self.cursor -= consume as f64;
        }
        let settled = self.ola.pos().floor() as usize;
        self.ola.drain_below(settled, out);
    }
}

/// Collect the spectral-peak bins of `mag` (local maxima over a ±2-bin
/// neighbourhood, above a tiny fraction of the frame's peak so noise-floor
/// ripple is ignored) into `peaks`, ascending. Used for identity phase
/// locking.
fn find_peaks(mag: &[f32], peaks: &mut Vec<usize>) {
    peaks.clear();
    let n = mag.len();
    if n == 0 {
        return;
    }
    let max = mag.iter().copied().fold(0.0f32, f32::max);
    let threshold = max * 1e-4;
    for b in 0..n {
        let m = mag[b];
        if m <= threshold {
            continue;
        }
        let lo = b.saturating_sub(2);
        let hi = (b + 2).min(n - 1);
        let is_peak = (lo..=hi).all(|k| k == b || mag[k] <= m);
        if is_peak {
            peaks.push(b);
        }
    }
}

/// Wrap a phase to the principal range (−π, π].
fn princ_arg(phase: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let mut p = phase;
    while p > PI {
        p -= TAU;
    }
    while p < -PI {
        p += TAU;
    }
    p
}
