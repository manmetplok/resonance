//! Rule-based decision engine.
//!
//! Takes an offline [`AnalysisResult`] plus a [`Target`] (a stored
//! genre curve or a loaded reference track), compares the analyzed
//! spectrum to the target, derives practical suggestions (input trim,
//! tonal shelves, glue compressor, stereo imager, limiter), and
//! packages them into a [`Suggestions`] struct with human-readable
//! rationale. The UI displays the rationale verbatim so the user can
//! see *why* each decision was made before applying it.

use super::analyze::{AnalysisResult, NUM_SPECTRUM_BINS};
use super::reference::ReferenceTrack;
use super::targets::{band_center_hz, target_curve, Genre};
use crate::params::MasteringParams;
use crate::stages::linear_phase_eq::BandType;

/// What the decision engine should compare the analyzed input against.
#[derive(Debug, Clone)]
pub enum Target {
    /// Stored genre target curve.
    Genre(Genre),
    /// Ad-hoc target derived from a loaded reference track.
    Reference(ReferenceTrack),
}

impl Target {
    pub fn label(&self) -> String {
        match self {
            Target::Genre(g) => g.label().to_string(),
            Target::Reference(r) => r.display_name.clone(),
        }
    }

    /// Target spectral shape (60 dB values at 1/6-octave spacing).
    pub fn curve(&self) -> [f32; NUM_SPECTRUM_BINS] {
        match self {
            Target::Genre(g) => target_curve(*g),
            Target::Reference(r) => {
                let mut out = [0.0_f32; NUM_SPECTRUM_BINS];
                let src = &r.analysis.spectrum_db;
                let len = src.len().min(NUM_SPECTRUM_BINS);
                out[..len].copy_from_slice(&src[..len]);
                out
            }
        }
    }

    /// Target integrated loudness.
    pub fn target_lufs(&self) -> f32 {
        match self {
            Target::Genre(g) => g.target_lufs(),
            Target::Reference(r) => r.analysis.integrated_lufs,
        }
    }
}

/// Frequency boundaries (Hz) of the spectral bands the decision
/// engine reasons about. [`bins_for_range`] resolves these to
/// concrete 1/6-octave bin indices at runtime so the logic stays
/// correct if `NUM_SPECTRUM_BINS` or the octave grid ever change.
const LOW_BAND_HZ: (f32, f32) = (20.0, 100.0);
const MID_BAND_HZ: (f32, f32) = (400.0, 2_500.0);
const HIGH_BAND_HZ: (f32, f32) = (5_000.0, 20_000.0);

/// Resolve a `(freq_lo, freq_hi)` range to `(bin_start, bin_end)` in
/// the 1/6-octave grid used by the live spectrum analyser. `end` is
/// exclusive. Returns the widest possible range if the requested
/// frequencies fall outside the grid.
fn bins_for_range(range: (f32, f32)) -> (usize, usize) {
    let (lo, hi) = range;
    let mut start = NUM_SPECTRUM_BINS;
    let mut end = 0;
    for i in 0..NUM_SPECTRUM_BINS {
        let f = band_center_hz(i);
        if f >= lo && start == NUM_SPECTRUM_BINS {
            start = i;
        }
        if f <= hi {
            end = i + 1;
        }
    }
    if start >= end {
        (0, NUM_SPECTRUM_BINS)
    } else {
        (start, end)
    }
}

#[derive(Debug, Clone)]
pub struct Suggestions {
    pub target_label: String,
    pub target_lufs: f32,
    pub input_trim_db: f32,
    pub limiter_enabled: bool,
    pub limiter_ceiling_db: f32,
    pub limiter_release_ms: f32,
    pub glue_enabled: bool,
    pub glue_threshold_db: f32,
    pub glue_ratio: f32,
    pub glue_attack_ms: f32,
    pub glue_release_ms: f32,
    pub glue_makeup_db: f32,
    pub tonal_low_shelf_gain_db: f32,
    pub tonal_high_shelf_gain_db: f32,
    pub imager_enabled: bool,
    pub imager_width: f32,
    pub imager_side_hpf: bool,
    pub rationale: Vec<String>,
}

