//! The band curve as the editor draws it: magnitude in dB at a set of
//! frequencies, split into what the mid and the side channel get.
//!
//! A `Stereo` band shapes both channels, a `Mid` band only the mid and a
//! `Side` band only the side, so once any band is in Mid or Side mode
//! there is no single curve that says what the EQ does — folding a Side
//! band into one composite shows a boost the mono sum never gets. The
//! editor therefore draws two curves when (and only when) the bands
//! differ per channel: the mid (Stereo + Mid bands) solid, the side
//! (Stereo + Side bands) dashed. With every band in Stereo mode the two
//! are the same curve and only one is drawn, exactly as before M/S.
//!
//! Headless on purpose: the editor caches the result per band state, and
//! the tests check it without a GUI.

use resonance_dsp::Biquad;

use crate::band::{configure_stages, BandMs, MAX_STAGES_PER_BAND};
use crate::params::BandSnapshot;

/// Magnitude of the band curve, dB, per requested frequency.
#[derive(Clone, Debug, PartialEq)]
pub struct ResponseCurves {
    /// What the mid channel gets — the Stereo and Mid bands. With no band
    /// in Mid or Side mode this is the whole EQ's curve.
    pub mid: Vec<f32>,
    /// What the side channel gets — the Stereo and Side bands. `None`
    /// when no band is in Mid or Side mode, so the two would be identical.
    pub side: Option<Vec<f32>>,
}

/// Evaluate the band curve at `freqs` (Hz) for sample rate `sr`.
pub fn response_curves(snapshots: &[BandSnapshot], sr: f32, freqs: &[f32]) -> ResponseCurves {
    let bands: Vec<(BandMs, usize, [Biquad; MAX_STAGES_PER_BAND])> = snapshots
        .iter()
        .map(|s| {
            let mut stages = [Biquad::identity(); MAX_STAGES_PER_BAND];
            let n = configure_stages(s, sr, &mut stages);
            (s.ms, n, stages)
        })
        .filter(|(_, n, _)| *n > 0)
        .collect();
    let split = bands.iter().any(|(ms, _, _)| *ms != BandMs::Stereo);

    let mut mid = Vec::with_capacity(freqs.len());
    let mut side = Vec::with_capacity(if split { freqs.len() } else { 0 });
    for &f in freqs {
        let (mut m, mut s) = (1.0f32, 1.0f32);
        for (ms, n, stages) in &bands {
            let g: f32 = stages[..*n].iter().map(|b| b.magnitude(f, sr)).product();
            match ms {
                BandMs::Stereo => {
                    m *= g;
                    s *= g;
                }
                BandMs::Mid => m *= g,
                BandMs::Side => s *= g,
            }
        }
        mid.push(to_db(m));
        if split {
            side.push(to_db(s));
        }
    }
    ResponseCurves {
        mid,
        side: split.then_some(side),
    }
}

fn to_db(lin: f32) -> f32 {
    20.0 * lin.max(1e-10).log10()
}
