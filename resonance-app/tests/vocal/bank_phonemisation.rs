//! Any-bank phonemisation (vocal-mcp-control-and-quality.md §4.4,
//! slice 5): the bank's own English dictionary sits between the user's
//! dictionaries and CMU, and whatever the G2P produces is mapped onto the
//! bank's inventory — language prefix, substitution hints, then the
//! nearest phone of the same articulatory class. A word that still cannot
//! be sung is an error naming it.

use std::collections::HashMap;
use std::path::PathBuf;

use resonance_app::compose::vocal_svs::{
    resolve_clip_with_bank, validate_for_voicebank, BankProfile, DictionaryEntry, DictionaryScope,
    InvalidPhonemeReason, VoicebankRegistry, VoicebankRoots,
};
use resonance_app::{write_voicebank_fixture, VoicebankFixture};
use resonance_music_theory::derive::LyricLine;
use resonance_music_theory::g2p::{AssignedSyllable, G2pSource, PhonemeProvenance};
use resonance_music_theory::VoicebankId;

/// A registry holding one bank `id` written from `spec`.
fn bank(id: &str, spec: VoicebankFixture) -> BankProfile {
    let root: PathBuf = std::env::temp_dir()
        .join(format!("resonance-bank-g2p-{}", std::process::id()))
        .join(id);
    let _ = std::fs::remove_dir_all(&root);
    write_voicebank_fixture(&root.join(id), &spec);
    let reg = VoicebankRegistry::scan(VoicebankRoots {
        banks: Some(root),
        ..VoicebankRoots::default()
    });
    let profile = reg.profile(&VoicebankId::new(id));
    assert!(profile.is_installed(), "fixture {id} scans");
    profile
}

fn entry(word: &str, phonemes: &[&str]) -> (String, Vec<String>) {
    (
        word.into(),
        phonemes.iter().map(|p| p.to_string()).collect(),
    )
}

fn resolve(
    text: &str,
    notes: usize,
    bank: &BankProfile,
    project: &[DictionaryEntry],
) -> Vec<AssignedSyllable> {
    let draft = vec![LyricLine {
        n: 1,
        rhyme: 'A',
        syllables: notes as u8,
        text: text.into(),
        locked: false,
    }];
    let dict = bank.dictionary();
    resolve_clip_with_bank(
        &draft,
        &[],
        notes,
        &HashMap::new(),
        project,
        &[],
        dict.as_deref(),
    )
}

#[test]
fn the_banks_dictionary_beats_cmu_and_reports_bank_dict() {
    let profile = bank(
        "dicted",
        VoicebankFixture {
            english_dict: vec![entry("tomato", &["t", "ah", "m", "aa", "t", "ow"])],
            ..VoicebankFixture::default()
        },
    );
    let got = resolve("to·ma·to", 3, &profile, &[]);
    let phonemes: Vec<&str> = got
        .iter()
        .flat_map(|s| s.phonemes.iter().copied())
        .collect();
    assert_eq!(
        phonemes,
        ["t", "ah", "m", "aa", "t", "ow"],
        "the bank's reading, not CMU's ey"
    );
    assert!(
        got.iter().all(|s| s.source == G2pSource::BankDict),
        "{got:?}"
    );
    assert!(got.iter().all(|s| s.provenance == PhonemeProvenance::Auto));

    // A word the bank does not list still goes to CMU.
    let cmu = resolve("hel·lo", 2, &profile, &[]);
    assert!(cmu.iter().all(|s| s.source == G2pSource::Cmu), "{cmu:?}");
}

#[test]
fn the_project_dictionary_beats_the_banks() {
    let profile = bank(
        "dicted2",
        VoicebankFixture {
            english_dict: vec![entry("tomato", &["t", "ah", "m", "aa", "t", "ow"])],
            ..VoicebankFixture::default()
        },
    );
    let project = [DictionaryEntry::new(
        "tomato",
        &["t", "ah", "m", "ey", "t", "ow"],
        DictionaryScope::Project,
    )];
    let got = resolve("to·ma·to", 3, &profile, &project);
    assert_eq!(got[1].phonemes, ["m", "ey"]);
    assert!(got.iter().all(|s| s.source == G2pSource::ProjectDict));
}

#[test]
fn bank_entries_outside_arpabet_fall_through_to_cmu() {
    // Lilia's dictionary writes some words with its own multi-language
    // symbols (`want` → `L aa n`); those are not ARPAbet, so CMU sings
    // the word instead. An `en/` prefix is stripped where the bank
    // namespaces English.
    let profile = bank(
        "mixed",
        VoicebankFixture {
            english_dict: vec![
                entry("want", &["L", "aa", "n"]),
                entry("it", &["ih", "dx"]),
                entry("br", &["AP"]),
            ],
            ..VoicebankFixture::default()
        },
    );
    let dict = profile.dictionary().expect("the dictionary loads");
    assert!(dict.is_empty(), "every entry was unusable: {dict:?}");
    let got = resolve("want", 1, &profile, &[]);
    assert_eq!(got[0].source, G2pSource::Cmu);

    let mut namespaced = VoicebankFixture::shipped("meiji");
    namespaced.english_dict = vec![entry("go", &["en/g", "en/ow"]), entry("ja", &["ja/a"])];
    let meiji = bank("meijiish", namespaced);
    let dict = meiji.dictionary().unwrap();
    assert_eq!(dict.get("go").map(Vec::as_slice), Some(&["g", "ow"][..]));
    assert!(
        !dict.contains_key("ja"),
        "another language's entry is dropped"
    );
}

#[test]
fn missing_phonemes_map_onto_the_bank_by_hint_partner_and_class() {
    // No `v`, `zh` or `sh`: `v` takes its voiceless partner `f`; `zh`'s
    // partner `sh` is missing too, so the nearest fricative `z` stands in.
    let profile = bank(
        "gappy",
        VoicebankFixture::default().without(&["v", "zh", "sh"]),
    );
    let resolved = resolve("vi·sion", 2, &profile, &[]);
    let sung = validate_for_voicebank(&resolved, &profile).expect("everything maps");
    let phonemes: Vec<&str> = sung
        .iter()
        .flat_map(|s| s.phonemes.iter().copied())
        .collect();
    assert!(
        !phonemes.contains(&"v") && !phonemes.contains(&"zh"),
        "{phonemes:?}"
    );
    assert!(
        phonemes.contains(&"f") && phonemes.contains(&"z"),
        "{phonemes:?}"
    );
    // The segment spells what the bank stores: bare here.
    assert_eq!(profile.phoneme_name("f"), "f");
}

#[test]
fn a_word_the_bank_cannot_sing_is_an_error_naming_the_word() {
    // No liquids or glides at all: `l` has nothing near enough.
    let profile = bank(
        "noliquids",
        VoicebankFixture::default().without(&["l", "r", "w", "y"]),
    );
    let resolved = resolve("so lit·tle", 3, &profile, &[]);
    let invalid = validate_for_voicebank(&resolved, &profile).expect_err("`l` cannot be sung");
    assert!(invalid
        .iter()
        .all(|i| i.reason == InvalidPhonemeReason::UnsupportedByVoicebank));
    assert!(invalid.iter().all(|i| i.phoneme == "l"));
    assert!(invalid.iter().all(|i| i.word == "little"), "{invalid:?}");
    let report =
        resonance_app::update::compose::vocal_render_plan::describe_invalid(&invalid, &profile);
    assert!(
        report.contains("little") && report.contains("noliquids"),
        "{report}"
    );
}
