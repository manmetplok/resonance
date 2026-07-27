//! Voicebank manifest: scan one DiffSinger voicebank folder, auto-detect
//! its on-disk layout, and validate it actually loads.
//!
//! A *voicebank* is a folder shipping an acoustic `dsconfig.yaml`, a
//! vocoder config, a phoneme dictionary, the referenced ONNX models, and
//! — for multi-speaker banks — a `speakers` list inside the acoustic
//! config. Newer banks (LIEE Lilia, Gahata Meiji) add a `languages.json`
//! and a JSON phoneme dict; older banks (TIGER) use a `.txt` dict. The
//! exact filenames vary, so [`scan`] probes a handful of conventional
//! locations rather than assuming one layout.
//!
//! The descriptor reuses the existing [`crate::config`] loaders for the
//! heavy lifting: [`scan`] resolves paths and reads the cheap metadata
//! (speakers, phoneme inventory, languages), and
//! [`VoicebankManifest::validate`] re-exercises
//! `config::load_acoustic` / `load_vocoder` / `load_phoneme_dict` so an
//! unusable bank is rejected with a descriptive error before the pipeline
//! ever touches an ONNX session.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

use crate::config::{self, AcousticConfig};

/// The phoneme alphabet a voicebank's dictionary is written in. The
/// Resonance G2P emits English ARPAbet (lowercase, stress-stripped); a
/// bank whose inventory is X-SAMPA needs a different symbol set, so the
/// pipeline can warn rather than feed it mismatched tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhonemeTarget {
    /// Lowercase ARPAbet (`ah`, `ae`, `f`, ...) — what every shipped bank
    /// and the G2P use.
    Arpabet,
    /// X-SAMPA (`@`, `{`, `r\`, ...). Detected but not yet G2P-supported.
    XSampa,
}

/// One of the four editable vocal expression curves. Mirrors the
/// `CurveKind` the vocal-roll Expression dock and SVS segment builder use;
/// kept here so [`VoicebankManifest::supports_curve`] can answer the
/// capability question from the bank's loaded acoustic config rather than a
/// per-bank enum match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExpressionCurve {
    /// Dynamics / energy (loudness envelope) — needs the `energy` embed.
    Dynamics,
    /// Vocal tension — needs the `tension` embed.
    Tension,
    /// Breathiness — needs the `breathiness` embed.
    Breathiness,
    /// Pitch bend — applied as a pre-synthesis f0 edit, so always
    /// available regardless of the acoustic model's inputs.
    PitchBend,
}

/// The language the Resonance G2P transcribes into. All transcription is
/// English ARPAbet today; banks that namespace phonemes by language (Gahata
/// Meiji) prefix the English set with `en/`, so this is the prefix
/// [`VoicebankManifest::phoneme_name`] reaches for when a bare symbol is
/// absent.
const G2P_LANGUAGE: &str = "en";

/// One selectable speaker inside a multi-speaker voicebank. Single-speaker
/// banks carry no singers (the manifest's `singers` is empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingerInfo {
    /// The identifier the pipeline passes as `speaker` — the raw name from
    /// the acoustic config's `speakers` list.
    pub id: String,
    /// Human-readable label. Currently identical to `id`; kept separate so
    /// a future prettifier (or a `character.yaml` lookup) can diverge it
    /// without changing the selection key.
    pub display_name: String,
}

/// A scanned, resolved description of one voicebank folder.
///
/// Every path is absolute (resolved against the bank root). Optional model
/// configs are `None` when the bank doesn't ship that stage. The manifest
/// is data-only: nothing here opens an ONNX session — see [`Self::validate`]
/// for the load gate.
#[derive(Debug, Clone)]
pub struct VoicebankManifest {
    /// Normalized slug derived from the folder name (lowercase, ASCII
    /// alphanumerics, single `-` between runs). Stable id for lookups.
    pub id: String,
    /// The folder name as shipped, for display.
    pub display_name: String,
    /// Absolute path to the bank root.
    pub root: PathBuf,

