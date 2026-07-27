//! Draft resolution and per-note syllable assignment.
//!
//! Converts a `&[LyricLine]` into a resolved syllable list and maps
//! that list onto individual notes, honouring slur annotations and
//! per-syllable phoneme overrides.

use std::collections::HashMap;

use super::lyric_parse::{is_slur_lyric, tokenize_line, LyricToken, SLUR_MARKER};
use super::phonemes::{is_consonant, word_to_phonemes_variant, SyllableStress};
use super::syllabify::{split_into_syllables, syllabify_word};

/// Where a syllable's phonemes came from, so the UI can badge edited /
/// dictionary syllables and downstream code can reason about how much to
/// trust the transcription. The resolution precedence is
/// `Edited` > `Dict` > `Auto`: a per-syllable override (or an inline
/// `[..]` block) beats a caller-supplied dictionary hit, which beats the
/// CMU / rule-based auto transcription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PhonemeProvenance {
    /// CMU dictionary or rule-based fallback — the default path.
    #[default]
    Auto,
    /// A caller-supplied word→phonemes dictionary entry replaced CMU.
    Dict,
    /// A per-syllable phoneme override: an inline `[..]` lyric block or
    /// a caller-supplied per-syllable edit.
    Edited,
}

/// A caller-supplied pronunciation dictionary: cleaned lowercase word →
/// the flat phoneme list to sing for the *whole* word, overriding CMU.
/// Build the phoneme vec with [`canonical_phoneme`] so every symbol is a
/// valid `&'static str` the pipeline recognises. List phonemes flat (no
/// `·`); the resolver re-splits them across the word's syllable count
/// exactly like the CMU path. Dictionary phonemes carry no stress.
pub type PhonemeDictionary = HashMap<String, Vec<&'static str>>;

/// A caller-supplied per-syllable phoneme override, keyed by the
/// resolved-syllable index (the `syllable_index` an [`AssignedSyllable`]
/// reports). Highest precedence — replaces whatever the resolver picked
/// for that syllable. Build values with [`canonical_phoneme`].
pub type SyllableOverrides = HashMap<usize, Vec<&'static str>>;

/// One syllable resolved against the lyric draft. Carries the surface
/// label (the glyphs you'd write on a score), the phoneme list the
/// SVS model will sing, a `is_word_end` flag that drives SP injection
/// between words, the syllable's lexical stress (drawn from CMU's
/// stress marks on its vowel), and the phonemes' [`PhonemeProvenance`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSyllable {
    pub label: String,
    pub phonemes: Vec<&'static str>,
    pub is_word_end: bool,
    pub stress: SyllableStress,
    pub provenance: PhonemeProvenance,
}

/// One note's assignment after the lyric side-table annotations have
/// been applied to the resolved draft. The cursor in
/// [`assign_syllables_to_notes`] produces a `Vec<AssignedSyllable>`
/// of exactly `note_count` entries — the single source of truth
/// shared between the vocal roll (lyrics on notes + phoneme strip)
/// and the SVS pipeline (`build_segment`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignedSyllable {
    /// Glyphs to draw on the note body or in the phoneme strip's
    /// label column. For slurs this is `"+"`. For overrides it's the
    /// user-typed string; otherwise the resolved-syllable surface.
    pub label: String,
    /// Phoneme list the SVS pipeline sings for this note. For slurs
    /// this is the held vowel from the previous syllable (single
    /// element vec).
    pub phonemes: Vec<&'static str>,
    pub is_slur: bool,
    /// `true` when the underlying resolved syllable was the last in
    /// its word *and* this note is non-slur. Slur notes never sit on
    /// a word boundary by definition.
    pub is_word_end: bool,
    /// Which resolved-syllable index this note maps to. Slur notes
    /// inherit the previous non-slur note's index. Out-of-range when
    /// the draft has fewer syllables than non-slur notes.
    pub syllable_index: usize,
    /// Lexical stress for this syllable, drawn from CMU. Slur notes
    /// inherit the held syllable's stress. Drives the SVS pipeline's
    /// per-syllable velocity / tension bump and the stress overlay
    /// in the vocal roll.
    pub stress: SyllableStress,
    /// Where this note's phonemes came from. Slur notes inherit the
    /// held syllable's provenance. See [`PhonemeProvenance`].
    pub provenance: PhonemeProvenance,
}

