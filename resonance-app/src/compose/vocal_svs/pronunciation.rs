//! App-side data model for **pronunciation control** (design #173, notes
//! #174): per-syllable phoneme overrides and a word→phonemes dictionary,
//! plus the pure resolver that folds them — together with the global
//! (user) dictionary supplied at resolve time — into the per-note
//! [`AssignedSyllable`] stream the vocal roll and the `.ds` builder share.
//!
//! This module owns *only* the types, the [`ComposeState`]-resident state
//! ([`PronunciationState`]), and the resolution helper. There is no view,
//! no message handling, and no persistence here — those land in the
//! sibling todos (#496/#497 editing, #498/#499 persistence, #500/#494
//! strip + `.ds` apply).
//!
//! ## Precedence
//!
//! Resolution per syllable is `override > project-dict > global-dict >
//! CMU-auto`. The actual phoneme work is delegated to
//! [`resonance_music_theory::g2p`]:
//!
//! * the **dictionary** layers are merged (project overlays global) into a
//!   single [`g2p::PhonemeDictionary`] and fed to
//!   [`g2p::resolve_draft_with_dict`], which reports
//!   [`PhonemeProvenance::Dict`] for any hit — g2p does not distinguish
//!   project from global, matching the three-way strip palette
//!   (`auto` / `EDIT` / `DICT`);
//! * the **per-syllable overrides** (keyed app-side by `(ClipId,
//!   note_index)` to mirror [`clip_lyrics`]) are translated to g2p's
//!   resolved-syllable-index keying and applied via
//!   [`g2p::assign_syllables_to_notes_with`], the highest-precedence
//!   layer ([`PhonemeProvenance::Edited`]).
//!
//! [`clip_lyrics`]: crate::compose::state::VocalAudioRegistry::clip_lyrics
//! [`ComposeState`]: crate::compose::state::ComposeState

use std::collections::HashMap;

use resonance_audio::types::ClipId;
use resonance_music_theory::derive::LyricLine;
use resonance_music_theory::g2p::{
    self, AssignedSyllable, PhonemeDictionary, ResolvedSyllable, SyllableOverrides,
};

// Re-export g2p's provenance enum as the module's own so callers (the
// strip, the `.ds` builder) reason about a single `PhonemeProvenance`
// type rather than two that mean the same thing.
pub use resonance_music_theory::g2p::PhonemeProvenance;

/// Which dictionary an entry belongs to. Project-scoped entries persist
/// in the `.rproj`; global entries live in a user config file outside any
/// project and are merged in at resolve time. Project beats global on a
/// key collision (see module precedence).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DictionaryScope {
    /// Travels with the project file.
    Project,
    /// User-wide, shared across every project.
    Global,
}

/// One pronunciation dictionary entry: a spelling mapped to the flat
/// ARPAbet phoneme list to sing for the *whole* word, overriding CMU /
/// rule-based transcription. The `word` is stored in the cleaned,
/// lowercased form g2p looks dictionary keys up by, and `phonemes` are
/// canonical `&'static str` symbols (built through
/// [`g2p::canonical_phoneme`]) so every symbol is one the SVS pipeline
/// recognises. List phonemes flat (no `·`); the resolver re-splits them
/// across the word's syllable count exactly like the CMU path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictionaryEntry {
    /// Cleaned, lowercased spelling — the dictionary lookup key.
    pub word: String,
    /// Canonical ARPAbet phonemes for the whole word.
    pub phonemes: Vec<&'static str>,
    /// Project- or user-wide scope.
    pub scope: DictionaryScope,
}

impl DictionaryEntry {
    /// Build an entry, [cleaning](clean_word) the spelling and
    /// [canonicalising](canonicalize_phonemes) the phonemes (silently
    /// dropping any symbol that is not valid ARPAbet — the editor layer
    /// surfaces invalid input to the user before it reaches here).
    pub fn new<S: AsRef<str>>(word: &str, phonemes: &[S], scope: DictionaryScope) -> Self {
        Self {
            word: clean_word(word),
            phonemes: canonicalize_phonemes(phonemes),
            scope,
        }
    }

    /// Build an entry from phonemes already known to be canonical (e.g.
    /// taken straight from [`g2p::cmu_variants`] or another resolver
    /// output), skipping re-validation.
    pub fn from_canonical(word: &str, phonemes: Vec<&'static str>, scope: DictionaryScope) -> Self {
        Self {
            word: clean_word(word),
            phonemes,
            scope,
        }
    }
}

