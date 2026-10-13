//! On-disk voicebank layout probing: the only part of the voicebank module
//! that touches `std::fs`.
//!
//! The exact filenames vary between vendors — newer banks (LIEE Lilia,
//! Gahata Meiji) ship a `languages.json` and a JSON phoneme dict, older ones
//! (TIGER) a `.txt` dict, and some nest the acoustic model one directory
//! down — so [`probe`] tries a handful of conventional locations rather
//! than assuming one layout. Everything it returns is either a resolved
//! absolute path or a cheap metadata read (speakers, phoneme inventory,
//! languages, capability flags); nothing here interprets a phoneme, which
//! is [`super::phonetics`]'s job.
//!
//! This module changes when a new vendor ships a new folder layout.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

use super::{CurveSupport, SingerInfo};
use crate::config::{self, AcousticConfig};

/// Conventional acoustic-config filenames, in priority order.
const ACOUSTIC_NAMES: [&str; 2] = ["dsconfig.yaml", "acoustic.yaml"];
/// Conventional vocoder-config locations relative to the bank root.
const VOCODER_RELS: [&str; 3] = [
    "vocoder.yaml",
    "nsf_hifigan/vocoder.yaml",
    "vocoder/vocoder.yaml",
];

/// What one voicebank folder holds, as found on disk: resolved paths plus
/// the metadata the manifest needs, with no interpretation applied.
#[derive(Debug, Clone)]
pub struct BankLayout {
    /// Normalized slug derived from the folder name.
    pub id: String,
    /// The folder name as shipped, for display.
    pub display_name: String,
    /// Absolute, canonicalized bank root.
    pub root: PathBuf,

    /// Acoustic `dsconfig.yaml`.
    pub acoustic_config: PathBuf,
    /// Vocoder config, if one was found.
    pub vocoder_config: Option<PathBuf>,
    /// Phoneme dictionary the acoustic config points at.
    pub phoneme_dict: PathBuf,

    /// Resolved model paths declared by the acoustic config.
    pub variance_model: Option<PathBuf>,
    pub dur_model: Option<PathBuf>,
    pub linguistic_model: Option<PathBuf>,
    pub pitch_model: Option<PathBuf>,

    /// Selectable speakers. Empty for a single-speaker bank.
    pub singers: Vec<SingerInfo>,
    /// Phoneme inventory in token-id order.
    pub phonemes: Vec<String>,
    /// `languages.json` contents (name -> language id), empty if absent.
    pub languages: BTreeMap<String, i64>,

    /// What the bank says about itself and where its extras live.
    pub details: BankDetails,

    /// Model input flags read from the acoustic config.
    pub accepts_language_id: bool,
    pub accepts_energy: bool,
    pub accepts_breathiness: bool,
    pub accepts_tension: bool,
    pub accepts_voicing: bool,
}

/// What a bank folder says about itself beyond the acoustic model: its
/// name and version (OpenUtau `character.txt` / `character.yaml`), the
/// vocoder its config asks for, and its English dictionary. All optional;
/// a bare config-only bank has none of them.
#[derive(Debug, Clone, Default)]
pub struct BankDetails {
    /// `name` from the character file.
    pub name: Option<String>,
    /// `version` from the character file.
    pub version: Option<String>,
    /// The acoustic config's `vocoder` value: a bundled folder
    /// (`dsvocoder`) or the id of a shared vocoder
    /// (`nsf_hifigan_44.1k_hop512_128bin_2024.02`). Empty when unset.
    pub vocoder_name: String,
    /// The bank's English OpenUtau dictionary (`dsdict-en.yaml`), if it
    /// ships one ([`find_dsdict`]).
    pub english_dict: Option<PathBuf>,
    /// That dictionary's `replacements:` as `(from, to)` pairs: how the
    /// bank spells the phonemizer's symbols (empty when it has none).
    pub english_replacements: Vec<(String, String)>,
}