/// Resolve a draft into one phoneme list per syllable. For each
/// syllable in the draft we look up the *whole word* it belongs to in
/// CMU, then slice the resulting phoneme stream across the word's
/// syllables. This matches how the SVS model expects phonemes to land
/// on note boundaries when one word spans multiple notes (e.g.
/// `hou·ses` → note 1 gets `[hh aw z]`, note 2 gets `[ah z]`).
///
/// Power-user escape hatch: `[hh ah l ow]` in the lyric is taken
/// verbatim as phonemes for one syllable, bypassing CMU. Use
/// `[l ih · l iy · ah]` (or `[l ih]·[l iy]·[ah]`) for multi-syllable
/// overrides. Helpful for proper nouns and foreign-language words
/// where the CMU dict or rule-based fallback misfire.
///
/// Returns one `Vec<&str>` per output syllable. Use `resolve_draft`
/// when you also need stress / surface-label information.
pub fn phonemes_for_draft(draft: &[crate::derive::LyricLine]) -> Vec<Vec<&'static str>> {
    resolve_draft(draft)
        .into_iter()
        .map(|s| s.phonemes)
        .collect()
}

/// Resolve every syllable in a lyric draft to its (surface, phonemes,
/// word-end) tuple. One pass through `tokenize_line` — guarantees the
/// surface labels and phoneme groups stay in lockstep, so a per-
/// syllable assertion like `labels.len() == phonemes.len()` becomes a
/// property of the type rather than a discipline.
///
/// Phoneme-block tokens (`[hh ah]` overrides) get their label set to
/// the bracketed phoneme list — the user explicitly typed phonemes,
/// not glyphs, so that's the most faithful surface to display.
///
/// Equivalent to [`resolve_draft_with_dict`] with an empty dictionary.
pub fn resolve_draft(draft: &[crate::derive::LyricLine]) -> Vec<ResolvedSyllable> {
    resolve_draft_with_dict(draft, &PhonemeDictionary::new())
}

/// Like [`resolve_draft`], but a caller-supplied [`PhonemeDictionary`]
/// takes precedence over the CMU / rule-based transcription for any word
/// it contains (matched on the cleaned, lowercased word). Dictionary
/// phonemes are re-split across the word's syllable count just like CMU
/// output and reported with [`PhonemeProvenance::Dict`]; words absent
/// from the dictionary resolve exactly as before (`Auto`). Inline
/// `[..]` blocks always win and report `Edited`.
pub fn resolve_draft_with_dict(
    draft: &[crate::derive::LyricLine],
    dictionary: &PhonemeDictionary,
) -> Vec<ResolvedSyllable> {
    let mut tokens: Vec<LyricToken> = Vec::new();
    for line in draft {
        tokens.extend(tokenize_line(&line.text));
    }
    let mut out: Vec<ResolvedSyllable> = Vec::new();
    for token in tokens {
        match token {
            LyricToken::PhonemeBlock(groups) => {
                let last_idx = groups.len().saturating_sub(1);
                for (i, g) in groups.into_iter().enumerate() {
                    out.push(ResolvedSyllable {
                        label: format!("[{}]", g.join(" ")),
                        phonemes: g,
                        is_word_end: i == last_idx,
                        // Bracket overrides carry no stress info.
                        stress: SyllableStress::None,
                        // An inline block is the user typing phonemes
                        // directly — the highest-precedence source.
                        provenance: PhonemeProvenance::Edited,
                    });
                }
            }
            LyricToken::Word { cleaned, syl_count, variant_idx } => {
                let (phonemes, provenance) = match dictionary.get(&cleaned) {
                    Some(dict_phonemes) => (
                        dict_phonemes
                            .iter()
                            .map(|p| (*p, SyllableStress::None))
                            .collect::<Vec<_>>(),
                        PhonemeProvenance::Dict,
                    ),
                    None => (
                        word_to_phonemes_variant(&cleaned, variant_idx),
                        PhonemeProvenance::Auto,
                    ),
                };
                let phoneme_groups: Vec<Vec<(&'static str, SyllableStress)>> = if syl_count <= 1 {
                    vec![phonemes]
                } else {
                    split_into_syllables(&phonemes, syl_count)
                };
                // Surface labels via `syllabify_word`, which inserts
                // `·` markers using the same CMU syllable count the
                // phoneme split uses. Falling back to the cleaned word
                // when syllabify-word can't reach the target keeps the
                // two slices balanced.
                let with_dots = syllabify_word(&cleaned, syl_count);
                let labels: Vec<String> = with_dots
                    .split('\u{00B7}')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let n = phoneme_groups.len();
                let last_idx = n.saturating_sub(1);
                for i in 0..n {
                    let label = labels.get(i).cloned().unwrap_or_default();
                    let group = phoneme_groups.get(i).cloned().unwrap_or_default();
                    let stress = group
                        .iter()
                        .filter(|(p, _)| !is_consonant(p))
                        .map(|(_, s)| *s)
                        .max()
                        .unwrap_or(SyllableStress::None);
                    let phonemes: Vec<&'static str> = group.into_iter().map(|(p, _)| p).collect();
                    out.push(ResolvedSyllable {
                        label,
                        phonemes,
                        is_word_end: i == last_idx,
                        stress,
                        provenance,
                    });
                }
            }
        }
    }
    out
}