    /// Acoustic `dsconfig.yaml` (always present in a usable bank).
    pub acoustic_config: PathBuf,
    /// Vocoder config (`vocoder.yaml`), if one was found.
    pub vocoder_config: Option<PathBuf>,
    /// Phoneme dictionary the acoustic config points at.
    pub phoneme_dict: PathBuf,

    /// Resolved variance / duration / linguistic / pitch model paths from
    /// the acoustic config, when the bank declares them.
    pub variance_model: Option<PathBuf>,
    pub dur_model: Option<PathBuf>,
    pub linguistic_model: Option<PathBuf>,
    pub pitch_model: Option<PathBuf>,

    /// Selectable speakers. Empty for a single-speaker bank.
    pub singers: Vec<SingerInfo>,
    /// Phoneme inventory in token-id order (index = id for array/`.txt`
    /// dicts; sorted by explicit id for object dicts).
    pub phonemes: Vec<String>,
    /// `languages.json` contents (name -> language id), empty if absent.
    pub languages: BTreeMap<String, i64>,

    /// Alphabet the dict is written in, detected from the inventory.
    pub phoneme_target: PhonemeTarget,
    /// Acoustic model accepts a per-token `languages` input (multi-language
    /// bank). Gates [`Self::language_id`].
    pub accepts_language_id: bool,
    /// Acoustic model accepts an `energy` curve input — drives the
    /// Dynamics expression curve.
    pub accepts_energy: bool,
    /// Acoustic model accepts a `breathiness` curve input.
    pub accepts_breathiness: bool,
    /// Acoustic model accepts a `tension` curve input.
    pub accepts_tension: bool,
    /// Acoustic model accepts a `voicing` curve input.
    pub accepts_voicing: bool,

    /// O(1) membership index over [`Self::phonemes`], built at scan time so
    /// the per-token [`Self::phoneme_name`] / [`Self::language_id`] /
    /// [`Self::substitute_phoneme`] lookups stay cheap on the render path.
    phoneme_set: HashSet<String>,
}

/// Scan a voicebank folder into a [`VoicebankManifest`].
///
/// Errors if `dir` is not a directory, if no acoustic `dsconfig.yaml` can
/// be located, or if that config fails to parse / resolve its phoneme
/// dict. Model ONNX files are *not* required to exist at scan time (they
/// are large and may be absent in a config-only fixture); their existence
/// is the pipeline's concern, while [`VoicebankManifest::validate`] gates
/// on the configs loading.
pub fn scan(dir: &Path) -> Result<VoicebankManifest> {
    if !dir.is_dir() {
        bail!("voicebank path is not a directory: {}", dir.display());
    }
    let root = dir
        .canonicalize()
        .with_context(|| format!("resolving voicebank path {}", dir.display()))?;

    let acoustic_config = find_acoustic_config(&root).ok_or_else(|| {
        anyhow!(
            "no acoustic dsconfig.yaml found under {} (looked for dsconfig.yaml / acoustic.yaml \
             at the root and one level down)",
            root.display()
        )
    })?;

    let acoustic = config::load_acoustic(&acoustic_config)
        .with_context(|| format!("loading acoustic config {}", acoustic_config.display()))?;

    // Phoneme inventory — cheap text/JSON read, drives the `phonemes` field
    // and confirms the dict resolves before validate() ever runs.
    let phoneme_dict = acoustic.phonemes_path.clone();
    let phonemes = read_phoneme_inventory(&phoneme_dict).with_context(|| {
        format!(
            "reading phoneme inventory for {}",
            acoustic_config.display()
        )
    })?;

    let singers = singers_from(&acoustic);
    let languages = read_languages(&root)?;
    let vocoder_config = find_vocoder_config(&root);

    let display_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    let id = slugify(&display_name);

    let phoneme_target = detect_phoneme_target(&phonemes);
    let phoneme_set = phonemes.iter().cloned().collect();

    Ok(VoicebankManifest {
        id,
        display_name,
        root,
        acoustic_config,
        vocoder_config,
        phoneme_dict,
        variance_model: acoustic.variance_model.clone(),
        dur_model: acoustic.dur_model.clone(),
        linguistic_model: acoustic.linguistic_model.clone(),
        pitch_model: acoustic.pitch_model.clone(),
        singers,
        phonemes,
        languages,
        phoneme_target,
        accepts_language_id: acoustic.use_lang_id,
        accepts_energy: acoustic.use_energy_embed,
        accepts_breathiness: acoustic.use_breathiness_embed,
        accepts_tension: acoustic.use_tension_embed,
        accepts_voicing: acoustic.use_voicing_embed,
        phoneme_set,
    })
}

