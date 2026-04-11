//! Stored tonal-balance target curves per genre.
//!
//! Each curve is 60 dB values at 1/6-octave spacing from 20 Hz to 20 kHz,
//! matching the layout of the live spectrum analyzer. Values are
//! **relative** — they describe the spectral shape, not an absolute
//! loudness. The assistant normalizes the analyzed spectrum to match the
//! target's midrange average before comparing.
//!
//! The curves are simple heuristics: a pink-tilted base (−3 dB/octave
//! through the midrange) plus gaussian bumps or dips that approximate
//! the characteristic spectral fingerprint of each genre. They're not
//! derived from commercial-master analysis — that's a later-phase
//! project. For now they give the rule engine enough shape information
//! to produce sensible shelf suggestions.

use super::analyze::NUM_SPECTRUM_BINS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Genre {
    Rock,
    Indie,
    Acoustic,
    Jazz,
    Pop,
}

impl Genre {
    pub const ALL: &'static [Self] = &[
        Self::Rock,
        Self::Indie,
        Self::Acoustic,
        Self::Jazz,
        Self::Pop,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Rock => "Rock",
            Self::Indie => "Indie",
            Self::Acoustic => "Acoustic",
            Self::Jazz => "Jazz",
            Self::Pop => "Pop",
        }
    }

    /// Integrated-LUFS target appropriate for the genre. Matches the
    /// research brief's band-music guidelines — modern streaming is
    /// normalized to ~−14 LUFS, so the louder rock/pop targets are
    /// deliberately above that.
    pub fn target_lufs(&self) -> f32 {
        match self {
            Self::Rock => -11.0,
            Self::Indie => -13.0,
            Self::Acoustic => -16.0,
            Self::Jazz => -17.0,
            Self::Pop => -10.0,
        }
    }
}

impl Default for Genre {
    fn default() -> Self {
        Genre::Rock
    }
}

/// 1/6-octave target curve for the given genre. Indexed from 20 Hz bin
/// at `[0]` to 20 kHz bin at `[NUM_SPECTRUM_BINS - 1]`.
pub fn target_curve(genre: Genre) -> [f32; NUM_SPECTRUM_BINS] {
    let mut curve = [0.0_f32; NUM_SPECTRUM_BINS];
    for i in 0..NUM_SPECTRUM_BINS {
        let freq = band_center_hz(i);
        // Pink slope: −3 dB/octave, normalized so 1 kHz = 0 dB.
        curve[i] = -3.0 * (freq / 1000.0).log2();
    }
    for i in 0..NUM_SPECTRUM_BINS {
        curve[i] += genre_tweak(genre, band_center_hz(i));
    }
    curve
}

/// Centre frequency of the `i`th 1/6-octave bin between 20 Hz and 20 kHz.
pub fn band_center_hz(i: usize) -> f32 {
    let ratio = (20_000.0_f32 / 20.0).powf(1.0 / NUM_SPECTRUM_BINS as f32);
    20.0 * ratio.powi(i as i32)
}

fn gauss(freq: f32, center: f32, octaves: f32) -> f32 {
    let dist = (freq / center).log2() / octaves;
    (-dist * dist).exp()
}

fn genre_tweak(genre: Genre, freq: f32) -> f32 {
    match genre {
        Genre::Rock => {
            // Mid-scoop at 400 Hz, presence bump at 3 kHz — classic
            // rock mastering template.
            gauss(freq, 400.0, 1.5) * -1.5 + gauss(freq, 3000.0, 1.5) * 1.5
        }
        Genre::Indie => {
            // Gentle midrange bump, otherwise flat.
            gauss(freq, 1000.0, 2.0) * 0.8
        }
        Genre::Acoustic => {
            // Less sub, flat mids, slightly airier top.
            let low_cut = if freq < 60.0 { -3.0 } else { 0.0 };
            low_cut + gauss(freq, 10_000.0, 1.5) * 0.5
        }
        Genre::Jazz => {
            // Fat mids, tamed top.
            let top_cut = if freq > 8000.0 { -2.0 } else { 0.0 };
            gauss(freq, 800.0, 2.0) * 1.0 + top_cut
        }
        Genre::Pop => {
            // Brighter presence region.
            gauss(freq, 6000.0, 1.5) * 1.5
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_has_correct_length() {
        for &g in Genre::ALL {
            let c = target_curve(g);
            assert_eq!(c.len(), NUM_SPECTRUM_BINS);
            for v in c {
                assert!(v.is_finite());
            }
        }
    }

    #[test]
    fn rock_has_midrange_dip() {
        let c = target_curve(Genre::Rock);
        // Find 400 Hz bin and a neighbor one octave below.
        let mut idx_400 = 0;
        let mut idx_100 = 0;
        for i in 0..NUM_SPECTRUM_BINS {
            if band_center_hz(i) >= 400.0 && idx_400 == 0 {
                idx_400 = i;
            }
            if band_center_hz(i) >= 100.0 && idx_100 == 0 {
                idx_100 = i;
            }
        }
        // 400 Hz should be below 100 Hz by at least the gaussian dip
        // amount *above* the natural pink tilt (which favors 100 Hz).
        let dip = c[idx_100] - c[idx_400];
        assert!(dip > 0.5, "expected 400 Hz dip under 100 Hz, got {dip}");
    }

    #[test]
    fn target_lufs_is_genre_dependent() {
        assert!(Genre::Rock.target_lufs() > Genre::Acoustic.target_lufs());
        assert!(Genre::Pop.target_lufs() > Genre::Jazz.target_lufs());
    }
}
