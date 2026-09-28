//! Linear-phase parametric EQ stage.
//!
//! The engine is shared between the corrective and tonal EQ slots in
//! the mastering chain. Parameters specify a chain of parametric biquad
//! bands (bell / shelf / cut); the magnitude response of that chain is
//! sampled on an FFT grid and the corresponding zero-phase symmetric FIR
//! is fed to the overlap-save convolver.
//!
//! A band parameter change redesigns the FIR and crossfades to it over
//! one convolver hop (DSP-10), so an automated band morphs instead of
//! stepping at hop boundaries. The design runs on a background worker
//! and lands on the next hop boundary, falling back to an identical
//! inline design if the worker is late, so the output never depends on
//! thread timing (FU-M2a, see [`worker`]). Redesigns are rate-limited
//! to one per hop: while one waits for its boundary, further changes
//! wait and the newest settings are designed next.
//!
//! # Mid/side bands
//!
//! Each band filters the stereo pair, the mid or the side
//! ([`MsMode`]). The stereo result is `L' = A·L + B·R`, `R' = A·R + B·L`
//! (see [`design`]): the usual pair of convolvers carries `A`, and a
//! second, *cross* pair carries `B` on the opposite channel. `B` is zero
//! while every band is on `Stereo`, so the cross pair only runs while a
//! band needs it:
//!
//! - **Engage.** The first mid/side band starts the cross pair from
//!   silence with a zero filter and feeds it for two hops before the new
//!   design is requested, so its overlap-save history is real input by
//!   the time `B` crossfades in. The band change waits those ~170 ms.
//! - **Release.** When the last mid/side band goes back to `Stereo`,
//!   `B` crossfades to zero and the pair keeps running until that has
//!   drained, then stops. From then on the output is the plain pair's
//!   again, bit-for-bit the path that never engaged.
//!
//! The cross pair has the same geometry, so the latency is the same
//! whether it runs or not. It also runs on the direct pair's hop grid
//! (each cross convolver iterates on its direct twin's sample), so the
//! two halves of a band change crossfade over the same samples.

pub mod band;
pub mod convolver;
pub mod design;
pub mod worker;

pub use band::{BandConfig, BandType, MsMode};
pub use convolver::{FirGeometry, OverlapSaveConvolver, FIR_LENGTH, GROUP_DELAY, HOP_SIZE};
pub use design::{FirDesigner, FirPart};
pub use worker::{DesignWorker, SpectrumDesigner, StereoFir};

use std::sync::Arc;

/// Number of parametric bands exposed by the plugin per EQ instance.
/// Phase 3 ships with four bands; the chain can grow later without
/// touching the convolver or designer — they're band-count-agnostic.
pub const NUM_BANDS: usize = 4;

/// Frames per pass of the cross pair's scratch (see
/// [`LinearPhaseEq::process_stereo`]); longer blocks run in chunks.
const CROSS_CHUNK: usize = 256;

/// Where the mid/side cross pair is in its life cycle (module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cross {
    /// Not running; the output is the plain pair's.
    Off,
    /// Running with a zero filter until its history is real input.
    Warming { remaining: usize },
    /// Running, its output added.
    On,
    /// Its filter is back at zero; running until the tail has drained.
    Draining { remaining: usize },
}

/// Stereo linear-phase parametric EQ.
///
/// A [`StereoFir`] (two convolvers plus the off-thread designer), the
/// mid/side cross pair, and a cached snapshot of the band parameters the
/// current filter was designed for. Any difference between the supplied
/// `bands` and the cache requests a redesign on the next
/// `process_stereo` call.
pub struct LinearPhaseEq {
    fir: StereoFir,
    /// `B = (Hm − Hs)/2`, fed the opposite channel (module docs).
    cross: StereoFir,
    cross_state: Cross,
    /// One hop of the geometry, the unit of the warm-up and drain.
    hop: usize,
    /// Cross-pair scratch: the right input on its way to the left
    /// output, and vice versa.
    cross_l: Box<[f32; CROSS_CHUNK]>,
    cross_r: Box<[f32; CROSS_CHUNK]>,
    /// Band parameters of the current (or pending) FIR. Compared on
    /// every `process_stereo` to decide whether to redesign.
    cached_bands: [BandConfig; NUM_BANDS],
}

impl LinearPhaseEq {
    /// An EQ with its own design worker thread.
    pub fn new(sample_rate: f32) -> Self {
        Self::with_worker(sample_rate, Some(&DesignWorker::spawn()))
    }

    /// An EQ designing through `worker` (shared with other filters), or
    /// always inline with `None`. The output is identical either way.
    pub fn with_worker(sample_rate: f32, worker: Option<&Arc<DesignWorker>>) -> Self {
        // FIR length scales with the rate so the low bands keep their
        // resolution (DSP-06).
        let fir = StereoFir::new(sample_rate, worker);
        let mut cross = StereoFir::with_part(sample_rate, worker, FirPart::Cross);
        // The cross filter of an all-stereo set is exactly zero.
        cross.design_now(&[BandConfig::off(); NUM_BANDS]);
        let hop = fir.geometry().hop;
        Self {
            fir,
            cross,
            cross_state: Cross::Off,
            hop,
            cross_l: Box::new([0.0; CROSS_CHUNK]),
            cross_r: Box::new([0.0; CROSS_CHUNK]),
            cached_bands: [BandConfig::off(); NUM_BANDS],
        }
    }

