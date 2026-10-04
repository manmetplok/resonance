//! Headless coverage for the pointer-button bookkeeping in `InputState`.
//!
//! A window hidden with a mouse button held never sees that button's
//! release: input is dropped while hidden and an unmapped surface gets
//! none. Unless the show that follows releases it, egui keeps the button
//! down — a slider drag that never ends (FU-M1c).

// `InputState` translates SCTK events, so it (and its re-export) only
// exists in the Linux build of the crate.
#![cfg(target_os = "linux")]

use egui::{Event, PointerButton};
use wayland_plugin_gui::InputState;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

fn releases(events: &[Event]) -> Vec<PointerButton> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::PointerButton {
                button,
                pressed: false,
                ..
            } => Some(*button),
            _ => None,
        })
        .collect()
}

#[test]
fn a_held_button_is_released_by_release_all() {
    let mut input = InputState::new();
    let mut out = Vec::new();
    input.pointer_button(BTN_LEFT, true, &mut out);
    input.pointer_button(BTN_RIGHT, true, &mut out);
    input.pointer_button(BTN_RIGHT, false, &mut out);
    out.clear();

    input.release_all(&mut out);
    assert_eq!(releases(&out), vec![PointerButton::Primary]);

    // Released once: a second call has nothing left to release.
    out.clear();
    input.release_all(&mut out);
    assert!(out.is_empty(), "{out:?}");
}

#[test]
fn release_all_with_nothing_held_is_silent() {
    let mut input = InputState::new();
    let mut out = Vec::new();
    input.release_all(&mut out);
    assert!(out.is_empty());
}

// ---------------------------------------------------------------------------
// Keyboard: client-side repeat flag, and release on focus loss (PUX-10).
// ---------------------------------------------------------------------------

use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym};

fn key_event(keysym: Keysym) -> KeyEvent {
    KeyEvent { time: 0, raw_code: 0, keysym, utf8: None }
}

fn key_events(events: &[Event]) -> Vec<(egui::Key, bool, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Key { key, pressed, repeat, .. } => Some((*key, *pressed, *repeat)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_real_press_and_release_are_never_marked_repeat() {
    let mut input = InputState::new();
    let mut out = Vec::new();
    input.process_key(&key_event(Keysym::a), true, false, &mut out);
    input.process_key(&key_event(Keysym::a), false, false, &mut out);
    assert_eq!(
        key_events(&out),
        vec![(egui::Key::A, true, false), (egui::Key::A, false, false)]
    );
}

#[test]
fn a_repeated_key_down_is_marked_repeat() {
    let mut input = InputState::new();
    let mut out = Vec::new();
    input.process_key(&key_event(Keysym::a), true, false, &mut out);
    out.clear();
    input.process_key(&key_event(Keysym::a), true, true, &mut out);
    assert_eq!(key_events(&out), vec![(egui::Key::A, true, true)]);
}

#[test]
fn a_held_key_is_released_when_focus_leaves() {
    let mut input = InputState::new();
    let mut out = Vec::new();
    input.process_key(&key_event(Keysym::Left), true, false, &mut out);
    input.process_key(&key_event(Keysym::a), true, false, &mut out);
    out.clear();

    input.release_held_keys(&mut out);
    let released: Vec<_> = key_events(&out)
        .into_iter()
        .filter(|(_, pressed, _)| !pressed)
        .map(|(key, _, _)| key)
        .collect();
    assert_eq!(released, vec![egui::Key::ArrowLeft, egui::Key::A]);
    assert!(
        out.iter().any(|e| matches!(e, Event::WindowFocused(false))),
        "expected a WindowFocused(false) event; got {out:?}"
    );

    // Released once: a second leave has nothing left to release.
    out.clear();
    input.release_held_keys(&mut out);
    assert!(key_events(&out).is_empty(), "no key should be released twice: {out:?}");
}

#[test]
fn release_held_keys_with_nothing_held_still_reports_focus_lost() {
    let mut input = InputState::new();
    let mut out = Vec::new();
    input.release_held_keys(&mut out);
    assert!(key_events(&out).is_empty());
    assert!(matches!(out.as_slice(), [Event::WindowFocused(false)]));
}
