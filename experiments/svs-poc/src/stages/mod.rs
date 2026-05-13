//! ONNX stage wrappers. The combined acoustic + vocoder path (mirroring
//! Jobsecond/diffsinger-onnx-infer) is the one exercised by this PoC's smoke test.
//! Linguistic / duration / pitch / variance stages are scaffolded for the newer split
//! pipeline produced by recent openvpi exporters; they are not driven end-to-end yet.

pub mod acoustic;
pub mod common;
pub mod duration;
pub mod linguistic;
pub mod pitch;
pub mod variance;
pub mod vocoder;

pub use acoustic::AcousticStage;
pub use vocoder::VocoderStage;