    pub fn reset(&mut self) {
        self.fir.reset();
        self.align_cross_grid();
        if let Cross::Warming { remaining } = &mut self.cross_state {
            *remaining = 2 * self.hop;
        }
    }

    /// Reported per-channel latency. Same for both channels; constant
    /// in ms across sample rates, and the same whether or not a band is
    /// mid/side.
    pub fn latency(&self) -> usize {
        self.fir.latency()
    }

    /// Designs taken from the worker vs. designed inline (diagnostics).
    pub fn design_counts(&self) -> (u64, u64) {
        self.fir.design_counts()
    }

    /// Stagger the channels' FFT iterations (see
    /// [`StereoFir::set_phase_offsets`]). The cross pair follows.
    pub fn set_phase_offsets(&mut self, offsets: [usize; 2]) {
        self.fir.set_phase_offsets(offsets);
        self.align_cross_grid();
    }

    /// Put the cross pair on the direct pair's hop grid, clearing its
    /// streaming state: each cross convolver iterates on the same sample
    /// as its direct twin, so a band change crossfades `A` and `B` over
    /// the same samples and `A + B` is the designed filter at every one
    /// of them. On a grid of its own the two halves landed up to a hop
    /// apart, and a side band's change leaked into the mono sum for that
    /// long (review M1). The price: while a band is mid/side, each cross
    /// convolver spends its FFT in the same callback as its twin.
    fn align_cross_grid(&mut self) {
        let hop = self.hop;
        let [l, r] = self.fir.iteration_countdowns();
        // A convolver with phase offset `p` iterates after `hop − p`.
        self.cross.set_phase_offsets([(hop - l) % hop, (hop - r) % hop]);
    }

    /// Samples until each cross channel's next FFT iteration
    /// (diagnostics: equal to [`Self::iteration_countdowns`] whenever
    /// the cross pair runs).
    pub fn cross_iteration_countdowns(&self) -> [usize; 2] {
        self.cross.iteration_countdowns()
    }

    /// Samples until each channel's next FFT iteration.
    pub fn iteration_countdowns(&self) -> [usize; 2] {
        self.fir.iteration_countdowns()
    }

    /// True while the mid/side cross pair is running (diagnostics).
    pub fn cross_active(&self) -> bool {
        self.cross_state != Cross::Off
    }

    /// Process one stereo block in place, requesting a redesign first if
    /// any band parameter has changed since the last one.
    pub fn process_stereo(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        bands: &[BandConfig; NUM_BANDS],
    ) {
        if self.cross_state == Cross::Off && bands.iter().any(BandConfig::is_ms) {
            // Start the cross pair from silence, on the direct pair's
            // grid; the design waits until its history is real input.
            self.align_cross_grid();
            self.cross_state = Cross::Warming {
                remaining: 2 * self.hop,
            };
        }

        // At most one redesign per hop: while one is pending the newest
        // settings wait (they differ from `cached_bands`, so they are
        // picked up on the first block after it lands). The two pairs
        // move together, so neither may be pending.
        let warming = matches!(self.cross_state, Cross::Warming { .. });
        if *bands != self.cached_bands
            && !warming
            && !self.fir.is_pending()
            && !self.cross.is_pending()
        {
            self.fir.request(bands);
            if self.cross_state != Cross::Off {
                self.cross.request(bands);
            }
            self.cached_bands = *bands;
        }

        let designed_ms = self.cached_bands.iter().any(BandConfig::is_ms);
        self.cross_state = match self.cross_state {
            Cross::On if !designed_ms && !self.cross.is_pending() => Cross::Draining {
                // One hop for the later channel's crossfade to land, one
                // for its transition output, one of margin.
                remaining: 3 * self.hop,
            },
            Cross::Draining { .. } if designed_ms => Cross::On,
            s => s,
        };

        if self.cross_state == Cross::Off {
            self.fir.process(left, right);
            return;
        }

        let add = matches!(self.cross_state, Cross::On | Cross::Draining { .. });
        let n = left.len().min(right.len());
        let mut start = 0;
        while start < n {
            let end = (start + CROSS_CHUNK).min(n);
            let m = end - start;
            let (l, r) = (&mut left[start..end], &mut right[start..end]);
            self.cross_l[..m].copy_from_slice(r);
            self.cross_r[..m].copy_from_slice(l);
            self.cross
                .process(&mut self.cross_l[..m], &mut self.cross_r[..m]);
            self.fir.process(l, r);
            if add {
                for i in 0..m {
                    l[i] += self.cross_l[i];
                    r[i] += self.cross_r[i];
                }
            }
            start = end;
        }

        self.cross_state = match self.cross_state {
            Cross::Warming { remaining } if remaining > n => Cross::Warming {
                remaining: remaining - n,
            },
            Cross::Warming { .. } => Cross::On,
            Cross::Draining { remaining } if remaining > n => Cross::Draining {
                remaining: remaining - n,
            },
            Cross::Draining { .. } => Cross::Off,
            s => s,
        };
    }
}