impl VoicebankManifest {
    /// True for a single-speaker bank (no `speakers` declared).
    pub fn is_single_speaker(&self) -> bool {
        self.singers.is_empty()
    }

    /// Confirm the bank is loadable by exercising every config loader the
    /// pipeline depends on: the acoustic config, the vocoder config (when
    /// present), and the phoneme dictionary. Returns the first descriptive
    /// error, or `Ok(())` when the bank parses cleanly.
    ///
    /// This re-reads the configs rather than trusting the scan so a bank
    /// edited or truncated after scanning is still caught.
    pub fn validate(&self) -> Result<()> {
        config::load_acoustic(&self.acoustic_config).with_context(|| {
            format!(
                "voicebank '{}' has an unusable acoustic config {}",
                self.id,
                self.acoustic_config.display()
            )
        })?;

        config::load_phoneme_dict(&self.phoneme_dict).with_context(|| {
            format!(
                "voicebank '{}' has an unreadable phoneme dict {}",
                self.id,
                self.phoneme_dict.display()
            )
        })?;

        if let Some(vocoder) = &self.vocoder_config {
            config::load_vocoder(vocoder).with_context(|| {
                format!(
                    "voicebank '{}' has an unusable vocoder config {}",
                    self.id,
                    vocoder.display()
                )
            })?;
        }

        Ok(())
    }

    // -----------------------------------------------------------------
    // Data-driven per-bank quirks (doc #164)
    //
    // These reproduce — from the scanned inventory / languages / acoustic
    // config — the behaviour previously hardcoded per `VocalVoicebank`
    // enum arm in `resonance-app`'s `vocal_svs/paths.rs`. Working from the
    // data means a freshly-dropped bank gets the right treatment without a
    // code change.
    // -----------------------------------------------------------------

    /// Resolve a bare G2P ARPAbet symbol to the dict key this bank actually
    /// stores it under, or `None` if neither the bare form nor the
    /// language-namespaced (`en/…`) form is in the inventory.
    ///
    /// Single-language banks (TIGER, Lilia) store bare ARPAbet, so the bare
    /// lookup hits. Banks that namespace by language (Meiji) keep a small
    /// universal bucket bare (`AP`, `SP`, `hh`, `cl`, …) and prefix the
    /// rest, so the `en/` fallback hits for those.
    fn dict_key_for(&self, ph: &str) -> Option<String> {
        if self.phoneme_set.contains(ph) {
            return Some(ph.to_string());
        }
        let namespaced = format!("{G2P_LANGUAGE}/{ph}");
        if self.phoneme_set.contains(&namespaced) {
            return Some(namespaced);
        }
        None
    }

    /// The dict key to feed the acoustic model for a G2P ARPAbet symbol —
    /// bare for single-language banks, `en/`-prefixed for the namespaced
    /// portion of a multi-language bank. Falls back to the bare symbol when
    /// the bank stores it in neither form (matching the old code, which
    /// passed unknown symbols through unchanged).
    pub fn phoneme_name(&self, ph: &str) -> String {
        self.dict_key_for(ph).unwrap_or_else(|| ph.to_string())
    }

    /// Per-token id for the acoustic model's `languages` input, or `None`
    /// when this bank takes no language input (`use_lang_id` is off).
    ///
    /// Namespaced symbols (`en/ah`) report their language's id from
    /// `languages.json`; the bare universal bucket (silence markers and
    /// shared consonants) reports `0`, the default/silence language —
    /// reproducing Meiji's `0` for `AP`/`hh`/… and `3` for English.
    pub fn language_id(&self, ph: &str) -> Option<i64> {
        if !self.accepts_language_id {
            return None;
        }
        let key = self.phoneme_name(ph);
        match key.split_once('/') {
            Some((lang, _)) => Some(self.languages.get(lang).copied().unwrap_or(0)),
            None => Some(0),
        }
    }

