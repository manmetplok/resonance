//! The user's voice presets and the lane inspector's save / rename /
//! delete prompt (`docs/features/compose/voice-presets.md`). Machine and
//! transient UI state, never project state: a project load leaves it
//! alone, and nothing here is undoable.

use std::path::PathBuf;

use resonance_audio::types::TrackId;

use crate::compose::vocal_svs::voice_presets::{VoicePreset, VoicePresetStore};

#[derive(Debug, Clone, Default)]
pub struct VoicePresetState {
    /// Where user presets are read and written.
    pub store: VoicePresetStore,
    /// The user presets, by name (re-read after every save, rename and
    /// delete).
    pub user: Vec<VoicePreset>,
    /// The open name prompt, if any.
    pub prompt: Option<VoicePresetPrompt>,
    /// The user preset whose Delete was pressed once and waits for the
    /// confirming press.
    pub confirm_delete: Option<String>,
    /// The last save / rename / delete failure, shown in the card until
    /// the next action.
    pub error: Option<String>,
}

impl VoicePresetState {
    /// The state over the presets in `dir` (read now).
    pub fn new(dir: Option<PathBuf>) -> Self {
        let store = VoicePresetStore::new(dir);
        let user = store.load();
        Self {
            store,
            user,
            ..Self::default()
        }
    }

    /// Re-read the user presets from disk.
    pub fn reload(&mut self) {
        self.user = self.store.load();
    }
}

/// What the name prompt does on confirm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptMode {
    /// Save the lane's voice as a new user preset.
    Save,
    /// Rename the user preset named here.
    Rename(String),
}

/// The inline name prompt in a vocal lane's Voice & delivery card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoicePresetPrompt {
    pub definition_id: u64,
    pub track_id: TrackId,
    pub mode: PromptMode,
    pub name: String,
}
