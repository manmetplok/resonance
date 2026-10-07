//! Draft resolution and per-note syllable assignment.
//!
//! Converts a `&[LyricLine]` into a resolved syllable list and maps
//! that list onto individual notes, honouring slur annotations and
//! per-syllable phoneme overrides.
//!
//! Two draft markers take part in the note walk without being sung
//! syllables themselves:
//!
//! * a standalone `_` (alias `+`) is a **melisma**: it does not
//!   become a [`ResolvedSyllable`] but bumps the previous syllable's
//!   [`ResolvedSyllable::slurs_after`], and the note walk turns each one
//!   into a slur note holding that syllable's vowel. Keeping holds off
//!   the syllable list means typing a `_` never shifts the
//!   `syllable_index` of later syllables (which pronunciation overrides
//!   are keyed by).
//! * a standalone uppercase `AP` is a **breath**: it takes no note and
//!   sets [`ResolvedSyllable::breath_before`] on the next sung syllable.

use std::collections::HashMap;

use super::lyric_parse::{is_slur_lyric, tokenize_line, LyricToken, SLUR_MARKER};
use super::phonemes::{
    cmu_variant_count, cmu_variants, is_consonant, word_g2p_source, word_to_phonemes_sourced,
    G2pSource, SyllableStress,
};
use super::syllabify::{split_at_user_breaks, split_into_syllables};

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
    /// Finer-grained source of the phonemes (`cmu` vs `rules`, project
    /// vs bank dictionary). See [`G2pSource`].
    pub source: G2pSource,
    /// How many `_` melisma tokens the draft writes straight after this
    /// syllable: each one is an extra note that holds this syllable's
    /// vowel. A syllable therefore consumes `1 + slurs_after` notes —
    /// see [`note_unit_count`].
    pub slurs_after: usize,
    /// The draft has a breath (`AP`) right before this syllable.
    pub breath_before: bool,
    /// The user broke this syllable's word with `·`, but the number of
    /// pieces did not match the word's vowel count, so the phonemes were
    /// split automatically and may not line up with the written pieces.
    pub syllable_warning: bool,
}

/// Notes a resolved draft fills: one per sung syllable plus one per `_`
/// hold. Breaths take none. This is what a clip's note count is compared
/// against; it agrees with summing
/// [`count_syllables`](crate::count_syllables) over the draft's lines
/// for well-formed text.
pub fn note_unit_count(syllables: &[ResolvedSyllable]) -> usize {
    syllables.iter().map(|s| 1 + s.slurs_after).sum()
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
    /// this is the held vowel from the previous syllable; the *last*
    /// slur of a hold also carries the syllable's final consonants,
    /// which leave the head note (`love _ _` → `l ah` · `ah` · `ah v`).
    pub phonemes: Vec<&'static str>,
    pub is_slur: bool,
    /// `true` on the note where the word ends: the note of the word's
    /// last syllable, or — when that syllable is held — the last slur
    /// note of the hold, which is where its final consonants are sung.
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
    /// Finer-grained source of this note's phonemes; slur notes inherit
    /// the held syllable's. See [`G2pSource`].
    pub source: G2pSource,
    /// The draft asks for a breath before this note (an `AP` token in
    /// front of its syllable). Only ever set on the syllable's first,
    /// non-slur note.
    pub breath_before: bool,
    /// Copied from [`ResolvedSyllable::syllable_warning`]; `false` on
    /// slur notes.
    pub syllable_warning: bool,
}

/// Resolve a draft into one phoneme list per syllable. For each
/// syllable in the draft we look up the *whole word* it belongs to in
/// CMU, then slice the resulting phoneme stream across the word's
/// syllables. This matches how the SVS model expects phonemes to land
/// on note boundaries when one word spans multiple notes (e.g.
/// `hou·ses` → note 1 gets `[hh aw]`, note 2 gets `[s ax z]`).
///
/// **A word with no `·` is one syllable and therefore one note**, so an
/// unbroken `resolution` puts all nine of its phonemes on a single note
/// and is sung as a smear. Ingest points normalise this away with
/// [`auto_syllabify_text`](super::auto_syllabify_text); this resolver
/// deliberately does not second-guess the breaks it is handed.
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
    resolve_draft_with_layers(
        draft,
        DictionaryLayers {
            project: dictionary,
            bank: None,
        },
    )
}