    /// Replace a G2P ARPAbet symbol the bank's dict lacks with its nearest
    /// available substitute; symbols the bank knows pass through unchanged.
    ///
    /// Only fires when the symbol resolves to neither a bare nor a
    /// namespaced dict key — e.g. Lilia ships every ARPAbet phone except
    /// the voiced `v`, so `v` (absent) maps to `f` (its voiceless
    /// counterpart, present) while everything else is left alone. Banks
    /// with the full inventory (TIGER, Meiji) substitute nothing.
    pub fn substitute_phoneme(&self, ph: &str) -> String {
        if self.dict_key_for(ph).is_some() {
            return ph.to_string();
        }
        for candidate in nearest_substitutes(ph) {
            if self.dict_key_for(candidate).is_some() {
                return candidate.to_string();
            }
        }
        ph.to_string()
    }

    /// Whether this bank's pipeline accepts the given expression curve.
    ///
    /// Pitch bend is a pre-synthesis f0 edit, so it is always available;
    /// the other three each require their matching acoustic embed
    /// (`energy`/`tension`/`breathiness`). TIGER's model exposes no
    /// tension or breathiness input, so those report `false` there while
    /// Lilia and Meiji accept all three.
    pub fn supports_curve(&self, curve: ExpressionCurve) -> bool {
        match curve {
            ExpressionCurve::PitchBend => true,
            ExpressionCurve::Dynamics => self.accepts_energy,
            ExpressionCurve::Tension => self.accepts_tension,
            ExpressionCurve::Breathiness => self.accepts_breathiness,
        }
    }
}

/// Nearest acceptable ARPAbet substitutes for a phone, most-similar first.
/// Consulted only for symbols a bank's dict is missing, so it never alters
/// a bank with the full inventory. Pairs voiced phones with their voiceless
/// counterpart (same place + manner), the substitution least likely to be
/// noticed; the reverse direction covers the rarer voiceless-gap case.
fn nearest_substitutes(ph: &str) -> &'static [&'static str] {
    match ph {
        "v" => &["f", "b"],
        "f" => &["v"],
        "dh" => &["th", "d"],
        "th" => &["dh", "t"],
        "z" => &["s"],
        "s" => &["z"],
        "zh" => &["sh"],
        "sh" => &["zh"],
        "jh" => &["ch"],
        "ch" => &["jh"],
        _ => &[],
    }
}

/// Detect a bank's phoneme alphabet from its inventory. ARPAbet here is
/// lowercase ASCII letters with optional `lang/` prefixes; X-SAMPA mixes in
/// glyphs ARPAbet never uses (`@ { } \ ~ = & |` and bare uppercase
/// vowels like `O`/`I`/`E`). Any such glyph flips the verdict to X-SAMPA.
fn detect_phoneme_target(phonemes: &[String]) -> PhonemeTarget {
    let is_xsampa_glyph = |c: char| matches!(c, '@' | '{' | '}' | '\\' | '~' | '=' | '&' | '|');
    for ph in phonemes {
        if ph.chars().any(is_xsampa_glyph) {
            return PhonemeTarget::XSampa;
        }
    }
    PhonemeTarget::Arpabet
}

// ---------------------------------------------------------------------------
// Layout auto-detection
// ---------------------------------------------------------------------------

/// Conventional acoustic-config filenames, in priority order.
const ACOUSTIC_NAMES: [&str; 2] = ["dsconfig.yaml", "acoustic.yaml"];
/// Conventional vocoder-config locations relative to the bank root.
const VOCODER_RELS: [&str; 3] = [
    "vocoder.yaml",
    "nsf_hifigan/vocoder.yaml",
    "vocoder/vocoder.yaml",
];

