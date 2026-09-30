//! P0 parity: the registry dispatch reproduces the pre-refactor global key
//! table (command-palette.md §9 P0, §10).
//!
//! `legacy_key_press_message` below is a verbatim copy of the hand-written
//! `update::key_press_message` match that the registry replaced. It is kept
//! frozen here as the oracle, so the parity check still pins the old
//! behaviour now the live function is gone. (Its faithfulness was checked
//! against the live function over the whole key space in the commit that
//! introduced it, before the function was removed.)
//!
//! For every chord the legacy table handled, the new path — the chord looked
//! up in `BindingMap::resonance_default()`, then the command's typing gate
//! and message — yields the same message behind the same gate. The only
//! intended differences are the ones `intended()` accepts:
//!
//! * Esc (exit Performance mode) is now typing-gated, like every bare key
//!   (§3.2). A focused text field consumes Esc itself, so the gate never
//!   changes what the user sees.
//! * The legacy match ignored modifiers it didn't test for (Alt+⌘S saved,
//!   Shift+F toggled Performance mode). The registry matches chords
//!   exactly, so those modifier-sloppy variants no longer fire.

use iced::keyboard::{self, key::Named, Key, Modifiers};
use resonance_app::commands::{BindingMap, CommandId, KeyChord, KeyGate};
use resonance_app::message::*;
use resonance_app::update::shortcuts::key_press_command;
use resonance_app::Resonance;

// ---------------------------------------------------------------------------
// The frozen oracle (copied from update.rs @ f71bdc10).
// ---------------------------------------------------------------------------

fn focus_gated(message: Message) -> Message {
    Message::Ui(UiMessage::RequestShortcut(Box::new(message)))
}

/// Verbatim copy of the pre-registry `update::key_press_message`, with the
/// two hand-rolled gates (`RequestPerformanceToggle`, `RequestMarkerNav`)
/// already written as what they resolved to (see [`Legacy`]).
fn legacy_key_press_message(key: keyboard::Key, modifiers: keyboard::Modifiers) -> Option<Legacy> {
    if modifiers.command() {
        match key {
            keyboard::Key::Character(ref c) if c.as_str() == "s" => {
                if modifiers.shift() {
                    Some(Legacy::Plain(Message::ProjectIo(ProjectIoMessage::SaveProjectAs)))
                } else {
                    Some(Legacy::Plain(Message::ProjectIo(ProjectIoMessage::SaveProject)))
                }
            }
            keyboard::Key::Character(ref c) if c.as_str() == "o" => {
                Some(Legacy::Plain(Message::ProjectIo(ProjectIoMessage::OpenProject)))
            }
            keyboard::Key::Character(ref c) if c.as_str() == "z" => {
                if modifiers.shift() {
                    Some(Legacy::of(focus_gated(Message::Redo)))
                } else {
                    Some(Legacy::of(focus_gated(Message::Undo)))
                }
            }
            keyboard::Key::Character(ref c) if c.as_str() == "y" => {
                Some(Legacy::of(focus_gated(Message::Redo)))
            }
            keyboard::Key::Character(ref c) if c.as_str() == "g" => {
                Some(Legacy::Plain(Message::Group(GroupMessage::CreateGroupFromSelection)))
            }
            keyboard::Key::Character(ref c) if c.as_str().eq_ignore_ascii_case("f") => {
                if modifiers.shift() {
                    Some(Legacy::Plain(Message::Freeze(FreezeMessage::FreezeAllTracks)))
                } else {
                    Some(Legacy::Plain(Message::Freeze(FreezeMessage::FreezeSelectedTracks)))
                }
            }
            _ => None,
        }
    } else {
        match key {
            keyboard::Key::Named(keyboard::key::Named::Enter) => Some(Legacy::of(focus_gated(
                Message::MidiEditor(MidiEditorMessage::OpenSelectedMidiClip),
            ))),
            // Was `UiMessage::RequestPerformanceToggle`: a focus probe that
            // toggled only when no text field was focused.
            keyboard::Key::Character(ref c) if c.as_str() == "f" || c.as_str() == "F" => {
                Some(Legacy::Gated(Message::Ui(UiMessage::TogglePerformanceMode)))
            }
            keyboard::Key::Named(keyboard::key::Named::Escape) => {
                Some(Legacy::Plain(Message::Ui(UiMessage::ExitPerformanceMode)))
            }
            // Was `UiMessage::RequestMarkerNav { forward }`: the same probe,
            // resolving to `JumpToNext` / `JumpToPrev`.
            keyboard::Key::Character(ref c) if c.as_str() == "." => {
                Some(Legacy::Gated(Message::Marker(MarkerMessage::JumpToNext)))
            }
            keyboard::Key::Character(ref c) if c.as_str() == "," => {
                Some(Legacy::Gated(Message::Marker(MarkerMessage::JumpToPrev)))
            }
            _ => None,
        }
    }
}

/// What a legacy key produced: a message dispatched directly, or one
/// behind the typing gate.
#[derive(Debug)]
enum Legacy {
    Plain(Message),
    Gated(Message),
}

impl Legacy {
    fn of(message: Message) -> Legacy {
        match message {
            Message::Ui(UiMessage::RequestShortcut(inner)) => Legacy::Gated(*inner),
            other => Legacy::Plain(other),
        }
    }

    /// `(typing-gated, message)` in a comparable form.
    fn key(&self) -> (bool, String) {
        match self {
            Legacy::Plain(m) => (false, format!("{m:?}")),
            Legacy::Gated(m) => (true, format!("{m:?}")),
        }
    }
}