/// The caller-supplied dictionaries [`resolve_draft_with_layers`] reads,
/// highest precedence first. Both hold canonical ARPAbet.
#[derive(Debug, Clone, Copy)]
pub struct DictionaryLayers<'a> {
    /// The user's dictionaries (project over global), reported as
    /// [`G2pSource::ProjectDict`] / [`PhonemeProvenance::Dict`].
    pub project: &'a PhonemeDictionary,
    /// The voicebank's own English dictionary (its `dsdict-en.yaml`,
    /// mapped to ARPAbet), consulted after the user's and before CMU —
    /// the order OpenUtau uses (vocal-mcp-control-and-quality.md §4.4).
    /// Reported as [`G2pSource::BankDict`] with [`PhonemeProvenance::Auto`]:
    /// the user did not choose it.
    pub bank: Option<&'a PhonemeDictionary>,
}

/// [`resolve_draft_with_dict`] with the voicebank's dictionary as a third
/// layer: `[..]` block > project dictionary > bank dictionary > CMU >
/// spelling rules.
///
/// A bank entry is skipped for a word with a `(N)` CMU-variant hint (the
/// user asked for a CMU reading), and for a word whose `·` pieces do not
/// match the entry's vowel count (CMU's variants may fit the breaks; the
/// bank's single reading cannot be re-split exactly).
pub fn resolve_draft_with_layers(
    draft: &[crate::derive::LyricLine],
    layers: DictionaryLayers<'_>,
) -> Vec<ResolvedSyllable> {
    let dictionary = layers.project;
    let mut tokens: Vec<LyricToken> = Vec::new();
    for line in draft {
        tokens.extend(tokenize_line(&line.text));
    }
    let mut out: Vec<ResolvedSyllable> = Vec::new();
    // An `AP` waits here until the next sung syllable claims it; one at
    // the very end of the draft has nothing to precede and is dropped.
    let mut pending_breath = false;
    for token in tokens {
        match token {
            LyricToken::Breath => pending_breath = true,
            LyricToken::Slur => {
                // `tokenize_line` never emits a slur before a line's
                // first sung token, so `out` is non-empty here; the
                // guard only keeps a malformed stream from panicking.
                if let Some(prev) = out.last_mut() {
                    prev.slurs_after += 1;
                }
            }
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
                        source: G2pSource::Edited,
                        slurs_after: 0,
                        breath_before: std::mem::take(&mut pending_breath),
                        syllable_warning: false,
                    });
                }
            }
            LyricToken::Word { cleaned, pieces, variant_idx } => {
                let (phonemes, provenance, source) = match dictionary.get(&cleaned) {
                    Some(dict_phonemes) => (
                        dict_phonemes
                            .iter()
                            .map(|p| (*p, SyllableStress::None))
                            .collect::<Vec<_>>(),
                        PhonemeProvenance::Dict,
                        G2pSource::ProjectDict,
                    ),
                    None => match bank_entry(layers.bank, &cleaned, &pieces, variant_idx) {
                        Some(bank_phonemes) => (
                            with_lexical_stress(&cleaned, bank_phonemes),
                            PhonemeProvenance::Auto,
                            G2pSource::BankDict,
                        ),
                        None => {
                            let (phonemes, source) =
                                auto_word_phonemes(&cleaned, &pieces, variant_idx);
                            (phonemes, PhonemeProvenance::Auto, source)
                        }
                    },
                };
                // The user's breaks are exact: labels are the written
                // pieces and the phonemes split at the matching
                // consonant boundary. When the pieces do not match the
                // vowel count the phonemes fall back to the automatic
                // onset-maximising split (same number of syllables, so
                // the note count is unchanged) and every syllable of the
                // word is flagged.
                let syl_count = pieces.len().max(1);
                let (phoneme_groups, warning) = match split_at_user_breaks(&phonemes, &pieces) {
                    Some(groups) => (groups, false),
                    None => (split_into_syllables(&phonemes, syl_count), true),
                };
                let n = phoneme_groups.len();
                let last_idx = n.saturating_sub(1);
                for (i, group) in phoneme_groups.into_iter().enumerate() {
                    let label = pieces.get(i).cloned().unwrap_or_default();
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
                        source,
                        slurs_after: 0,
                        breath_before: std::mem::take(&mut pending_breath),
                        syllable_warning: warning,
                    });
                }
            }
        }
    }
    out
}

