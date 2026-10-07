//! What the installed SVS stack can do independent of any one bank: the
//! expression curves a lane can shape ([`CurveKind`]) and which ONNX
//! backend renders ([`svs_backend`]).
//!
//! Everything bank-specific — where a bank's files live, how it spells its
//! phonemes, its language ids and which curves its acoustic model takes —
//! is the voicebank registry's ([`super::registry`], asked through a
//! [`BankProfile`](super::BankProfile)).

pub use resonance_svs::voicebank::CurveSupport;

/// The four vocal expression curves (vocal-mcp-control-and-quality.md
/// §10): the automation target's [`VocalCurveKind`](resonance_common::VocalCurveKind),
/// under the name the render path has always used. Each maps to a
/// per-frame input of the SVS segment builder; not every voicebank's
/// acoustic model accepts every curve — see
/// [`BankProfile::curve_supported`](super::BankProfile::curve_supported).
pub use resonance_common::VocalCurveKind as CurveKind;

/// The ONNX Runtime backend (`cpu`, `cuda`, `migraphx`) the most recent
/// vocal render's acoustic model really ran on, after any CPU fallback;
/// `None` until a render has created a session in this process. Reported
/// as `svs_backend` by `control.hello`.
pub fn svs_backend() -> Option<&'static str> {
    resonance_svs::active_backend().map(resonance_svs::ExecutionProvider::as_str)
}