/// Probe `dir` for a voicebank layout.
///
/// Errors if `dir` is not a directory, if no acoustic `dsconfig.yaml` can
/// be located, or if that config fails to parse / resolve its phoneme
/// dict. Model ONNX files are *not* required to exist (they are large and
/// may be absent in a config-only fixture).
pub fn probe(dir: &Path) -> Result<BankLayout> {
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
    let languages = match &acoustic.languages_path {
        Some(path) if path.is_file() => read_languages_file(path)?,
        _ => read_languages(&root)?,
    };
    let acoustic_dir = acoustic_config.parent().unwrap_or(&root).to_path_buf();
    let vocoder_config = find_vocoder_config(&root, &acoustic_dir, &acoustic.vocoder_name);
    let (name, version) = read_character(&acoustic_dir)
        .or_else(|| read_character(&root))
        .unwrap_or_default();
    let english_dict = find_dsdict(&root, &acoustic_dir, "en");
    let english_replacements = english_dict
        .as_deref()
        .map(read_replacements)
        .unwrap_or_default();
    let details = BankDetails {
        name,
        version,
        vocoder_name: acoustic.vocoder_name.clone(),
        english_dict,
        english_replacements,
    };

    let display_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    let id = folder_id(&display_name);

    Ok(BankLayout {
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
        details,
        accepts_language_id: acoustic.use_lang_id,
        accepts_energy: acoustic.use_energy_embed,
        accepts_breathiness: acoustic.use_breathiness_embed,
        accepts_tension: acoustic.use_tension_embed,
        accepts_voicing: acoustic.use_voicing_embed,
    })
}

/// Read only the expression-curve capability flags
/// (`use_{energy,breathiness,tension}_embed`) from one acoustic
/// `dsconfig.yaml`, without probing the rest of the bank. For callers
/// that already know where the acoustic config sits (the app's fixed
/// per-bank paths) and only need the curve answer; [`probe`] reads the
/// same flags into a full manifest.
pub fn read_curve_support(acoustic_config: &Path) -> Result<CurveSupport> {
    let acoustic = config::load_acoustic(acoustic_config)
        .with_context(|| format!("loading acoustic config {}", acoustic_config.display()))?;
    Ok(CurveSupport {
        energy: acoustic.use_energy_embed,
        tension: acoustic.use_tension_embed,
        breathiness: acoustic.use_breathiness_embed,
    })
}

