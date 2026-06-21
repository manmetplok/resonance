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

use resonance_svs::voicebank::{scan, ExpressionCurve, PhonemeTarget, VoicebankManifest};

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
    /// Acoustic-config capability flags (written only when `true`).
    use_lang_id: bool,
    use_energy_embed: bool,
    use_breathiness_embed: bool,
    use_tension_embed: bool,
    use_voicing_embed: bool,
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
            use_lang_id: false,
            use_energy_embed: false,
            use_breathiness_embed: false,
            use_tension_embed: false,
            use_voicing_embed: false,
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
    if spec.use_lang_id {
        cfg.push_str("use_lang_id: true\n");
    }
    if spec.use_energy_embed {
        cfg.push_str("use_energy_embed: true\n");
    }
    if spec.use_breathiness_embed {
        cfg.push_str("use_breathiness_embed: true\n");
    }
    if spec.use_tension_embed {
        cfg.push_str("use_tension_embed: true\n");
    }
    if spec.use_voicing_embed {
        cfg.push_str("use_voicing_embed: true\n");
    }
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
// Data-driven per-bank quirks (doc #164).
//
// These fixtures reproduce the relevant on-disk shape of the three shipped
// banks so the manifest's data-driven methods can be asserted against the
// values previously hardcoded per `VocalVoicebank` arm in resonance-app's
// `vocal_svs/paths.rs` (substitute_phoneme / voicebank_phoneme_name /
// voicebank_language_id / curve_supported). The `// was: <fn>(...)`
// comments quote the old hardcoded result each assertion must match.
// ---------------------------------------------------------------------------

/// Full lowercase-ARPAbet inventory (silence markers, `cl`, every CMU
/// consonant + vowel), one per line — the shape TIGER ships.
const ARPABET_FULL: &str = "AP\nSP\ncl\naa\nae\nah\nao\naw\nay\nb\nch\nd\ndh\neh\ner\ney\nf\ng\nhh\nih\niy\njh\nk\nl\nm\nn\nng\now\noy\np\nr\ns\nsh\nt\nth\nuh\nuw\nv\nw\ny\nz\nzh\n";

/// Same inventory minus the voiced `v` — Lilia's MM set, which covers all
/// of ARPAbet except the voiced labiodental fricative.
const ARPABET_NO_V: &str = "AP\nSP\ncl\naa\nae\nah\nao\naw\nay\nb\nch\nd\ndh\neh\ner\ney\nf\ng\nhh\nih\niy\njh\nk\nl\nm\nn\nng\now\noy\np\nr\ns\nsh\nt\nth\nuh\nuw\nw\ny\nz\nzh\n";

/// Meiji's namespaced dict: a bare universal bucket (silence + shared
/// consonants, notably no `en/hh`) plus the full English set under `en/`.
const MEIJI_DICT: &str = r#"["AP", "SP", "hh", "cl", "ban", "vf",
  "en/aa", "en/ae", "en/ah", "en/ao", "en/aw", "en/ay", "en/b", "en/ch",
  "en/d", "en/dh", "en/eh", "en/er", "en/ey", "en/f", "en/g", "en/ih",
  "en/iy", "en/jh", "en/k", "en/l", "en/m", "en/n", "en/ng", "en/ow",
  "en/oy", "en/p", "en/r", "en/s", "en/sh", "en/t", "en/th", "en/uh",
  "en/uw", "en/v", "en/w", "en/y", "en/z", "en/zh"]"#;

/// A representative spread of G2P-emitted ARPAbet symbols to exercise the
/// phoneme/language methods across silence markers, consonants and vowels.
const SAMPLE_PHONEMES: &[&str] = &["AP", "SP", "cl", "hh", "ah", "ae", "f", "v", "s", "t"];

fn tiger_like(name: &str) -> VoicebankManifest {
    let dir = fresh_dir(name);
    build_bank(
        &dir,
        &BankSpec {
            speakers: vec!["s0", "s1", "s2", "s3", "s4", "s5", "s6"],
            phoneme_body: ARPABET_FULL,
            // Energy is present on every shipped bank (doc #154: dynamics
            // maps to the universally-present energy curve); TIGER's model
            // exposes no tension/breathiness input.
            use_energy_embed: true,
            ..BankSpec::default()
        },
    );
    scan(&dir).expect("tiger-like bank scans")
}

fn lilia_like(name: &str) -> VoicebankManifest {
    let dir = fresh_dir(name);
    build_bank(
        &dir,
        &BankSpec {
            speakers: vec![], // single-speaker
            phoneme_body: ARPABET_NO_V,
            use_energy_embed: true,
            use_breathiness_embed: true,
            use_tension_embed: true,
            ..BankSpec::default()
        },
    );
    scan(&dir).expect("lilia-like bank scans")
}

