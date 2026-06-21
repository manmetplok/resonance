//! Headless focus probing for the Performance-mode keyboard shortcut.
//!
//! iced 0.14 exposes no synchronous "is a text field focused?" query, and
//! `text_input` has no focus/blur callbacks, so the global
//! `keyboard::listen()` subscription receives the `F` key press even while the
//! user is typing into a text field. Firing the Performance-mode toggle from
//! that raw subscription would yank the user in/out of Performance mode
//! mid-edit (every `text_input` in the app is affected: track names, section
//! name/length, BPM, lyrics, drum-group names, pad/pattern filters, …).
//!
//! To gate the unmodified `F` shortcut we probe the live widget tree with a
//! focus [`Operation`]. Only `text_input` and `text_editor` are focusable in
//! iced, so "any focusable widget holds keyboard focus" is exactly "a text
//! field is being edited". The probe runs as a `Task` the moment `F` is
//! pressed, and the toggle is suppressed when it reports an active edit.

use iced::advanced::widget::operation::{Focusable, Operation, Outcome};
use iced::advanced::widget::Id;
use iced::Rectangle;

/// Focus [`Operation`] that resolves to `true` when any focusable widget
/// (i.e. a `text_input` / `text_editor`) currently holds keyboard focus.
#[derive(Default)]
pub struct AnyTextInputFocused {
    focused: bool,
}

impl Operation<bool> for AnyTextInputFocused {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<bool>)) {
        operate(self);
    }

    fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if state.is_focused() {
            self.focused = true;
        }
    }

    fn finish(&self) -> Outcome<bool> {
        Outcome::Some(self.focused)
    }
}

/// Build a `Task` that resolves to whether a text field currently holds focus.
pub fn any_text_input_focused() -> iced::Task<bool> {
    iced::advanced::widget::operate(AnyTextInputFocused::default())
}
