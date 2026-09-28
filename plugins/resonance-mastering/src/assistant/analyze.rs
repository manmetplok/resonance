//! Offline analysis of a captured stereo buffer.
//!
//! Runs the loudness and peak meters the live plugin uses, the range's
//! own crest factor and L/R correlation, and a one-shot Welch LTAS, then
//! packages the readings into an [`AnalysisResult`] for the decision
//! engine to consume.
//!
//! Every figure here comes from `resonance-metering`, and the engine's
//! offline measurement (`master.assist`) reads the same functions off a
//! rendered range — so the live panel and the control API produce the
//! same analysis from the same audio.

use resonance_metering::offline::{range_correlation, range_crest_db};
use resonance_metering::spectrum::offline::sixth_octave_ltas;
use resonance_metering::{LufsMeter, TruePeakMeter};

/// Number of 1/6-octave bands in the analysis spectrum. Must match
/// [`resonance_metering::NUM_OCTAVE_BINS`].
pub const NUM_SPECTRUM_BINS: usize = resonance_metering::NUM_OCTAVE_BINS;

/// Minimum dB value reported when the analyzed signal is silent.
#[doc(hidden)]
pub const FLOOR_DB: f32 = resonance_metering::spectrum::offline::LTAS_FLOOR_DB;

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

    // The whole buffer's crest and correlation. The live `CrestMeter` /
    // `CorrelationMeter` are 100 ms sliding readouts: fed a 10 s capture
    // they only describe its last 100 ms.
    let crest_db = range_crest_db(&left[..n], &right[..n]);
    let correlation = range_correlation(&left[..n], &right[..n]);

    let spectrum_db = sixth_octave_ltas(sample_rate, &left[..n], &right[..n]);

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