/// Re-exercise every config loader the pipeline depends on, so a bank
/// edited or truncated after scanning is still caught. `bank_id` only
/// names the bank in the error messages.
pub fn validate_configs(
    bank_id: &str,
    acoustic_config: &Path,
    phoneme_dict: &Path,
    vocoder_config: Option<&Path>,
) -> Result<()> {
    config::load_acoustic(acoustic_config).with_context(|| {
        format!(
            "voicebank '{}' has an unusable acoustic config {}",
            bank_id,
            acoustic_config.display()
        )
    })?;

    config::load_phoneme_dict(phoneme_dict).with_context(|| {
        format!(
            "voicebank '{}' has an unreadable phoneme dict {}",
            bank_id,
            phoneme_dict.display()
        )
    })?;

    if let Some(vocoder) = vocoder_config {
        config::load_vocoder(vocoder).with_context(|| {
            format!(
                "voicebank '{}' has an unusable vocoder config {}",
                bank_id,
                vocoder.display()
            )
        })?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Layout auto-detection
// ---------------------------------------------------------------------------

/// Locate the *acoustic* dsconfig: the one declaring an `acoustic` model
/// key (a sibling variance/dur/pitch dsconfig has the same filename but no
/// `acoustic` key). Searches the root first, then each level of
/// subdirectories down to [`ACOUSTIC_SEARCH_DEPTH`], so banks that nest the
/// acoustic model in `acoustic/` or, like Gahata Meiji, in
/// `configs/configs/` are still found. A shallower config always wins.
fn find_acoustic_config(root: &Path) -> Option<PathBuf> {
    let mut fallback: Option<PathBuf> = None;
    let mut level = vec![root.to_path_buf()];
    for depth in 0..=ACOUSTIC_SEARCH_DEPTH {
        for dir in &level {
            for name in ACOUSTIC_NAMES {
                let path = dir.join(name);
                if !path.is_file() {
                    continue;
                }
                if declares_acoustic(&path) {
                    return Some(path);
                }
                fallback.get_or_insert(path);
            }
        }
        if depth < ACOUSTIC_SEARCH_DEPTH {
            level = level.iter().flat_map(|d| sorted_subdirs(d)).collect();
        }
    }

    // No config with an explicit `acoustic` key: fall back to the first
    // dsconfig we saw (some minimal banks omit the key and the loader
    // surfaces a clear error downstream).
    fallback
}

/// How many directory levels below the bank root an acoustic config may
/// sit (vocal-mcp-control-and-quality.md §4.2: "at most two levels deep").
pub const ACOUSTIC_SEARCH_DEPTH: usize = 2;

/// The subdirectories of `dir`, sorted for deterministic selection across
/// filesystems.
fn sorted_subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}

/// Whether `dir` holds a voicebank: an acoustic `dsconfig.yaml` at most
/// [`ACOUSTIC_SEARCH_DEPTH`] levels down. Cheap enough to run over every
/// folder of a voicebank library.
pub fn looks_like_bank(dir: &Path) -> bool {
    dir.is_dir() && find_acoustic_config(dir).is_some()
}

/// Does this YAML declare an `acoustic:` model key? Reads cheaply and
/// tolerates parse failures (treated as "no", so a malformed sibling
/// never masquerades as the acoustic config).
fn declares_acoustic(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_yaml_ng::from_str::<config::DsAcousticConfigRaw>(&text).ok())
        .is_some_and(|raw| raw.acoustic.is_some())
}

/// The vocoder config a bank bundles. The acoustic config's `vocoder`
/// value names either a folder next to it (`vocoder: dsvocoder`, TIGER and
/// Meiji) or a shared vocoder by id, in which case OpenUtau still prefers a
/// `dsvocoder/` the bank ships (Lilia). Then the older root-level layouts.
/// `None` means the bank relies on a separately installed vocoder.
fn find_vocoder_config(root: &Path, acoustic_dir: &Path, vocoder_name: &str) -> Option<PathBuf> {
    let named = (!vocoder_name.is_empty()
        && !vocoder_name.contains("..")
        && !Path::new(vocoder_name).is_absolute())
    .then(|| acoustic_dir.join(vocoder_name).join("vocoder.yaml"));
    named
        .into_iter()
        .chain([
            acoustic_dir.join("dsvocoder/vocoder.yaml"),
            root.join("dsvocoder/vocoder.yaml"),
        ])
        .chain(VOCODER_RELS.iter().map(|rel| root.join(rel)))
        .find(|p| p.is_file())
}

/// Find a vocoder by id in a vocoder library: `<library>/<id>/vocoder.yaml`,
/// or any `<library>/*/vocoder.yaml` whose `name` is `id`.
pub fn find_library_vocoder(library: &Path, id: &str) -> Option<PathBuf> {
    if id.is_empty() {
        return None;
    }
    let direct = library.join(id).join("vocoder.yaml");
    if !id.contains("..") && direct.is_file() {
        return Some(direct);
    }
    sorted_subdirs(library)
        .into_iter()
        .map(|d| d.join("vocoder.yaml"))
        .filter(|p| p.is_file())
        .find(|p| config::load_vocoder(p).is_ok_and(|v| v.name == id))
}

/// The `name` of the vocoder a config describes, when the config parses
/// and its model file is on disk — what "the vocoder is installed" means
/// to a caller about to render. `None` otherwise.
pub fn installed_vocoder_name(vocoder_config: &Path) -> Option<String> {
    config::load_vocoder(vocoder_config)
        .ok()
        .filter(|v| v.model_path.is_file())
        .map(|v| v.name)
}