/// The bank dictionary's reading of `cleaned`, when it should be used —
/// see [`resolve_draft_with_layers`] for when it is skipped.
fn bank_entry<'d>(
    bank: Option<&'d PhonemeDictionary>,
    cleaned: &str,
    pieces: &[String],
    variant_idx: Option<usize>,
) -> Option<&'d Vec<&'static str>> {
    if variant_idx.is_some() {
        return None;
    }
    let entry = bank?.get(cleaned)?;
    let vowels = entry.iter().filter(|p| !is_consonant(p)).count();
    (pieces.len() <= 1 || vowels == pieces.len()).then_some(entry)
}

/// Lexical stress for a bank-dictionary reading, which carries none of
/// its own (`dsdict` phonemes have no stress digits). When CMU knows the
/// word and has a variant with the same vowel count, its per-vowel
/// stresses are copied in order; otherwise the first vowel is marked
/// [`SyllableStress::Primary`], so a bank hit never sings flatter (lower
/// velocity) than the CMU reading it replaces.
fn with_lexical_stress(
    cleaned: &str,
    phonemes: &[&'static str],
) -> Vec<(&'static str, SyllableStress)> {
    let vowel_count = phonemes.iter().filter(|p| !is_consonant(p)).count();
    let from_cmu: Option<Vec<SyllableStress>> = (word_g2p_source(cleaned) == G2pSource::Cmu)
        .then(|| {
            cmu_variants(cleaned).into_iter().find_map(|v| {
                let stresses: Vec<SyllableStress> = v
                    .phonemes
                    .iter()
                    .filter(|(p, _)| !is_consonant(p))
                    .map(|(_, s)| *s)
                    .collect();
                (stresses.len() == vowel_count).then_some(stresses)
            })
        })
        .flatten();
    let mut vowel_idx = 0;
    phonemes
        .iter()
        .map(|p| {
            if is_consonant(p) {
                return (*p, SyllableStress::None);
            }
            let stress = match &from_cmu {
                Some(stresses) => stresses[vowel_idx],
                None if vowel_idx == 0 => SyllableStress::Primary,
                None => SyllableStress::None,
            };
            vowel_idx += 1;
            (*p, stress)
        })
        .collect()
}

/// The CMU / rules transcription of a word for the resolver.
///
/// An explicit `(N)` hint always picks variant `N`. Without one, a word
/// the user broke into pieces that the default variant cannot hold (its
/// vowel count differs from the piece count) takes the first CMU variant
/// whose vowel count does match — `ev·ery` is the two-vowel `EH V R IY`,
/// `ev·er·y` the three-vowel `EH V ER IY` — so writing the breaks is
/// enough to choose between a word's syllabic readings. When no variant
/// fits, the default is used and the caller flags the mismatch.
fn auto_word_phonemes(
    cleaned: &str,
    pieces: &[String],
    variant_idx: Option<usize>,
) -> (Vec<(&'static str, SyllableStress)>, G2pSource) {
    let vowels = |ph: &[(&'static str, SyllableStress)]| {
        ph.iter().filter(|(p, _)| !is_consonant(p)).count()
    };
    let default = word_to_phonemes_sourced(cleaned, variant_idx.unwrap_or(1));
    if variant_idx.is_some() || pieces.len() <= 1 || vowels(&default.0) == pieces.len() {
        return default;
    }
    (2..=cmu_variant_count(cleaned))
        .map(|n| word_to_phonemes_sourced(cleaned, n))
        .find(|(ph, _)| vowels(ph) == pieces.len())
        .unwrap_or(default)
}

/// Map a per-note annotation vec to per-note `AssignedSyllable`s,
/// walking a cursor through `syllables` and skipping it on slur
/// notes. The single source of truth for the cursor model — every
/// view + the SVS pipeline use this.
///
/// `annotations[i]` is interpreted as:
///
/// * `""` (empty)  — consume the next resolved syllable.
/// * `"+"` / `"-"` / `"_"` — slur: inherit the previous note's vowel,
///   no cursor advance.
/// * anything else — explicit label override (cursor still advances;
///   phonemes still come from the resolved syllable).
///
/// The draft's own `_` holds ([`ResolvedSyllable::slurs_after`]) are
/// honoured too: after a syllable is consumed, its next `slurs_after`
/// notes are slur notes holding it. A per-note slur annotation that
/// lands on one of those notes is the same hold, not an extra one, so
/// it uses up one of the pending draft holds. An explicit label
/// annotation landing on a note the draft would hold ends the remaining
/// holds instead: that note takes the next syllable under its label.
///
/// A held syllable's final consonants move from its first note to the
/// last note of the hold (see `carry_coda_to_last_hold`), together with
/// `is_word_end`.
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
    let mut last_source: G2pSource = G2pSource::default();
    // Draft `_` holds still owed by the last consumed syllable.
    let mut pending_slurs: usize = 0;
    for i in 0..note_count {
        let entry = annotations.get(i).map(|s| s.trim()).unwrap_or("");
        let annotated_slur = is_slur_lyric(entry);
        // An explicit per-note label is the user naming the syllable this
        // note starts, so it ends any draft holds still owed: the note
        // takes the next syllable instead of silently becoming a hold
        // that ignores its label.
        if !annotated_slur && !entry.is_empty() {
            pending_slurs = 0;
        }
        if annotated_slur || pending_slurs > 0 {
            pending_slurs = pending_slurs.saturating_sub(1);
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
                source: last_source,
                breath_before: false,
                syllable_warning: false,
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
                // Nothing transcribed this note; the pipeline sings a
                // fallback vowel, which is the rules layer's job.
                source: G2pSource::Rules,
                breath_before: false,
                syllable_warning: false,
            });
            continue;
        };
        // Override > dictionary > CMU-auto: a per-syllable override (keyed
        // by resolved-syllable index) replaces the phonemes and stamps
        // `Edited`; otherwise we keep the syllable's own phonemes and
        // provenance.
        let (phonemes, provenance, source) = match overrides.get(&syl_index) {
            Some(ov) => (ov.clone(), PhonemeProvenance::Edited, G2pSource::Edited),
            None => (syl.phonemes.clone(), syl.provenance, syl.source),
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
        last_source = source;
        pending_slurs = syl.slurs_after;
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
            source,
            breath_before: syl.breath_before,
            syllable_warning: syl.syllable_warning,
        });
    }
    carry_coda_to_last_hold(&mut out);
    out
}

