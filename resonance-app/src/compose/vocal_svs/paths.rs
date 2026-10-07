//! What the installed SVS stack can do independent of any one bank: the
//! expression curves a lane can shape ([`CurveKind`]) and which ONNX
//! backend renders ([`svs_backend`]).
//!
//! Everything bank-specific — where a bank's files live, how it spells its
//! phonemes, its language ids and which curves its acoustic model takes —
//! is the voicebank registry's ([`super::registry`], asked through a
//! [`BankProfile`](super::BankProfile)).

pub use resonance_svs::voicebank::CurveSupport;

/// The four editable vocal expression curves (see doc #154). Each maps
/// to a per-frame curve fed into the SVS segment builder and is shaped
/// by the user in the vocal-roll Expression dock. Not every voicebank's
/// acoustic model accepts every curve — see
/// [`BankProfile::curve_supported`](super::BankProfile::curve_supported).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum CurveKind {
    /// Dynamics / energy (loudness envelope). Needs the acoustic model's
    /// `energy` input, which no shipped bank has.
    Dynamics,
    /// Vocal tension (relaxed/breathy ↔ compressed/belted). Accepted by
    /// Lilia and Meiji; TIGER's acoustic model has no `tension` input.
    Tension,
    /// Breathiness (added breath/air in the delivery). Needs the
    /// `breathiness` input, which no shipped bank has.
    Breathiness,
    /// Pitch bend — f0 offset / portamento in cents. Always applied as a
    /// pre-synthesis f0 modification, so every voicebank supports it.
    PitchBend,
}

impl CurveKind {
    /// All four curve kinds, in rail display order.
    pub const ALL: [CurveKind; 4] = [
        CurveKind::Dynamics,
        CurveKind::Tension,
        CurveKind::Breathiness,
        CurveKind::PitchBend,
    ];

    /// Inclusive `(min, max)` range a curve's values live in (doc #154).
    ///
    /// Dynamics, tension and breathiness are normalised `0..=1` envelopes;
    /// pitch bend is an f0 offset in cents, drawn on a `-50..=+50` axis.
    /// Used to clamp both auto-derived baselines and user overlay edits so
    /// no curve can carry an out-of-range value.
    pub fn value_range(self) -> (f32, f32) {
        match self {
            CurveKind::Dynamics | CurveKind::Tension | CurveKind::Breathiness => (0.0, 1.0),
            CurveKind::PitchBend => (-50.0, 50.0),
        }
    }
}

/// The ONNX Runtime backend (`cpu`, `cuda`, `migraphx`) the most recent
/// vocal render's acoustic model really ran on, after any CPU fallback;
/// `None` until a render has created a session in this process. Reported
/// as `svs_backend` by `control.hello`.
pub fn svs_backend() -> Option<&'static str> {
    resonance_svs::active_backend().map(resonance_svs::ExecutionProvider::as_str)
}
