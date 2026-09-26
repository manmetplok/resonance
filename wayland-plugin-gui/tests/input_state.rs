//! Headless coverage for the pointer-button bookkeeping in `InputState`.
//!
//! A window hidden with a mouse button held never sees that button's
//! release: input is dropped while hidden and an unmapped surface gets
//! none. Unless the show that follows releases it, egui keeps the button
//! down — a slider drag that never ends (FU-M1c).

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
