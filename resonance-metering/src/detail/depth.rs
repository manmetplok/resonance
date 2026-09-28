//! Depth proxies (warmth-width-depth.md §2.3, §7.6 `depth`, decision D5).
//!
//! Depth is judged by ORDERING — front sources drier and brighter than
//! back ones — so these are estimates made to rank tracks, not absolute
//! room acoustics:
//!
//! * [`hf_tilt_db`]: `E(6–16 kHz) / E(1–4 kHz)`, which falls from front to
//!   back as distance darkens a source.
//! * [`drr_db_estimate`]: direct-to-reverberant ratio from the send
//!   levels and each return's measured gain, no per-source renders
//!   (D5). See its docs for the formula.
//! * [`layer_hints`]: front / middle / back from DRR tertiles.

use super::ratio_db;
use crate::spectrum::offline::{Channel, StereoSpectrum};

/// Mean square of a stereo buffer over both channels (0 when empty).
pub fn mean_square(left: &[f32], right: &[f32]) -> f64 {
    let n = left.len().min(right.len());
    if n == 0 {
        return 0.0;
    }
    let sum: f64 = left[..n]
        .iter()
        .zip(&right[..n])
        .map(|(&l, &r)| (l as f64) * (l as f64) + (r as f64) * (r as f64))
        .sum();
    sum / (2 * n) as f64
}

/// `E(6–16 kHz) / E(1–4 kHz)` of the stereo LTAS, dB; `None` when either
/// band is empty.
pub fn hf_tilt_db(spec: &StereoSpectrum) -> Option<f32> {
    ratio_db(
        spec.band(Channel::Both, 6_000.0, 16_000.0),
        spec.band(Channel::Both, 1_000.0, 4_000.0),
    )
}

/// One of a track's sends, as the estimate needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SendTerm {
    /// The send's level, dB.
    pub send_level_db: f64,
    /// The return's measured gain, dB: its output energy over the energy
    /// its sends feed it, so it covers the return's chain AND its fader
    /// (`return_fader_db + return_wet_gain_db` in the spec's terms).
    pub return_gain_db: f64,
    /// A pre-fader send bypasses the source's fader, so the dry path is
    /// `source_fader_db` louder or quieter than the send path sees.
    pub pre_fader: bool,
}

/// The direct-to-reverberant estimate of a source, dB:
/// `−10·log10 Σ 10^(wet_i/10)` with, per send,
/// `wet_i = send_level_db + return_gain_db − (pre_fader ? source_fader_db : 0)`
/// — the wet level each return adds relative to the dry signal, summed
/// in power. For one post-fader send this is exactly the spec's
/// `−(send_level_db + return_fader_db + return_wet_gain_db)`; a
/// pre-fader send adds the source's own fader to the DRR.
///
/// `None` with no sends (the source is dry-only).
pub fn drr_db_estimate(source_fader_db: f64, sends: &[SendTerm]) -> Option<f64> {
    if sends.is_empty() {
        return None;
    }
    let wet: f64 = sends
        .iter()
        .map(|s| {
            let fader = if s.pre_fader { source_fader_db } else { 0.0 };
            10f64.powf((s.send_level_db + s.return_gain_db - fader) / 10.0)
        })
        .sum();
    (wet > 0.0 && wet.is_finite()).then(|| -10.0 * wet.log10())
}

/// Depth layer of a source, relative to the other sources measured with
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Front,
    Middle,
    Back,
}

/// Front / middle / back per source from DRR tertiles: sources ranked by
/// DRR, highest (driest) first, the first third front, the next middle,
/// the rest back. A dry-only source (`None`) ranks as the driest of all.
/// Relative by construction: one source alone is `Front`.
pub fn layer_hints(drr: &[Option<f64>]) -> Vec<Layer> {
    let key = |v: Option<f64>| v.unwrap_or(f64::INFINITY);
    let mut order: Vec<usize> = (0..drr.len()).collect();
    order.sort_by(|&a, &b| key(drr[b]).total_cmp(&key(drr[a])));
    let n = drr.len() as f64;
    let mut out = vec![Layer::Front; drr.len()];
    for (rank, &i) in order.iter().enumerate() {
        let q = rank as f64 / n;
        out[i] = if q < 1.0 / 3.0 {
            Layer::Front
        } else if q < 2.0 / 3.0 {
            Layer::Middle
        } else {
            Layer::Back
        };
    }
    out
}
