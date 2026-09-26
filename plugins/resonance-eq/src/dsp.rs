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
    /// The stages a band ran *before* its last kind change, still on their
    /// old coefficients and state, crossfaded out while the restarted new
    /// stages fade in (FU-M6c): [channel][band][stage].
    fade_stages: [[[Biquad; MAX_STAGES_PER_BAND]; NUM_BANDS]; 2],
    /// Active stage count of `fade_stages` per band.
    fade_active: [usize; NUM_BANDS],
    /// Samples left in each band's kind-change crossfade; 0 = none running.
    fade_remaining: [u32; NUM_BANDS],
    /// Crossfade length in samples (~5 ms at the current rate).
    fade_len: u32,
}

/// Length of the crossfade a band runs when its kind changes. Long enough
/// that the old-to-new difference (up to the signal's own size) is spread
/// into a ramp well under the steady tone's own slope; short enough to read
/// as an instant switch.
const KIND_FADE_MS: f32 = 5.0;

impl EqDsp {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            channels: [[[Biquad::identity(); MAX_STAGES_PER_BAND]; NUM_BANDS]; 2],
            active_stages: [0; NUM_BANDS],
            last_snapshot: [None; NUM_BANDS],
            fade_stages: [[[Biquad::identity(); MAX_STAGES_PER_BAND]; NUM_BANDS]; 2],
            fade_active: [0; NUM_BANDS],
            fade_remaining: [0; NUM_BANDS],
            fade_len: ((KIND_FADE_MS * 0.001 * sample_rate).round() as u32).max(1),
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
        self.fade_remaining = [0; NUM_BANDS];
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
                // A kind change restarts the band's stages from zero (below),
                // which on loud material is a click. Keep the old stages
                // running as they are and crossfade out of them. Only when
                // they were producing something: from a bypassed band there
                // is nothing to fade from. A second change mid-fade starts
                // over from the current (new) stages.
                let kind_changed = self.last_snapshot[i].is_some_and(|p| p.kind != snapshot.kind);
                let prev_n = self.active_stages[i];
                if kind_changed && prev_n > 0 {
                    for ch in 0..2 {
                        self.fade_stages[ch][i] = self.channels[ch][i];
                    }
                    self.fade_active[i] = prev_n;
                    self.fade_remaining[i] = self.fade_len;
                }
                // Write coefficients into both channels (L and R share coeffs
                // but carry independent delay-line state).
                let n = configure_stages(&snapshot, self.sample_rate, &mut self.channels[0][i]);
                let _ = configure_stages(&snapshot, self.sample_rate, &mut self.channels[1][i]);
                // Stages that were not running hold z1/z2 frozen from
                // another input (and maybe other coefficients); TDF-II
                // injects that straight into the output. Start them clean.
                // A kind change repurposes every stage, so clear them all.
                let fresh_from = if kind_changed { 0 } else { prev_n.min(n) };
                for ch in self.channels.iter_mut() {
                    for stage in &mut ch[i][fresh_from..n] {
                        stage.reset();
                    }
                }
                self.active_stages[i] = n;
                self.last_snapshot[i] = Some(snapshot);
            }
        }
    }

    /// Process a stereo block in-place. `output_gain` is a smoother over the
    /// *linear* output gain (the caller converts its dB param to linear once
    /// when retargeting), ramped per sample to avoid audible zippering when
    /// the output knob moves.
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
                let (in_l, in_r) = (l, r);
                let n = self.active_stages[b];
                for s in 0..n {
                    l = self.channels[0][b][s].process(l);
                    r = self.channels[1][b][s].process(r);
                }
                let left_in_fade = self.fade_remaining[b];
                if left_in_fade > 0 {
                    let (mut ol, mut or) = (in_l, in_r);
                    for s in 0..self.fade_active[b] {
                        ol = self.fade_stages[0][b][s].process(ol);
                        or = self.fade_stages[1][b][s].process(or);
                    }
                    // Weight of the old stages: 1 -> 0 over the fade.
                    let w = left_in_fade as f32 / self.fade_len as f32;
                    l += w * (ol - l);
                    r += w * (or - r);
                    self.fade_remaining[b] = left_in_fade - 1;
                }
            }
            let gain_lin = output_gain.next();
            left[i] = l * gain_lin;
            right[i] = r * gain_lin;
        }
    }
}

fn snapshots_equal(a: &BandSnapshot, b: &BandSnapshot) -> bool {
    a.enabled == b.enabled
        && a.kind == b.kind
        && a.slope == b.slope
        && (a.freq - b.freq).abs() < 1e-4
        && (a.gain_db - b.gain_db).abs() < 1e-4
        && (a.q - b.q).abs() < 1e-4
}
