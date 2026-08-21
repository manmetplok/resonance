//! Translate NSEvent values into `egui::Event` values.
//!
//! Deliberately pure: every function here takes the *values* already read
//! out of an `NSEvent` (coordinates, modifier bits, characters, deltas),
//! never the event object itself, so the whole module is testable headlessly
//! on any platform. The `EditorView` overrides in
//! `window_main_thread/view.rs` do the (trivial) NSEvent field reads and
//! call in here.

// Via the core crate's re-export, not a direct dependency: `egui` itself is
// only pulled in on macOS (it comes paired with the glow painter), and this
// module compiles everywhere.
use plugin_gui_core::egui;
use egui::{Modifiers, PointerButton, Pos2, Vec2};

// NSEventModifierFlags bits (AppKit's NSEvent.h; stable ABI constants).
pub const NS_FLAG_SHIFT: u64 = 1 << 17;
pub const NS_FLAG_CONTROL: u64 = 1 << 18;
pub const NS_FLAG_OPTION: u64 = 1 << 19;
pub const NS_FLAG_COMMAND: u64 = 1 << 20;

/// One scroll "line" in points, for wheel events without precise deltas.
/// Same estimate the Wayland runtime uses for discrete axis steps.
const SCROLL_LINE_POINTS: f32 = 50.0;

/// Incremental input state owned by the editor view.
#[derive(Default)]
pub struct InputState {
    pointer_pos: Pos2,
    modifiers: Modifiers,
}

impl InputState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update the modifier set from an `NSEvent.modifierFlags` value.
    /// AppKit carries the current flags on every event (and delivers
    /// `flagsChanged:` when they change with no other event), so this is
    /// called for every translated event.
    ///
    /// egui's convention: `command` is the platform's primary shortcut
    /// modifier — ⌘ on macOS — and `mac_cmd` is ⌘ specifically.
    pub fn set_modifier_flags(&mut self, flags: u64) {
        let cmd = flags & NS_FLAG_COMMAND != 0;
        self.modifiers = Modifiers {
            alt: flags & NS_FLAG_OPTION != 0,
            ctrl: flags & NS_FLAG_CONTROL != 0,
            shift: flags & NS_FLAG_SHIFT != 0,
            mac_cmd: cmd,
            command: cmd,
        };
    }

    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Pointer moved to view-local top-left-origin coordinates in points
    /// (the view is flipped, so AppKit already reports them that way).
    pub fn pointer_moved(&mut self, x: f32, y: f32, out: &mut Vec<egui::Event>) {
        self.pointer_pos = Pos2::new(x, y);
        out.push(egui::Event::PointerMoved(self.pointer_pos));
    }

    pub fn pointer_left(&mut self, out: &mut Vec<egui::Event>) {
        out.push(egui::Event::PointerGone);
    }

    /// A press/release of the given [`PointerButton`] at the last known
    /// pointer position. AppKit reports the position on button events too;
    /// callers pass it through [`InputState::pointer_moved`]-style by
    /// calling [`set_pointer_pos`](Self::set_pointer_pos) first.
    pub fn pointer_button(
        &mut self,
        button: PointerButton,
        pressed: bool,
        out: &mut Vec<egui::Event>,
    ) {
        out.push(egui::Event::PointerButton {
            pos: self.pointer_pos,
            button,
            pressed,
            modifiers: self.modifiers,
        });
    }

    /// Update the pointer position without emitting a move event (button
    /// events carry a location too, and emitting a move for it would make
    /// every click look like a drag start).
    pub fn set_pointer_pos(&mut self, x: f32, y: f32) {
        self.pointer_pos = Pos2::new(x, y);
    }

    /// A scroll-wheel event. `precise` is `hasPreciseScrollingDeltas`:
    /// trackpads and Magic Mice report exact point deltas, wheel mice
    /// report line counts which we convert with the same points-per-line
    /// estimate as the Wayland runtime. AppKit's sign convention (positive
    /// = scroll up/left, with natural scrolling already applied) matches
    /// egui's, so the deltas pass through un-negated — unlike Wayland's
    /// axis values.
    pub fn scroll(&mut self, dx: f64, dy: f64, precise: bool, out: &mut Vec<egui::Event>) {
        let mut delta = Vec2::new(dx as f32, dy as f32);
        if !precise {
            delta *= SCROLL_LINE_POINTS;
        }
        if delta.x != 0.0 || delta.y != 0.0 {
            out.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta,
                phase: egui::TouchPhase::Move,
                modifiers: self.modifiers,
            });
        }
    }

    /// A key press/release. `chars_ignoring_mods` is
    /// `charactersIgnoringModifiers` (used to identify the key — it ignores
    /// ⌥ but not ⇧, hence the lowercasing in [`map_key_char`]);
    /// `chars` is `characters` (the text the key produced, dead-key and
    /// ⌥-composition applied).
    ///
    /// Text is emitted only for printable presses without ⌘/⌃ held: those
    /// are shortcut chords, not typing. ⌥ is deliberately allowed through —
    /// on macOS option-composition is how "å" and friends are typed.
    pub fn key(
        &mut self,
        chars_ignoring_mods: &str,
        chars: &str,
        pressed: bool,
        repeat: bool,
        out: &mut Vec<egui::Event>,
    ) {
        if let Some(key) = chars_ignoring_mods.chars().next().and_then(map_key_char) {
            out.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat,
                modifiers: self.modifiers,
            });
        }
        if pressed
            && !self.modifiers.mac_cmd
            && !self.modifiers.ctrl
            && !chars.is_empty()
            && chars.chars().all(|c| !c.is_control() && !is_function_key(c))
        {
            out.push(egui::Event::Text(chars.to_string()));
        }
    }
}

