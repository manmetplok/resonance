//! Grapheme-to-phoneme module for English lyrics.
//!
//! Organised into four focused sub-modules, each with a single reason
//! to change:
//!
//! * [`phonemes`]    — CMU dict loading, ARPAbet inventory, `SyllableStress`
//! * [`syllabify`]   — syllable counting, `·`-marker insertion, phoneme splitting
//! * [`lyric_parse`] — lyric tokenising, `[..]` block parsing, slur detection
//! * [`note_assign`] — draft resolution and per-note syllable assignment
//!
//! All public items are re-exported here so call sites using
//! `resonance_music_theory::g2p::*` are unaffected.

mod lyric_parse;
mod note_assign;
mod phonemes;
mod syllabify;

// ── phonemes ──────────────────────────────────────────────────────────────────
pub use phonemes::{
    canonical_phoneme, cmu_variant_count, cmu_variants, is_consonant, ARPABET_PHONEMES,
    ARPABET_SYMBOLS, CONSONANTS,
};
pub use phonemes::{PronunciationVariant, SyllableStress};

// ── syllabify ─────────────────────────────────────────────────────────────────
pub use syllabify::{
    auto_syllabify_text, cmu_syllable_count, has_syllable_marks, normalize_syllable_marks,
    syllabify_word, HYPHEN_SYLLABLE_MARKER,
};

// ── lyric_parse ───────────────────────────────────────────────────────────────
pub use lyric_parse::{is_slur_lyric, SLUR_MARKER};

// ── note_assign ───────────────────────────────────────────────────────────────
pub use note_assign::{
    assign_syllables_to_notes, assign_syllables_to_notes_with, phonemes_for_draft, resolve_draft,
    resolve_draft_with_dict,
};
pub use note_assign::{
    AssignedSyllable, PhonemeProvenance, PhonemeDictionary, ResolvedSyllable, SyllableOverrides,
};
