//! Linear-phase lowpass used to build the multiband crossover network.
//!
//! A [`StereoFir`] driven with a cascade of parametric LowPass bands to
//! produce a 24 dB/oct-equivalent rolloff with exact linear-phase
//! reconstruction. It gets the EQ's treatment (FU-M2b / DSP-12): the FIR
//! length scales with the sample rate, a cutoff move is designed off the
//! audio thread (inline fallback, identical output) and crossfaded in
//! over one hop instead of swapped hard, at most once per hop, and
//! nothing on the audio path allocates.

use std::sync::Arc;

use crate::stages::linear_phase_eq::{
    BandConfig, BandType, DesignWorker, FirGeometry, StereoFir, NUM_BANDS,
};

/// Number of cascaded 12 dB/oct biquad sections. Two sections give a
/// ~24 dB/oct slope — the classic LR4 choice for a mastering multiband
/// crossover.
const CASCADE_ORDER: usize = 2;
const _: () = assert!(CASCADE_ORDER <= NUM_BANDS);

/// Smallest cutoff move that triggers a redesign.
const CUTOFF_EPSILON_HZ: f32 = 0.5;

pub struct LinearPhaseLowpass {
    /// Cutoff of the current (or pending) filter.
    cutoff_hz: f32,
    fir: StereoFir,
}

impl LinearPhaseLowpass {
    /// A lowpass with its own design worker thread.
    pub fn new(sample_rate: f32, cutoff_hz: f32) -> Self {
        Self::with_worker(sample_rate, cutoff_hz, Some(&DesignWorker::spawn()))
    }

    /// A lowpass designing through `worker`, or always inline with
    /// `None`. The output is identical either way.
    pub fn with_worker(
        sample_rate: f32,
        cutoff_hz: f32,
        worker: Option<&Arc<DesignWorker>>,
    ) -> Self {
        let mut fir = StereoFir::new(sample_rate, worker);
        fir.design_now(&cascade(cutoff_hz));
        Self { cutoff_hz, fir }
    }

    /// Move the cutoff. Allocation-free; the new filter crossfades in on
    /// the next hop boundary. While a move is pending, further moves
    /// wait for it to land (call again on a later block).
    pub fn set_cutoff(&mut self, cutoff_hz: f32) {
        if (self.cutoff_hz - cutoff_hz).abs() > CUTOFF_EPSILON_HZ
            && self.fir.request(&cascade(cutoff_hz))
        {
            self.cutoff_hz = cutoff_hz;
        }
    }

    pub fn reset(&mut self) {
        self.fir.reset();
    }

    /// Convolver latency in samples (identical for both channels);
    /// scales with the sample rate.
    pub fn latency(&self) -> usize {
        self.fir.latency()
    }

    /// [`Self::latency`] of a lowpass built for `sample_rate`.
    pub fn latency_for(sample_rate: f32) -> usize {
        FirGeometry::for_sample_rate(sample_rate).latency()
    }

    /// Stagger the channels' FFT iterations (see
    /// [`StereoFir::set_phase_offsets`]).
    pub fn set_phase_offsets(&mut self, offsets: [usize; 2]) {
        self.fir.set_phase_offsets(offsets);
    }

    /// Samples until each channel's next FFT iteration.
    pub fn iteration_countdowns(&self) -> [usize; 2] {
        self.fir.iteration_countdowns()
    }

    /// Designs taken from the worker vs. designed inline (diagnostics).
    pub fn design_counts(&self) -> (u64, u64) {
        self.fir.design_counts()
    }

    /// Process a stereo block in place. After the call, `left[i]` holds
    /// the lowpass output corresponding to the input that arrived
    /// `latency()` samples earlier.
    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.fir.process(left, right);
    }
}

/// `CASCADE_ORDER` LowPass bands at `cutoff_hz` (LR4 at order 2), built
/// on the stack — this used to be a `Vec` per redesign.
fn cascade(cutoff_hz: f32) -> [BandConfig; NUM_BANDS] {
    std::array::from_fn(|i| BandConfig {
        enabled: i < CASCADE_ORDER,
        band_type: BandType::LowPass,
        freq_hz: cutoff_hz,
        q: 0.707,
        gain_db: 0.0,
    })
}