/// AppKit encodes function/navigation keys as characters in the Unicode
/// private-use range U+F700..=U+F8FF (NSUpArrowFunctionKey and friends).
/// They must never leak into `egui::Event::Text`.
fn is_function_key(c: char) -> bool {
    ('\u{f700}'..='\u{f8ff}').contains(&c)
}

/// Map an AppKit `buttonNumber` from `otherMouseDown:`/`otherMouseUp:` to an
/// egui button. Left (0) and right (1) arrive via their own dedicated
/// `mouseDown:`/`rightMouseDown:` overrides and never reach this.
pub fn map_other_button(button_number: i64) -> Option<PointerButton> {
    match button_number {
        2 => Some(PointerButton::Middle),
        3 => Some(PointerButton::Extra1),
        4 => Some(PointerButton::Extra2),
        _ => None,
    }
}

/// Map the first character of `charactersIgnoringModifiers` to an
/// [`egui::Key`]. Covers the same key set as the Wayland runtime's keysym
/// map: navigation, editing, space/enter/escape/tab, letters, digits.
pub fn map_key_char(c: char) -> Option<egui::Key> {
    use plugin_gui_core::egui::Key;
    Some(match c {
        '\r' | '\u{3}' => Key::Enter, // Return / keypad Enter
        '\u{1b}' => Key::Escape,
        '\t' | '\u{19}' => Key::Tab, // Tab / Backtab (shift-tab)
        // The Delete key AppKit reports as BS/DEL depending on source;
        // both mean "erase left" on macOS keyboards.
        '\u{8}' | '\u{7f}' => Key::Backspace,
        '\u{f728}' => Key::Delete, // NSDeleteFunctionKey (fn-delete)
        '\u{f727}' => Key::Insert,
        '\u{f702}' => Key::ArrowLeft,
        '\u{f703}' => Key::ArrowRight,
        '\u{f700}' => Key::ArrowUp,
        '\u{f701}' => Key::ArrowDown,
        '\u{f729}' => Key::Home,
        '\u{f72b}' => Key::End,
        '\u{f72c}' => Key::PageUp,
        '\u{f72d}' => Key::PageDown,
        ' ' => Key::Space,
        // charactersIgnoringModifiers ignores ⌥ but not ⇧: with shift held
        // a letter arrives uppercase, so fold case before matching.
        c => match c.to_ascii_lowercase() {
            'a' => Key::A,
            'b' => Key::B,
            'c' => Key::C,
            'd' => Key::D,
            'e' => Key::E,
            'f' => Key::F,
            'g' => Key::G,
            'h' => Key::H,
            'i' => Key::I,
            'j' => Key::J,
            'k' => Key::K,
            'l' => Key::L,
            'm' => Key::M,
            'n' => Key::N,
            'o' => Key::O,
            'p' => Key::P,
            'q' => Key::Q,
            'r' => Key::R,
            's' => Key::S,
            't' => Key::T,
            'u' => Key::U,
            'v' => Key::V,
            'w' => Key::W,
            'x' => Key::X,
            'y' => Key::Y,
            'z' => Key::Z,
            '0' => Key::Num0,
            '1' => Key::Num1,
            '2' => Key::Num2,
            '3' => Key::Num3,
            '4' => Key::Num4,
            '5' => Key::Num5,
            '6' => Key::Num6,
            '7' => Key::Num7,
            '8' => Key::Num8,
            '9' => Key::Num9,
            _ => return None,
        },
    })
}
