//! Modal peakiness of a late tail: how far its strongest resonance stands
//! out of its spectrum.
//!
//! 1. Take `PEAKINESS_SEGMENT_S` of the response from `start_s` (a fixed
//!    length, so two responses are compared at the same frequency
//!    resolution: an FDN mode at an 8 s T60 is only ~0.3 Hz wide, and a
//!    longer segment would resolve it, and so score it, more sharply). The
//!    first 5 ms are faded in, so the cut does not splash broadband
//!    leakage over the modes.
//! 2. Power spectrum, zero-padded to at least one second (≤ 1 Hz bins).
//! 3. 1/24-octave smoothing, evaluated every 1/48 octave over
//!    `[f_lo, f_hi]`: each point is the mean power of the bins within
//!    ±1/48 octave of it (the nearest bin if none falls inside).
//! 4. Peakiness = max − median of those points, dB.
//!
//! A colourless tail is noise-like, and its smoothed spectrum still
//! scatters: white noise scores several dB here, not 0. Compare responses
//! with each other, not with zero.

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// Default start of the late tail, seconds.
pub const PEAKINESS_START_S: f32 = 0.200;
/// Length of the analysed segment, seconds.
pub const PEAKINESS_SEGMENT_S: f32 = 1.0;
/// Default analysed band, Hz.
pub const PEAKINESS_BAND_HZ: (f32, f32) = (100.0, 8_000.0);
/// Fade-in at the segment start, seconds.
const FADE_S: f32 = 0.005;

/// Modal peakiness in dB of `ir` from `start_s` over `[f_lo, f_hi]`. `None`
/// if there is no audio from `start_s` on or the band is empty.
pub fn modal_peakiness_db(
    ir: &[f32],
    sample_rate: f32,
    start_s: f32,
    f_lo: f32,
    f_hi: f32,
) -> Option<f32> {
    let start = (start_s * sample_rate).round() as usize;
    if start >= ir.len() || f_hi.is_nan() || f_hi <= f_lo || f_lo <= 0.0 {
        return None;
    }
    let seg_len = ((PEAKINESS_SEGMENT_S * sample_rate) as usize).min(ir.len() - start);
    let seg = &ir[start..start + seg_len];
    if seg.iter().all(|&x| x == 0.0) {
        return None;
    }
    let fft_len = seg_len.max(sample_rate.ceil() as usize).next_power_of_two();
    let fade = ((FADE_S * sample_rate) as usize).max(1);
    let mut buf: Vec<Complex<f64>> = (0..fft_len)
        .map(|i| {
            let x = seg.get(i).copied().unwrap_or(0.0) as f64;
            let g = if i < fade {
                0.5 - 0.5 * (std::f64::consts::PI * i as f64 / fade as f64).cos()
            } else {
                1.0
            };
            Complex::new(x * g, 0.0)
        })
        .collect();
    FftPlanner::<f64>::new()
        .plan_fft_forward(fft_len)
        .process(&mut buf);

    let bin_hz = sample_rate as f64 / fft_len as f64;
    let nyq_bin = fft_len / 2;
    let power: Vec<f64> = buf[..=nyq_bin].iter().map(|c| c.norm_sqr()).collect();

    let half = 2f64.powf(1.0 / 48.0);
    let mut points = Vec::new();
    let mut f = f_lo as f64;
    let f_top = (f_hi as f64).min(sample_rate as f64 / 2.0 / half);
    while f <= f_top {
        let lo = ((f / half / bin_hz).ceil() as usize).min(nyq_bin);
        let hi = ((f * half / bin_hz).floor() as usize).min(nyq_bin);
        let p = if hi >= lo {
            power[lo..=hi].iter().sum::<f64>() / (hi - lo + 1) as f64
        } else {
            power[((f / bin_hz).round() as usize).min(nyq_bin)]
        };
        points.push(10.0 * p.max(1e-300).log10());
        f *= half;
    }
    if points.is_empty() {
        return None;
    }
    let max = points.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    points.sort_by(f64::total_cmp);
    let median = points[points.len() / 2];
    Some((max - median) as f32)
}
