//! The Preferences › Keyboard panel's transient state and the preset it
//! edits against (command-palette.md §8). The reducer is
//! `update::keymap`; the types live here because `UiTransientState` holds
//! them (ARCH2-05).

use crate::commands::{BindingMap, CommandId, KeyChord, KeymapPreset};
use crate::settings::KeymapSettings;

/// Which page the Settings overlay shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    General,
    Keyboard,
}

/// A rebinding that would take a chord from another command, waiting for
/// the user to confirm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeymapConflict {
    pub command: CommandId,
    pub chord: KeyChord,
    pub owner: CommandId,
}

/// The Keyboard panel's transient state.
#[derive(Debug, Clone, Default)]
pub struct KeymapEditorState {
    pub tab: SettingsTab,
    pub filter: String,
    /// The command whose next key press becomes its chord.
    pub capturing: Option<CommandId>,
    pub conflict: Option<KeymapConflict>,
    /// The active preset's table, to tell edited rows from default ones
    /// without rebuilding it per frame.
    pub baseline: BindingMap,
    /// What the active preset leaves unbound compared with the defaults.
    pub unbound_by_preset: Vec<CommandId>,
}

impl KeymapEditorState {
    /// Fresh editor state for `settings`' preset.
    pub fn for_settings(settings: &KeymapSettings) -> Self {
        let preset = preset_of(settings);
        Self {
            baseline: preset.bindings(),
            unbound_by_preset: preset.unbound(),
            ..Self::default()
        }
    }
}

/// The preset a persisted name selects (the Resonance default for an
/// unknown one).
pub fn preset_of(settings: &KeymapSettings) -> KeymapPreset {
    KeymapPreset::ALL
        .iter()
        .copied()
        .find(|p| p.key() == settings.preset)
        .unwrap_or(KeymapPreset::Resonance)
}
