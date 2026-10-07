//! App-side data model for **pronunciation control** (design #173, notes
//! #174): per-syllable phoneme overrides and a word→phonemes dictionary,
//! plus the pure resolver that folds them — together with the global
//! (user) dictionary supplied at resolve time — into the per-note
//! [`AssignedSyllable`] stream the vocal roll and the `.ds` builder share.
//!
//! This module owns *only* the types, the [`ComposeState`]-resident state
//! ([`PronunciationState`]), its conversion to and from the persisted
//! [`ProjectPronunciation`], and the resolution helper. There is no view
//! and no message handling here.
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
//! * the **per-syllable overrides** are keyed by vocal lane
//!   (`(definition_id, track_id)`) and the index of the syllable in the
//!   lane's resolved draft — g2p's own keying, so a regenerate that
//!   reuses or replaces the lane's clip keeps them. Each carries the
//!   syllable's text; one whose syllable no longer reads that text (the
//!   lyrics changed under it) is dropped at resolve time. The survivors
//!   are applied via [`g2p::assign_syllables_to_notes_with`], the
//!   highest-precedence layer ([`PhonemeProvenance::Edited`]).
//!
//! [`ComposeState`]: crate::compose::state::ComposeState

use std::collections::HashMap;

use resonance_audio::types::TrackId;
use resonance_music_theory::derive::LyricLine;
use resonance_music_theory::g2p::{
    self, AssignedSyllable, PhonemeDictionary, ResolvedSyllable, SyllableOverrides,
};

use crate::project::{ProjectPronunciation, ProjectPronunciationEntry, ProjectSyllableOverride};

/// A vocal lane: `(section definition id, track id)`.
pub type LaneKey = (u64, TrackId);

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

/// A per-syllable phoneme override, attached to one syllable of a vocal
/// lane's resolved draft (the `(lane, syllable_index)` key on
/// [`PronunciationState`]). Highest-precedence resolution layer: when
/// present, non-empty and still on the syllable it was made for (`text`
/// matches) it replaces whatever the dictionary / CMU path produced for
/// that syllable and stamps [`PhonemeProvenance::Edited`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyllableOverride {
    /// The syllable's surface label ([`ResolvedSyllable::label`]) when
    /// the override was made. Compared case-insensitively, trimmed; a
    /// mismatch means the lyrics changed under the override and it no
    /// longer applies.
    pub text: String,
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
    /// An override for the syllable reading `text`, from explicit phoneme
    /// symbols, [canonicalised](canonicalize_phonemes) (invalid symbols
    /// dropped). No variant.
    pub fn new<S: AsRef<str>>(text: &str, phonemes: &[S]) -> Self {
        Self {
            text: text.to_owned(),
            phonemes: canonicalize_phonemes(phonemes),
            variant_idx: None,
        }
    }

    /// An override for the syllable reading `text`, from phonemes already
    /// known to be canonical, recording the originating CMU `variant_idx`
    /// for display / round-trip.
    pub fn from_variant(text: &str, phonemes: Vec<&'static str>, variant_idx: usize) -> Self {
        Self {
            text: text.to_owned(),
            phonemes,
            variant_idx: Some(variant_idx),
        }
    }

    /// Whether this override still belongs on `syllable`: its text
    /// matches the syllable's label (trimmed, case-insensitive).
    pub fn matches(&self, syllable: &ResolvedSyllable) -> bool {
        self.text.trim().to_lowercase() == syllable.label.trim().to_lowercase()
    }

    /// Whether this override actually changes anything (non-empty
    /// phonemes). Empty overrides are inert and skipped by the resolver.
    pub fn is_effective(&self) -> bool {
        !self.phonemes.is_empty()
    }
}

