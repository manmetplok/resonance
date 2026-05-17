//! Linear-phase lowpass used to build the multiband crossover network.
//!
//! Wraps two [`OverlapSaveConvolver`] instances (one per channel) and a
//! [`FirDesigner`] that's driven with a cascade of parametric LowPass
//! bands to produce a 24 dB/oct-equivalent rolloff with exact
//! linear-phase reconstruction. Recomputes the FIR whenever the cutoff
//! changes, and otherwise just runs the convolution per sample.

use crate::stages::linear_phase_eq::{
    BandConfig, BandType, FirDesigner, OverlapSaveConvolver, GROUP_DELAY, HOP_SIZE,
};

/// Number of cascaded 12 dB/oct biquad sections. Two sections give a
/// ~24 dB/oct slope — the classic LR4 choice for a mastering multiband
/// crossover.
const CASCADE_ORDER: usize = 2;

pub struct LinearPhaseLowpass {
    sample_rate: f32,
    cutoff_hz: f32,
    designer: FirDesigner,
    left: OverlapSaveConvolver,
    right: OverlapSaveConvolver,
}

impl LinearPhaseLowpass {
    pub fn new(sample_rate: f32, cutoff_hz: f32) -> Self {
        let mut s = Self {
            sample_rate,
            cutoff_hz,
            designer: FirDesigner::new(),
            left: OverlapSaveConvolver::new(),
            right: OverlapSaveConvolver::new(),
        };
        s.redesign();
        s
    }

    pub fn set_cutoff(&mut self, cutoff_hz: f32) {
        if (self.cutoff_hz - cutoff_hz).abs() > 0.5 {
            self.cutoff_hz = cutoff_hz;
            self.redesign();
        }
    }

    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }

    /// Convolver latency in samples (identical for both channels).
    pub const fn latency() -> usize {
        GROUP_DELAY + HOP_SIZE
    }

    /// Process a stereo block in place. After the call, `left[i]` holds
    /// the lowpass output corresponding to the input that arrived
    /// `latency()` samples earlier.
    pub fn process_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.left.process_in_place(left);
        self.right.process_in_place(right);
    }

    fn redesign(&mut self) {
        // Cascade CASCADE_ORDER LowPass bands at the same cutoff so the
        // rolloff is CASCADE_ORDER × 12 dB/oct (LR4 at order=2).
        let mut bands = Vec::with_capacity(CASCADE_ORDER);
        for _ in 0..CASCADE_ORDER {
            bands.push(BandConfig {
                enabled: true,
                band_type: BandType::LowPass,
                freq_hz: self.cutoff_hz,
                q: 0.707,
                gain_db: 0.0,
            });
        }
        let h = self.designer.design(&bands, self.sample_rate);
        self.left.set_impulse_response(h);
        self.right.set_impulse_response(h);
    }
}

