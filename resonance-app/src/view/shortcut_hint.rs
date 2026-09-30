//! Shortcut hints read from the registry (command-palette.md §8): a
//! tooltip naming a command and its chord, and the chord as inline text.
//! Nothing here hardcodes a chord.

use iced::widget::{container, text, tooltip};
use iced::Element;

use crate::commands::CommandId;
use crate::message::Message;
use crate::theme;
use crate::Resonance;

/// The chord bound to `command`, formatted for this platform (`⇧⌘S` /
/// `Ctrl+Shift+S`), if it has one.
pub(crate) fn chord_text(r: &Resonance, command: CommandId) -> Option<String> {
    r.ui.keymap.chord_for(command).map(|c| c.format_for_platform())
}

/// `Save (Ctrl+S)`, or just the name for an unbound command.
pub(crate) fn hint_text(r: &Resonance, command: CommandId) -> String {
    match chord_text(r, command) {
        Some(chord) => format!("{} ({chord})", command.display_name()),
        None => command.display_name().to_string(),
    }
}

/// Wrap `content` in a tooltip naming `command` and its chord.
pub(crate) fn with_hint<'a>(
    r: &Resonance,
    content: impl Into<Element<'a, Message>>,
    command: CommandId,
) -> Element<'a, Message> {
    let label = container(text(hint_text(r, command)).size(11).color(theme::TEXT_1))
        .padding([4, 8])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_3)),
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_XS.into(),
            },
            ..Default::default()
        });
    tooltip(content, label, tooltip::Position::Bottom)
        .gap(6)
        .into()
}
