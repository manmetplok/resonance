//! Octave-band filtering for per-band decay times.
//!
//! Each band is an 8th-order Butterworth high-pass at `fc / √2` cascaded
//! with an 8th-order Butterworth low-pass at `fc · √2` (four
//! [`BiquadCoeffs`] sections each, run in f64). That is steeper than an
//! IEC 61260 class-1 octave filter: at the neighbouring band's centre the
//! rejection is ~24 dB, two octaves away ~70 dB, which keeps a slower
//! neighbour from bending a band's decay curve in its last −35 dB.
//!
//! ## Causal filtering and its bias
//!
//! The filter is causal (not zero-phase), so it delays and smears the onset
//! of each band by its group delay — a few ms at 8 kHz, roughly 10–15 ms at
//! 125 Hz — and adds its own ringing to the response. The ringing of the
//! sharpest pole (the Q 2.56 high-pass section) has a T60 of about
//! `8 / fc` seconds: ~65 ms at 125 Hz, ~1 ms at 8 kHz. Band T30s well above
//! that are unaffected (the fit starts at −5 dB, after the onset); EDT and
//! T20/T30 values below ~0.2 s in the 125 Hz band are biased long. The
//! onset delay is harmless because every decay time is measured from the
//! band's own onset (see [`super::edc`]).

use resonance_dsp::BiquadCoeffs;

/// ISO 266 octave-band centres analysed by [`super::band_decay_times`].
pub const OCTAVE_BANDS_HZ: [f32; 7] = [125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0];

/// Section Qs of an 8th-order Butterworth response (four biquads).
const BUTTERWORTH_8_QS: [f64; 4] = [0.509_795_6, 0.601_344_9, 0.899_976_2, 2.562_915_4];

/// One f64 transposed-direct-form-II biquad.
#[derive(Clone, Copy, Debug)]
struct Section {
    c: BiquadCoeffs,
    z1: f64,
    z2: f64,
}

impl Section {
    fn new(c: BiquadCoeffs) -> Self {
        Self {
            c,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.c.b0 * x + self.z1;
        self.z1 = self.c.b1 * x - self.c.a1 * y + self.z2;
        self.z2 = self.c.b2 * x - self.c.a2 * y;
        y
    }
}

/// An octave band-pass around one centre frequency. See the module docs.
#[derive(Clone, Debug)]
pub struct OctaveBandFilter {
    sections: [Section; 8],
}

impl OctaveBandFilter {
    /// Band-pass for the octave centred on `center_hz`. Edges above
    /// Nyquist are clamped by the biquad designs.
    pub fn new(sample_rate: f32, center_hz: f32) -> Self {
        let sr = sample_rate as f64;
        let lo = center_hz as f64 / std::f64::consts::SQRT_2;
        let hi = center_hz as f64 * std::f64::consts::SQRT_2;
        let hp = BUTTERWORTH_8_QS.map(|q| Section::new(BiquadCoeffs::high_pass(sr, lo, q)));
        let lp = BUTTERWORTH_8_QS.map(|q| Section::new(BiquadCoeffs::low_pass(sr, hi, q)));
        Self {
            sections: [hp[0], hp[1], hp[2], hp[3], lp[0], lp[1], lp[2], lp[3]],
        }
    }

    /// Filter one sample.
    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        self.sections.iter_mut().fold(x, |acc, s| s.process(acc))
    }

    /// Filter a whole buffer from a cleared state, in f64.
    pub fn filter(&mut self, input: &[f32]) -> Vec<f64> {
        for s in &mut self.sections {
            s.z1 = 0.0;
            s.z2 = 0.0;
        }
        input.iter().map(|&x| self.process(x as f64)).collect()
    }
}

/// `input` band-passed to the octave around `center_hz`, as f32.
pub fn octave_band(input: &[f32], sample_rate: f32, center_hz: f32) -> Vec<f32> {
    OctaveBandFilter::new(sample_rate, center_hz)
        .filter(input)
        .into_iter()
        .map(|x| x as f32)
        .collect()
}
