//! Parametric EQ band — the smallest unit the design stage consumes.
//!
//! A [`BandConfig`] describes one filter section and is configured by
//! the plugin's parameters. The [`design`] module takes a slice of
//! enabled bands and produces the composite magnitude response of the
//! cascaded chain.

use resonance_dsp::Biquad;

pub use resonance_dsp::BandType;

/// Parameter snapshot for one EQ band. Plain-data struct so the design
/// stage can work off a simple slice without touching plugin atomics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandConfig {
    pub enabled: bool,
    pub band_type: BandType,
    pub freq_hz: f32,
    pub q: f32,
    pub gain_db: f32,
}

impl BandConfig {
    pub fn off() -> Self {
        Self {
            enabled: false,
            band_type: BandType::Bell,
            freq_hz: 1000.0,
            q: 0.707,
            gain_db: 0.0,
        }
    }

    /// Apply this band's configuration to a biquad at the given sample
    /// rate. Returns a freshly configured biquad ready for magnitude
    /// response evaluation.
    pub fn to_biquad(&self, sample_rate: f32) -> Biquad {
        self.band_type
            .to_biquad(sample_rate, self.freq_hz, self.q, self.gain_db)
    }
}
