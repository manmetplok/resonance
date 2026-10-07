//! Grapheme-to-phoneme module for English lyrics.
//!
//! Organised into five focused sub-modules, each with a single reason
//! to change:
//!
//! * [`phonemes`]    — CMU dict loading, ARPAbet inventory, `SyllableStress`
//! * [`syllabify`]   — syllable counting, `·`-marker insertion, phoneme splitting
//! * [`lyric_parse`] — lyric tokenising, `[..]` block parsing, slur detection
//! * [`note_assign`] — draft resolution and per-note syllable assignment
//! * [`holds`]       — melisma hold origins and `_` draft edits
//!
//! All public items are re-exported here so call sites using
//! `resonance_music_theory::g2p::*` are unaffected.

mod holds;
mod lyric_parse;
mod note_assign;
mod phonemes;
mod syllabify;

// ── phonemes ──────────────────────────────────────────────────────────────────
pub use phonemes::{
    canonical_phoneme, cmu_variant_count, cmu_variants, is_consonant, word_g2p_source,
    ARPABET_PHONEMES, ARPABET_SYMBOLS, CONSONANTS,
};
pub use phonemes::{G2pSource, PronunciationVariant, SyllableStress};

// ── syllabify ─────────────────────────────────────────────────────────────────
pub use syllabify::{
    auto_syllabify_text, cmu_syllable_count, has_syllable_marks, normalize_syllable_marks,
    syllabify_word, HYPHEN_SYLLABLE_MARKER,
};

// ── lyric_parse ───────────────────────────────────────────────────────────────
pub use lyric_parse::{is_breath_token, is_slur_lyric, is_slur_token, SLUR_MARKER};
pub(crate) use lyric_parse::note_units;

// ── note_assign ───────────────────────────────────────────────────────────────
pub use note_assign::{
    assign_syllables_to_notes, assign_syllables_to_notes_with, note_unit_count, phonemes_for_draft,
    resolve_draft, resolve_draft_with_dict, resolve_draft_with_layers, DictionaryLayers,
};
pub use note_assign::{
    AssignedSyllable, PhonemeProvenance, PhonemeDictionary, ResolvedSyllable, SyllableOverrides,
};

// ── holds ─────────────────────────────────────────────────────────────────────
pub use holds::{
    hold_origins, line_first_syllables, set_holds_after, strip_syllable_marks, HoldOrigin,
};
