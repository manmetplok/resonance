//! Late-tail Feedback Delay Network: 8-channel delay bank with
//! Householder mixing matrix, per-channel one-pole damping and decay
//! gain. The recirculating feedback never passes through the diffusion
//! network — that would multiply the Hadamard's non-unit broadband gain
//! into every cycle and crush the requested RT60.

use resonance_dsp::{DelayLine, Lfo, OnePole};

use super::modulation;
use super::CHANNELS;

/// Maximum FDN channel delay multiplier at the longest channel.
/// The classic `2^(c/(CHANNELS-1))` spread with `c = CHANNELS-1 = 7`
/// gives ~2.0 — a narrow range (1×..2×) that produces dense
/// feedback reflection pile-up instead of audibly separated echoes.
pub(super) const MAX_FDN_MULT: f32 = 2.0;

/// In-place Householder reflection: y[i] = x[i] - (2/N) * sum(x).
pub(super) fn householder_in_place(data: &mut [f32; CHANNELS]) {
    let sum: f32 = data.iter().sum();
    let factor = -2.0 / CHANNELS as f32 * sum;
    for x in data.iter_mut() {
        *x += factor;
    }
}

/// FDN feedback loop: per-channel delay lines, modulation LFOs,
/// one-pole damping filters and recirculating feedback state.
pub(super) struct FdnBank {
    delays: [DelayLine; CHANNELS],
    delay_samples: [usize; CHANNELS],
    damping: [OnePole; CHANNELS],
    lfos: [Lfo; CHANNELS],
    /// Persistent recirculating feedback state, fed back into the
    /// delay-line inputs on the next sample.
    feedback: [f32; CHANNELS],
    /// Smoothed |feedback| per channel for the tank view. Range ~0..1.
    energy_smoothed: [f32; CHANNELS],
}

impl FdnBank {
    pub(super) fn new(sample_rate: f32, max_size_ms: f32) -> Self {
        // Initial FDN delay lengths. These get immediately overridden
        // by `set_size()` on the first process block, so the exact
        // numbers don't matter — they just need to be non-zero so an
        // impulse arriving before the host calls `set_size` still
        // produces valid output. Uses the same `2^(c/(N-1))` spread as
        // `set_size` for internal consistency.
        let base_delay_ms = 60.0; // medium-room ballpark
        let base_samples = base_delay_ms * 0.001 * sample_rate;
        let mut delay_samples = [0usize; CHANNELS];
        for (c, slot) in delay_samples.iter_mut().enumerate() {
            let r = c as f32 / (CHANNELS - 1).max(1) as f32;
            let mult = MAX_FDN_MULT.powf(r);
            *slot = ((mult * base_samples) as usize).max(1);
        }

        // Size each FDN delay line for the longest possible channel
        // delay at `size=1.0`, plus headroom for modulation and the
        // `tap_linear` interpolator (which reads delay+1) and a small
        // safety margin.
        let max_fdn_samples = (max_size_ms * 0.001 * sample_rate * MAX_FDN_MULT) as usize + 256;
        let delays = std::array::from_fn(|_| DelayLine::new(max_fdn_samples));
        let damping = std::array::from_fn(|_| OnePole::new());

        // LFOs with staggered phases and randomized rates
        let lfos = modulation::build_fdn_lfos(sample_rate);

        Self {
            delays,
            delay_samples,
            damping,
            lfos,
            feedback: [0.0; CHANNELS],
            energy_smoothed: [0.0; CHANNELS],
        }
    }

    /// Update FDN delay lengths from the room-size base in samples.
    /// Uses the classic `2^(c/(N-1))` channel spread to pack 8 delay
    /// lines into a factor-of-two range so feedback reflections arrive
    /// densely instead of as audibly separated echoes.
    pub(super) fn set_size(&mut self, base_samples: f32) {
        for c in 0..CHANNELS {
            let r = c as f32 / (CHANNELS - 1).max(1) as f32;
            let mult = MAX_FDN_MULT.powf(r);
            self.delay_samples[c] = ((mult * base_samples) as usize).max(1);
        }
    }

    /// Set the one-pole damping cutoff on every channel.
    pub(super) fn set_damping(&mut self, cutoff_hz: f32, sample_rate: f32) {
        for filter in &mut self.damping {
            filter.set_cutoff(cutoff_hz, sample_rate);
        }
    }

    /// Re-tune the LFO bank around a new target rate.
    pub(super) fn set_mod_rate(&mut self, rate_hz: f32, sample_rate: f32) {
        modulation::update_fdn_rates(&mut self.lfos, rate_hz, sample_rate);
    }

    /// Process one sample through the feedback loop. Takes the
    /// freshly-diffused 8-channel input, the current per-sample
    /// modulation depth (in samples) and the decay gain, and returns
    /// the FDN's 8-channel output for the user-visible wet sum.
    /// The recirculating feedback state is updated for the next sample.
    pub(super) fn process(
        &mut self,
        diffused: &[f32; CHANNELS],
        mod_depth_samples: f32,
        decay_gain: f32,
    ) -> [f32; CHANNELS] {
        // FDN input = diffused fresh input + recirculated feedback from
        // the previous sample. This is the point where the tail
        // re-enters the delay lines; the feedback path NEVER goes
        // through diffusion.
        let mut fdn_input = [0.0f32; CHANNELS];
        for c in 0..CHANNELS {
            fdn_input[c] = diffused[c] + self.feedback[c];
        }

        // FDN: read the previous loop's output out of the delay lines
        // and push the new input in. `fdn_output` is what the user
        // hears as the tail.
        let mut fdn_output = [0.0f32; CHANNELS];
        for c in 0..CHANNELS {
            // Modulated read position
            let mod_offset = self.lfos[c].next() * mod_depth_samples;
            let delay_f = self.delay_samples[c] as f32 + mod_offset;
            let delay_f = delay_f.max(1.0);

            fdn_output[c] = self.delays[c].tap_linear(delay_f);
            self.delays[c].push(fdn_input[c]);
        }

        // Householder feedback: mix the FDN output, apply damping and
        // decay, and store for the next sample's FDN input.
        let mut feedback = fdn_output;
        householder_in_place(&mut feedback);
        for (c, f) in feedback.iter_mut().enumerate() {
            *f = self.damping[c].process(*f) * decay_gain;
        }
        self.feedback = feedback;

        // Smooth the |feedback| magnitudes toward a display envelope for the
        // tank view. Simple one-pole follower — cheap, no allocation. The
        // 0.995 coefficient gives a ~200-sample time constant (~4 ms @ 48 k).
        for c in 0..CHANNELS {
            let mag = self.feedback[c].abs();
            self.energy_smoothed[c] = self.energy_smoothed[c] * 0.995 + mag * 0.005;
        }

        fdn_output
    }

    /// Current FDN delay lengths in ms (affected by `size`).
    pub(super) fn delay_ms(&self, sample_rate: f32) -> [f32; CHANNELS] {
        let mut out = [0.0f32; CHANNELS];
        for (c, o) in out.iter_mut().enumerate() {
            *o = self.delay_samples[c] as f32 * 1000.0 / sample_rate;
        }
        out
    }

    /// Snapshot the smoothed per-channel energies for the tank view.
    pub(super) fn channel_energies(&self) -> [f32; CHANNELS] {
        self.energy_smoothed
    }

    pub(super) fn clear(&mut self) {
        for d in &mut self.delays {
            d.clear();
        }
        for f in &mut self.damping {
            f.clear();
        }
        self.feedback = [0.0; CHANNELS];
        self.energy_smoothed = [0.0; CHANNELS];
    }
}
