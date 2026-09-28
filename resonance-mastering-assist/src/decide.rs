//! Rule-based decision engine.
//!
//! Takes an offline [`AnalysisResult`] plus a [`Target`] (a built-in
//! genre band or a loaded reference track), compares the analyzed
//! spectrum to the target's band, derives practical suggestions (input
//! trim, tonal shelves, glue compressor, stereo imager, limiter), and
//! packages them into a [`Suggestions`] struct with human-readable
//! rationale. The UI displays the rationale verbatim so the user can
//! see *why* each decision was made before applying it.
//!
//! The spectral comparison is against a **band**, not a curve
//! (warmth-width-depth.md §7.4, D6): wherever the analyzed spectrum lies
//! inside the target's `[lo, hi]` it counts as on target, and only the
//! part outside the band — the excess over `hi` or the shortfall under
//! `lo` — drives a shelf.

use resonance_dsp::BandType;

use crate::analyze::{AnalysisResult, NUM_SPECTRUM_BINS};
use crate::reference::ReferenceTrack;
use crate::targets::{
    band_center_hz, target_band, target_band_center_hz, Genre, NUM_TARGET_BANDS,
};

/// Half-width of the band around a reference track's spectrum, dB. A
/// reference is one recording, not an average, so it gets no genre-style
/// tolerance of its own (it generates no target, D6) — only enough slack
/// that measurement noise between two different songs does not read as a
/// tonal fault.
pub const REFERENCE_TOLERANCE_DB: f32 = 1.0;

/// What the decision engine should compare the analyzed input against.
#[derive(Debug, Clone)]
pub enum Target {
    /// Built-in genre target band.
    Genre(Genre),
    /// A loaded reference track: its spectrum ±
    /// [`REFERENCE_TOLERANCE_DB`], and its loudness.
    Reference(ReferenceTrack),
}

impl Target {
    pub fn label(&self) -> String {
        match self {
            Target::Genre(g) => g.label().to_string(),
            Target::Reference(r) => r.display_name.clone(),
        }
    }

    /// Target band on the 1/6-octave analysis grid: `(lo, hi)`.
    pub fn band(&self) -> ([f32; NUM_SPECTRUM_BINS], [f32; NUM_SPECTRUM_BINS]) {
        match self {
            Target::Genre(g) => target_band(*g),
            Target::Reference(r) => {
                let mut lo = [0.0_f32; NUM_SPECTRUM_BINS];
                let mut hi = [0.0_f32; NUM_SPECTRUM_BINS];
                let src = &r.analysis.spectrum_db;
                for i in 0..NUM_SPECTRUM_BINS {
                    let v = src.get(i).copied().unwrap_or(0.0);
                    lo[i] = v - REFERENCE_TOLERANCE_DB;
                    hi[i] = v + REFERENCE_TOLERANCE_DB;
                }
                (lo, hi)
            }
        }
    }

    /// Target spectral shape (the band's midline, 60 values at
    /// 1/6-octave spacing).
    pub fn curve(&self) -> [f32; NUM_SPECTRUM_BINS] {
        let (lo, hi) = self.band();
        let mut out = [0.0_f32; NUM_SPECTRUM_BINS];
        for i in 0..NUM_SPECTRUM_BINS {
            out[i] = 0.5 * (lo[i] + hi[i]);
        }
        out
    }

    /// Target integrated loudness.
    pub fn target_lufs(&self) -> f32 {
        match self {
            Target::Genre(g) => g.target_lufs(),
            Target::Reference(r) => r.analysis.integrated_lufs,
        }
    }
}

/// How one 1/3-octave band of the analyzed spectrum sits against the
/// target band, after the midrange alignment [`build`] applies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandDeviation {
    /// Band centre, Hz (exact ISO series, see
    /// [`target_band_center_hz`]).
    pub center_hz: f32,
    /// Lowest on-target level, dB (relative).
    pub lo_db: f32,
    /// Highest on-target level, dB (relative).
    pub hi_db: f32,
    /// The analyzed spectrum here, aligned to the target's midrange, dB.
    pub measured_db: f32,
    /// How far outside the band it lies: positive above `hi`, negative
    /// below `lo`, 0 inside.
    pub deviation_db: f32,
}