fn meiji_like(name: &str) -> VoicebankManifest {
    let dir = fresh_dir(name);
    build_bank(
        &dir,
        &BankSpec {
            speakers: vec!["m0", "m1", "m2", "m3"],
            phoneme_file: "phonemes.json",
            phoneme_body: MEIJI_DICT,
            languages: Some(r#"{"zh": 0, "ja": 1, "ko": 2, "en": 3}"#),
            use_lang_id: true,
            use_energy_embed: true,
            use_breathiness_embed: true,
            use_tension_embed: true,
            ..BankSpec::default()
        },
    );
    scan(&dir).expect("meiji-like bank scans")
}

#[test]
fn all_shipped_banks_target_arpabet() {
    // was: every VocalVoicebank uses bare/`en/`-prefixed ARPAbet, none x-sampa.
    assert_eq!(tiger_like("q_tiger_tgt").phoneme_target, PhonemeTarget::Arpabet);
    assert_eq!(lilia_like("q_lilia_tgt").phoneme_target, PhonemeTarget::Arpabet);
    assert_eq!(meiji_like("q_meiji_tgt").phoneme_target, PhonemeTarget::Arpabet);
}

#[test]
fn xsampa_inventory_is_detected() {
    // A bank whose dict mixes in X-SAMPA-only glyphs is classified XSampa.
    let dir = fresh_dir("q_xsampa");
    build_bank(
        &dir,
        &BankSpec {
            phoneme_body: "AP\nSP\n@\n{\nr\\\nO\n",
            ..BankSpec::default()
        },
    );
    let m = scan(&dir).expect("scan");
    assert_eq!(m.phoneme_target, PhonemeTarget::XSampa);
}

#[test]
fn tiger_quirks_match_hardcoded() {
    let m = tiger_like("q_tiger");

    for &ph in SAMPLE_PHONEMES {
        // was: substitute_phoneme(Tiger, ph) == ph (identity — full inventory).
        assert_eq!(m.substitute_phoneme(ph), ph, "tiger sub {ph}");
        // was: voicebank_phoneme_name(Tiger, ph) == ph (bare ARPAbet).
        assert_eq!(m.phoneme_name(ph), ph, "tiger name {ph}");
        // was: voicebank_language_id(Tiger, ph) == None (no languages input).
        assert_eq!(m.language_id(ph), None, "tiger lang {ph}");
    }

    // was: curve_supported(Tiger, …) — dynamics/pitch yes, tension/breath no.
    assert!(m.supports_curve(ExpressionCurve::Dynamics));
    assert!(m.supports_curve(ExpressionCurve::PitchBend));
    assert!(!m.supports_curve(ExpressionCurve::Tension));
    assert!(!m.supports_curve(ExpressionCurve::Breathiness));
}

#[test]
fn lilia_quirks_match_hardcoded() {
    let m = lilia_like("q_lilia");

    // was: substitute_phoneme(Lilia, "v") == "f" (only documented sub).
    assert_eq!(m.substitute_phoneme("v"), "f", "lilia v->f");
    // Every other sampled phone is present, so substitution is identity.
    for &ph in SAMPLE_PHONEMES.iter().filter(|p| **p != "v") {
        assert_eq!(m.substitute_phoneme(ph), ph, "lilia sub {ph}");
    }
    for &ph in SAMPLE_PHONEMES {
        // was: voicebank_phoneme_name(Lilia, ph) == ph (bare ARPAbet).
        assert_eq!(m.phoneme_name(ph), ph, "lilia name {ph}");
        // was: voicebank_language_id(Lilia, ph) == None.
        assert_eq!(m.language_id(ph), None, "lilia lang {ph}");
    }

    // was: curve_supported(Lilia, …) — all four supported.
    for c in [
        ExpressionCurve::Dynamics,
        ExpressionCurve::Tension,
        ExpressionCurve::Breathiness,
        ExpressionCurve::PitchBend,
    ] {
        assert!(m.supports_curve(c), "lilia curve {c:?}");
    }
}

#[test]
fn meiji_quirks_match_hardcoded() {
    let m = meiji_like("q_meiji");

    // was: substitute_phoneme(Meiji, ph) == ph (full English set present).
    for &ph in SAMPLE_PHONEMES {
        assert_eq!(m.substitute_phoneme(ph), ph, "meiji sub {ph}");
    }

    // was: voicebank_phoneme_name(Meiji, ph) — universal bucket bare, rest `en/`.
    for &uni in &["AP", "SP", "cl", "hh"] {
        assert_eq!(m.phoneme_name(uni), uni, "meiji universal {uni}");
    }
    for &(ph, want) in &[("ah", "en/ah"), ("ae", "en/ae"), ("f", "en/f"), ("v", "en/v")] {
        assert_eq!(m.phoneme_name(ph), want, "meiji name {ph}");
    }

    // was: voicebank_language_id(Meiji, ph) — 0 for the bucket, 3 for English.
    for &uni in &["AP", "SP", "cl", "hh"] {
        assert_eq!(m.language_id(uni), Some(0), "meiji lang {uni}");
    }
    for &ph in &["ah", "ae", "f", "v"] {
        assert_eq!(m.language_id(ph), Some(3), "meiji lang {ph}");
    }

    // was: curve_supported(Meiji, …) — all four supported.
    for c in [
        ExpressionCurve::Dynamics,
        ExpressionCurve::Tension,
        ExpressionCurve::Breathiness,
        ExpressionCurve::PitchBend,
    ] {
        assert!(m.supports_curve(c), "meiji curve {c:?}");
    }
}

// ---------------------------------------------------------------------------
// Real-bank assertions (env-gated, mirroring smoke.rs).
// ---------------------------------------------------------------------------

/// Scan an env-named real bank, or `None` (with a skip note) when unset.
fn real_bank(env: &str) -> Option<VoicebankManifest> {
    let path = std::env::var_os(env).map(PathBuf::from);
    let Some(path) = path else {
        eprintln!("{env} unset; skipping real-voicebank assertion");
        return None;
    };
    let m: VoicebankManifest = scan(&path).unwrap_or_else(|e| panic!("scan {env}: {e:#}"));
    m.validate()
        .unwrap_or_else(|e| panic!("validate {env}: {e:#}"));
    Some(m)
}

fn assert_singers(m: &VoicebankManifest, env: &str, expected: usize) {
    assert_eq!(
        m.singers.len(),
        expected,
        "{env} ({}) should report {expected} singers, got {:?}",
        m.display_name,
        m.singers.iter().map(|s| &s.id).collect::<Vec<_>>()
    );
}

#[test]
fn real_tiger_has_seven_singers() {
    let Some(m) = real_bank("SVS_TIGER_DIR") else {
        return;
    };
    assert_singers(&m, "SVS_TIGER_DIR", 7);
    // The data-driven quirks must match TIGER's hardcoded behaviour.
    assert_eq!(m.phoneme_target, PhonemeTarget::Arpabet);
    assert_eq!(m.phoneme_name("ah"), "ah");
    assert_eq!(m.substitute_phoneme("v"), "v", "TIGER ships v");
    assert_eq!(m.language_id("ah"), None, "TIGER takes no languages input");
    assert!(!m.supports_curve(ExpressionCurve::Tension));
    assert!(!m.supports_curve(ExpressionCurve::Breathiness));
    assert!(m.supports_curve(ExpressionCurve::Dynamics));
    assert!(m.supports_curve(ExpressionCurve::PitchBend));
}

#[test]
fn real_lilia_is_single_speaker() {
    let Some(m) = real_bank("SVS_LILIA_DIR") else {
        return;
    };
    assert_singers(&m, "SVS_LILIA_DIR", 0);
    assert_eq!(m.phoneme_target, PhonemeTarget::Arpabet);
    assert_eq!(m.substitute_phoneme("v"), "f", "Lilia lacks v -> f");
    assert_eq!(m.phoneme_name("ah"), "ah");
    assert_eq!(m.language_id("ah"), None);
    assert!(m.supports_curve(ExpressionCurve::Tension));
    assert!(m.supports_curve(ExpressionCurve::Breathiness));
}

#[test]
fn real_meiji_has_four_singers() {
    let Some(m) = real_bank("SVS_MEIJI_DIR") else {
        return;
    };
    assert_singers(&m, "SVS_MEIJI_DIR", 4);
    assert_eq!(m.phoneme_target, PhonemeTarget::Arpabet);
    // Universal bucket stays bare with language id 0; English is `en/` / 3.
    assert_eq!(m.phoneme_name("hh"), "hh");
    assert_eq!(m.phoneme_name("ah"), "en/ah");
    assert_eq!(m.language_id("hh"), Some(0));
    assert_eq!(m.language_id("ah"), Some(3));
    assert_eq!(m.substitute_phoneme("v"), "v", "Meiji ships en/v");
    assert!(m.supports_curve(ExpressionCurve::Tension));
    assert!(m.supports_curve(ExpressionCurve::Breathiness));
}
