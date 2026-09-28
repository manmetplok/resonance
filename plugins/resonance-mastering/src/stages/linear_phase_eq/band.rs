//! Parametric EQ band — the smallest unit the design stage consumes.
//!
//! A [`BandConfig`] describes one filter section and is configured by
//! the plugin's parameters. The [`design`] module takes a slice of
//! enabled bands and produces the composite magnitude response of the
//! cascaded chain.

use resonance_dsp::Biquad;

pub use resonance_dsp::BandType;

/// Which part of the stereo image a band filters.
///
/// `Stereo` filters left and right alike (every band did, before the
/// selector existed); `Mid` filters only `(L + R)/2` and `Side` only
/// `(L − R)/2`, so a side-only cut or shelf leaves the mono sum alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MsMode {
    #[default]
    Stereo,
    Mid,
    Side,
}

impl MsMode {
    /// Display labels, indexed by [`MsMode::to_index`].
    pub const LABELS: &'static [&'static str] = &["Stereo", "Mid", "Side"];

    pub fn from_index(i: i32) -> Self {
        match i {
            1 => MsMode::Mid,
            2 => MsMode::Side,
            _ => MsMode::Stereo,
        }
    }

    pub fn to_index(self) -> i32 {
        match self {
            MsMode::Stereo => 0,
            MsMode::Mid => 1,
            MsMode::Side => 2,
        }
    }
}

/// Parameter snapshot for one EQ band. Plain-data struct so the design
/// stage can work off a simple slice without touching plugin atomics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandConfig {
    pub enabled: bool,
    pub band_type: BandType,
    pub freq_hz: f32,
    pub q: f32,
    pub gain_db: f32,
    /// Stereo, mid-only or side-only.
    pub ms: MsMode,
}

impl BandConfig {
    pub fn off() -> Self {
        Self {
            enabled: false,
            band_type: BandType::Bell,
            freq_hz: 1000.0,
            q: 0.707,
            gain_db: 0.0,
            ms: MsMode::Stereo,
        }
    }

    /// Enabled, and filtering only the mid or only the side.
    pub fn is_ms(&self) -> bool {
        self.enabled && self.ms != MsMode::Stereo
    }

    /// Apply this band's configuration to a biquad at the given sample
    /// rate. Returns a freshly configured biquad ready for magnitude
    /// response evaluation.
    pub fn to_biquad(&self, sample_rate: f32) -> Biquad {
        self.band_type
            .to_biquad(sample_rate, self.freq_hz, self.q, self.gain_db)
    }
}
