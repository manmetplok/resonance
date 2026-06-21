//! Coverage for the voicebank manifest scanner.
//!
//! The synthetic-fixture tests build minimal on-disk banks under the temp
//! dir (configs + phoneme dict + vocoder config, no ONNX needed — the
//! loaders the manifest exercises only read text/YAML/JSON) and assert
//! scan/validate behaviour and layout auto-detection.
//!
//! The real-bank assertions (TIGER 7 / Lilia 0 / Meiji 4 singers) run only
//! when `SVS_TIGER_DIR` / `SVS_LILIA_DIR` / `SVS_MEIJI_DIR` point at the
//! banks on disk — mirroring `smoke.rs`'s `SVS_POC_VOICEBANK_DIR` gate so
//! CI never needs the multi-gigabyte downloads.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use resonance_svs::voicebank::{scan, VoicebankManifest};

/// A fresh, empty fixture dir named `name` under the temp dir. Removed
/// first so reruns start clean; named per test so parallel tests don't
/// collide.
fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("resonance_svs_vb").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(path, body).expect("write fixture file");
}

/// Options for [`build_bank`].
struct BankSpec {
    /// `speakers:` list (empty => single-speaker, omitted from YAML).
    speakers: Vec<&'static str>,
    /// Phoneme dict filename + body (extension drives txt vs json parsing).
    phoneme_file: &'static str,
    phoneme_body: &'static str,
    /// Optional `languages.json` body.
    languages: Option<&'static str>,
    /// Write a `vocoder.yaml`? (and is it valid?)
    vocoder: VocoderFixture,
}

enum VocoderFixture {
    None,
    Valid,
    MissingModelKey,
}

impl Default for BankSpec {
    fn default() -> Self {
        Self {
            speakers: vec![],
            phoneme_file: "phonemes.txt",
            phoneme_body: "AP\nSP\na\ne\ni\no\nu\n",
            languages: None,
            vocoder: VocoderFixture::Valid,
        }
    }
}

/// Write a minimal but loadable acoustic bank into `dir`.
fn build_bank(dir: &Path, spec: &BankSpec) {
    let mut cfg = String::new();
    cfg.push_str(&format!("phonemes: {}\n", spec.phoneme_file));
    cfg.push_str("acoustic: acoustic.onnx\n");
    cfg.push_str("vocoder: nsf_hifigan\n");
    cfg.push_str("dur: dur.onnx\n");
    cfg.push_str("pitch: pitch.onnx\n");
    cfg.push_str("variance: variance.onnx\n");
    cfg.push_str("linguistic: linguistic.onnx\n");
    if !spec.speakers.is_empty() {
        cfg.push_str("speakers:\n");
        for s in &spec.speakers {
            cfg.push_str(&format!("  - {s}\n"));
        }
    }
    write(dir, "dsconfig.yaml", &cfg);
    write(dir, spec.phoneme_file, spec.phoneme_body);
    match spec.vocoder {
        VocoderFixture::None => {}
        VocoderFixture::Valid => write(
            dir,
            "vocoder.yaml",
            "name: nsf_hifigan\nmodel: vocoder.onnx\n",
        ),
        VocoderFixture::MissingModelKey => write(dir, "vocoder.yaml", "name: nsf_hifigan\n"),
    }
    if let Some(langs) = spec.languages {
        write(dir, "languages.json", langs);
    }
}

