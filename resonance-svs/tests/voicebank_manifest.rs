//! Coverage for the voicebank manifest scanner — the on-disk half.
//!
//! The synthetic-fixture tests build minimal on-disk banks under the temp
//! dir (configs + phoneme dict + vocoder config, no ONNX needed — the
//! loaders the manifest exercises only read text/YAML/JSON) and assert
//! scan/validate behaviour and layout auto-detection, plus that the scan
//! wires the resulting inventory into the phonetic model.
//!
//! The phonetic model itself (substitution table, alphabet detection,
//! dict-key resolution, curve capabilities) is asserted without any
//! filesystem at all in `tests/voicebank_phonetics.rs`.
//!
//! The real-bank assertions (TIGER 7 / Lilia 0 / Meiji 4 singers) run only
//! when `SVS_TIGER_DIR` / `SVS_LILIA_DIR` / `SVS_MEIJI_DIR` point at the
//! banks on disk — mirroring `smoke.rs`'s `SVS_POC_VOICEBANK_DIR` gate so
//! CI never needs the multi-gigabyte downloads.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use resonance_svs::voicebank::layout::read_curve_support;
use resonance_svs::voicebank::{
    scan, CurveSupport, ExpressionCurve, PhonemeTarget, VoicebankManifest,
};

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
// Data-driven per-bank quirks (doc #164): the wiring.
//
// These fixtures reproduce the relevant on-disk shape of the three shipped
// banks and assert that a scan hands the right inventory, languages map and
// capability flags to the phonetic model, so the manifest's data-driven
// methods reproduce the values previously hardcoded per `VocalVoicebank` arm
// in resonance-app's `vocal_svs/paths.rs`. The exhaustive per-phone matrices
// live in `tests/voicebank_phonetics.rs`, which needs no folder on disk.
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

fn tiger_like(name: &str) -> VoicebankManifest {
    let dir = fresh_dir(name);
    build_bank(
        &dir,
        &BankSpec {
            speakers: vec!["s0", "s1", "s2", "s3", "s4", "s5", "s6"],
            phoneme_body: ARPABET_FULL,
            // Mirrors the shipped TIGER v106 acoustic config: no energy,
            // breathiness or tension embed.
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
            // Shipped Lilia MM 2.8: tension only.
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
            // Shipped Meiji: tension only.
            use_tension_embed: true,
            ..BankSpec::default()
        },
    );
    scan(&dir).expect("meiji-like bank scans")
}