/// Frequency boundaries (Hz) of the spectral bands the decision
/// engine reasons about. [`bins_for_range`] resolves these to
/// concrete 1/6-octave bin indices at runtime so the logic stays
/// correct if `NUM_SPECTRUM_BINS` or the octave grid ever change.
#[doc(hidden)]
pub const LOW_BAND_HZ: (f32, f32) = (20.0, 100.0);
const MID_BAND_HZ: (f32, f32) = (400.0, 2_500.0);
#[doc(hidden)]
pub const HIGH_BAND_HZ: (f32, f32) = (5_000.0, 20_000.0);

/// Resolve a `(freq_lo, freq_hi)` range to `(bin_start, bin_end)` in
/// the 1/6-octave grid used by the live spectrum analyser. `end` is
/// exclusive. Returns the widest possible range if the requested
/// frequencies fall outside the grid.
#[doc(hidden)]
pub fn bins_for_range(range: (f32, f32)) -> (usize, usize) {
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
    /// The limiter's input gain (`lim_gain`): the part of the loudness
    /// gap the input trim leaves for the limiter to push.
    pub limiter_gain_db: f32,
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
    /// Every rationale line with the stage it is about (one of the
    /// `STAGE_*` ids), in the same order as `rationale`.
    pub stage_notes: Vec<(&'static str, String)>,
    /// Per-1/3-octave comparison against the target band, 20 Hz first
    /// ([`NUM_TARGET_BANDS`] entries).
    pub deviations: Vec<BandDeviation>,
}

/// Stage ids of [`StageSuggestion::stage`], in the order [`build`]
/// decides them.
pub const STAGE_INPUT_TRIM: &str = "input_trim";
pub const STAGE_TONAL_LOW_SHELF: &str = "tonal_low_shelf";
pub const STAGE_TONAL_HIGH_SHELF: &str = "tonal_high_shelf";
pub const STAGE_GLUE: &str = "glue";
pub const STAGE_IMAGER: &str = "imager";
pub const STAGE_LIMITER: &str = "limiter";
pub const STAGE_TARGET_LUFS: &str = "target_lufs";
/// A fact about the input, not a move: never carries params.
pub const STAGE_DIAGNOSTIC: &str = "diagnostic";

/// Frequency and Q the tonal shelves are placed at.
const LOW_SHELF_HZ: f32 = 120.0;
const HIGH_SHELF_HZ: f32 = 8_000.0;
const SHELF_Q: f32 = 0.707;
/// A shelf smaller than this is not worth a band.
const SHELF_MIN_DB: f32 = 0.25;
/// Where the imager's side high-pass is put when it is suggested.
const SIDE_HPF_HZ: f32 = 120.0;
/// How much of the loudness gap goes to the limiter's input gain rather
/// than the input trim: the trim puts the chain this far under the
/// target, and `lim_gain` pushes the limiter the rest of the way.
const LIMITER_PUSH_DB: f32 = 3.0;
/// `lim_gain`'s range.
const LIMITER_GAIN_MAX_DB: f32 = 18.0;

/// The mastering plugin's M/S selector value for a stereo band (its
/// `MsMode::Stereo` index; the plugin's lockstep test pins the two
/// together).
pub const MS_STEREO_INDEX: f32 = 0.0;

/// Every param key [`Suggestions::stages`] can emit. The plugin's test
/// resolves each one against its params; this crate's test checks that
/// nothing emitted is missing here.
pub const EMITTED_PARAM_KEYS: &[&str] = &[
    "input_trim_db",
    "tone_b0_on",
    "tone_b0_type",
    "tone_b0_freq",
    "tone_b0_q",
    "tone_b0_gain",
    "tone_b0_ms",
    "tone_b3_on",
    "tone_b3_type",
    "tone_b3_freq",
    "tone_b3_q",
    "tone_b3_gain",
    "tone_b3_ms",
    "glue_on",
    "glue_threshold",
    "glue_ratio",
    "glue_attack",
    "glue_release",
    "glue_makeup",
    "img_on",
    "img_width",
    "img_side_hpf_on",
    "img_side_hpf_freq",
    "lim_on",
    "lim_ceiling",
    "lim_release",
    "lim_gain",
    "target_lufs",
];

/// Where suggested param writes go: the mastering plugin implements it
/// for its params by key. Keys a sink does not know are its to ignore.
pub trait ParamSink {
    /// Set the param whose string id is `key` to the plain `value`.
    fn set_param(&self, key: &str, value: f32);
}

/// One parameter write: a plugin param key (`"lim_ceiling"`,
/// `"tone_b0_gain"`, ...) and its plain value — a bool as 0/1, a choice as
/// its index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamChange {
    pub key: &'static str,
    pub value: f32,
}

