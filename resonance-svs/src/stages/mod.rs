//! ONNX stage wrappers: the combined acoustic model and the vocoder (the
//! path mirroring Jobsecond/diffsinger-onnx-infer), the session plumbing
//! they share ([`common`]), the seeded noise that makes the diffusion
//! models repeatable ([`noise`], [`seeded`]) and the process-wide session
//! cache ([`session_cache`]). The bank's own duration, pitch and variance
//! models live in [`crate::predict`].

pub mod acoustic;
pub mod common;
pub mod noise;
pub mod seeded;
pub mod session_cache;
pub mod vocoder;

pub use acoustic::AcousticStage;
pub use vocoder::VocoderStage;