impl Suggestions {
    /// Write every suggested value into the plugin's atomic parameters.
    /// Only the stages the engine has an opinion about are touched —
    /// the rest of the chain is left alone. Note that the tonal-EQ
    /// low/high shelves overwrite band 0 and band 3 of the tonal EQ
    /// respectively: any custom shapes the user had placed on those
    /// slots are replaced.
    pub fn apply_to(&self, params: &MasteringParams) {
        params.target_lufs.set_value(self.target_lufs);
        params.input_trim_db.set_value(self.input_trim_db);

        params.limiter.on.set_value(self.limiter_enabled);
        params.limiter.ceiling.set_value(self.limiter_ceiling_db);
        params.limiter.release.set_value(self.limiter_release_ms);

        params.glue_compressor.on.set_value(self.glue_enabled);
        params
            .glue_compressor
            .threshold
            .set_value(self.glue_threshold_db);
        params.glue_compressor.ratio.set_value(self.glue_ratio);
        params.glue_compressor.attack.set_value(self.glue_attack_ms);
        params
            .glue_compressor
            .release
            .set_value(self.glue_release_ms);
        params.glue_compressor.makeup.set_value(self.glue_makeup_db);

        if self.tonal_low_shelf_gain_db.abs() > 0.25 {
            let b = &params.tonal_eq.bands[0];
            b.on.set_value(true);
            b.band_type.set_value(BandType::LowShelf.to_index());
            b.freq.set_value(120.0);
            b.q.set_value(0.707);
            b.gain.set_value(self.tonal_low_shelf_gain_db);
        }
        if self.tonal_high_shelf_gain_db.abs() > 0.25 {
            let b = &params.tonal_eq.bands[3];
            b.on.set_value(true);
            b.band_type.set_value(BandType::HighShelf.to_index());
            b.freq.set_value(8_000.0);
            b.q.set_value(0.707);
            b.gain.set_value(self.tonal_high_shelf_gain_db);
        }

        params.imager.on.set_value(self.imager_enabled);
        if self.imager_enabled {
            params.imager.width.set_value(self.imager_width);
            params.imager.side_hpf_on.set_value(self.imager_side_hpf);
            if self.imager_side_hpf {
                params.imager.side_hpf_freq.set_value(120.0);
            }
        }
    }
}