/// App-side pronunciation control state, resident on `ComposeState`
/// alongside the vocal-render side tables. Holds the per-lane override map
/// and the project dictionary; the global dictionary is *not* stored here
/// — it lives in a user config file and is passed to [`resolve_lane`](
/// Self::resolve_lane) at resolve time so a single project never owns
/// user-wide data.
///
/// Persisted in [`ProjectFile::pronunciation`](crate::project::ProjectFile)
/// ([`to_project`](Self::to_project)) and restored from it on load and on
/// every undo/redo ([`restore_from_project`](Self::restore_from_project),
/// the `pronunciation` reconcile domain).
#[derive(Debug, Default)]
pub struct PronunciationState {
    /// Per-syllable overrides keyed by vocal lane, then by the syllable's
    /// index in the lane's resolved draft. Sparse — only edited syllables
    /// appear.
    pub overrides: HashMap<LaneKey, HashMap<usize, SyllableOverride>>,
    /// The project-scoped pronunciation dictionary. Every entry here has
    /// [`DictionaryScope::Project`].
    pub project_dictionary: Vec<DictionaryEntry>,
}

impl PronunciationState {
    /// Reset to empty.
    pub fn clear(&mut self) {
        self.overrides.clear();
        self.project_dictionary.clear();
    }

    /// The override map for one vocal lane, if any syllable in it has
    /// been edited.
    pub fn lane_overrides(
        &self,
        definition_id: u64,
        track_id: TrackId,
    ) -> Option<&HashMap<usize, SyllableOverride>> {
        self.overrides.get(&(definition_id, track_id))
    }

    /// Set (or replace) the override on one syllable of a lane. An
    /// override whose phonemes are empty is inert; callers that want to
    /// drop an override should use [`remove_override`](Self::remove_override).
    pub fn set_override(
        &mut self,
        definition_id: u64,
        track_id: TrackId,
        syllable_index: usize,
        ov: SyllableOverride,
    ) {
        self.overrides
            .entry((definition_id, track_id))
            .or_default()
            .insert(syllable_index, ov);
    }

    /// Remove the override on one syllable of a lane, returning it if
    /// present. Drops the lane's sub-map once its last override is gone so
    /// an untouched lane never leaves an empty entry behind.
    pub fn remove_override(
        &mut self,
        definition_id: u64,
        track_id: TrackId,
        syllable_index: usize,
    ) -> Option<SyllableOverride> {
        let key = (definition_id, track_id);
        let map = self.overrides.get_mut(&key)?;
        let removed = map.remove(&syllable_index);
        if map.is_empty() {
            self.overrides.remove(&key);
        }
        removed
    }

    /// The persisted form: the dictionary in its live order, the
    /// overrides sorted by `(definition, track, syllable)` so the file
    /// (and every undo snapshot) is deterministic.
    pub fn to_project(&self) -> ProjectPronunciation {
        let dictionary = self
            .project_dictionary
            .iter()
            .map(|e| ProjectPronunciationEntry {
                word: e.word.clone(),
                phonemes: e.phonemes.iter().map(|p| (*p).to_owned()).collect(),
            })
            .collect();
        let mut overrides: Vec<ProjectSyllableOverride> = self
            .overrides
            .iter()
            .flat_map(|(&(definition_id, track_id), lane)| {
                lane.iter().map(move |(&syllable_index, ov)| ProjectSyllableOverride {
                    definition_id,
                    track_id,
                    syllable_index,
                    text: ov.text.clone(),
                    phonemes: ov.phonemes.iter().map(|p| (*p).to_owned()).collect(),
                    variant_idx: ov.variant_idx,
                })
            })
            .collect();
        overrides.sort_by_key(|o| (o.definition_id, o.track_id, o.syllable_index));
        ProjectPronunciation {
            dictionary,
            overrides,
        }
    }

