//! Global keyboard dispatch through the command registry
//! (command-palette.md §3.2, §4.1).
//!
//! The keyboard subscription can't read state, so it forwards every key
//! press as a [`UiMessage::ShortcutKey`] and the reducer here does the
//! rest, in this order:
//!
//! 1. a key a widget already consumed (a focused text field, a canvas that
//!    owns the keys) is dropped — except Esc, which first cancels an open
//!    mixer inline rename, then closes one layer of the inspector CHAIN's
//!    transient state (drag, preset prompt, replace mode, popovers),
//!    whose fields may have captured it;
//! 2. Esc resolves to closing the topmost root overlay (or, with none up,
//!    the generic plugin window) before it means anything else;
//! 3. while a modal root overlay shows, only ⌘/Ctrl chords dispatch;
//! 4. the chord is looked up in the active [`BindingMap`] (global scope);
//! 5. key repeat is dropped unless the command wants it;
//! 6. [`run_shortcut`]: availability, then the typing gate, then dispatch.

use iced::Task;

use crate::commands::{Available, BindingMap, CommandId, KeyChord, KeyGate, NamedKey, Scope};
use crate::message::{Message, UiMessage};
use crate::Resonance;

/// The command a global key press maps to under `bindings`, if any. A
/// thin, state-free seam so the chord table is testable without a live
/// keyboard subscription.
pub fn key_press_command(
    bindings: &BindingMap,
    key: &iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> Option<CommandId> {
    let chord = KeyChord::from_iced(key, modifiers)?;
    bindings.command_for(Scope::Global, chord)
}

/// The keyboard-subscription mapper: every key press becomes a
/// [`UiMessage::ShortcutKey`], carrying whether a widget captured it.
pub(crate) fn key_event_message(
    event: iced::Event,
    status: iced::event::Status,
) -> Option<Message> {
    let iced::Event::Keyboard(event) = event else {
        return None;
    };
    match event {
        iced::keyboard::Event::KeyPressed {
            key,
            modifiers,
            repeat,
            ..
        } => {
            let chord = KeyChord::from_iced(&key, modifiers)?;
            Some(Message::Ui(UiMessage::ShortcutKey {
                chord,
                repeat,
                captured: status == iced::event::Status::Captured,
            }))
        }
        // Track the live modifier state so a track-header click can tell
        // a plain select from an additive (Cmd/Shift) one — the mouse
        // press itself carries no modifiers (todo #684).
        iced::keyboard::Event::ModifiersChanged(mods) => {
            Some(Message::Ui(UiMessage::ModifiersChanged(mods)))
        }
        _ => None,
    }
}

fn is_plain_escape(chord: KeyChord) -> bool {
    chord == KeyChord::named(NamedKey::Escape, crate::commands::Mods::NONE)
}

/// Whether `chord` carries the ⌘ (macOS) / Ctrl (elsewhere) accelerator.
fn has_accelerator(chord: KeyChord) -> bool {
    chord.mods.cmd || chord.mods.ctrl
}

/// Reduce one global key press (see the module docs for the order).
pub(crate) fn handle_key(
    r: &mut Resonance,
    chord: KeyChord,
    repeat: bool,
    captured: bool,
) -> Task<Message> {
    // A Keyboard-panel rebind listening for its chord takes every key.
    if let Some(task) = crate::update::keymap::key(r, chord, repeat) {
        return task;
    }
    // The open palette takes Esc, ↑/↓ and ↵ first — Esc even though its
    // query field captured it.
    if r.ui.palette.is_some() {
        if let Some(task) = crate::update::palette::key(r, chord, captured) {
            return task;
        }
    }
    // Esc drops an armed preset drag, whatever else is open.
    if r.presets.dragging.is_some() && is_plain_escape(chord) {
        r.presets.dragging = None;
        return Task::none();
    }
    // The preset browser takes Esc (revert), ↑/↓ (audition) and ↵ (keep)
    // first, its search field focused or not.
    if r.presets.host_browser.is_some() {
        if let Some(task) = crate::update::plugin_preset_ui::key(r, chord) {
            return task;
        }
    }
    // Esc cancels an open inline rename. Its field captured the key (a
    // focused `text_input` takes Esc and unfocuses itself), so this sits
    // before the drop below. An Esc the field did not capture closes a
    // rename left open too, but goes on to mean what Esc means.
    if is_plain_escape(chord) && !repeat && crate::update::inline_rename::escape(r, captured) {
        return Task::none();
    }
    // Esc closes the inspector CHAIN's transient state, one layer per
    // press: a drag, the preset-name prompt (whose field captured the
    // key), replace mode, then a slot menu / colour palette. Not under a
    // modal overlay, which Esc closes first, and — the prompt aside —
    // not when a widget captured the key (a pick_list closing its
    // dropdown spent it).
    if is_plain_escape(chord)
        && !repeat
        && r.modal_overlay().is_none()
        && crate::update::chain_ui::escape(r, captured)
    {
        return Task::none();
    }
    // A focused text field or a key-owning canvas already acted on it.
    if captured {
        return Task::none();
    }
    let modal = r.modal_overlay();
    if is_plain_escape(chord) && !repeat && modal.is_some() {
        return dismiss_overlay(r);
    }
    // The generic plugin window is non-modal, but Esc closes it before
    // the key means anything else (mixer-cleanup.md §4).
    if is_plain_escape(chord)
        && !repeat
        && modal.is_none()
        && crate::update::plugin_window::escape(r)
    {
        return Task::none();
    }
    // An armed MIDI Learn is non-modal too (the user is reaching for the
    // controller); Esc disarms it.
    if is_plain_escape(chord)
        && !repeat
        && modal.is_none()
        && crate::update::midi_map::escape(r)
    {
        return Task::none();
    }
    if modal.is_some() && !has_accelerator(chord) {
        return Task::none();
    }
    let Some(command) = r.ui.keymap.command_for(Scope::Global, chord) else {
        return Task::none();
    };
    if repeat && !command.repeat() {
        return Task::none();
    }
    // A bare key is typing-gated whatever the command says, so a rebound
    // or preset keymap can't put an ungated letter on a command.
    run_shortcut(r, command, !has_accelerator(chord))
}

/// How the typing gate learns whether a text field holds focus; held by
/// `UiTransientState`, so defined in `state` (ARCH2-05).
pub use crate::state::TypingProbe;

/// Run `command` as a keyboard shortcut: dropped when unavailable, and
/// probed through the typing gate when it is [`KeyGate::NotWhileTyping`]
/// (or `force_gate` is set, for a bare-key chord).
pub(crate) fn run_shortcut(r: &mut Resonance, command: CommandId, force_gate: bool) -> Task<Message> {
    if let Available::No(_) = command.availability(r) {
        return Task::none();
    }
    let gate = if force_gate { KeyGate::NotWhileTyping } else { command.gate() };
    match gate {
        KeyGate::Always => execute(r, command),
        KeyGate::NotWhileTyping => match r.ui.typing_probe {
            TypingProbe::Live => crate::focus::any_text_input_focused().map(move |editing| {
                Message::Ui(UiMessage::ShortcutProbed { command, editing })
            }),
            TypingProbe::Assume { editing } => {
                r.update(Message::Ui(UiMessage::ShortcutProbed { command, editing }))
            }
        },
    }
}

/// The typing gate's answer for a `NotWhileTyping` shortcut.
pub(crate) fn probed(r: &mut Resonance, command: CommandId, editing: bool) -> Task<Message> {
    if editing {
        return Task::none();
    }
    execute(r, command)
}

/// Dispatch `command`'s message now. Availability is re-checked because a
/// typing probe resolves a frame after the key press; the message
/// re-enters `update()`, so it meets every gate and is classified for undo
/// on its own.
pub(crate) fn execute(r: &mut Resonance, command: CommandId) -> Task<Message> {
    if let Available::No(_) = command.availability(r) {
        return Task::none();
    }
    let Some(message) = command.build(r) else {
        return Task::none();
    };
    // Only a command that got past the startup / bounce / freeze gates
    // counts as run.
    let passes = !r.gates_message(&message);
    let task = r.update(message);
    if passes && command.records_recent() {
        crate::palette::record_recent(r, command);
    }
    task
}

/// Close the topmost modal root overlay, as its backdrop click would.
pub(crate) fn dismiss_overlay(r: &mut Resonance) -> Task<Message> {
    match r.modal_overlay().and_then(|o| o.dismiss_message(r)) {
        Some(message) => r.update(message),
        None => Task::none(),
    }
}
