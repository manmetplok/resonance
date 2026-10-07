//! Persisted pronunciation control: the project dictionary and the
//! per-syllable overrides (BUG-50, vocal-mcp-control-and-quality slice
//! 11). The live counterpart is
//! [`PronunciationState`](crate::compose::vocal_svs::PronunciationState),
//! which converts to and from these.
//!
//! Phonemes are stored as plain lowercase ARPAbet strings; the load path
//! canonicalises them again and drops any symbol the SVS pipeline does
//! not know (and an entry left with no phonemes), so a hand-edited file
//! cannot smuggle an unsingable symbol into the resolver.

use resonance_audio::types::TrackId;
use serde::{Deserialize, Serialize};

/// The project's pronunciation control. Left out of the file when empty,
/// so a project that never touched pronunciation saves exactly as before;
/// a file without the field (every project saved before it existed)
/// loads with no entries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPronunciation {
    /// Word → phonemes entries, in the order the live dictionary holds
    /// them (`vocal.set_pronunciation` replaces an entry by removing and
    /// re-appending it).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dictionary: Vec<ProjectPronunciationEntry>,
    /// Per-syllable overrides, sorted by `(definition_id, track_id,
    /// syllable_index)`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<ProjectSyllableOverride>,
}

impl ProjectPronunciation {
    pub fn is_empty(&self) -> bool {
        self.dictionary.is_empty() && self.overrides.is_empty()
    }
}

/// One project-dictionary entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPronunciationEntry {
    /// Cleaned, lowercased spelling (the lookup key).
    pub word: String,
    /// Phonemes for the whole word.
    pub phonemes: Vec<String>,
}

/// One per-syllable override, keyed by the vocal lane (section
/// definition + track) and the index of the syllable in the lane's
/// resolved lyric draft. `text` is the syllable's surface label when the
/// override was made: an override whose syllable no longer reads `text`
/// (the lyrics changed under it) is ignored by the resolver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSyllableOverride {
    pub definition_id: u64,
    pub track_id: TrackId,
    pub syllable_index: usize,
    pub text: String,
    pub phonemes: Vec<String>,
    /// The CMU variant the override came from, if it was picked rather
    /// than typed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant_idx: Option<usize>,
}
