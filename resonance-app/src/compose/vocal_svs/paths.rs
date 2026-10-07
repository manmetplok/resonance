//! Voicebank path resolution and per-bank phoneme/language conventions.
//!
//! Each voicebank ships with its own on-disk layout, phoneme inventory,
//! and quirks (Meiji prefixes English phonemes with `en/`; Lilia lacks
//! a voiced `v`; etc.). All of that lives here so the rendering code
//! in `segment.rs` and the post-processor in `post.rs` can stay
//! voicebank-agnostic. It also answers what the installed SVS stack can
//! do: which expression curves a bank's acoustic model takes
//! ([`curve_supported`]) and which ONNX backend renders ([`svs_backend`]).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use resonance_music_theory::{VocalParams, VocalVoicebank};
pub use resonance_svs::voicebank::CurveSupport;
use resonance_svs::voicebank::layout::read_curve_support;

/// Meiji's per-token language id for English-prefixed phonemes. Lifted
/// from `voicebanks/meiji/files/languages.json` (`"en": 3`). Silence
/// markers (`AP`/`SP`) get `0` since they're un-prefixed in the dict.
const MEIJI_LANG_EN: i64 = 3;
const MEIJI_LANG_SILENCE: i64 = 0;

/// Resolved on-disk paths for one voicebank. The render pipeline only
/// needs the two YAML files (it loads everything else by reference from
/// inside them), plus the optional speaker id we send via `spk_embed`.
pub(super) struct VoicebankPaths {
    pub(super) acoustic_config: PathBuf,
    pub(super) vocoder_config: PathBuf,
    /// `Some(name)` for multi-speaker banks (TIGER, Meiji); `None` for
    /// single-speaker banks (Lilia).
    pub(super) speaker: Option<String>,
}

/// Find the on-disk paths for `voicebank`. Resolution order:
///   1. `RESONANCE_SVS_MODELS_DIR` env var (workspace override)
///   2. workspace-root `resonance-svs/models/` (default)
///
/// Returns `None` when the requested voicebank isn't installed —
/// callers treat that as "SVS unavailable, skip silently".
pub(super) fn locate_voicebank(
    voicebank: VocalVoicebank,
    params: &VocalParams,
) -> Option<VoicebankPaths> {
    let roots: Vec<PathBuf> = std::iter::once(std::env::var_os("RESONANCE_SVS_MODELS_DIR"))
        .flatten()
        .map(PathBuf::from)
        .chain(std::iter::once(default_models_dir()))
        .collect();

    for root in roots {
        if let Some(paths) = try_voicebank(&root, voicebank, params) {
            return Some(paths);
        }
    }
    None
}

fn try_voicebank(
    root: &Path,
    voicebank: VocalVoicebank,
    params: &VocalParams,
) -> Option<VoicebankPaths> {
    match voicebank {
        VocalVoicebank::Tiger => {
            let acoustic = root.join("singer/extracted/dsacoustic/dsconfig.yaml");
            // TIGER ships its own bundled vocoder
            // (`tgm_hifigan.onnx`, r03) trained against the same mel
            // statistics as the acoustic model. A foreign vocoder
            // (`tgm_hifigan_v110.onnx`) produces noticeably rougher
            // audio because the mel-spectrogram statistics don't match.
            // Prefer the bundled vocoder, fall back to the generic one
            // (`models/vocoder/dsvocoder/`) for setups that only have
            // the generic version installed.
            let bundled = root.join("singer/extracted/dsvocoder/vocoder.yaml");
            let generic = root.join("vocoder/dsvocoder/vocoder.yaml");
            let vocoder = if bundled.exists() { bundled } else { generic };
            if acoustic.exists() && vocoder.exists() {
                Some(VoicebankPaths {
                    acoustic_config: acoustic,
                    vocoder_config: vocoder,
                    speaker: Some(params.singer.speaker_id().to_string()),
                })
            } else {
                None
            }
        }
        VocalVoicebank::Lilia => {
            let acoustic = root.join("voicebanks/lilia/dsconfig.yaml");
            let vocoder = root.join("voicebanks/lilia/dsvocoder/vocoder.yaml");
            if acoustic.exists() && vocoder.exists() {
                Some(VoicebankPaths {
                    acoustic_config: acoustic,
                    vocoder_config: vocoder,
                    // Lilia is single-speaker.
                    speaker: None,
                })
            } else {
                None
            }
        }
        VocalVoicebank::Meiji => {
            let acoustic = root.join("voicebanks/meiji/configs/configs/dsconfig.yaml");
            let vocoder = root.join("voicebanks/meiji/configs/configs/dsvocoder/vocoder.yaml");
            if acoustic.exists() && vocoder.exists() {
                Some(VoicebankPaths {
                    acoustic_config: acoustic,
                    vocoder_config: vocoder,
                    speaker: Some(params.singer_meiji.speaker_id().to_string()),
                })
            } else {
                None
            }
        }
    }
}

/// Workspace-relative default — the SVS crate ships its model dir at
/// `resonance-svs/models/`. Anchored against the binary's
/// `CARGO_MANIFEST_DIR` so it resolves from a `cargo run` in any subdir.
fn default_models_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../resonance-svs/models")
}