#[test]
fn scan_detects_the_alphabet_from_the_dict_on_disk() {
    // was: every VocalVoicebank uses bare/`en/`-prefixed ARPAbet, none x-sampa.
    assert_eq!(
        tiger_like("q_tiger_tgt").phoneme_target,
        PhonemeTarget::Arpabet
    );
    assert_eq!(
        lilia_like("q_lilia_tgt").phoneme_target,
        PhonemeTarget::Arpabet
    );
    assert_eq!(
        meiji_like("q_meiji_tgt").phoneme_target,
        PhonemeTarget::Arpabet
    );

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
fn scan_wires_the_inventory_into_the_phonetic_model() {
    // The scanned inventory is what the phonetic model answers from, so a
    // bank shipping `v` substitutes nothing while one missing it sings `f`.
    // was: substitute_phoneme(Tiger, "v") == "v" / (Lilia, "v") == "f".
    let tiger = tiger_like("q_tiger");
    assert_eq!(tiger.substitute_phoneme("v"), "v", "tiger ships v");
    assert_eq!(tiger.inventory().substitute_phoneme("v"), "v");

    let lilia = lilia_like("q_lilia");
    assert_eq!(lilia.substitute_phoneme("v"), "f", "lilia v->f");
    assert_eq!(lilia.inventory().substitute_phoneme("v"), "f");
    // was: voicebank_phoneme_name(Lilia, ph) == ph (bare ARPAbet).
    assert_eq!(lilia.phoneme_name("ah"), "ah");
}

#[test]
fn scan_wires_languages_and_namespacing_into_the_phonetic_model() {
    let m = meiji_like("q_meiji");

    // was: voicebank_phoneme_name(Meiji, ph) — universal bucket bare, rest `en/`.
    assert_eq!(m.phoneme_name("hh"), "hh", "meiji universal hh");
    assert_eq!(m.phoneme_name("ah"), "en/ah", "meiji name ah");
    // was: substitute_phoneme(Meiji, ph) == ph (full English set present).
    assert_eq!(m.substitute_phoneme("v"), "v", "meiji ships en/v");

    // was: voicebank_language_id(Meiji, ph) — 0 for the bucket, 3 for English.
    // The ids come from the scanned languages.json, not a hardcoded table.
    assert_eq!(m.language_id("hh"), Some(0), "meiji lang hh");
    assert_eq!(m.language_id("ah"), Some(3), "meiji lang ah");
}

#[test]
fn language_id_is_gated_on_the_acoustic_models_languages_input() {
    // was: voicebank_language_id(Tiger|Lilia, ph) == None — banks whose
    // model takes no `languages` input report nothing, whatever their dict
    // says. This gate is the manifest's, not the inventory's.
    for m in [tiger_like("q_tiger_lang"), lilia_like("q_lilia_lang")] {
        assert!(!m.accepts_language_id);
        for ph in ["AP", "ah", "f"] {
            assert_eq!(m.language_id(ph), None, "lang {ph}");
        }
    }
}

#[test]
fn scan_wires_the_acoustic_embeds_into_curve_support() {
    // The shipped configs: TIGER takes none of the three embeds, Lilia and
    // Meiji take tension only. Pitch bend needs no embed.
    let tiger = tiger_like("q_tiger_curves");
    assert!(tiger.supports_curve(ExpressionCurve::PitchBend));
    assert!(!tiger.supports_curve(ExpressionCurve::Dynamics));
    assert!(!tiger.supports_curve(ExpressionCurve::Tension));
    assert!(!tiger.supports_curve(ExpressionCurve::Breathiness));

    for m in [lilia_like("q_lilia_curves"), meiji_like("q_meiji_curves")] {
        assert!(m.supports_curve(ExpressionCurve::PitchBend), "{}", m.id);
        assert!(m.supports_curve(ExpressionCurve::Tension), "{}", m.id);
        assert!(!m.supports_curve(ExpressionCurve::Dynamics), "{}", m.id);
        assert!(!m.supports_curve(ExpressionCurve::Breathiness), "{}", m.id);
    }

    // A bank that does declare energy and breathiness gets both.
    let dir = fresh_dir("q_full_curves");
    build_bank(
        &dir,
        &BankSpec {
            use_energy_embed: true,
            use_breathiness_embed: true,
            use_tension_embed: true,
            ..BankSpec::default()
        },
    );
    let full = scan(&dir).expect("full-embed bank scans");
    assert!(full.supports_curve(ExpressionCurve::Dynamics));
    assert!(full.supports_curve(ExpressionCurve::Breathiness));
}

#[test]
fn read_curve_support_reads_only_the_acoustic_flags() {
    // The app's cheap path: one acoustic dsconfig, no bank probe.
    let dir = fresh_dir("q_read_curve_support");
    build_bank(
        &dir,
        &BankSpec {
            use_energy_embed: true,
            use_tension_embed: true,
            ..BankSpec::default()
        },
    );
    let support = read_curve_support(&dir.join("dsconfig.yaml")).expect("flags read");
    assert_eq!(
        support,
        CurveSupport {
            energy: true,
            tension: true,
            breathiness: false,
        }
    );

    // Absent flags default to false (the dsconfig loader's default).
    let bare = fresh_dir("q_read_curve_support_bare");
    build_bank(&bare, &BankSpec::default());
    let support = read_curve_support(&bare.join("dsconfig.yaml")).expect("flags read");
    assert_eq!(support, CurveSupport::default());

    // A missing config is an error, not a silent "nothing supported".
    assert!(read_curve_support(&bare.join("nope.yaml")).is_err());
}

// ---------------------------------------------------------------------------
// Real-bank assertions (env-gated, mirroring smoke.rs).
// ---------------------------------------------------------------------------

/// Scan an env-named real bank. Panics with the env var to set when it's unset — these
/// tests are `#[ignore]`d, so a run that reaches this point asked for the real thing.
fn real_bank(env: &str) -> VoicebankManifest {
    let path = std::env::var_os(env)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("set {env} to a real voicebank directory, run with -- --ignored"));
    let m: VoicebankManifest = scan(&path).unwrap_or_else(|e| panic!("scan {env}: {e:#}"));
    m.validate()
        .unwrap_or_else(|e| panic!("validate {env}: {e:#}"));
    m
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
#[ignore = "needs a voicebank: set SVS_TIGER_DIR, run with -- --ignored"]
fn real_tiger_has_seven_singers() {
    let m = real_bank("SVS_TIGER_DIR");
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
#[ignore = "needs a voicebank: set SVS_LILIA_DIR, run with -- --ignored"]
fn real_lilia_is_single_speaker() {
    let m = real_bank("SVS_LILIA_DIR");
    assert_singers(&m, "SVS_LILIA_DIR", 0);
    assert_eq!(m.phoneme_target, PhonemeTarget::Arpabet);
    assert_eq!(m.substitute_phoneme("v"), "f", "Lilia lacks v -> f");
    assert_eq!(m.phoneme_name("ah"), "ah");
    assert_eq!(m.language_id("ah"), None);
    assert!(m.supports_curve(ExpressionCurve::Tension));
    assert!(m.supports_curve(ExpressionCurve::Breathiness));
}

#[test]
#[ignore = "needs a voicebank: set SVS_MEIJI_DIR, run with -- --ignored"]
fn real_meiji_has_four_singers() {
    let m = real_bank("SVS_MEIJI_DIR");
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

// ---------------------------------------------------------------------------
// Registry-facing layout facts (vocal-mcp-control-and-quality.md §4.2–4.4)
// ---------------------------------------------------------------------------

#[test]
fn finds_an_acoustic_config_two_levels_down_with_its_named_languages_file() {
    // Gahata Meiji's layout: configs/configs/dsconfig.yaml naming
    // ../../files/languages.json, and a bundled dsvocoder next to it.
    let dir = fresh_dir("two_levels");
    write(
        &dir,
        "configs/configs/dsconfig.yaml",
        "phonemes: ../../files/phonemes.txt\nacoustic: ../../files/acoustic.onnx\n\
         languages: ../../files/languages.json\nvocoder: dsvocoder\nuse_lang_id: true\n",
    );
    write(&dir, "files/phonemes.txt", "AP\nSP\nen/ah\n");
    write(&dir, "files/languages.json", r#"{"en": 3}"#);
    write(&dir, "configs/configs/dsvocoder/vocoder.yaml", "name: v\nmodel: v.onnx\n");
    write(&dir, "configs/configs/dsdur/dsdict-en.yaml", "entries: []\n");
    write(&dir, "configs/configs/character.txt", "name=Two Levels\nversion=v170\n");

    let m = scan(&dir).expect("found two levels down");
    assert!(m.acoustic_config.ends_with("configs/configs/dsconfig.yaml"));
    assert_eq!(m.languages.get("en"), Some(&3), "read from the config's path");
    assert_eq!(m.language_id("ah"), Some(3));
    assert!(m.vocoder_config.unwrap().ends_with("configs/configs/dsvocoder/vocoder.yaml"));
    assert_eq!(m.details.vocoder_name, "dsvocoder");
    assert_eq!(m.details.name.as_deref(), Some("Two Levels"));
    assert_eq!(m.details.version.as_deref(), Some("v170"));
    assert!(m.details.english_dict.unwrap().ends_with("dsdur/dsdict-en.yaml"));
}

#[test]
fn a_shared_vocoder_is_found_by_folder_or_by_name() {
    use resonance_svs::voicebank::layout::{find_library_vocoder, installed_vocoder_name};
    let lib = fresh_dir("vocoder_library");
    write(&lib, "nsf_2024/vocoder.yaml", "name: nsf_2024\nmodel: m.onnx\n");
    write(&lib, "other-folder/vocoder.yaml", "name: pc_nsf_2025\nmodel: m.onnx\n");
    write(&lib, "other-folder/m.onnx", "");
    assert!(find_library_vocoder(&lib, "nsf_2024").is_some(), "by folder");
    let by_name = find_library_vocoder(&lib, "pc_nsf_2025").expect("by its name");
    assert_eq!(installed_vocoder_name(&by_name).as_deref(), Some("pc_nsf_2025"));
    // A config whose model file is absent is not an installed vocoder.
    let no_model = find_library_vocoder(&lib, "nsf_2024").unwrap();
    assert_eq!(installed_vocoder_name(&no_model), None);
    assert!(find_library_vocoder(&lib, "missing").is_none());
}

#[test]
fn dsdict_entries_parse_tolerantly() {
    use resonance_svs::voicebank::dsdict::parse_dsdict;
    let entries = parse_dsdict(
        "symbols:\n- {symbol: aa, type: vowel}\nentries:\n\
         - {grapheme: the, phonemes: [dh, ax]}\n\
         - {grapheme: no, phonemes: [n, ow]}\n\
         - {grapheme: universe, phonemes: [j, uw, n, ih, w , er, s]}\n\
         - {grapheme: broken}\n\
         - {phonemes: [a]}\n\
         replacements:\n- {from: a, to: b}\n",
    )
    .expect("parses");
    let words: Vec<&str> = entries.iter().map(|e| e.grapheme.as_str()).collect();
    assert_eq!(words, ["the", "no", "universe"], "malformed rows are skipped");
    assert_eq!(entries[2].phonemes.len(), 7);
    assert!(parse_dsdict("symbols: []\n").unwrap().is_empty());
}

#[test]
fn dsdict_replacements_parse_from_their_own_section() {
    use resonance_svs::voicebank::dsdict::{parse_replacements, DsDictReplacement};
    // LIEE Lilia's shape: entries first (one malformed), then a commented
    // `replacements:` header, then another top-level key.
    let pairs = parse_replacements(
        "entries:\n- {grapheme: want, phonemes: [L, aa, n]}\n- {grapheme: broken\n\
         replacements: # crothlang moment\n- {from: t, to: t0}\n- {from: w, to: L}\n\
         - {from: x}\nsymbols:\n- {symbol: aa, type: vowel}\n",
    )
    .expect("the section parses although an entry does not");
    assert_eq!(
        pairs,
        [
            DsDictReplacement {
                from: "t".into(),
                to: "t0".into()
            },
            DsDictReplacement {
                from: "w".into(),
                to: "L".into()
            },
        ]
    );
    assert!(parse_replacements("entries: []\n").unwrap().is_empty());
}

#[test]
fn a_scan_spells_phonemes_the_way_the_english_dictionary_replaces_them() {
    // Lilia's layout: dsconfig at the root, the dictionary in dsdur/, a
    // multi-language inventory where English `w` is `L` and `v` is `w`.
    let dir = fresh_dir("replacements");
    write(
        &dir,
        "dsconfig.yaml",
        "phonemes: phonemes.txt\nacoustic: acoustic.onnx\nvocoder: dsvocoder\n",
    );
    write(&dir, "phonemes.txt", "AP\nSP\nah\nt\nt0\nw\nL\n");
    write(&dir, "dsvocoder/vocoder.yaml", "name: v\nmodel: v.onnx\n");
    write(
        &dir,
        "dsdur/dsdict-en.yaml",
        "entries: []\nreplacements:\n- {from: t, to: t0}\n- {from: v, to: w}\n\
         - {from: w, to: L}\n- {from: y, to: missing}\n",
    );
    let m = scan(&dir).expect("scans");
    assert_eq!(m.details.english_replacements.len(), 4);
    assert_eq!(m.phoneme_name("t"), "t0");
    assert_eq!(m.phoneme_name("w"), "L");
    assert_eq!(m.phoneme_name("v"), "w");
    assert_eq!(m.phoneme_name("ah"), "ah");
    // A replacement into a symbol the inventory lacks is ignored.
    assert_eq!(m.phoneme_name("y"), "y");
    assert_eq!(m.inventory().dict_key_for("y"), None);
}

/// The bank's own duration / pitch / variance models sit in `dsdur/`,
/// `dspitch/`, `dsvariance/` next to the acoustic config (OpenUtau's
/// layout). One counts when its config names a linguistic encoder and the
/// model and both files exist; its speakers map onto the acoustic ones by
/// their last path segment, falling back to its first.
#[test]
fn the_banks_own_models_are_found_next_to_the_acoustic_config() {
    use resonance_svs::predict::{discover, ModelConfig, ModelKind};
    let dir = fresh_dir("bank_models");
    write(
        &dir,
        "dsconfig.yaml",
        "phonemes: phonemes.txt\nacoustic: acoustic.onnx\nvocoder: dsvocoder\n",
    );
    write(&dir, "phonemes.txt", "AP\nSP\nah\n");
    // Duration: complete, one speaker for every mode.
    write(
        &dir,
        "dsdur/dsconfig.yaml",
        "phonemes: ph.json\nlanguages: lang.json\nuse_lang_id: true\nlinguistic: l.onnx\n\
         dur: d.onnx\nspeakers:\n- embeds/standard\n",
    );
    write(&dir, "dsdur/ph.json", r#"{"AP": 1, "SP": 2, "en/ah": 7}"#);
    write(&dir, "dsdur/lang.json", r#"{"en": 3}"#);
    write(&dir, "dsdur/l.onnx", "");
    write(&dir, "dsdur/d.onnx", "");
    // Pitch: its model file is missing, so it does not count.
    write(
        &dir,
        "dspitch/dsconfig.yaml",
        "phonemes: ph.txt\nlinguistic: l.onnx\npitch: p.onnx\n",
    );
    write(&dir, "dspitch/ph.txt", "AP\nSP\n");
    write(&dir, "dspitch/l.onnx", "");
    // Variance: complete, two speakers.
    write(
        &dir,
        "dsvariance/dsconfig.yaml",
        "phonemes: ph.txt\nlinguistic: l.onnx\nvariance: v.onnx\nhop_size: 256\n\
         sample_rate: 24000\nspeakers:\n- embeds/standard\n- embeds/phantom\n",
    );
    write(&dir, "dsvariance/ph.txt", "AP\nSP\n");
    write(&dir, "dsvariance/l.onnx", "");
    write(&dir, "dsvariance/v.onnx", "");

    let acoustic_languages = [("ja".to_owned(), 1), ("en".to_owned(), 2)].into_iter().collect();
    let found = discover(&dir.join("dsconfig.yaml"), &acoustic_languages);
    assert_eq!(found.duration, Some(dir.join("dsdur/dsconfig.yaml")));
    assert_eq!(found.pitch, None, "pitch model file missing");
    assert_eq!(found.variance, Some(dir.join("dsvariance/dsconfig.yaml")));
    // What discovery read of each: the phonemes, the acoustic language
    // ids mapped through their names onto the model's (`en` is 2 to the
    // acoustic model, 3 to the duration model; `ja` it does not list).
    let dur_facts = found.facts(ModelKind::Duration).unwrap();
    assert!(dur_facts.phonemes.contains("en/ah") && !dur_facts.phonemes.contains("ah"));
    assert_eq!(dur_facts.language_map, [(2, 3)].into_iter().collect());
    assert_eq!(dur_facts.language(2), 3);
    assert_eq!(dur_facts.language(1), 1, "an unlisted id goes through");
    assert!(found.facts(ModelKind::Variance).unwrap().language_map.is_empty());
    assert!(found.facts(ModelKind::Pitch).is_none());

    let dur = ModelConfig::load(found.duration.as_deref().unwrap(), ModelKind::Duration).unwrap();
    assert_eq!(dur.phonemes.get("en/ah"), Some(&7));
    assert_eq!(dur.languages.get("en"), Some(&3));
    assert!(dur.use_lang_id);
    assert_eq!(dur.speaker_for(Some("embeds/phantom")), Some("embeds/standard"));
    let var = ModelConfig::load(found.variance.as_deref().unwrap(), ModelKind::Variance).unwrap();
    assert_eq!(var.speaker_for(Some("embeds/phantom")), Some("embeds/phantom"));
    assert_eq!(var.speaker_for(Some("dsacoustic/phantom")), Some("embeds/phantom"));
    assert_eq!(var.speaker_for(None), Some("embeds/standard"));
    assert!((var.frame_sec() - 256.0 / 24000.0).abs() < 1e-12);

    // A bank without the folders has none.
    let bare = fresh_dir("bank_models_none");
    write(&bare, "dsconfig.yaml", "phonemes: p.txt\nacoustic: a.onnx\n");
    assert_eq!(discover(&bare.join("dsconfig.yaml"), &Default::default()), Default::default());
}

/// A render unit uses a model only when the model knows every phoneme it
/// sings; the variance model goes with the pitch model. Replacing a
/// model's file moves its digest.
#[test]
fn a_unit_uses_only_the_bank_models_that_know_its_phonemes() {
    use resonance_svs::predict::{discover, ModelKind};
    let dir = fresh_dir("bank_models_cover");
    write(&dir, "dsconfig.yaml", "phonemes: phonemes.txt\nacoustic: acoustic.onnx\n");
    write(&dir, "phonemes.txt", "AP\nSP\nah\nio\n");
    for (folder, key, phonemes) in [
        ("dsdur", "dur", "AP\nSP\nah\nio\n"),
        ("dspitch", "pitch", "AP\nSP\nah\n"),
        ("dsvariance", "variance", "AP\nSP\nah\nio\n"),
    ] {
        write(
            &dir,
            &format!("{folder}/dsconfig.yaml"),
            &format!("phonemes: ph.txt\nlinguistic: l.onnx\n{key}: m.onnx\n"),
        );
        write(&dir, &format!("{folder}/ph.txt"), phonemes);
        write(&dir, &format!("{folder}/l.onnx"), "");
        write(&dir, &format!("{folder}/m.onnx"), "");
    }
    let all = discover(&dir.join("dsconfig.yaml"), &Default::default());
    assert_eq!(all.covering(["SP", "ah", "SP"]), all, "every model knows these");
    let without_pitch = all.covering(["SP", "io", "SP"]);
    assert!(without_pitch.duration.is_some(), "the duration model knows io");
    assert_eq!(without_pitch.pitch, None, "the pitch model does not");
    assert_eq!(without_pitch.variance, None, "nor does the variance model run without it");
    assert!(without_pitch.facts(ModelKind::Pitch).is_none());
    assert!(!all.covers(ModelKind::Pitch, ["io"]));

    let before = all.facts(ModelKind::Pitch).unwrap().files_digest;
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(&dir, "dspitch/m.onnx", "new weights");
    let after = discover(&dir.join("dsconfig.yaml"), &Default::default());
    assert_ne!(after.facts(ModelKind::Pitch).unwrap().files_digest, before);
    assert_eq!(
        after.facts(ModelKind::Duration).unwrap().files_digest,
        all.facts(ModelKind::Duration).unwrap().files_digest
    );
}
