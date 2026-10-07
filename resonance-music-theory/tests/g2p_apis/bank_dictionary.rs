//! The voicebank dictionary layer of the resolver
//! (vocal-mcp-control-and-quality.md §4.4): `[..]` block > project
//! dictionary > bank dictionary > CMU > rules.

use resonance_music_theory::derive::LyricLine;
use resonance_music_theory::g2p::{
    resolve_draft_with_layers, DictionaryLayers, G2pSource, PhonemeDictionary, PhonemeProvenance,
};

fn draft(text: &str) -> Vec<LyricLine> {
    vec![LyricLine {
        n: 1,
        rhyme: 'A',
        syllables: 1,
        text: text.into(),
        locked: false,
    }]
}

fn dict(entries: &[(&str, &[&'static str])]) -> PhonemeDictionary {
    entries
        .iter()
        .map(|(w, p)| (w.to_string(), p.to_vec()))
        .collect()
}

#[test]
fn a_bank_entry_beats_cmu_and_reports_bank_dict() {
    let bank = dict(&[("the", &["dh", "ax"])]);
    let layers = DictionaryLayers {
        project: &PhonemeDictionary::new(),
        bank: Some(&bank),
    };
    let got = resolve_draft_with_layers(&draft("the sun"), layers);
    assert_eq!(got[0].phonemes, ["dh", "ax"]);
    assert_eq!(got[0].source, G2pSource::BankDict);
    assert_eq!(
        got[0].provenance,
        PhonemeProvenance::Auto,
        "not a user edit"
    );
    assert_eq!(
        got[1].source,
        G2pSource::Cmu,
        "unlisted words still go to CMU"
    );
}

#[test]
fn project_and_inline_blocks_beat_the_bank() {
    let bank = dict(&[("sun", &["s", "aa", "n"])]);
    let project = dict(&[("sun", &["s", "uw", "n"])]);
    let layers = DictionaryLayers {
        project: &project,
        bank: Some(&bank),
    };
    let got = resolve_draft_with_layers(&draft("sun [s ih n]"), layers);
    assert_eq!(got[0].phonemes, ["s", "uw", "n"]);
    assert_eq!(got[0].source, G2pSource::ProjectDict);
    assert_eq!(got[1].source, G2pSource::Edited);
}

#[test]
fn a_variant_hint_or_mismatched_breaks_skip_the_bank_entry() {
    // `(2)` asks for a CMU reading; a bank entry with one vowel cannot
    // carry a word written in two pieces.
    let bank = dict(&[
        ("read", &["r", "ah", "d"]),
        ("every", &["eh", "v", "r", "iy"]),
    ]);
    let layers = DictionaryLayers {
        project: &PhonemeDictionary::new(),
        bank: Some(&bank),
    };
    let got = resolve_draft_with_layers(&draft("read(2)"), layers);
    assert_eq!(got[0].source, G2pSource::Cmu);
    let got = resolve_draft_with_layers(&draft("ev·er·y"), layers);
    assert!(got.iter().all(|s| s.source == G2pSource::Cmu), "{got:?}");
    // The two-piece spelling the entry does fit uses it.
    let got = resolve_draft_with_layers(&draft("ev·ery"), layers);
    assert!(
        got.iter().all(|s| s.source == G2pSource::BankDict),
        "{got:?}"
    );
}