/// A per-syllable phoneme override, attached app-side to one note of a
/// clip (the `(ClipId, note_index)` key on [`PronunciationState`]).
/// Highest-precedence resolution layer: when present and non-empty it
/// replaces whatever the dictionary / CMU path produced for that
/// syllable and stamps [`PhonemeProvenance::Edited`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyllableOverride {
    /// The explicit phonemes to sing for this syllable. Canonical
    /// ARPAbet; empty means "no effective override" (the resolver falls
    /// back to dictionary / auto).
    pub phonemes: Vec<&'static str>,
    /// The CMU variant the user picked, if the override originated from
    /// the variant picker rather than free phoneme editing. Carried for
    /// round-tripping / display (the picker resolves a variant into
    /// `phonemes` when the override is created); resolution itself only
    /// ever reads `phonemes`. `None` for hand-edited overrides.
    pub variant_idx: Option<usize>,
}

impl SyllableOverride {
    /// An override from explicit phoneme symbols, [canonicalised](
    /// canonicalize_phonemes) (invalid symbols dropped). No variant.
    pub fn new<S: AsRef<str>>(phonemes: &[S]) -> Self {
        Self {
            phonemes: canonicalize_phonemes(phonemes),
            variant_idx: None,
        }
    }

    /// An override from phonemes already known to be canonical, recording
    /// the originating CMU `variant_idx` for display / round-trip.
    pub fn from_variant(phonemes: Vec<&'static str>, variant_idx: usize) -> Self {
        Self {
            phonemes,
            variant_idx: Some(variant_idx),
        }
    }

    /// Whether this override actually changes anything (non-empty
    /// phonemes). Empty overrides are inert and skipped by the resolver.
    pub fn is_effective(&self) -> bool {
        !self.phonemes.is_empty()
    }
}

/// App-side pronunciation control state, resident on `ComposeState`
/// alongside the vocal-render side tables. Holds the per-clip override map
/// and the project dictionary; the global dictionary is *not* stored here
/// — it lives in a user config file and is passed to [`resolve_clip`](
/// Self::resolve_clip) at resolve time so a single project never owns
/// user-wide data.
///
/// Cleared on project load (like [`clip_lyrics`]); the persistence layer
/// (#498) repopulates it from the loaded `.rproj`.
///
/// [`clip_lyrics`]: crate::compose::state::VocalAudioRegistry::clip_lyrics
#[derive(Debug, Default)]
pub struct PronunciationState {
    /// Per-syllable overrides keyed by clip, then by note index (the same
    /// index used into a clip's note list and its `clip_lyrics` vec).
    /// Sparse — only edited notes appear.
    pub overrides: HashMap<ClipId, HashMap<usize, SyllableOverride>>,
    /// The project-scoped pronunciation dictionary. Every entry here has
    /// [`DictionaryScope::Project`].
    pub project_dictionary: Vec<DictionaryEntry>,
}

impl PronunciationState {
    /// Reset to empty. Called from `ComposeState`'s load path so a freshly
    /// opened project starts with no carried-over overrides or dictionary.
    pub fn clear(&mut self) {
        self.overrides.clear();
        self.project_dictionary.clear();
    }

    /// The override map for one clip, if any note in it has been edited.
    pub fn clip_overrides(&self, clip: ClipId) -> Option<&HashMap<usize, SyllableOverride>> {
        self.overrides.get(&clip)
    }

    /// Set (or replace) the override on one note of a clip. An override
    /// whose phonemes are empty is inert; callers that want to drop an
    /// override should use [`remove_override`](Self::remove_override).
    pub fn set_override(&mut self, clip: ClipId, note_index: usize, ov: SyllableOverride) {
        self.overrides.entry(clip).or_default().insert(note_index, ov);
    }

    /// Remove the override on one note of a clip, returning it if present.
    /// Drops the clip's sub-map once its last override is gone so an
    /// untouched clip never leaves an empty entry behind.
    pub fn remove_override(&mut self, clip: ClipId, note_index: usize) -> Option<SyllableOverride> {
        let map = self.overrides.get_mut(&clip)?;
        let removed = map.remove(&note_index);
        if map.is_empty() {
            self.overrides.remove(&clip);
        }
        removed
    }