// ---------------------------------------------------------------------------
// The key space both paths are compared over.
// ---------------------------------------------------------------------------

fn all_keys() -> Vec<Key> {
    let mut keys: Vec<Key> = "abcdefghijklmnopqrstuvwxyz0123456789.,;'[]=-/`\\"
        .chars()
        .map(|c| Key::Character(c.to_string().into()))
        .collect();
    for named in [
        Named::Enter,
        Named::Escape,
        Named::Space,
        Named::Tab,
        Named::Backspace,
        Named::Delete,
        Named::ArrowUp,
        Named::ArrowDown,
        Named::ArrowLeft,
        Named::ArrowRight,
        Named::Home,
        Named::End,
        Named::PageUp,
        Named::PageDown,
    ] {
        keys.push(Key::Named(named));
    }
    keys
}

fn all_modifiers() -> Vec<Modifiers> {
    let bits = [
        Modifiers::COMMAND,
        Modifiers::SHIFT,
        Modifiers::ALT,
        Modifiers::LOGO,
    ];
    (0u8..16)
        .map(|mask| {
            bits.iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .fold(Modifiers::empty(), |acc, (_, b)| acc | *b)
        })
        .collect()
}

/// The new path: chord → command → `(typing-gated, message)`.
fn registry(app: &Resonance, key: &Key, mods: Modifiers) -> Option<(CommandId, (bool, String))> {
    let command = key_press_command(&BindingMap::resonance_default(), key, mods)?;
    let message = command
        .to_message(app)
        .unwrap_or_else(|| panic!("{command:?} built no message"));
    // The dispatcher gates a bare chord whatever the command says.
    let chord = KeyChord::from_iced(key, mods)?;
    let bare = !chord.mods.cmd && !chord.mods.ctrl;
    let gated = bare || command.gate() == KeyGate::NotWhileTyping;
    Some((command, (gated, format!("{message:?}"))))
}

/// The intended differences (see the module docs).
fn intended(command: CommandId, legacy: (bool, String), new: (bool, String)) -> bool {
    command == CommandId::ExitPerformanceMode && !legacy.0 && new.0 && legacy.1 == new.1
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The modifiers the legacy match actually looked at for `key`: the ⌘
/// branch tested Shift only on S, Z and F; the bare branch tested nothing.
fn legacy_relevant(key: &Key, mods: Modifiers) -> Modifiers {
    if mods.command() {
        let shift_matters =
            matches!(key, Key::Character(c) if ["s", "z", "f"].contains(&c.as_str()));
        if shift_matters {
            mods & (Modifiers::COMMAND | Modifiers::SHIFT)
        } else {
            Modifiers::COMMAND
        }
    } else {
        Modifiers::empty()
    }
}

/// Every chord the legacy table handled on purpose (its exact modifiers)
/// yields the same message behind the same gate.
#[test]
fn every_legacy_chord_matches_the_registry() {
    let (app, _task) = Resonance::new_for_test();
    let mut checked = 0;
    for key in all_keys() {
        for mods in all_modifiers() {
            let Some(legacy) = legacy_key_press_message(key.clone(), mods) else {
                continue;
            };
            if legacy_relevant(&key, mods) != mods {
                continue;
            }
            let legacy = legacy.key();
            let (command, new) = registry(&app, &key, mods)
                .unwrap_or_else(|| panic!("{key:?}+{mods:?}: legacy {legacy:?}, registry nothing"));
            if new != legacy {
                assert!(
                    intended(command, legacy.clone(), new.clone()),
                    "{key:?}+{mods:?}: registry {new:?}, legacy {legacy:?}"
                );
            }
            checked += 1;
        }
    }
    // ⌘S ⇧⌘S ⌘O ⌘Z ⇧⌘Z ⌘Y ⌘G ⌘F ⇧⌘F ↵ F Esc . ,
    assert_eq!(checked, 14, "every legacy chord was exercised");
}

/// A legacy chord with a modifier the old match ignored (Alt+⌘S, Shift+F)
/// either no longer fires, still means the same thing, or was reassigned
/// on purpose by the §5 keymap.
#[test]
fn modifier_sloppy_legacy_chords_are_dropped_or_reassigned_on_purpose() {
    // §5.2: ⇧, / ⇧. jump between section starts; §1.1: global tracks
    // moved to ⌥⌘G.
    let reassigned = [
        CommandId::PrevSectionStart,
        CommandId::NextSectionStart,
        CommandId::ToggleGlobalTracks,
    ];
    let (app, _task) = Resonance::new_for_test();
    for key in all_keys() {
        for mods in all_modifiers() {
            let Some(legacy) = legacy_key_press_message(key.clone(), mods) else {
                continue;
            };
            if legacy_relevant(&key, mods) == mods {
                continue;
            }
            match registry(&app, &key, mods) {
                None => {}
                Some((_, new)) if new.1 == legacy.key().1 => {}
                Some((command, _)) => assert!(
                    reassigned.contains(&command),
                    "{key:?}+{mods:?} now runs {command:?}"
                ),
            }
        }
    }
}

/// `KeyChord::from_iced` folds the platform accelerator into `cmd`, so the
/// table the tests exercise is the table the app reads.
#[test]
fn the_app_starts_with_the_default_keymap() {
    let (app, _task) = Resonance::new_for_test();
    let defaults: Vec<(CommandId, KeyChord)> = BindingMap::resonance_default().iter().collect();
    let live: Vec<(CommandId, KeyChord)> = app.test_keymap().iter().collect();
    assert_eq!(defaults, live);
}
