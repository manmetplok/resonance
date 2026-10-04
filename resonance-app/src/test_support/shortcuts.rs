//! Keyboard-dispatch test hooks: the typing-gate answer and the active
//! keymap (command-palette.md §10).

use crate::state::TypingProbe;
use crate::Resonance;

impl Resonance {
    /// Test-only: answer the typing gate without a widget tree, so a
    /// `NotWhileTyping` shortcut runs (or is dropped) inside `update()`.
    #[doc(hidden)]
    pub fn test_set_typing_probe(&mut self, probe: TypingProbe) {
        self.ui.typing_probe = probe;
    }

    /// Test-only: run a registry command as a keyboard shortcut would
    /// (availability, typing gate, dispatch), without a chord.
    #[doc(hidden)]
    pub fn test_run_shortcut(&mut self, command: crate::commands::CommandId) {
        let _ = crate::update::shortcuts::run_shortcut(self, command, false);
    }

    /// Test-only: close the topmost modal overlay, as Esc does.
    #[doc(hidden)]
    pub fn test_dismiss_overlay(&mut self) {
        let _ = crate::update::shortcuts::dismiss_overlay(self);
    }

    /// Test-only: whether a *Recent* write to settings.json is pending.
    #[doc(hidden)]
    pub fn test_recent_write_pending(&self) -> bool {
        self.ui.recent_dirty_since.is_some()
    }

    /// Test-only: the open command palette.
    #[doc(hidden)]
    pub fn test_palette(&self) -> Option<&crate::palette::PaletteState> {
        self.ui.palette.as_ref()
    }

    /// Test-only: the query the palette will reopen with.
    #[doc(hidden)]
    pub fn test_palette_memory(&self) -> &str {
        &self.ui.palette_memory
    }

    /// Test-only: every MIDI clip id on the timeline.
    #[doc(hidden)]
    pub fn test_midi_clip_ids(&self) -> Vec<resonance_audio::types::ClipId> {
        self.midi_clips.iter().map(|c| c.id).collect()
    }

    /// Test-only: the Keyboard panel's state.
    #[doc(hidden)]
    pub fn test_keymap_editor(&self) -> &crate::update::keymap::KeymapEditorState {
        &self.ui.keymap_editor
    }

    /// Test-only: the active keymap.
    #[doc(hidden)]
    pub fn test_keymap(&self) -> &crate::commands::BindingMap {
        &self.ui.keymap
    }
}
