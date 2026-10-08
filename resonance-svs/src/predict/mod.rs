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
//! * [`models`] — [`BankModels`]: loading them and chaining a prediction.
//!
//! [`BankModels::predict`] chains them: durations (aligned to the beats),
//! then pitch on those durations (plus the caller's pitch-bend cents),
//! then the variance curves on that pitch. Each diffusion model's noise is
//! seeded ([`crate::stages::noise`]), so a prediction is a pure function
//! of the score, the speaker and the seed.

pub mod config;
pub mod models;
pub mod run;
pub mod score;

pub use config::{discover, language_map, BankModelPaths, ModelConfig, ModelFacts, ModelKind};
pub use models::{BankModels, PredictOptions, Prediction, PITCH_STEPS, VARIANCE_STEPS};
pub use run::{DurationModel, PitchModel, VarianceCurves, VarianceModel};
pub use score::{
    align_durations, fit_frames, hz_to_midi, midi_to_hz, pitch_notes, BankScore, ScoreGroup,
    ScoreNote, ScorePhone,
};
