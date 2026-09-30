//! Keyboard-dispatch test hooks: the typing-gate answer and the active
//! keymap (command-palette.md §10).

use crate::update::shortcuts::TypingProbe;
use crate::Resonance;

impl Resonance {
    /// Test-only: answer the typing gate without a widget tree, so a
    /// `NotWhileTyping` shortcut runs (or is dropped) inside `update()`.
    #[doc(hidden)]
    pub fn test_set_typing_probe(&mut self, probe: TypingProbe) {
        self.ui.typing_probe = probe;
    }

    /// Test-only: the active keymap.
    #[doc(hidden)]
    pub fn test_keymap(&self) -> &crate::commands::BindingMap {
        &self.ui.keymap
    }
}