pub fn build(analysis: &AnalysisResult, target: &Target) -> Suggestions {
    let target_curve = target.curve();
    let target_label = target.label();
    let target_lufs = target.target_lufs();
    let mut rationale = Vec::new();

    // Resolve band boundaries to bin indices. These depend on the
    // 1/6-octave grid so they're computed, not hard-coded.
    let (mid_start, mid_end) = bins_for_range(MID_BAND_HZ);
    let (low_start, low_end) = bins_for_range(LOW_BAND_HZ);
    let (high_start, high_end) = bins_for_range(HIGH_BAND_HZ);

    // 1. Normalize analyzed spectrum so its midrange average matches
    //    the target's. Without this step the absolute dB difference is
    //    meaningless — we only care about spectral *shape*.
    let analyzed = &analysis.spectrum_db;
    let analyzed_mid = mean_range(analyzed, mid_start, mid_end);
    let target_mid = mean_range(&target_curve, mid_start, mid_end);
    let offset = target_mid - analyzed_mid;

    // 2. Input trim — bring the signal close to the target loudness so
    //    that the rest of the chain (compressor, limiter) operates in a
    //    useful range. Clamped to the param's ±24 dB range.
    let loudness_gap = target_lufs - analysis.integrated_lufs;
    // Leave a few dB of headroom for the limiter to work with rather
    // than slamming exactly to target.
    let input_trim_db = (loudness_gap - 3.0).clamp(-24.0, 24.0);
    if input_trim_db.abs() >= 0.5 {
        rationale.push(format!(
            "Input trim: {:+.1} dB (input is {:.1} LU {} target)",
            input_trim_db,
            loudness_gap.abs(),
            direction_word(-loudness_gap),
        ));
    } else {
        rationale.push("Input level already near target.".to_string());
    }

    // 3. Measure low- and high-band divergence from the target curve.
    let low_diff = mean_diff(analyzed, &target_curve, low_start, low_end, offset);
    let high_diff = mean_diff(analyzed, &target_curve, high_start, high_end, offset);

    // Negative `diff` means the input is *below* the target → we'd
    // boost to match. Positive means *above* → we'd cut.
    let tonal_low_shelf_gain_db = (-low_diff).clamp(-6.0, 6.0);
    let tonal_high_shelf_gain_db = (-high_diff).clamp(-6.0, 6.0);

    if tonal_low_shelf_gain_db.abs() >= 0.25 {
        rationale.push(format!(
            "Low shelf: {:+.1} dB (input is {:.1} dB {} target in the low band)",
            tonal_low_shelf_gain_db,
            low_diff.abs(),
            direction_word(low_diff),
        ));
    } else {
        rationale.push("Low band already matches target.".to_string());
    }
    if tonal_high_shelf_gain_db.abs() >= 0.25 {
        rationale.push(format!(
            "High shelf: {:+.1} dB (input is {:.1} dB {} target in the high band)",
            tonal_high_shelf_gain_db,
            high_diff.abs(),
            direction_word(high_diff),
        ));
    } else {
        rationale.push("High band already matches target.".to_string());
    }

    // 4. Glue compressor decision based on crest factor.
    // Use the post-trim level to estimate how much the glue compressor
    // will reduce gain, so the makeup suggestion accounts for the trim.
    let estimated_lufs = analysis.integrated_lufs + input_trim_db;

    let (glue_enabled, glue_threshold_db, glue_ratio, glue_attack_ms, glue_release_ms, glue_makeup_db) =
        if analysis.crest_db > 15.0 {
            // Wide dynamics — gentle glue with slow attack to preserve transients.
            let makeup = estimate_glue_makeup(-18.0, 2.0, estimated_lufs);
            rationale.push(format!(
                "Glue compressor: gentle 2:1 at \u{2212}18 dB, {:.1} dB makeup (crest {:.1} dB leaves room for glue)",
                makeup, analysis.crest_db
            ));
            (true, -18.0, 2.0, 30.0, 200.0, makeup)
        } else if analysis.crest_db > 10.0 {
            // Moderate dynamics — slightly faster and heavier.
            let makeup = estimate_glue_makeup(-14.0, 2.5, estimated_lufs);
            rationale.push(format!(
                "Glue compressor: moderate 2.5:1 at \u{2212}14 dB, {:.1} dB makeup (crest {:.1} dB)",
                makeup, analysis.crest_db
            ));
            (true, -14.0, 2.5, 20.0, 150.0, makeup)
        } else {
            rationale.push(format!(
                "Glue compressor: disabled (crest {:.1} dB is already dense)",
                analysis.crest_db
            ));
            (false, -18.0, 2.0, 30.0, 150.0, 0.0)
        };

    // 5. Stereo imager decision based on correlation.
    let (imager_enabled, imager_width, imager_side_hpf) = if analysis.correlation > 0.92 {
        // Very mono / narrow — suggest gentle widening with a side HPF
        // to keep the low-end centered.
        rationale.push(format!(
            "Stereo imager: widen to 130% (correlation {:.2} is very narrow)",
            analysis.correlation
        ));
        (true, 1.3, true)
    } else if analysis.correlation > 0.80 {
        rationale.push(format!(
            "Stereo imager: widen to 115% (correlation {:.2} is slightly narrow)",
            analysis.correlation
        ));
        (true, 1.15, true)
    } else if analysis.correlation < 0.3 {
        // Very wide / out of phase — pull it in a bit.
        rationale.push(format!(
            "Stereo imager: narrow to 85% (correlation {:.2} is very wide, may collapse in mono)",
            analysis.correlation
        ));
        (true, 0.85, false)
    } else {
        rationale.push(format!(
            "Stereo width OK (correlation {:.2}).",
            analysis.correlation
        ));
        (false, 1.0, false)
    };

    // 6. Limiter + loudness target.
    let limiter_enabled = true;
    let limiter_ceiling_db = -0.3;
    let limiter_release_ms = 50.0;
    rationale.push(format!(
        "Limiter: on at {:.1} dBTP, release 50 ms",
        limiter_ceiling_db
    ));
    rationale.push(format!(
        "Target loudness: {:.1} LUFS ({})",
        target_lufs, target_label
    ));

    // 7. Loudness diagnostic — not a suggestion itself, just a fact.
    rationale.push(format!(
        "Input integrated loudness: {:.1} LUFS ({:+.1} LU from target)",
        analysis.integrated_lufs, loudness_gap
    ));

    Suggestions {
        target_label,
        target_lufs,
        input_trim_db,
        limiter_enabled,
        limiter_ceiling_db,
        limiter_release_ms,
        glue_enabled,
        glue_threshold_db,
        glue_ratio,
        glue_attack_ms,
        glue_release_ms,
        glue_makeup_db,
        tonal_low_shelf_gain_db,
        tonal_high_shelf_gain_db,
        imager_enabled,
        imager_width,
        imager_side_hpf,
        rationale,
    }
}

/// Rough makeup gain estimate: uses the estimated post-trim average
/// level (LUFS, close enough to dBFS for this purpose) to figure out
/// how much signal sits above the compressor threshold, then
/// compensates ~60% of the resulting gain reduction.
fn estimate_glue_makeup(threshold_db: f32, ratio: f32, estimated_lufs: f32) -> f32 {
    let above = (estimated_lufs - threshold_db).max(0.0);
    let reduction = above * (1.0 - 1.0 / ratio);
    // Compensate ~60% of the estimated reduction, clamped to sane range.
    (reduction * 0.6).clamp(0.0, 12.0)
}

fn mean_range(values: &[f32], start: usize, end: usize) -> f32 {
    let end = end.min(values.len());
    if start >= end {
        return 0.0;
    }
    let sum: f32 = values[start..end].iter().sum();
    sum / (end - start) as f32
}

