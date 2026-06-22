//! Voicebank validation gate for the resolved pronunciation (todo #494).
//!
//! After the #493 resolver folds overrides + dictionaries + CMU-auto into
//! a per-note [`AssignedSyllable`] stream, every phoneme must clear two
//! gates before it reaches the SVS model:
//!
//! 1. **Valid ARPAbet** (#491): the symbol must canonicalise through
//!    [`g2p::canonical_phoneme`]. The upstream layers already canonicalise
//!    their input, but this is the final guarantee that no garbage token
//!    reaches the model as a PAD / token-0 and silently corrupts the
//!    segment.
//! 2. **Singable by the active voicebank** (#492): the symbol is checked
//!    against [`VoicebankPhonemes`]. One the bank covers is kept as-is;
//!    one it lacks but can substitute (e.g. Lilia `v` → `f`) is rewritten
//!    to the substitute; one with no acceptable substitute blocks the
//!    render.
//!
//! On success the returned syllables carry the *effective* (substituted)
//! phonemes — exactly what the segment builder feeds the model — with
//! every other field (label, slur, stress, [`PhonemeProvenance`]) carried
//! through, so downstream re-render scoping and the phoneme strip keep the
//! provenance / affected state. On failure the offending syllables are
//! reported rather than corrupting the segment.
//!
//! [`PhonemeProvenance`]: super::PhonemeProvenance

use resonance_music_theory::g2p::{self, AssignedSyllable};
use resonance_music_theory::VocalVoicebank;

use super::phonemes::{PhonemeFate, VoicebankPhonemes};

/// Why one phoneme failed the voicebank gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidPhonemeReason {
    /// Not a valid ARPAbet symbol (#491) — [`g2p::canonical_phoneme`]
    /// rejected it, so it would reach the model as an unknown token.
    NotArpabet,
    /// Valid ARPAbet, but the active voicebank can neither sing nor
    /// substitute it (#492 — [`PhonemeFate::Unsupported`]).
    UnsupportedByVoicebank,
}

impl InvalidPhonemeReason {
    /// A short human-readable tag for error messages.
    pub fn as_str(&self) -> &'static str {
        match self {
            InvalidPhonemeReason::NotArpabet => "not ARPAbet",
            InvalidPhonemeReason::UnsupportedByVoicebank => "unsupported by voicebank",
        }
    }
}

/// One syllable that blocks the render, with the offending phoneme and
/// why. `note_index` is the position in the clip's note list (so the UI
/// can badge the right note); `label` is the syllable's surface glyphs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidSyllable {
    /// Index of the offending note in the clip's note list.
    pub note_index: usize,
    /// The syllable's surface label (glyphs to display).
    pub label: String,
    /// The exact symbol that failed the gate.
    pub phoneme: String,
    /// Whether it failed the ARPAbet or the voicebank gate.
    pub reason: InvalidPhonemeReason,
}

/// Validate and substitute every phoneme in the resolved per-note stream
/// against `voicebank`. Returns the syllables with their *effective*
/// (substituted) phonemes on success — every other field carried through
/// unchanged so provenance survives — or every blocking syllable on
/// failure.
///
/// All invalid phonemes across all syllables are collected before
/// returning, so the caller can report every problem at once rather than
/// one-per-render-press. Empty phoneme lists (e.g. a note the draft never
/// reached) pass through untouched; the segment builder handles them with
/// its own vowel fallback.
pub fn validate_for_voicebank(
    assigned: &[AssignedSyllable],
    voicebank: VocalVoicebank,
) -> Result<Vec<AssignedSyllable>, Vec<InvalidSyllable>> {
    let bank = VoicebankPhonemes::new(voicebank);
    let mut out = Vec::with_capacity(assigned.len());
    let mut invalid = Vec::new();

    for (note_index, syl) in assigned.iter().enumerate() {
        let mut effective = Vec::with_capacity(syl.phonemes.len());
        for &ph in &syl.phonemes {
            // Gate 1: must be a real ARPAbet symbol. `canonical_phoneme`
            // also accepts the `AP`/`SP` silence markers, but those are
            // never part of a syllable's phoneme list (the builder inserts
            // them separately), so a hit here means a genuine phone.
            if g2p::canonical_phoneme(ph).is_none() {
                invalid.push(InvalidSyllable {
                    note_index,
                    label: syl.label.clone(),
                    phoneme: ph.to_string(),
                    reason: InvalidPhonemeReason::NotArpabet,
                });
                continue;
            }
            // Gate 2: the active voicebank must sing it directly or have a
            // substitute. `Substituted` rewrites to the singable form;
            // `Unsupported` blocks.
            match bank.resolve(ph) {
                PhonemeFate::Direct => effective.push(ph),
                PhonemeFate::Substituted(sub) => effective.push(sub),
                PhonemeFate::Unsupported => invalid.push(InvalidSyllable {
                    note_index,
                    label: syl.label.clone(),
                    phoneme: ph.to_string(),
                    reason: InvalidPhonemeReason::UnsupportedByVoicebank,
                }),
            }
        }
        let mut s = syl.clone();
        s.phonemes = effective;
        out.push(s);
    }

    if invalid.is_empty() {
        Ok(out)
    } else {
        Err(invalid)
    }
}
