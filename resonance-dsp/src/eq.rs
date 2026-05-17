//! Shared parametric-EQ building blocks.
//!
//! [`BandType`] is the minimal enum every parametric EQ plugin needs:
//! a shape tag plus the `to_biquad` / index-encoding helpers. Plugins
//! that need richer slope selection (e.g. variable-order cuts) can
//! layer their own wrapper types on top without duplicating the
//! single-biquad case.

use crate::Biquad;

/// Filter shape for a single EQ band.
///
/// The integer encoding is stable on disk — saved plugin state stores
/// `to_index()`, so new variants must only be appended. Existing
/// entries must not be renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BandType {
    Bell,
    LowShelf,
    HighShelf,
    HighPass,
    LowPass,
}

impl BandType {
    /// Convert an integer encoding (used in saved state) to a band type.
    /// Unknown integers fall back to [`BandType::Bell`].
    pub fn from_index(i: i32) -> Self {
        match i {
            0 => BandType::Bell,
            1 => BandType::LowShelf,
            2 => BandType::HighShelf,
            3 => BandType::HighPass,
            4 => BandType::LowPass,
            _ => BandType::Bell,
        }
    }

    /// Inverse of [`from_index`].
    pub fn to_index(self) -> i32 {
        match self {
            BandType::Bell => 0,
            BandType::LowShelf => 1,
            BandType::HighShelf => 2,
            BandType::HighPass => 3,
            BandType::LowPass => 4,
        }
    }

    /// Whether this shape has a gain parameter (bells and shelves do;
    /// high-pass and low-pass do not).
    pub fn uses_gain(self) -> bool {
        matches!(
            self,
            BandType::Bell | BandType::LowShelf | BandType::HighShelf
        )
    }

    /// Build a configured [`Biquad`] matching this shape.
    pub fn to_biquad(self, sample_rate: f32, freq_hz: f32, q: f32, gain_db: f32) -> Biquad {
        let mut b = Biquad::identity();
        match self {
            BandType::Bell => b.set_bell(sample_rate, freq_hz, q, gain_db),
            BandType::LowShelf => b.set_low_shelf(sample_rate, freq_hz, q, gain_db),
            BandType::HighShelf => b.set_high_shelf(sample_rate, freq_hz, q, gain_db),
            BandType::HighPass => b.set_high_pass(sample_rate, freq_hz, q),
            BandType::LowPass => b.set_low_pass(sample_rate, freq_hz, q),
        }
        b
    }
}