#[test]
fn scans_multi_speaker_bank() {
    let dir = fresh_dir("multi_speaker");
    build_bank(
        &dir,
        &BankSpec {
            speakers: vec!["alice", "bob", "carol"],
            languages: Some(r#"{"zh": 0, "en": 1}"#),
            ..BankSpec::default()
        },
    );

    let m = scan(&dir).expect("scan should succeed");

    assert_eq!(m.singers.len(), 3, "three speakers => three singers");
    assert!(!m.is_single_speaker());
    let ids: Vec<&str> = m.singers.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["alice", "bob", "carol"]);
    assert_eq!(m.singers[0].display_name, "alice");

    // Phoneme inventory in token-id (line) order.
    assert_eq!(m.phonemes, ["AP", "SP", "a", "e", "i", "o", "u"]);

    // Languages parsed into a sorted map.
    let langs: BTreeMap<String, i64> = m.languages.clone();
    assert_eq!(langs.get("zh"), Some(&0));
    assert_eq!(langs.get("en"), Some(&1));

    // Resolved paths live under the bank root and exist where written.
    assert_eq!(
        m.acoustic_config,
        dir.canonicalize().unwrap().join("dsconfig.yaml")
    );
    assert!(m.acoustic_config.is_file());
    assert!(m.phoneme_dict.is_file());
    assert_eq!(
        m.vocoder_config.as_deref(),
        Some(dir.canonicalize().unwrap().join("vocoder.yaml").as_path())
    );
    // Optional model paths resolved from the config keys.
    assert!(m.variance_model.is_some());
    assert!(m.dur_model.is_some());
    assert!(m.pitch_model.is_some());
    assert!(m.linguistic_model.is_some());

    m.validate().expect("a complete bank validates");
}

#[test]
fn single_speaker_bank_has_no_singers() {
    let dir = fresh_dir("single_speaker");
    build_bank(&dir, &BankSpec::default());

    let m = scan(&dir).expect("scan should succeed");
    assert!(m.singers.is_empty(), "no speakers key => empty singers");
    assert!(m.is_single_speaker());
    assert!(m.languages.is_empty(), "no languages.json => empty map");
    m.validate().expect("single-speaker bank validates");
}

#[test]
fn json_phoneme_dict_sorted_by_explicit_id() {
    let dir = fresh_dir("json_dict");
    build_bank(
        &dir,
        &BankSpec {
            phoneme_file: "phonemes.json",
            // Deliberately out of id order to prove sorting.
            phoneme_body: r#"{"a": 2, "AP": 0, "SP": 1, "e": 3}"#,
            languages: Some(r#"["zh", "ja", "en"]"#),
            ..BankSpec::default()
        },
    );

    let m = scan(&dir).expect("scan should succeed");
    assert_eq!(
        m.phonemes,
        ["AP", "SP", "a", "e"],
        "inventory ordered by id"
    );
    // Array-form languages.json: index = id.
    assert_eq!(m.languages.get("zh"), Some(&0));
    assert_eq!(m.languages.get("ja"), Some(&1));
    assert_eq!(m.languages.get("en"), Some(&2));
}

#[test]
fn slug_and_display_name_derive_from_folder() {
    let dir = fresh_dir("LIEE Lilia");
    build_bank(&dir, &BankSpec::default());

    let m = scan(&dir).expect("scan should succeed");
    assert_eq!(m.display_name, "LIEE Lilia");
    assert_eq!(m.id, "liee-lilia");
}

#[test]
fn finds_acoustic_config_one_level_down() {
    // Bank that nests the acoustic config in an `acoustic/` subdir, with a
    // decoy variance dsconfig (no `acoustic:` key) at the root.
    let dir = fresh_dir("nested_layout");
    write(
        &dir,
        "dsconfig.yaml",
        "linguistic: linguistic.onnx\ndur: dur.onnx\npitch: pitch.onnx\n",
    );
    write(
        &dir,
        "acoustic/dsconfig.yaml",
        "phonemes: phonemes.txt\nacoustic: acoustic.onnx\nspeakers:\n  - solo\n",
    );
    write(&dir, "acoustic/phonemes.txt", "AP\nSP\na\n");

    let m = scan(&dir).expect("scan should find the nested acoustic config");
    assert_eq!(
        m.acoustic_config,
        dir.canonicalize().unwrap().join("acoustic/dsconfig.yaml")
    );
    assert_eq!(m.singers.len(), 1);
    assert_eq!(m.phonemes, ["AP", "SP", "a"]);
}

#[test]
fn rejects_non_directory() {
    let dir = fresh_dir("not_a_dir");
    let file = dir.join("file.txt");
    std::fs::write(&file, "x").unwrap();
    let err = scan(&file).expect_err("a file is not a voicebank");
    assert!(
        err.to_string().contains("not a directory"),
        "error should explain the path is not a directory: {err}"
    );
}

#[test]
fn rejects_folder_without_acoustic_config() {
    let dir = fresh_dir("empty_bank");
    write(&dir, "readme.txt", "no configs here");
    let err = scan(&dir).expect_err("a folder with no dsconfig is not a voicebank");
    assert!(
        err.to_string().contains("no acoustic dsconfig"),
        "error should explain the missing acoustic config: {err}"
    );
}

#[test]
fn rejects_acoustic_config_missing_required_key() {
    // A dsconfig with neither a phonemes nor acoustic key: find_acoustic
    // falls back to it, and load_acoustic surfaces the missing key.
    let dir = fresh_dir("broken_acoustic");
    write(&dir, "dsconfig.yaml", "hidden_size: 256\n");
    let err = scan(&dir).expect_err("an acoustic config missing required keys is rejected");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("acoustic") || msg.contains("phonemes"),
        "error should name the missing required key: {msg}"
    );
}