/// What the engine suggests for one stage: why, and exactly which param
/// writes that is. Empty `params` means the stage needs no change (the
/// rationale says why).
#[derive(Debug, Clone, PartialEq)]
pub struct StageSuggestion {
    /// One of the `STAGE_*` ids.
    pub stage: &'static str,
    pub rationale: Vec<String>,
    pub params: Vec<ParamChange>,
}

impl Suggestions {
    /// The suggestions stage by stage, each with its rationale and the
    /// exact param writes it consists of — what [`Self::apply_to`] writes,
    /// and what an agent sets through the control API.
    ///
    /// The tonal shelves use band 0 (low shelf) and band 3 (high shelf)
    /// of the tonal EQ: applying one replaces whatever the user had placed
    /// on that band, its M/S selector included (a shelf is Stereo).
    pub fn stages(&self) -> Vec<StageSuggestion> {
        let bool_value = |b: bool| if b { 1.0 } else { 0.0 };
        let change = |key: &'static str, value: f32| ParamChange { key, value };
        // A shelf is written as a stereo band: left on Mid or Side from
        // an earlier edit, it would move only half the image while the
        // rationale speaks of the whole spectrum.
        let shelf = |prefix: [&'static str; 6], band: BandType, gain: f32| {
            if gain.abs() > SHELF_MIN_DB {
                let freq = if band == BandType::LowShelf {
                    LOW_SHELF_HZ
                } else {
                    HIGH_SHELF_HZ
                };
                vec![
                    change(prefix[0], 1.0),
                    change(prefix[1], band.to_index() as f32),
                    change(prefix[2], freq),
                    change(prefix[3], SHELF_Q),
                    change(prefix[4], gain),
                    change(prefix[5], MS_STEREO_INDEX),
                ]
            } else {
                Vec::new()
            }
        };
        let mut imager = vec![change("img_on", bool_value(self.imager_enabled))];
        if self.imager_enabled {
            imager.push(change("img_width", self.imager_width));
            imager.push(change("img_side_hpf_on", bool_value(self.imager_side_hpf)));
            if self.imager_side_hpf {
                imager.push(change("img_side_hpf_freq", SIDE_HPF_HZ));
            }
        }
        let params_of = |stage: &str| -> Vec<ParamChange> {
            match stage {
                STAGE_INPUT_TRIM => vec![change("input_trim_db", self.input_trim_db)],
                STAGE_TONAL_LOW_SHELF => shelf(
                    [
                        "tone_b0_on",
                        "tone_b0_type",
                        "tone_b0_freq",
                        "tone_b0_q",
                        "tone_b0_gain",
                        "tone_b0_ms",
                    ],
                    BandType::LowShelf,
                    self.tonal_low_shelf_gain_db,
                ),
                STAGE_TONAL_HIGH_SHELF => shelf(
                    [
                        "tone_b3_on",
                        "tone_b3_type",
                        "tone_b3_freq",
                        "tone_b3_q",
                        "tone_b3_gain",
                        "tone_b3_ms",
                    ],
                    BandType::HighShelf,
                    self.tonal_high_shelf_gain_db,
                ),
                STAGE_GLUE => vec![
                    change("glue_on", bool_value(self.glue_enabled)),
                    change("glue_threshold", self.glue_threshold_db),
                    change("glue_ratio", self.glue_ratio),
                    change("glue_attack", self.glue_attack_ms),
                    change("glue_release", self.glue_release_ms),
                    change("glue_makeup", self.glue_makeup_db),
                ],
                STAGE_IMAGER => imager.clone(),
                STAGE_LIMITER => vec![
                    change("lim_on", bool_value(self.limiter_enabled)),
                    change("lim_ceiling", self.limiter_ceiling_db),
                    change("lim_release", self.limiter_release_ms),
                    change("lim_gain", self.limiter_gain_db),
                ],
                STAGE_TARGET_LUFS => vec![change("target_lufs", self.target_lufs)],
                _ => Vec::new(),
            }
        };
        let mut out: Vec<StageSuggestion> = Vec::new();
        for (stage, line) in &self.stage_notes {
            match out.iter_mut().find(|s| s.stage == *stage) {
                Some(existing) => existing.rationale.push(line.clone()),
                None => out.push(StageSuggestion {
                    stage,
                    rationale: vec![line.clone()],
                    params: params_of(stage),
                }),
            }
        }
        out
    }