fn mean_diff(
    analyzed: &[f32],
    target: &[f32],
    start: usize,
    end: usize,
    analyzed_offset: f32,
) -> f32 {
    let end = end.min(analyzed.len()).min(target.len());
    if start >= end {
        return 0.0;
    }
    let mut sum = 0.0_f32;
    for i in start..end {
        sum += (analyzed[i] + analyzed_offset) - target[i];
    }
    sum / (end - start) as f32
}

fn direction_word(diff: f32) -> &'static str {
    if diff > 0.0 {
        "above"
    } else {
        "below"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_analysis(crest_db: f32, spectrum: Vec<f32>) -> AnalysisResult {
        AnalysisResult {
            sample_rate: 48_000.0,
            duration_s: 10.0,
            integrated_lufs: -14.0,
            short_term_lufs: -14.0,
            true_peak_dbtp: -1.0,
            crest_db,
            correlation: 0.8,
            spectrum_db: spectrum,
        }
    }

    #[test]
    fn high_crest_enables_gentle_glue() {
        let flat = vec![0.0_f32; NUM_SPECTRUM_BINS];
        let a = dummy_analysis(18.0, flat);
        let s = build(&a, &Target::Genre(Genre::Rock));
        assert!(s.glue_enabled);
        assert!((s.glue_ratio - 2.0).abs() < 1e-6);
        assert!(s.glue_makeup_db > 0.0, "expected positive makeup gain");
    }

    #[test]
    fn low_crest_disables_glue() {
        let flat = vec![0.0_f32; NUM_SPECTRUM_BINS];
        let a = dummy_analysis(7.0, flat);
        let s = build(&a, &Target::Genre(Genre::Rock));
        assert!(!s.glue_enabled);
    }

    #[test]
    fn bass_heavy_input_suggests_low_shelf_cut() {
        // Build an analyzed spectrum that is +6 dB louder than target
        // in the low band. Expect a negative low-shelf gain suggestion.
        let target = target_curve(Genre::Rock);
        let mut analyzed: Vec<f32> = target.to_vec();
        let (lo, hi) = bins_for_range(LOW_BAND_HZ);
        for i in lo..hi {
            analyzed[i] += 6.0;
        }
        let a = dummy_analysis(14.0, analyzed);
        let s = build(&a, &Target::Genre(Genre::Rock));
        assert!(
            s.tonal_low_shelf_gain_db < -2.0,
            "expected a low-shelf cut, got {}",
            s.tonal_low_shelf_gain_db
        );
    }

    #[test]
    fn dim_top_suggests_high_shelf_boost() {
        let target = target_curve(Genre::Rock);
        let mut analyzed: Vec<f32> = target.to_vec();
        let (lo, hi) = bins_for_range(HIGH_BAND_HZ);
        for i in lo..hi {
            analyzed[i] -= 6.0;
        }
        let a = dummy_analysis(14.0, analyzed);
        let s = build(&a, &Target::Genre(Genre::Rock));
        assert!(
            s.tonal_high_shelf_gain_db > 2.0,
            "expected a high-shelf boost, got {}",
            s.tonal_high_shelf_gain_db
        );
    }

    #[test]
    fn quiet_input_suggests_positive_trim() {
        let flat = vec![0.0_f32; NUM_SPECTRUM_BINS];
        let mut a = dummy_analysis(14.0, flat);
        a.integrated_lufs = -34.0;
        let s = build(&a, &Target::Genre(Genre::Rock));
        // Target is -11 LUFS, input is -34 → gap of 23 LU → trim ~20 dB
        assert!(
            s.input_trim_db > 15.0,
            "expected large positive trim, got {}",
            s.input_trim_db
        );
    }

    #[test]
    fn narrow_stereo_suggests_widening() {
        let flat = vec![0.0_f32; NUM_SPECTRUM_BINS];
        let mut a = dummy_analysis(14.0, flat);
        a.correlation = 0.95;
        let s = build(&a, &Target::Genre(Genre::Rock));
        assert!(s.imager_enabled);
        assert!(
            s.imager_width > 1.0,
            "expected widening, got {}",
            s.imager_width
        );
    }

    #[test]
    fn very_wide_stereo_suggests_narrowing() {
        let flat = vec![0.0_f32; NUM_SPECTRUM_BINS];
        let mut a = dummy_analysis(14.0, flat);
        a.correlation = 0.1;
        let s = build(&a, &Target::Genre(Genre::Rock));
        assert!(s.imager_enabled);
        assert!(
            s.imager_width < 1.0,
            "expected narrowing, got {}",
            s.imager_width
        );
    }
}