#[test]
fn bank_without_vocoder_scans_and_validates() {
    // Some banks rely on a shared vocoder supplied separately; the
    // manifest reports `vocoder_config: None` and validate() skips the
    // (absent) vocoder rather than failing the otherwise-usable bank.
    let dir = fresh_dir("no_vocoder");
    build_bank(
        &dir,
        &BankSpec {
            vocoder: VocoderFixture::None,
            ..BankSpec::default()
        },
    );

    let m = scan(&dir).expect("scan should succeed without a vocoder");
    assert!(m.vocoder_config.is_none());
    m.validate()
        .expect("a bank with no bundled vocoder still validates its acoustic side");
}

#[test]
fn validate_rejects_unusable_vocoder() {
    let dir = fresh_dir("broken_vocoder");
    build_bank(
        &dir,
        &BankSpec {
            vocoder: VocoderFixture::MissingModelKey,
            ..BankSpec::default()
        },
    );

    // Scan succeeds — it doesn't parse the vocoder config — but validate,
    // which exercises load_vocoder, rejects the bank with a clear error.
    let m = scan(&dir).expect("scan ignores vocoder contents");
    let err = m
        .validate()
        .expect_err("missing vocoder model key is unusable");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("vocoder") && msg.contains("model"),
        "validate should explain the unusable vocoder: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Real-bank assertions (env-gated, mirroring smoke.rs).
// ---------------------------------------------------------------------------

fn check_real_bank(env: &str, expected_singers: usize) {
    let Some(path) = std::env::var_os(env).map(PathBuf::from) else {
        eprintln!("{env} unset; skipping real-voicebank assertion");
        return;
    };
    let m: VoicebankManifest = scan(&path).unwrap_or_else(|e| panic!("scan {env}: {e:#}"));
    m.validate()
        .unwrap_or_else(|e| panic!("validate {env}: {e:#}"));
    assert_eq!(
        m.singers.len(),
        expected_singers,
        "{env} ({}) should report {expected_singers} singers, got {:?}",
        m.display_name,
        m.singers.iter().map(|s| &s.id).collect::<Vec<_>>()
    );
}

#[test]
fn real_tiger_has_seven_singers() {
    check_real_bank("SVS_TIGER_DIR", 7);
}

#[test]
fn real_lilia_is_single_speaker() {
    check_real_bank("SVS_LILIA_DIR", 0);
}

#[test]
fn real_meiji_has_four_singers() {
    check_real_bank("SVS_MEIJI_DIR", 4);
}
