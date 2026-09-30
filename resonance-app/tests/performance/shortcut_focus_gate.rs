//! Global shortcuts that are also ordinary typing keys must not fire while
//! a text field has focus (UPD-11 / FU-C1).
//!
//! `keyboard::listen()` sees key presses a focused `text_input` already
//! consumed, so Enter (open the selected MIDI clip), `B` (momentary
//! reference audition) and Cmd-Z / Cmd-Y (project undo / redo) used to
//! act while the user was typing a track name. Registry shortcuts now go
//! through the focus probe (`crate::focus`) whenever their command is
//! `KeyGate::NotWhileTyping` (`ShortcutProbed` dispatches only when no text
//! field was focused); the held-`B` audition, which is not a registry
//! command, keeps `RequestShortcut` → `ShortcutResolved`.

use iced::keyboard::{self, key::Named, Key, Modifiers};
use resonance_app::commands::{BindingMap, CommandId, KeyGate};
use resonance_app::message::{Message, UiMessage};
use resonance_app::reference::ReferenceMessage;
use resonance_app::state::ViewMode;
use resonance_app::update::momentary_audition_message;
use resonance_app::update::shortcuts::key_press_command;
use resonance_app::Resonance;
use resonance_audio::types::ABSource;

fn gated_command(key: Key, mods: Modifiers) -> CommandId {
    let command = key_press_command(&BindingMap::resonance_default(), &key, mods)
        .unwrap_or_else(|| panic!("{key:?}+{mods:?} is unbound"));
    assert_eq!(command.gate(), KeyGate::NotWhileTyping, "{command:?} must be typing-gated");
    command
}

fn gated_inner(message: Option<Message>) -> Message {
    match message {
        Some(Message::Ui(UiMessage::RequestShortcut(inner))) => *inner,
        other => panic!("expected a focus-gated RequestShortcut, got {other:?}"),
    }
}

#[test]
fn enter_is_focus_gated() {
    assert_eq!(
        gated_command(Key::Named(Named::Enter), Modifiers::empty()),
        CommandId::OpenSelectedMidiClip
    );
}

#[test]
fn cmd_z_and_cmd_y_are_focus_gated() {
    let z = Key::Character("z".into());
    let y = Key::Character("y".into());
    assert_eq!(gated_command(z.clone(), Modifiers::COMMAND), CommandId::Undo);
    assert_eq!(gated_command(z, Modifiers::COMMAND | Modifiers::SHIFT), CommandId::Redo);
    assert_eq!(gated_command(y, Modifiers::COMMAND), CommandId::Redo);
}

#[test]
fn b_press_is_focus_gated() {
    let press = keyboard::Event::KeyPressed {
        key: Key::Character("b".into()),
        modified_key: Key::Character("b".into()),
        physical_key: keyboard::key::Physical::Code(keyboard::key::Code::KeyB),
        location: keyboard::Location::Standard,
        modifiers: Modifiers::empty(),
        text: Some("b".into()),
        repeat: false,
    };
    assert!(matches!(
        gated_inner(momentary_audition_message(press)),
        Message::Reference(ReferenceMessage::MomentaryAudition(true))
    ));
}

/// The resolution half dispatches the wrapped message only when no text
/// field held focus.
#[test]
fn a_resolved_shortcut_runs_only_when_not_editing() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_view_mode(ViewMode::Compose);
    let toggle = || Box::new(Message::Ui(UiMessage::TogglePerformanceMode));

    let _ = app.update(Message::Ui(UiMessage::ShortcutResolved {
        message: toggle(),
        editing: true,
    }));
    assert_eq!(app.test_view_mode(), ViewMode::Compose, "typed into a field: no-op");

    let _ = app.update(Message::Ui(UiMessage::ShortcutResolved {
        message: toggle(),
        editing: false,
    }));
    assert_eq!(app.test_view_mode(), ViewMode::Performance);
}

/// The `B` release is not gated (it cannot know whether its press was),
/// so a release whose press was suppressed must not touch the A/B source:
/// it used to "restore" to the default and knock a manually chosen
/// reference source back to the mix.
#[test]
fn a_release_without_a_press_leaves_the_source_alone() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    let _ = app.update(Message::Reference(ReferenceMessage::ToggleAbSource));
    assert_eq!(app.test_reference().monitor.ab_source, ABSource::Reference);

    let _ = app.update(Message::Reference(ReferenceMessage::MomentaryAudition(false)));
    assert_eq!(app.test_reference().monitor.ab_source, ABSource::Reference);
}
