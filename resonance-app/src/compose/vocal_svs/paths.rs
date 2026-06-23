//! Voicebank path resolution and per-bank phoneme/language conventions.
//!
//! Each voicebank ships with its own on-disk layout, phoneme inventory,
//! and quirks (Meiji prefixes English phonemes with `en/`; Lilia lacks
//! a voiced `v`; etc.). All of that lives here so the rendering code
//! in `segment.rs` and the post-processor in `post.rs` can stay
//! voicebank-agnostic.

use std::path::{Path, PathBuf};

use resonance_music_theory::{VocalParams, VocalVoicebank};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CurveKind {
    /// Dynamics / energy (loudness envelope). Universally supported.
    Dynamics,
    /// Vocal tension (relaxed/breathy ↔ compressed/belted). Accepted by
    /// Lilia and Meiji; TIGER's acoustic model has no `tension` input.
    Tension,
    /// Breathiness (added breath/air in the delivery). Accepted by
    /// Lilia and Meiji; TIGER has no `breathiness` input.
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
}

/// Whether `voicebank`'s pipeline accepts the given expression `curve`.
///
/// Single source of truth consumed by both the vocal-roll Expression
/// dock (an unsupported curve shows an `n/a` badge) and the SVS segment
/// builder (an unsupported curve is a cheap no-op rather than an error).
///
/// Matrix (doc #154):
/// - **Dynamics** and **PitchBend** — supported on every voicebank.
///   Pitch bend is a pre-synthesis f0 edit; dynamics maps to the
///   universally-present energy curve.
/// - **Tension** and **Breathiness** — TIGER's acoustic model exposes
///   neither input, so they're unsupported there; Lilia and Meiji
///   accept both.
///
/// Cheaper than introspecting the ONNX inputs at every render.
pub fn curve_supported(voicebank: VocalVoicebank, curve: CurveKind) -> bool {
    match curve {
        CurveKind::Dynamics | CurveKind::PitchBend => true,
        CurveKind::Tension | CurveKind::Breathiness => match voicebank {
            VocalVoicebank::Tiger => false,
            VocalVoicebank::Lilia | VocalVoicebank::Meiji => true,
        },
    }
}