/// Apply the voicebank's per-symbol naming convention. Meiji namespaces
/// every English phoneme with `en/` (e.g. `ah` → `en/ah`) but a small
/// shared inventory (`AP`, `SP`, `hh`, `cl`, ...) is left unprefixed so
/// it works across every language Meiji supports. TIGER and Lilia use
/// bare ARPAbet symbols.
pub(super) fn voicebank_phoneme_name(voicebank: VocalVoicebank, ph: &str) -> String {
    match voicebank {
        VocalVoicebank::Tiger | VocalVoicebank::Lilia => ph.to_string(),
        VocalVoicebank::Meiji => {
            if meiji_is_universal(ph) {
                ph.to_string()
            } else {
                format!("en/{ph}")
            }
        }
    }
}

/// Phonemes that live in Meiji's "no-prefix" bucket (silence markers
/// and a handful of language-agnostic consonants). Meiji's `en/` set
/// notably lacks `en/hh`; the universal `hh` covers it.
pub(super) fn meiji_is_universal(ph: &str) -> bool {
    matches!(ph, "AP" | "SP" | "hh" | "cl" | "ban" | "vf")
}

/// Per-token language id Meiji's `languages` ONNX input expects. `0`
/// for silence markers and the universal-consonant bucket; `3` for
/// English (matches Meiji's `languages.json: "en": 3`). Returns `None`
/// for voicebanks that don't accept a `languages` input.
pub(super) fn voicebank_language_id(voicebank: VocalVoicebank, ph: &str) -> Option<i64> {
    match voicebank {
        VocalVoicebank::Tiger | VocalVoicebank::Lilia => None,
        VocalVoicebank::Meiji => Some(if meiji_is_universal(ph) {
            MEIJI_LANG_SILENCE
        } else {
            MEIJI_LANG_EN
        }),
    }
}

/// The four editable vocal expression curves (see doc #154). Each maps
/// to a per-frame curve fed into the SVS segment builder and is shaped
/// by the user in the vocal-roll Expression dock. Not every voicebank's
/// acoustic model accepts every curve — see [`curve_supported`].
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

/// Whether `voicebank`'s acoustic model accepts the given expression `curve`.
///
/// Single source of truth consumed by both the vocal-roll Expression
/// dock (an unsupported curve shows an `n/a` badge) and the SVS segment
/// builder (an unsupported curve is a cheap no-op rather than an error).
///
/// Answered from the bank's acoustic `dsconfig.yaml`: `use_energy_embed`
/// gates Dynamics, `use_breathiness_embed` Breathiness and
/// `use_tension_embed` Tension. PitchBend is a pre-synthesis f0 edit and
/// always supported. The flags are read once per bank per process
/// ([`bank_curve_support`]); a bank that is not installed, or whose config
/// does not parse, answers from [`shipped_curve_support`].
pub fn curve_supported(voicebank: VocalVoicebank, curve: CurveKind) -> bool {
    let support = bank_curve_support(voicebank);
    match curve {
        CurveKind::PitchBend => true,
        CurveKind::Dynamics => support.energy,
        CurveKind::Tension => support.tension,
        CurveKind::Breathiness => support.breathiness,
    }
}

/// The conservative fallback when a bank's acoustic config can't be read:
/// exactly what the shipped configs declare (checked 2026-10-07 against
/// `resonance-svs/models/`). No shipped bank takes energy or breathiness;
/// Lilia (MM 2.8) and Meiji take tension, TIGER (v106) does not.
pub fn shipped_curve_support(voicebank: VocalVoicebank) -> CurveSupport {
    match voicebank {
        VocalVoicebank::Tiger => CurveSupport::default(),
        VocalVoicebank::Lilia | VocalVoicebank::Meiji => CurveSupport {
            tension: true,
            ..CurveSupport::default()
        },
    }
}

/// The curve flags of `voicebank`'s installed acoustic config, parsed on
/// first use and cached for the life of the process: the curve gate runs
/// per segment and per dock redraw, so it must not re-read YAML. A bank
/// installed or replaced after the first query keeps the cached answer
/// until restart; the fallback equals the shipped configs, so that only
/// matters for a hand-edited bank.
pub fn bank_curve_support(voicebank: VocalVoicebank) -> CurveSupport {
    static CACHE: [OnceLock<CurveSupport>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let slot = match voicebank {
        VocalVoicebank::Tiger => &CACHE[0],
        VocalVoicebank::Lilia => &CACHE[1],
        VocalVoicebank::Meiji => &CACHE[2],
    };
    *slot.get_or_init(|| read_bank_curve_support(voicebank))
}

/// The ONNX Runtime backend (`cpu`, `cuda`, `migraphx`) the most recent
/// vocal render's acoustic model really ran on, after any CPU fallback;
/// `None` until a render has created a session in this process. Reported
/// as `svs_backend` by `control.hello`.
pub fn svs_backend() -> Option<&'static str> {
    resonance_svs::active_backend().map(resonance_svs::ExecutionProvider::as_str)
}

fn read_bank_curve_support(voicebank: VocalVoicebank) -> CurveSupport {
    let Some(paths) = locate_voicebank(voicebank, &VocalParams::default()) else {
        return shipped_curve_support(voicebank);
    };
    match read_curve_support(&paths.acoustic_config) {
        Ok(support) => support,
        Err(e) => {
            tracing::warn!(
                "reading curve support for {voicebank:?} from {}: {e:#}; using the shipped table",
                paths.acoustic_config.display()
            );
            shipped_curve_support(voicebank)
        }
    }
}
