//! The voicebank's own duration, pitch and variance models (`dsdur`,
//! `dspitch`, `dsvariance`), run per render unit to replace the app's
//! rule-based phoneme timing and synthesized f0 with what the bank's
//! author trained.
//!
//! * [`config`] — finding the models next to the acoustic config and
//!   reading their `dsconfig.yaml`s.
//! * [`score`] — the unit as the models see it ([`BankScore`]) and the
//!   pure input building / alignment, testable without ONNX.
//! * [`run`] — the sessions.
//!
//! [`BankModels::predict`] chains them: durations (aligned to the beats),
//! then pitch on those durations (plus the caller's pitch-bend cents),
//! then the variance curves on that pitch. Each diffusion model's noise is
//! seeded ([`crate::stages::noise`]), so a prediction is a pure function
//! of the score, the speaker and the seed.

pub mod config;
pub mod run;
pub mod score;

use anyhow::Result;

pub use config::{discover, BankModelPaths, ModelConfig, ModelKind};
pub use run::{DurationModel, PitchModel, VarianceCurves, VarianceModel};
pub use score::{
    align_durations, fit_frames, hz_to_midi, midi_to_hz, pitch_notes, BankScore, ScoreGroup,
    ScoreNote, ScorePhone,
};

use crate::pipeline::phoneme_durations_to_frames;
use crate::stages::common::ExecutionProvider;

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
/// failed to load (a warning says why).
#[derive(Default)]
pub struct BankModels {
    pub duration: Option<DurationModel>,
    pub pitch: Option<PitchModel>,
    pub variance: Option<VarianceModel>,
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
}

impl BankModels {
    /// Load the models `paths` lists that `opts` asks for.
    pub fn load(paths: &BankModelPaths, opts: &PredictOptions, ep: ExecutionProvider) -> Self {
        fn warn<T>(kind: &str, r: Result<T>) -> Option<T> {
            r.map_err(|e| tracing::warn!("bank {kind} model not used: {e:#}")).ok()
        }
        Self {
            duration: paths
                .duration
                .as_deref()
                .filter(|_| opts.timing)
                .and_then(|p| warn("duration", DurationModel::load(p, ep))),
            pitch: paths
                .pitch
                .as_deref()
                .filter(|_| opts.pitch)
                .and_then(|p| warn("pitch", PitchModel::load(p, ep))),
            variance: paths
                .variance
                .as_deref()
                .filter(|_| opts.variance)
                .and_then(|p| warn("variance", VarianceModel::load(p, ep))),
        }
    }

    /// Run the loaded models over `score` for acoustic speaker `speaker`.
    /// `bend_cents(t)` is the pitch-bend offset at `t` seconds into the
    /// unit, added to the pitch model's output. A model that fails falls
    /// back to the rules for its part (a warning says why); the others
    /// still run.
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
        if let Some(model) = self.duration.as_mut().filter(|_| opts.timing) {
            match model.predict(score, speaker) {
                Ok(pred) => {
                    out.ph_dur = align_durations(score, &pred, model.frame_sec());
                    out.timed_by_model = true;
                }
                Err(e) => tracing::warn!("bank duration model failed, rule timing used: {e:#}"),
            }
        }
        let Some(pitch_model) = self.pitch.as_mut().filter(|_| opts.pitch) else {
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
                tracing::warn!("bank pitch model failed, rule pitch used: {e:#}");
                return out;
            }
        };
        if let Some(model) = self.variance.as_mut().filter(|_| opts.variance) {
            match model.predict(
                score,
                &ph_frames,
                &pitch,
                speaker,
                opts.variance_steps,
                seed ^ 0x5641_5249_414e_4345,
            ) {
                Ok(v) => out.variance = v,
                Err(e) => tracing::warn!("bank variance model failed: {e:#}"),
            }
        }
        out.pitch = Some(pitch);
        out
    }
}