/// Locate the *acoustic* dsconfig: the one declaring an `acoustic` model
/// key (a sibling variance/dur/pitch dsconfig has the same filename but no
/// `acoustic` key). Searches the root first, then one level of
/// subdirectories so banks that nest the acoustic model in `acoustic/`
/// are still found.
fn find_acoustic_config(root: &Path) -> Option<PathBuf> {
    let mut fallback: Option<PathBuf> = None;
    let consider = |path: PathBuf, fallback: &mut Option<PathBuf>| -> Option<PathBuf> {
        if !path.is_file() {
            return None;
        }
        if declares_acoustic(&path) {
            return Some(path);
        }
        fallback.get_or_insert(path);
        None
    };

    for name in ACOUSTIC_NAMES {
        if let Some(hit) = consider(root.join(name), &mut fallback) {
            return Some(hit);
        }
    }

    if let Ok(entries) = std::fs::read_dir(root) {
        // Sort for deterministic selection across filesystems.
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for sub in dirs {
            for name in ACOUSTIC_NAMES {
                if let Some(hit) = consider(sub.join(name), &mut fallback) {
                    return Some(hit);
                }
            }
        }
    }

    // No config with an explicit `acoustic` key: fall back to the first
    // dsconfig we saw (some minimal banks omit the key and the loader
    // surfaces a clear error downstream).
    fallback
}

/// Does this YAML declare an `acoustic:` model key? Reads cheaply and
/// tolerates parse failures (treated as "no", so a malformed sibling
/// never masquerades as the acoustic config).
fn declares_acoustic(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_yml::from_str::<config::DsAcousticConfigRaw>(&text).ok())
        .is_some_and(|raw| raw.acoustic.is_some())
}

fn find_vocoder_config(root: &Path) -> Option<PathBuf> {
    VOCODER_RELS
        .iter()
        .map(|rel| root.join(rel))
        .find(|p| p.is_file())
}

// ---------------------------------------------------------------------------
// Field extraction
// ---------------------------------------------------------------------------

fn singers_from(acoustic: &AcousticConfig) -> Vec<SingerInfo> {
    acoustic
        .speakers
        .iter()
        .map(|name| SingerInfo {
            id: name.clone(),
            display_name: name.clone(),
        })
        .collect()
}

/// Read the phoneme dictionary into an id-ordered name list. Reuses
/// [`config::load_phoneme_dict`] (which auto-detects `.txt` vs `.json`)
/// and sorts by token id so the inventory is deterministic regardless of
/// the dict's on-disk ordering.
fn read_phoneme_inventory(path: &Path) -> Result<Vec<String>> {
    let map = config::load_phoneme_dict(path)?;
    let mut by_id: Vec<(i64, String)> = map.into_iter().map(|(name, id)| (id, name)).collect();
    by_id.sort();
    Ok(by_id.into_iter().map(|(_, name)| name).collect())
}

/// Parse an optional `languages.json` at the bank root. Accepts the two
/// shapes seen in DiffSinger banks — `{"zh": 0, "en": 1}` or `["zh",
/// "en"]` (index = id) — mirroring the phoneme-dict parser. Absent file =
/// empty map (monolingual bank).
fn read_languages(root: &Path) -> Result<BTreeMap<String, i64>> {
    let path = root.join("languages.json");
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading languages.json at {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("parsing languages.json at {}", path.display()))?;
    let mut map = BTreeMap::new();
    match value {
        serde_json::Value::Object(entries) => {
            for (name, id_value) in entries {
                let id = id_value
                    .as_i64()
                    .ok_or_else(|| anyhow!("language `{name}` has a non-integer id"))?;
                map.insert(name, id);
            }
        }
        serde_json::Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                let name = item
                    .as_str()
                    .ok_or_else(|| anyhow!("languages.json array entry {idx} is not a string"))?;
                map.insert(name.to_string(), idx as i64);
            }
        }
        _ => bail!("languages.json must be a JSON object or array"),
    }
    Ok(map)
}

/// Lowercase ASCII slug: alphanumeric runs joined by single `-`, trimmed.
/// `"LIEE Lilia"` -> `"liee-lilia"`, `"TIGER_v2"` -> `"tiger-v2"`.
fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut pending_sep = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_sep && !slug.is_empty() {
                slug.push('-');
            }
            pending_sep = false;
            slug.push(ch.to_ascii_lowercase());
        } else {
            pending_sep = true;
        }
    }
    slug
}