    /// Write every suggested value into `params` (the plugin's atomic
    /// parameters) — exactly the param writes [`Self::stages`] lists, by
    /// key. Only the stages the engine has an opinion about are touched;
    /// the rest of the chain is left alone.
    pub fn apply_to<P: ParamSink + ?Sized>(&self, params: &P) {
        for stage in self.stages() {
            for change in stage.params {
                params.set_param(change.key, change.value);
            }
        }
    }
}

pub fn build(analysis: &AnalysisResult, target: &Target) -> Suggestions {
    let (band_lo, band_hi) = target.band();
    let target_label = target.label();
    let target_lufs = target.target_lufs();
    // Every rationale line, with the stage it is about.
    let mut stage_notes: Vec<(&'static str, String)> = Vec::new();
    let mut say = |stage: &'static str, line: String| stage_notes.push((stage, line));

    // Resolve band boundaries to bin indices. These depend on the
    // 1/6-octave grid so they're computed, not hard-coded.
    let (mid_start, mid_end) = bins_for_range(MID_BAND_HZ);
    let (low_start, low_end) = bins_for_range(LOW_BAND_HZ);
    let (high_start, high_end) = bins_for_range(HIGH_BAND_HZ);

    // 1. Align the analyzed spectrum so its midrange average matches the
    //    band midline's. Without this step the absolute dB difference is
    //    meaningless — we only care about spectral *shape*.
    let analyzed = &analysis.spectrum_db;
    let mut midline = [0.0_f32; NUM_SPECTRUM_BINS];
    for i in 0..NUM_SPECTRUM_BINS {
        midline[i] = 0.5 * (band_lo[i] + band_hi[i]);
    }
    let analyzed_mid = mean_range(analyzed, mid_start, mid_end);
    let target_mid = mean_range(&midline, mid_start, mid_end);
    let offset = target_mid - analyzed_mid;

    // Per-bin distance outside the band (0 inside it).
    let outside: Vec<f32> = (0..NUM_SPECTRUM_BINS)
        .map(|i| {
            let v = analyzed.get(i).copied().unwrap_or(crate::analyze::FLOOR_DB) + offset;
            outside_band(v, band_lo[i], band_hi[i])
        })
        .collect();
    let deviations = band_deviations(analyzed, offset, &band_lo, &band_hi);

    // 2. Input trim — bring the signal close to the target loudness so
    //    that the rest of the chain (compressor, limiter) operates in a
    //    useful range. Clamped to the param's ±24 dB range.
    let loudness_gap = target_lufs - analysis.integrated_lufs;
    // Stop a few dB short of the target: the last of the loudness is
    // the limiter's input gain (step 6), so it is pushed into the
    // limiter after the clipper instead of into every stage before it.
    let input_trim_db = (loudness_gap - LIMITER_PUSH_DB).clamp(-24.0, 24.0);
    let limiter_gain_db = (loudness_gap - input_trim_db).clamp(0.0, LIMITER_GAIN_MAX_DB);
    if input_trim_db.abs() >= 0.5 {
        say(STAGE_INPUT_TRIM, format!(
            "Input trim: {:+.1} dB (input is {:.1} LU {} target)",
            input_trim_db,
            loudness_gap.abs(),
            direction_word(-loudness_gap),
        ));
    } else {
        say(
            STAGE_INPUT_TRIM,
            "Input level already near target.".to_string(),
        );
    }

    // 3. Measure how far the low and high bands lie OUTSIDE the target
    //    band. Anything inside it is on target and moves nothing.
    let low_diff = mean_range(&outside, low_start, low_end);
    let high_diff = mean_range(&outside, high_start, high_end);

    // Negative `diff` means the input is *below* the band → we'd boost
    // to reach it. Positive means *above* → we'd cut.
    let tonal_low_shelf_gain_db = (-low_diff).clamp(-6.0, 6.0);
    let tonal_high_shelf_gain_db = (-high_diff).clamp(-6.0, 6.0);

    if tonal_low_shelf_gain_db.abs() >= 0.25 {
        say(STAGE_TONAL_LOW_SHELF, format!(
            "Low shelf: {:+.1} dB (input is {:.1} dB {} the target band in the low band)",
            tonal_low_shelf_gain_db,
            low_diff.abs(),
            direction_word(low_diff),
        ));
    } else {
        say(
            STAGE_TONAL_LOW_SHELF,
            "Low band is inside the target band.".to_string(),
        );
    }
    if tonal_high_shelf_gain_db.abs() >= 0.25 {
        say(STAGE_TONAL_HIGH_SHELF, format!(
            "High shelf: {:+.1} dB (input is {:.1} dB {} the target band in the high band)",
            tonal_high_shelf_gain_db,
            high_diff.abs(),
            direction_word(high_diff),
        ));
    } else {
        say(
            STAGE_TONAL_HIGH_SHELF,
            "High band is inside the target band.".to_string(),
        );
    }

    // 4. Glue compressor decision based on crest factor.
    // Use the post-trim level to estimate how much the glue compressor
    // will reduce gain, so the makeup suggestion accounts for the trim.
    let estimated_lufs = analysis.integrated_lufs + input_trim_db;

    let (glue_enabled, glue_threshold_db, glue_ratio, glue_attack_ms, glue_release_ms, glue_makeup_db) =
        if analysis.crest_db > 15.0 {
            // Wide dynamics — gentle glue with slow attack to preserve transients.
            let makeup = estimate_glue_makeup(-18.0, 2.0, estimated_lufs);
            say(STAGE_GLUE, format!(
                "Glue compressor: gentle 2:1 at \u{2212}18 dB, {:.1} dB makeup (crest {:.1} dB leaves room for glue)",
                makeup, analysis.crest_db
            ));
            (true, -18.0, 2.0, 30.0, 200.0, makeup)
        } else if analysis.crest_db > 10.0 {
            // Moderate dynamics — slightly faster and heavier.
            let makeup = estimate_glue_makeup(-14.0, 2.5, estimated_lufs);
            say(STAGE_GLUE, format!(
                "Glue compressor: moderate 2.5:1 at \u{2212}14 dB, {:.1} dB makeup (crest {:.1} dB)",
                makeup, analysis.crest_db
            ));
            (true, -14.0, 2.5, 20.0, 150.0, makeup)
        } else {
            say(STAGE_GLUE, format!(
                "Glue compressor: disabled (crest {:.1} dB is already dense)",
                analysis.crest_db
            ));
            (false, -18.0, 2.0, 30.0, 150.0, 0.0)
        };

    // 5. Stereo imager decision based on correlation.
    let (imager_enabled, imager_width, imager_side_hpf) = if analysis.correlation > 0.92 {
        // Very mono / narrow — suggest gentle widening with a side HPF
        // to keep the low-end centered.
        say(STAGE_IMAGER, format!(
            "Stereo imager: widen to 130% (correlation {:.2} is very narrow)",
            analysis.correlation
        ));
        (true, 1.3, true)
    } else if analysis.correlation > 0.80 {
        say(STAGE_IMAGER, format!(
            "Stereo imager: widen to 115% (correlation {:.2} is slightly narrow)",
            analysis.correlation
        ));
        (true, 1.15, true)
    } else if analysis.correlation < 0.3 {
        // Very wide / out of phase — pull it in a bit.
        say(STAGE_IMAGER, format!(
            "Stereo imager: narrow to 85% (correlation {:.2} is very wide, may collapse in mono)",
            analysis.correlation
        ));
        (true, 0.85, false)
    } else {
        say(STAGE_IMAGER, format!(
            "Stereo width OK (correlation {:.2}).",
            analysis.correlation
        ));
        (false, 1.0, false)
    };

    // 6. Limiter + loudness target.
    let limiter_enabled = true;
    let limiter_ceiling_db = -0.3;
    let limiter_release_ms = 50.0;
    say(STAGE_LIMITER, format!(
        "Limiter: on at {:.1} dBTP, release 50 ms, {:+.1} dB of gain into it",
        limiter_ceiling_db, limiter_gain_db
    ));
    say(STAGE_TARGET_LUFS, format!(
        "Target loudness: {:.1} LUFS ({})",
        target_lufs, target_label
    ));

    // 7. Loudness diagnostic — not a suggestion itself, just a fact.
    say(STAGE_DIAGNOSTIC, format!(
        "Input integrated loudness: {:.1} LUFS ({:+.1} LU from target)",
        analysis.integrated_lufs, loudness_gap
    ));

    let rationale = stage_notes.iter().map(|(_, line)| line.clone()).collect();
    Suggestions {
        target_label,
        target_lufs,
        input_trim_db,
        limiter_enabled,
        limiter_ceiling_db,
        limiter_release_ms,
        limiter_gain_db,
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
        stage_notes,
        deviations,
    }
}

/// How far `v` lies outside `[lo, hi]`: positive above, negative below,
/// 0 inside.
fn outside_band(v: f32, lo: f32, hi: f32) -> f32 {
    if v > hi {
        v - hi
    } else if v < lo {
        v - lo
    } else {
        0.0
    }
}

/// The analyzed spectrum (aligned by `offset`) against the band, read at
/// each 1/3-octave centre by log-frequency interpolation of the 1/6-octave
/// analysis grid.
fn band_deviations(
    analyzed: &[f32],
    offset: f32,
    band_lo: &[f32; NUM_SPECTRUM_BINS],
    band_hi: &[f32; NUM_SPECTRUM_BINS],
) -> Vec<BandDeviation> {
    (0..NUM_TARGET_BANDS)
        .map(|i| {
            let f = target_band_center_hz(i);
            let measured = grid_at(analyzed, f) + offset;
            let lo = grid_at(band_lo, f);
            let hi = grid_at(band_hi, f);
            BandDeviation {
                center_hz: f,
                lo_db: lo,
                hi_db: hi,
                measured_db: measured,
                deviation_db: outside_band(measured, lo, hi),
            }
        })
        .collect()
}

/// A value on the 1/6-octave analysis grid at `freq`: linear in
/// log-frequency between bin centres, held flat past either end.
fn grid_at(values: &[f32], freq: f32) -> f32 {
    let n = values.len().min(NUM_SPECTRUM_BINS);
    if n == 0 {
        return 0.0;
    }
    let first = band_center_hz(0);
    let step = (band_center_hz(1) / first).log2();
    let pos = (freq / first).log2() / step;
    if pos <= 0.0 {
        return values[0];
    }
    if pos >= (n - 1) as f32 {
        return values[n - 1];
    }
    let i = pos.floor() as usize;
    let t = pos - i as f32;
    values[i] + (values[i + 1] - values[i]) * t
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

fn direction_word(diff: f32) -> &'static str {
    if diff > 0.0 {
        "above"
    } else {
        "below"
    }
}