/// Map a per-note annotation vec to per-note `AssignedSyllable`s,
/// walking a cursor through `syllables` and skipping it on slur
/// notes. The single source of truth for the cursor model — every
/// view + the SVS pipeline use this.
///
/// `annotations[i]` is interpreted as:
///
/// * `""` (empty)  — consume the next resolved syllable.
/// * `"+"` / `"-"` — slur: inherit the previous note's vowel, no
///   cursor advance.
/// * anything else — explicit label override (cursor still advances;
///   phonemes still come from the resolved syllable).
///
/// Returns exactly `note_count` entries.
///
/// Equivalent to [`assign_syllables_to_notes_with`] with no per-syllable
/// overrides.
pub fn assign_syllables_to_notes(
    syllables: &[ResolvedSyllable],
    annotations: &[String],
    note_count: usize,
) -> Vec<AssignedSyllable> {
    assign_syllables_to_notes_with(syllables, annotations, note_count, &SyllableOverrides::new())
}

/// Like [`assign_syllables_to_notes`], but applies caller-supplied
/// per-syllable phoneme [`SyllableOverrides`] — the highest-precedence
/// resolution layer. When a non-slur note resolves to a syllable whose
/// index is present in `overrides`, that note sings the override
/// phonemes and reports [`PhonemeProvenance::Edited`]; otherwise it
/// keeps the resolved syllable's phonemes and provenance (`Dict` if the
/// syllable came from a dictionary, else `Auto`). A following slur note
/// holds the (possibly overridden) vowel and inherits its provenance.
///
/// With an empty `overrides` map this is byte-for-byte identical to the
/// pre-override behaviour, so existing call sites are unaffected.
pub fn assign_syllables_to_notes_with(
    syllables: &[ResolvedSyllable],
    annotations: &[String],
    note_count: usize,
    overrides: &SyllableOverrides,
) -> Vec<AssignedSyllable> {
    let mut out: Vec<AssignedSyllable> = Vec::with_capacity(note_count);
    let mut cursor: usize = 0;
    let mut last_syllable_idx: usize = 0;
    let mut last_vowel: Option<&'static str> = None;
    let mut last_stress: SyllableStress = SyllableStress::None;
    let mut last_provenance: PhonemeProvenance = PhonemeProvenance::Auto;
    for i in 0..note_count {
        let entry = annotations.get(i).map(|s| s.trim()).unwrap_or("");
        if is_slur_lyric(entry) {
            let phonemes: Vec<&'static str> =
                last_vowel.map(|v| vec![v]).unwrap_or_default();
            out.push(AssignedSyllable {
                label: SLUR_MARKER.to_string(),
                phonemes,
                is_slur: true,
                is_word_end: false,
                syllable_index: last_syllable_idx,
                stress: last_stress,
                provenance: last_provenance,
            });
            continue;
        }
        let syl_opt = syllables.get(cursor);
        let syl_index = cursor;
        cursor += 1;
        let Some(syl) = syl_opt else {
            out.push(AssignedSyllable {
                label: String::new(),
                phonemes: Vec::new(),
                is_slur: false,
                is_word_end: false,
                syllable_index: syl_index,
                stress: SyllableStress::None,
                provenance: PhonemeProvenance::Auto,
            });
            continue;
        };
        // Override > dictionary > CMU-auto: a per-syllable override (keyed
        // by resolved-syllable index) replaces the phonemes and stamps
        // `Edited`; otherwise we keep the syllable's own phonemes and
        // provenance.
        let (phonemes, provenance) = match overrides.get(&syl_index) {
            Some(ov) => (ov.clone(), PhonemeProvenance::Edited),
            None => (syl.phonemes.clone(), syl.provenance),
        };
        // Cache the last non-consonant phoneme so a following slur
        // note can hold the vowel. Falls back to the final phoneme
        // when the syllable is all-consonant (rare; only happens for
        // pathological overrides). Reads the resolved phonemes (after
        // any override) so a slur holds the edited vowel.
        if let Some(v) = phonemes.iter().rev().find(|p| !is_consonant(p)) {
            last_vowel = Some(*v);
        } else if let Some(v) = phonemes.last() {
            last_vowel = Some(*v);
        }
        last_syllable_idx = syl_index;
        last_stress = syl.stress;
        last_provenance = provenance;
        let label = if !entry.is_empty() {
            entry.to_string()
        } else {
            syl.label.clone()
        };
        out.push(AssignedSyllable {
            label,
            phonemes,
            is_slur: false,
            is_word_end: syl.is_word_end,
            syllable_index: syl_index,
            stress: syl.stress,
            provenance,
        });
    }
    out
}
