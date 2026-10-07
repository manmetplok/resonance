//! A bank's English dictionary (`dsdict-en.yaml`) as a
//! [`PhonemeDictionary`] the G2P resolver can consult between the user's
//! dictionaries and CMU (vocal-mcp-control-and-quality.md §4.4).
//!
//! The resolver and everything after it work in canonical ARPAbet, so an
//! entry is kept only when every phoneme, with the bank's `en/` prefix
//! stripped, is an ARPAbet phone (`ax` passes, `dx` and the
//! multi-language extras like `L` or `d0` do not). Entries that spell
//! silence (`br` → `AP`), have no vowel, or are not English letters are
//! skipped too. A skipped word falls through to CMU, which is what it
//! sang before this layer existed. The first entry for a spelling wins.

use std::path::Path;

use resonance_music_theory::g2p::{self, PhonemeDictionary};
use resonance_svs::voicebank::dsdict;

use crate::compose::vocal_svs::clean_word;

/// Read `path` into a dictionary keyed by cleaned, lowercased word.
/// `prefix` is the bank's English namespace (`en/`), stripped from each
/// phoneme; a phoneme in another language's namespace drops the entry.
pub(super) fn load_english_dictionary(
    path: &Path,
    prefix: Option<&str>,
) -> Result<PhonemeDictionary, String> {
    let entries = dsdict::load_dsdict(path).map_err(|e| format!("{e:#}"))?;
    let mut map = PhonemeDictionary::with_capacity(entries.len());
    for entry in entries {
        if !entry
            .grapheme
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '\'')
        {
            continue;
        }
        let word = clean_word(&entry.grapheme);
        if word.is_empty() || map.contains_key(&word) {
            continue;
        }
        if let Some(phonemes) = english_phonemes(&entry.phonemes, prefix) {
            map.insert(word, phonemes);
        }
    }
    Ok(map)
}

/// The entry's phonemes as canonical lexical ARPAbet, or `None` when any
/// of them is not one.
fn english_phonemes(phonemes: &[String], prefix: Option<&str>) -> Option<Vec<&'static str>> {
    let mut out = Vec::with_capacity(phonemes.len());
    for ph in phonemes {
        let bare = match (prefix, ph.split_once('/')) {
            (Some(p), Some(_)) => ph.strip_prefix(p)?,
            (_, Some(_)) => return None,
            (_, None) => ph.as_str(),
        };
        // Exact spelling only: `canonical_phoneme` folds case, and a
        // multi-language bank's `L` is not ARPAbet `l`.
        let canonical = g2p::canonical_phoneme(bare).filter(|c| *c == bare)?;
        if matches!(canonical, "AP" | "SP") {
            return None;
        }
        out.push(canonical);
    }
    out.iter().any(|p| !g2p::is_consonant(p)).then_some(out)
}
