//! Headless tests for the NSEvent-value → egui translation in
//! `cocoa_plugin_gui::input`. The module is pure (values in, egui events
//! out), so these run on any platform — no window, no AppKit.

use cocoa_plugin_gui::input::{
    map_key_char, map_other_button, InputState, NS_FLAG_COMMAND, NS_FLAG_CONTROL, NS_FLAG_OPTION,
    NS_FLAG_SHIFT,
};
use cocoa_plugin_gui::egui::{Event, Key, Modifiers, PointerButton};

#[test]
fn modifier_flags_map_to_egui_modifiers() {
    let mut input = InputState::new();
    input.set_modifier_flags(NS_FLAG_SHIFT | NS_FLAG_COMMAND);
    assert_eq!(
        input.modifiers(),
        Modifiers {
            alt: false,
            ctrl: false,
            shift: true,
            mac_cmd: true,
            command: true,
        },
        "⌘ must set both mac_cmd and command (egui's primary-modifier convention)"
    );

    input.set_modifier_flags(NS_FLAG_OPTION | NS_FLAG_CONTROL);
    let m = input.modifiers();
    assert!(m.alt && m.ctrl && !m.shift && !m.command && !m.mac_cmd);
}

#[test]
fn pointer_button_uses_last_known_position() {
    let mut input = InputState::new();
    let mut out = Vec::new();
    input.pointer_moved(10.0, 20.0, &mut out);
    input.set_pointer_pos(30.0, 40.0);
    input.pointer_button(PointerButton::Primary, true, &mut out);

    assert_eq!(out.len(), 2);
    match &out[1] {
        Event::PointerButton {
            pos,
            button,
            pressed,
            ..
        } => {
            assert_eq!((pos.x, pos.y), (30.0, 40.0));
            assert_eq!(*button, PointerButton::Primary);
            assert!(pressed);
        }
        other => panic!("expected PointerButton, got {other:?}"),
    }
}

#[test]
fn other_mouse_buttons_map_by_number() {
    assert_eq!(map_other_button(2), Some(PointerButton::Middle));
    assert_eq!(map_other_button(3), Some(PointerButton::Extra1));
    assert_eq!(map_other_button(4), Some(PointerButton::Extra2));
    assert_eq!(map_other_button(5), None);
}

#[test]
fn precise_scroll_passes_points_through_unnegated() {
    let mut input = InputState::new();
    let mut out = Vec::new();
    input.scroll(3.0, -7.5, true, &mut out);
    match &out[0] {
        Event::MouseWheel { delta, .. } => {
            // AppKit's sign convention already matches egui's — unlike the
            // Wayland axis values, which the Wayland runtime negates.
            assert_eq!((delta.x, delta.y), (3.0, -7.5));
        }
        other => panic!("expected MouseWheel, got {other:?}"),
    }
}

#[test]
fn line_scroll_scales_to_points_and_zero_is_dropped() {
    let mut input = InputState::new();
    let mut out = Vec::new();
    input.scroll(0.0, 2.0, false, &mut out);
    match &out[0] {
        Event::MouseWheel { delta, .. } => assert_eq!((delta.x, delta.y), (0.0, 100.0)),
        other => panic!("expected MouseWheel, got {other:?}"),
    }

    out.clear();
    input.scroll(0.0, 0.0, true, &mut out);
    assert!(out.is_empty(), "zero-delta scrolls must not emit events");
}

#[test]
fn key_map_covers_navigation_and_case_folds_letters() {
    // AppKit function-key private-use characters.
    assert_eq!(map_key_char('\u{f700}'), Some(Key::ArrowUp));
    assert_eq!(map_key_char('\u{f701}'), Some(Key::ArrowDown));
    assert_eq!(map_key_char('\u{f702}'), Some(Key::ArrowLeft));
    assert_eq!(map_key_char('\u{f703}'), Some(Key::ArrowRight));
    assert_eq!(map_key_char('\u{f728}'), Some(Key::Delete));
    assert_eq!(map_key_char('\u{f729}'), Some(Key::Home));
    assert_eq!(map_key_char('\u{f72b}'), Some(Key::End));
    assert_eq!(map_key_char('\u{f72c}'), Some(Key::PageUp));
    assert_eq!(map_key_char('\u{f72d}'), Some(Key::PageDown));
    assert_eq!(map_key_char('\r'), Some(Key::Enter));
    assert_eq!(map_key_char('\u{1b}'), Some(Key::Escape));
    assert_eq!(map_key_char('\u{7f}'), Some(Key::Backspace));
    // charactersIgnoringModifiers keeps ⇧ applied: 'A' must fold to the
    // same key as 'a'.
    assert_eq!(map_key_char('A'), Some(Key::A));
    assert_eq!(map_key_char('a'), Some(Key::A));
    assert_eq!(map_key_char('7'), Some(Key::Num7));
    assert_eq!(map_key_char('é'), None);
}

#[test]
fn key_press_emits_key_then_text() {
    let mut input = InputState::new();
    let mut out = Vec::new();
    input.key("a", "a", true, false, &mut out);
    assert!(matches!(
        out[0],
        Event::Key {
            key: Key::A,
            pressed: true,
            repeat: false,
            ..
        }
    ));
    assert!(matches!(&out[1], Event::Text(t) if t == "a"));

    // Release: key event only, no text.
    out.clear();
    input.key("a", "a", false, false, &mut out);
    assert_eq!(out.len(), 1);
}

#[test]
fn shortcut_chords_and_function_keys_produce_no_text() {
    let mut input = InputState::new();
    let mut out = Vec::new();

    // ⌘C is a chord, not typing.
    input.set_modifier_flags(NS_FLAG_COMMAND);
    input.key("c", "c", true, false, &mut out);
    assert!(
        !out.iter().any(|e| matches!(e, Event::Text(_))),
        "⌘-chords must not emit Text"
    );

    // ⌥ composition IS typing on macOS (this is how å is entered).
    out.clear();
    input.set_modifier_flags(NS_FLAG_OPTION);
    input.key("a", "å", true, false, &mut out);
    assert!(out.iter().any(|e| matches!(e, Event::Text(t) if t == "å")));

    // Arrow keys arrive as private-use characters; they must never leak
    // into Text even though they are "printable" chars.
    out.clear();
    input.set_modifier_flags(0);
    input.key("\u{f700}", "\u{f700}", true, false, &mut out);
    assert!(!out.iter().any(|e| matches!(e, Event::Text(_))));
}
