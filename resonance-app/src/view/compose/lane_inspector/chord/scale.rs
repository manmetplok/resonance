//! Scale block — compact root + mode pickers.

use iced::widget::{button, column, pick_list, text, Space};
use iced::{Element, Length};

use resonance_music_theory::{Mode, PitchClass, Scale};

use crate::compose::{ComposeMessage, SectionDefinitionState};
use crate::message::*;
use crate::theme;

use super::{field_label, section_header};

pub(in crate::view::compose::lane_inspector) fn scale_block<'a>(
    definition: &'a SectionDefinitionState,
) -> Element<'a, Message> {
    let definition_id = definition.id;
    let current = definition.scale;

    let current_label: Element<'a, Message> = match current {
        Some(scale) => text(scale.to_string())
            .size(15)
            .font(theme::SERIF_ITALIC_FONT)
            .color(theme::ACCENT_SOFT)
            .into(),
        None => text("(no scale set)")
            .size(13)
            .font(theme::SERIF_ITALIC_FONT)
            .color(theme::TEXT_3)
            .into(),
    };

    let roots: Vec<PitchClass> = PitchClass::ALL.to_vec();
    let modes: Vec<Mode> = Mode::ALL.to_vec();
    let current_root = current.map(|s| s.root).unwrap_or(PitchClass::C);
    let current_mode = current.map(|s| s.mode).unwrap_or(Mode::Major);

    let root_picker = pick_list(roots, Some(current_root), move |root| {
        let mode = current.map(|s| s.mode).unwrap_or(Mode::Major);
        Message::Compose(ComposeMessage::SetSectionScale {
            definition_id,
            scale: Some(Scale::new(root, mode)),
        })
    })
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let mode_picker = pick_list(modes, Some(current_mode), move |mode| {
        let root = current.map(|s| s.root).unwrap_or(PitchClass::C);
        Message::Compose(ComposeMessage::SetSectionScale {
            definition_id,
            scale: Some(Scale::new(root, mode)),
        })
    })
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let clear_btn = button(text("Clear scale").size(11).color(theme::TEXT_3))
        .on_press(Message::Compose(ComposeMessage::SetSectionScale {
            definition_id,
            scale: None,
        }))
        .padding([5, 10])
        .width(Length::Fill)
        .style(|_theme, status| theme::ghost_button_style(status));

    column![
        section_header("Scale"),
        Space::new().height(8),
        current_label,
        Space::new().height(8),
        field_label("ROOT"),
        Space::new().height(4),
        root_picker,
        Space::new().height(8),
        field_label("MODE"),
        Space::new().height(4),
        mode_picker,
        Space::new().height(8),
        clear_btn,
    ]
    .spacing(0)
    .into()
}