    /// Resolve one clip's per-note pronunciation, applying this state's
    /// project dictionary and overrides on top of the supplied `global`
    /// dictionary. See the module docs for precedence. Pure: reads `self`
    /// but mutates nothing.
    pub fn resolve_clip(
        &self,
        clip: ClipId,
        draft: &[LyricLine],
        annotations: &[String],
        note_count: usize,
        global: &[DictionaryEntry],
    ) -> Vec<AssignedSyllable> {
        let empty = HashMap::new();
        let overrides = self.overrides.get(&clip).unwrap_or(&empty);
        resolve_clip(
            draft,
            annotations,
            note_count,
            overrides,
            &self.project_dictionary,
            global,
        )
    }
}

/// Pure resolver: fold `overrides` (per-note), `project` and `global`
/// dictionaries over the CMU/rule-based transcription of `draft`, then
/// assign syllables to the clip's `note_count` notes via `annotations`
/// (the `clip_lyrics` slur / label side-table). Yields exactly
/// `note_count` [`AssignedSyllable`]s, each carrying the
/// [`PhonemeProvenance`] of its winning source.
///
/// Precedence is `override > project-dict > global-dict > auto`. All
/// phoneme work is delegated to [`resonance_music_theory::g2p`]; this
/// function only merges the layers and bridges the app's per-note
/// override keying to g2p's resolved-syllable-index keying.
pub fn resolve_clip(
    draft: &[LyricLine],
    annotations: &[String],
    note_count: usize,
    overrides: &HashMap<usize, SyllableOverride>,
    project: &[DictionaryEntry],
    global: &[DictionaryEntry],
) -> Vec<AssignedSyllable> {
    let dictionary = merged_dictionary(project, global);
    let syllables = g2p::resolve_draft_with_dict(draft, &dictionary);
    let syllable_overrides = note_overrides_to_syllable(overrides, &syllables, annotations, note_count);
    g2p::assign_syllables_to_notes_with(&syllables, annotations, note_count, &syllable_overrides)
}

/// Merge the project and global dictionaries into the single
/// [`g2p::PhonemeDictionary`] the g2p resolver consumes. Global entries go
/// in first, then project entries overlay them so a project redefinition
/// of the same word wins — realising the `project-dict > global-dict`
/// precedence.
fn merged_dictionary(project: &[DictionaryEntry], global: &[DictionaryEntry]) -> PhonemeDictionary {
    let mut map = PhonemeDictionary::with_capacity(project.len() + global.len());
    for entry in global {
        map.insert(entry.word.clone(), entry.phonemes.clone());
    }
    for entry in project {
        map.insert(entry.word.clone(), entry.phonemes.clone());
    }
    map
}

/// Translate the app's per-*note* overrides into g2p's per-*resolved-
/// syllable* keying. A first, override-free assignment pass establishes
/// each note's `syllable_index` (overrides change only phonemes, never the
/// cursor walk, so the mapping is identical with overrides applied).
/// Slur notes carry no override of their own — they hold the previous
/// syllable's vowel — so an override pinned to a slur note is dropped, as
/// are empty (inert) overrides.
fn note_overrides_to_syllable(
    overrides: &HashMap<usize, SyllableOverride>,
    syllables: &[ResolvedSyllable],
    annotations: &[String],
    note_count: usize,
) -> SyllableOverrides {
    if overrides.is_empty() {
        return SyllableOverrides::new();
    }
    let assigned = g2p::assign_syllables_to_notes(syllables, annotations, note_count);
    let mut out = SyllableOverrides::new();
    for (note_index, ov) in overrides {
        if !ov.is_effective() {
            continue;
        }
        let Some(note) = assigned.get(*note_index) else {
            continue;
        };
        if note.is_slur {
            continue;
        }
        out.insert(note.syllable_index, ov.phonemes.clone());
    }
    out
}

/// Clean a spelling into the dictionary lookup key g2p matches on: keep
/// only alphabetic characters and the apostrophe, lowercased. Mirrors the
/// word cleanup in `g2p::tokenize_line` (sans the `·` syllable marker a
/// dictionary key never carries) plus the lowercasing the
/// [`PhonemeDictionary`](g2p::PhonemeDictionary) contract specifies.
pub fn clean_word(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphabetic() || *c == '\'')
        .collect::<String>()
        .to_lowercase()
}

/// Canonicalise raw phoneme symbols through [`g2p::canonical_phoneme`],
/// dropping any that are not valid ARPAbet (or the `AP`/`SP` silence
/// markers). The returned `&'static str`s are the exact forms the SVS
/// pipeline sings.
pub fn canonicalize_phonemes<S: AsRef<str>>(phonemes: &[S]) -> Vec<&'static str> {
    phonemes
        .iter()
        .filter_map(|p| g2p::canonical_phoneme(p.as_ref()))
        .collect()
}
