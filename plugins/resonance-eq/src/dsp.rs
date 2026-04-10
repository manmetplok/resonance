//! Per-channel cascade state and the top-level EQ process loop.
//!
//! Coefficient updates are pulled from the live `EqParams` once per audio
//! block. Stage state (z1/z2) is preserved across updates so sweeping a
//! band doesn't click. The actual per-sample arithmetic is a simple two
//! channels × 8 bands × up-to-4 stages biquad cascade plus a trailing
//! output gain.

use resonance_dsp::Biquad;
use resonance_plugin::Smoother;

use crate::band::{configure_stages, MAX_STAGES_PER_BAND};
use crate::params::{BandSnapshot, EqParams, NUM_BANDS};

pub struct EqDsp {
    sample_rate: f32,
    /// Per-channel cascade: [channel][band][stage].
    channels: [[[Biquad; MAX_STAGES_PER_BAND]; NUM_BANDS]; 2],
    /// How many stages of each band are actually in use (same for both channels).
    active_stages: [usize; NUM_BANDS],
    /// Last-applied snapshots, used to skip coefficient work when nothing changed.
    last_snapshot: [Option<BandSnapshot>; NUM_BANDS],
}

impl EqDsp {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            channels: [[[Biquad::identity(); MAX_STAGES_PER_BAND]; NUM_BANDS]; 2],
            active_stages: [0; NUM_BANDS],
            last_snapshot: [None; NUM_BANDS],
        }
    }

    pub fn clear_state(&mut self) {
        for ch in self.channels.iter_mut() {
            for band in ch.iter_mut() {
                for stage in band.iter_mut() {
                    stage.reset();
                }
            }
        }
    }

    /// Refresh coefficients from the current parameter values for any band
    /// whose snapshot has changed since the last call. Called once per block.
    pub fn update_from_params(&mut self, params: &EqParams) {
        for (i, band) in params.bands.iter().enumerate() {
            let snapshot = band.snapshot();
            let changed = match self.last_snapshot[i] {
                Some(prev) => !snapshots_equal(&prev, &snapshot),
                None => true,
            };
            if changed {
                // Write coefficients into both channels (L and R share coeffs
                // but carry independent delay-line state).
                let n = configure_stages(&snapshot, self.sample_rate, &mut self.channels[0][i]);
                let _ = configure_stages(&snapshot, self.sample_rate, &mut self.channels[1][i]);
                self.active_stages[i] = n;
                self.last_snapshot[i] = Some(snapshot);
            }
        }
    }

    /// Process a stereo block in-place. `output_gain` is a smoother ramped
    /// per sample to avoid audible zippering when the output knob moves.
    pub fn process_stereo(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        output_gain: &mut Smoother,
    ) {
        let frames = left.len().min(right.len());
        for i in 0..frames {
            let mut l = left[i];
            let mut r = right[i];
            for b in 0..NUM_BANDS {
                let n = self.active_stages[b];
                for s in 0..n {
                    l = self.channels[0][b][s].process(l);
                    r = self.channels[1][b][s].process(r);
                }
            }
            let gain_db = output_gain.next();
            let gain_lin = db_to_linear(gain_db);
            left[i] = l * gain_lin;
            right[i] = r * gain_lin;
        }
    }
}

fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

fn snapshots_equal(a: &BandSnapshot, b: &BandSnapshot) -> bool {
    a.enabled == b.enabled
        && a.kind == b.kind
        && a.slope == b.slope
        && (a.freq - b.freq).abs() < 1e-4
        && (a.gain_db - b.gain_db).abs() < 1e-4
        && (a.q - b.q).abs() < 1e-4
}
