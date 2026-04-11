//! Offline analysis of a captured stereo buffer.
//!
//! Runs every metering stream the live plugin uses, plus a one-shot
//! Welch LTAS, then packages the readings into an [`AnalysisResult`]
//! for the decision engine to consume.

use resonance_metering::spectrum::octave::OctaveTable;
use resonance_metering::{CorrelationMeter, CrestMeter, LufsMeter, TruePeakMeter};
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// Number of 1/6-octave bands in the analysis spectrum. Must match
/// [`resonance_metering::NUM_OCTAVE_BINS`].
pub const NUM_SPECTRUM_BINS: usize = resonance_metering::NUM_OCTAVE_BINS;

/// FFT size for the Welch LTAS. 4096 is a good balance of resolution
/// and number of averages given a ~10 s captured buffer.
const LTAS_FFT_SIZE: usize = 4096;
const LTAS_HOP: usize = LTAS_FFT_SIZE / 2;

/// Minimum dB value reported when the analyzed signal is silent.
const FLOOR_DB: f32 = -120.0;

#[derive(Debug, Clone)]
pub struct AnalysisResult {
    pub sample_rate: f32,
    pub duration_s: f32,
    pub integrated_lufs: f32,
    pub short_term_lufs: f32,
    pub true_peak_dbtp: f32,
    pub crest_db: f32,
    pub correlation: f32,
    pub spectrum_db: Vec<f32>,
}

/// Run every analysis stream over the captured stereo buffer.
pub fn run(sample_rate: f32, left: &[f32], right: &[f32]) -> AnalysisResult {
    let n = left.len().min(right.len());
    let duration_s = n as f32 / sample_rate;

    let lufs = LufsMeter::analyze_offline(sample_rate, &left[..n], &right[..n]);

    let mut tp = TruePeakMeter::new();
    tp.push_stereo(&left[..n], &right[..n]);
    let true_peak_dbtp = tp.peak_dbtp();

    let mut crest = CrestMeter::new(sample_rate);
    crest.push_stereo(&left[..n], &right[..n]);
    let crest_db = crest.crest_db();

    let mut corr = CorrelationMeter::new(sample_rate);
    corr.push_stereo(&left[..n], &right[..n]);
    let correlation = corr.correlation();

    let spectrum_db = compute_ltas(sample_rate, &left[..n], &right[..n]);

    AnalysisResult {
        sample_rate,
        duration_s,
        integrated_lufs: lufs.integrated,
        short_term_lufs: lufs.short_term,
        true_peak_dbtp,
        crest_db,
        correlation,
        spectrum_db,
    }
}

/// Welch long-term average spectrum aggregated to 1/6-octave bins.
/// Averages per-frame power (not amplitude) and converts back to dB at
/// the end.
fn compute_ltas(sample_rate: f32, left: &[f32], right: &[f32]) -> Vec<f32> {
    let n = left.len().min(right.len());
    if n < LTAS_FFT_SIZE {
        return vec![FLOOR_DB; NUM_SPECTRUM_BINS];
    }

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(LTAS_FFT_SIZE);

    let window: Vec<f32> = (0..LTAS_FFT_SIZE)
        .map(|i| {
            let x = i as f32 / (LTAS_FFT_SIZE as f32 - 1.0);
            0.5 - 0.5 * (std::f32::consts::TAU * x).cos()
        })
        .collect();

    let mut scratch = vec![Complex::new(0.0, 0.0); LTAS_FFT_SIZE];
    let mut power_sum = vec![0.0_f64; LTAS_FFT_SIZE / 2];
    let mut frames = 0_usize;

    let mut start = 0_usize;
    while start + LTAS_FFT_SIZE <= n {
        for i in 0..LTAS_FFT_SIZE {
            let mono = 0.5 * (left[start + i] + right[start + i]) * window[i];
            scratch[i] = Complex::new(mono, 0.0);
        }
        fft.process(&mut scratch);
        let norm = 4.0 / LTAS_FFT_SIZE as f32;
        for k in 0..LTAS_FFT_SIZE / 2 {
            let re = scratch[k].re;
            let im = scratch[k].im;
            let mag = (re * re + im * im).sqrt() * norm;
            power_sum[k] += (mag as f64) * (mag as f64);
        }
        frames += 1;
        start += LTAS_HOP;
    }

    if frames == 0 {
        return vec![FLOOR_DB; NUM_SPECTRUM_BINS];
    }

    let mut mag_db = vec![FLOOR_DB; LTAS_FFT_SIZE / 2];
    for k in 0..LTAS_FFT_SIZE / 2 {
        let avg_power = power_sum[k] / frames as f64;
        let avg_mag = avg_power.sqrt() as f32;
        mag_db[k] = 20.0 * avg_mag.max(1e-10).log10();
    }

    let table = OctaveTable::new();
    let mut out = vec![FLOOR_DB; NUM_SPECTRUM_BINS];
    table.aggregate(&mag_db, sample_rate, &mut out, FLOOR_DB);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_produces_low_floor() {
        let l = vec![0.0_f32; 48_000];
        let r = vec![0.0_f32; 48_000];
        let result = run(48_000.0, &l, &r);
        assert!(result.integrated_lufs < -60.0);
        assert!(result.true_peak_dbtp < -100.0);
        for v in &result.spectrum_db {
            assert!(*v < -60.0, "silent spectrum bin = {v}");
        }
    }

    #[test]
    fn sine_at_minus_23_dbfs_reads_minus_23_lufs() {
        let sr = 48_000.0_f32;
        let n = (sr * 3.0) as usize;
        let amp = 10.0_f32.powf(-23.0 / 20.0);
        let mut l = vec![0.0_f32; n];
        let mut r = vec![0.0_f32; n];
        for i in 0..n {
            let s = (i as f32 / sr * 1000.0 * std::f32::consts::TAU).sin() * amp;
            l[i] = s;
            r[i] = s;
        }
        let result = run(sr, &l, &r);
        assert!((result.integrated_lufs - -23.0).abs() < 0.3);
        assert!(result.duration_s > 2.9);
    }
}
