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

/// Keyboard ownership for a canvas editor surface (timeline, piano roll,
/// vocal roll).
///
/// iced delivers every `KeyPressed` to every widget in the tree, and a
/// canvas `update` never learns whether a sibling already consumed it — so
/// a bare canvas key handler fires while the user types into a text field
/// elsewhere on the page, or while they work in a *different* canvas
/// (Delete in the piano roll used to delete the whole clip through the
/// timeline above it). A canvas instead owns the keyboard only while it was
/// the target of the most recent mouse press: a press inside its bounds
/// claims the keys, a press anywhere else (another canvas, a text field, a
/// button) releases them. Focusing a text field takes a click — the app
/// never focuses one programmatically — so "last pressed inside me" also
/// means "no text field is being typed into".
///
/// The state lives in the canvas's `Program::State`, which iced keeps in
/// the widget tree; every canvas sees every mouse press (containers forward
/// events to all children, and a scrollable hands off-screen children an
/// unavailable cursor), so ownership moves reliably.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyFocus {
    owned: bool,
    /// The app-side grant generation last seen; `None` until the first
    /// event, so a freshly built canvas adopts the current value instead
    /// of treating an old selection as new.
    seen_grant: Option<u64>,
}

impl KeyFocus {
    /// Take the keys when the app's grant generation moved since the last
    /// event: a selection aimed at this surface was made without a press
    /// on it (code review FU-C2). Call before [`track`](Self::track), so a
    /// press elsewhere in the same event still releases them.
    pub fn sync_grant(&mut self, grant: u64) {
        match self.seen_grant {
            Some(seen) if seen != grant => self.owned = true,
            _ => {}
        }
        self.seen_grant = Some(grant);
    }

    /// Feed every event the canvas receives; a mouse press moves ownership.
    pub fn track(&mut self, event: &iced::Event, bounds: Rectangle, cursor: iced::mouse::Cursor) {
        if let iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_)) = event {
            self.owned = cursor.is_over(bounds);
        }
    }

    /// Whether the canvas should act on key presses.
    pub fn owns_keys(&self) -> bool {
        self.owned
    }
}
