//! [`BankModels`]: the bank's own models loaded for a render, and
//! [`BankModels::predict`], which chains them over a unit (see the
//! [module docs](super)).

use anyhow::Result;

use super::config::{BankModelPaths, ModelKind};
use super::run::{DurationModel, PitchModel, VarianceCurves, VarianceModel};
use super::score::{align_durations, BankScore};
use crate::pipeline::phoneme_durations_to_frames;
use crate::stages::common::{warn_once, ExecutionProvider};

/// Which models to run, and how.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PredictOptions {
    /// Time the phonemes with the duration model.
    pub timing: bool,
    /// Take f0 from the pitch model.
    pub pitch: bool,
    /// Predict the variance curves (only worth it when the acoustic model
    /// takes one of them).
    pub variance: bool,
    /// The pitch model's expressiveness, 0..1 (1 = as trained).
    pub expr: f32,
    pub pitch_steps: i64,
    pub variance_steps: i64,
}

impl Default for PredictOptions {
    fn default() -> Self {
        Self {
            timing: true,
            pitch: true,
            variance: true,
            expr: 1.0,
            pitch_steps: PITCH_STEPS,
            variance_steps: VARIANCE_STEPS,
        }
    }
}

/// Sampling steps of the pitch model (OpenUtau's default; Meiji's notes
/// recommend the same).
pub const PITCH_STEPS: i64 = 10;
/// Sampling steps of the variance model (OpenUtau's default).
pub const VARIANCE_STEPS: i64 = 20;

/// The loaded models of one bank; `None` for one it does not ship or that
/// failed to load (a warning says why, once).
#[derive(Default)]
pub struct BankModels {
    pub duration: Option<DurationModel>,
    pub pitch: Option<PitchModel>,
    pub variance: Option<VarianceModel>,
    /// The models [`Self::load`] was asked for, loaded or not: a part
    /// asked for and not delivered is a fallback ([`Prediction::fallback`]).
    wanted: Wanted,
}

#[derive(Debug, Clone, Copy, Default)]
struct Wanted {
    timing: bool,
    pitch: bool,
    variance: bool,
}

/// What [`BankModels::predict`] gives a unit.
#[derive(Debug, Clone, Default)]
pub struct Prediction {
    /// Seconds per phoneme, adding up to the unit's length: the duration
    /// model's when it ran, else the rule-based ones.
    pub ph_dur: Vec<f64>,
    /// Whether [`Self::ph_dur`] came from the duration model.
    pub timed_by_model: bool,
    /// Pitch per frame (fractional MIDI, pitch bend included) when the
    /// pitch model ran.
    pub pitch: Option<Vec<f32>>,
    /// Seconds per pitch / variance frame.
    pub frame_sec: f64,
    /// The variance model's curves (per frame), when it ran.
    pub variance: VarianceCurves,
    /// A model this prediction was asked for did not load or failed to
    /// run, so its part came from the rules: the unit does not sound like
    /// its key says (the caller must not cache it as such).
    pub fallback: bool,
}

impl BankModels {
    /// Load the models `paths` lists that `opts` asks for.
    pub fn load(paths: &BankModelPaths, opts: &PredictOptions, ep: ExecutionProvider) -> Self {
        fn warn<T>(path: &std::path::Path, r: Result<T>) -> Option<T> {
            r.map_err(|e| warn_once(path, format!("bank model not used: {e:#}"))).ok()
        }
        let facts = |kind: ModelKind| paths.facts(kind).cloned().unwrap_or_default();
        let wanted = Wanted {
            timing: opts.timing && paths.duration.is_some(),
            pitch: opts.pitch && paths.pitch.is_some(),
            variance: opts.variance && opts.pitch && paths.variance.is_some(),
        };
        Self {
            duration: paths
                .duration
                .as_deref()
                .filter(|_| wanted.timing)
                .and_then(|p| warn(p, DurationModel::load(p, ep, facts(ModelKind::Duration)))),
            pitch: paths
                .pitch
                .as_deref()
                .filter(|_| wanted.pitch)
                .and_then(|p| warn(p, PitchModel::load(p, ep, facts(ModelKind::Pitch)))),
            variance: paths
                .variance
                .as_deref()
                .filter(|_| wanted.variance)
                .and_then(|p| warn(p, VarianceModel::load(p, ep, facts(ModelKind::Variance)))),
            wanted,
        }
    }

    /// Run the loaded models over `score` for acoustic speaker `speaker`.
    /// `bend_cents(t)` is the pitch-bend offset at `t` seconds into the
    /// unit, added to the pitch model's output. A model that fails falls
    /// back to the rules for its part (a warning says why, once per
    /// process) and marks the prediction [`Prediction::fallback`]; the
    /// others still run.
    pub fn predict(
        &mut self,
        score: &BankScore,
        speaker: Option<&str>,
        seed: u64,
        opts: &PredictOptions,
        bend_cents: &dyn Fn(f64) -> f64,
    ) -> Prediction {
        let mut out = Prediction {
            ph_dur: score.rule_durations(),
            ..Prediction::default()
        };
        let fail = |what: &str, e: anyhow::Error| {
            warn_once(std::path::Path::new(what), format!("failed, the rules stand in: {e:#}"));
        };
        if let Some(model) = self.duration.as_mut().filter(|_| opts.timing) {
            match model.predict(score, speaker) {
                Ok(pred) => {
                    out.ph_dur = align_durations(score, &pred, model.frame_sec());
                    out.timed_by_model = true;
                }
                Err(e) => fail("bank duration model", e),
            }
        }
        out.fallback |= self.wanted.timing && opts.timing && !out.timed_by_model;
        let wants_variance = self.wanted.variance && opts.variance;
        let Some(pitch_model) = self.pitch.as_mut().filter(|_| opts.pitch) else {
            out.fallback |= self.wanted.pitch && opts.pitch;
            return out;
        };
        let frame_sec = pitch_model.frame_sec();
        out.frame_sec = frame_sec;
        let ph_frames = phoneme_durations_to_frames(&out.ph_dur, frame_sec);
        let pitch = match pitch_model.predict(
            score,
            &ph_frames,
            speaker,
            opts.expr,
            opts.pitch_steps,
            seed ^ 0x5049_5443_485f_5345,
        ) {
            Ok(mut p) => {
                for (i, v) in p.iter_mut().enumerate() {
                    *v += (bend_cents(i as f64 * frame_sec) / 100.0) as f32;
                }
                p
            }
            Err(e) => {
                fail("bank pitch model", e);
                out.fallback = true;
                return out;
            }
        };
        let mut varied = false;
        if let Some(model) = self.variance.as_mut().filter(|_| opts.variance) {
            match model.predict(
                score,
                &ph_frames,
                &pitch,
                speaker,
                opts.variance_steps,
                seed ^ 0x5641_5249_414e_4345,
            ) {
                Ok(v) => {
                    out.variance = v;
                    varied = true;
                }
                Err(e) => fail("bank variance model", e),
            }
        }
        out.fallback |= wants_variance && !varied;
        out.pitch = Some(pitch);
        out
    }
}