    /// Replace this state with the persisted `file`. Phonemes are
    /// re-canonicalised (unknown symbols dropped); a dictionary entry left
    /// with none is skipped (only a hand-edited file can hold one), while
    /// an override left with none is kept — it is inert either way, and
    /// keeping it keeps `to_project(restore(file)) == file` for any file
    /// the app wrote.
    pub fn restore_from_project(&mut self, file: &ProjectPronunciation) {
        self.clear();
        for entry in &file.dictionary {
            let phonemes = canonicalize_phonemes(&entry.phonemes);
            if phonemes.is_empty() {
                continue;
            }
            let entry =
                DictionaryEntry::from_canonical(&entry.word, phonemes, DictionaryScope::Project);
            self.project_dictionary.retain(|e| e.word != entry.word);
            self.project_dictionary.push(entry);
        }
        for ov in &file.overrides {
            self.set_override(
                ov.definition_id,
                ov.track_id,
                ov.syllable_index,
                SyllableOverride {
                    text: ov.text.clone(),
                    phonemes: canonicalize_phonemes(&ov.phonemes),
                    variant_idx: ov.variant_idx,
                },
            );
        }
    }

    /// Resolve one lane's per-note pronunciation, applying this state's
    /// project dictionary and the lane's overrides on top of the supplied
    /// `global` dictionary. See the module docs for precedence. Pure:
    /// reads `self` but mutates nothing.
    pub fn resolve_lane(
        &self,
        definition_id: u64,
        track_id: TrackId,
        draft: &[LyricLine],
        annotations: &[String],
        note_count: usize,
        global: &[DictionaryEntry],
    ) -> Vec<AssignedSyllable> {
        let empty = HashMap::new();
        let overrides = self.lane_overrides(definition_id, track_id).unwrap_or(&empty);
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

/// Pure resolver: fold `overrides` (keyed by resolved-syllable index),
/// `project` and `global` dictionaries over the CMU/rule-based
/// transcription of `draft`, then assign syllables to the clip's
/// `note_count` notes via `annotations` (the `clip_lyrics` slur / label
/// side-table). Yields exactly `note_count` [`AssignedSyllable`]s, each
/// carrying the [`PhonemeProvenance`] of its winning source.
///
/// Precedence is `override > project-dict > global-dict > bank-dict > auto`. All
/// phoneme work is delegated to [`resonance_music_theory::g2p`]; this
/// function only merges the layers and drops the overrides that no
/// longer fit (see [`live_overrides`]).
pub fn resolve_clip(
    draft: &[LyricLine],
    annotations: &[String],
    note_count: usize,
    overrides: &HashMap<usize, SyllableOverride>,
    project: &[DictionaryEntry],
    global: &[DictionaryEntry],
) -> Vec<AssignedSyllable> {
    resolve_clip_with_bank(draft, annotations, note_count, overrides, project, global, None)
}

/// [`resolve_clip`] with the voicebank's own English dictionary as the
/// layer between the user's dictionaries and CMU (spec §4.4):
/// `override > project-dict > global-dict > bank-dict > CMU > rules`.
pub fn resolve_clip_with_bank(
    draft: &[LyricLine],
    annotations: &[String],
    note_count: usize,
    overrides: &HashMap<usize, SyllableOverride>,
    project: &[DictionaryEntry],
    global: &[DictionaryEntry],
    bank: Option<&PhonemeDictionary>,
) -> Vec<AssignedSyllable> {
    let dictionary = merged_dictionary(project, global);
    let layers = g2p::DictionaryLayers {
        project: &dictionary,
        bank,
    };
    let syllables = g2p::resolve_draft_with_layers(draft, layers);
    let syllable_overrides = live_overrides(overrides, &syllables);
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

/// The overrides that still apply, in g2p's keying: effective (non-empty
/// phonemes), on a syllable the draft still has, and on a syllable that
/// still reads the override's `text`. An override whose syllable's text
/// changed (or vanished) is dropped — it was made for different lyrics.
fn live_overrides(
    overrides: &HashMap<usize, SyllableOverride>,
    syllables: &[ResolvedSyllable],
) -> SyllableOverrides {
    let mut out = SyllableOverrides::new();
    for (&index, ov) in overrides {
        if !ov.is_effective() {
            continue;
        }
        if let Some(syllable) = syllables.get(index) {
            if ov.matches(syllable) {
                out.insert(index, ov.phonemes.clone());
            }
        }
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
