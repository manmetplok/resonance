//! Per-FDN-channel modulation LFOs that lightly detune the delay-line
//! read positions. Adds chorus-like motion to the tail and breaks up the
//! comb-filter character of static delay lines.

use resonance_dsp::Lfo;

use super::CHANNELS;

/// Build the staggered, randomized-rate LFO bank used by the FDN.
///
/// Each channel gets a phase offset of `c / CHANNELS` and a rate that
/// rises linearly from 0.5 Hz to ~2.6 Hz across the bank — wide enough
/// that the channels never beat against each other audibly.
pub(super) fn build_fdn_lfos(sample_rate: f32) -> [Lfo; CHANNELS] {
    std::array::from_fn(|c| {
        let phase = c as f32 / CHANNELS as f32;
        let rate = 0.5 + 0.3 * (c as f32); // 0.5..2.6 Hz spread
        Lfo::new(rate, sample_rate, phase)
    })
}

/// Re-tune the FDN LFO bank around a new target rate. Each channel
/// gets a ±50%-spread multiplier so the bank stays decorrelated.
pub(super) fn update_fdn_rates(lfos: &mut [Lfo; CHANNELS], rate_hz: f32, sample_rate: f32) {
    for (c, lfo) in lfos.iter_mut().enumerate() {
        // Spread rates around the target: ±50%
        let spread = 0.5 + (c as f32 / CHANNELS as f32);
        lfo.set_rate(rate_hz * spread, sample_rate);
    }
}