/// `name` and `version` from an OpenUtau character file in `dir`:
/// `character.txt` (`key=value` lines) or `character.yaml` (`key: value`).
/// `None` when neither exists or both lack a name.
fn read_character(dir: &Path) -> Option<(Option<String>, Option<String>)> {
    let mut name = None;
    let mut version = None;
    for (file, sep) in [("character.txt", '='), ("character.yaml", ':')] {
        let Ok(text) = std::fs::read_to_string(dir.join(file)) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim_start_matches('\u{feff}').trim();
            let Some((key, value)) = line.split_once(sep) else {
                continue;
            };
            let value = value.trim().trim_matches('"').trim();
            if value.is_empty() {
                continue;
            }
            match key.trim() {
                "name" if name.is_none() => name = Some(value.to_owned()),
                "version" if version.is_none() => version = Some(value.to_owned()),
                _ => {}
            }
        }
    }
    (name.is_some() || version.is_some()).then_some((name, version))
}

/// A dictionary's replacements as pairs; an unreadable one counts as none
/// (the scan must not fail over it — the dictionary is an extra).
fn read_replacements(path: &Path) -> Vec<(String, String)> {
    match super::dsdict::load_replacements(path) {
        Ok(rows) => rows.into_iter().map(|r| (r.from, r.to)).collect(),
        Err(e) => {
            tracing::warn!("ignoring the replacements of {}: {e:#}", path.display());
            Vec::new()
        }
    }
}

/// The bank's OpenUtau dictionary for `lang` (`dsdict-<lang>.yaml`): next
/// to the acoustic config, at the root, or one level below either (the
/// `dsdur/` / `dspitch/` folders that usually carry it), in that order.
pub fn find_dsdict(root: &Path, acoustic_dir: &Path, lang: &str) -> Option<PathBuf> {
    let file = format!("dsdict-{lang}.yaml");
    let mut dirs = vec![acoustic_dir.to_path_buf(), root.to_path_buf()];
    dirs.extend(sorted_subdirs(acoustic_dir));
    dirs.extend(sorted_subdirs(root));
    dirs.into_iter().map(|d| d.join(&file)).find(|p| p.is_file())
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

/// Parse an optional `languages.json` at the bank root. Absent file =
/// empty map (monolingual bank). Banks that name their languages file in
/// the acoustic config (`languages: ../../files/languages.json`) are read
/// from there instead ([`read_languages_file`]).
fn read_languages(root: &Path) -> Result<BTreeMap<String, i64>> {
    let path = root.join("languages.json");
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    read_languages_file(&path)
}

/// Parse a `languages.json`. Accepts the two shapes seen in DiffSinger
/// banks — `{"zh": 0, "en": 1}` or `["zh", "en"]` (index = id) —
/// mirroring the phoneme-dict parser.
fn read_languages_file(path: &Path) -> Result<BTreeMap<String, i64>> {
    let text = std::fs::read_to_string(path)
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

/// The id a bank folder named `name` gets: the name itself when it is
/// already one (lowercase ASCII letters, digits, `_` and `-`, starting
/// with a letter or digit), else its [`slugify`]. A plok.org install
/// unpacks into a folder named by the index id (`keiro_revenant`), so the
/// bank must come back under that same id, not `keiro-revenant`.
pub fn folder_id(name: &str) -> String {
    let mut chars = name.chars();
    let starts_well = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let rest_ok =
        chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if starts_well && rest_ok {
        name.to_owned()
    } else {
        slugify(name)
    }
}

/// Lowercase ASCII slug: alphanumeric runs joined by single `-`, trimmed.
/// `"LIEE Lilia"` -> `"liee-lilia"`, `"TIGER_v2"` -> `"tiger-v2"`.
pub fn slugify(name: &str) -> String {
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