/// A held syllable sings its final consonants at the end of the hold,
/// not before it: `love _ _` is `l ah` · `ah` · `ah v`, not `l ah v` ·
/// `ah` · `ah` (which closes the word on the first note and then re-opens
/// the vowel).
///
/// For every non-slur note followed by slur notes of the same
/// `syllable_index`, the phonemes after the head's last vowel move to the
/// end of the last of those slur notes, and so does `is_word_end` — the
/// word now ends there. Runs after overrides, so an edited syllable's
/// coda moves the same way. A head with no vowel keeps its phonemes.
fn carry_coda_to_last_hold(out: &mut [AssignedSyllable]) {
    let mut head = 0;
    while head < out.len() {
        if out[head].is_slur {
            head += 1;
            continue;
        }
        let index = out[head].syllable_index;
        let mut last = head;
        while out
            .get(last + 1)
            .is_some_and(|n| n.is_slur && n.syllable_index == index)
        {
            last += 1;
        }
        if last > head {
            if let Some(v) = out[head].phonemes.iter().rposition(|p| !is_consonant(p)) {
                let coda = out[head].phonemes.split_off(v + 1);
                out[last].phonemes.extend(coda);
            }
            out[last].is_word_end = std::mem::take(&mut out[head].is_word_end);
        }
        head = last + 1;
    }
}
