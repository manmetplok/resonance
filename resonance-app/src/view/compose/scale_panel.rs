use iced::widget::{button, column, container, pick_list, row, text, Space};
use iced::{alignment, Element, Length};

use resonance_music_theory::{Mode, PitchClass, Scale};

use crate::compose::{ComposeMessage, SectionDefinitionState};
use crate::message::Message;
use crate::theme;

pub const PANEL_WIDTH: f32 = 220.0;

pub fn view<'a>(definition: &'a SectionDefinitionState) -> Element<'a, Message> {
    let definition_id = definition.id;
    let current = definition.scale;

    let heading = text("Scale").size(13).color(theme::TEXT);
    let current_label: Element<'a, Message> = match current {
        Some(scale) => text(scale.to_string()).size(14).color(theme::ACCENT).into(),
        None => text("(none)").size(14).color(theme::TEXT_DIM).into(),
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
    .padding([4, 6])
    .width(Length::Fill);

    let mode_picker = pick_list(modes, Some(current_mode), move |mode| {
        let root = current.map(|s| s.root).unwrap_or(PitchClass::C);
        Message::Compose(ComposeMessage::SetSectionScale {
            definition_id,
            scale: Some(Scale::new(root, mode)),
        })
    })
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let clear_btn = button(text("Clear scale").size(12))
        .on_press(Message::Compose(ComposeMessage::SetSectionScale {
            definition_id,
            scale: None,
        }))
        .padding([4, 10])
        .width(Length::Fill)
        .style(|_theme, status| theme::transport_button_style(status));

    let helper = text(
        "Selected scale is highlighted in the note editor so in-scale \
         pitches are easier to spot.",
    )
    .size(10)
    .color(theme::TEXT_DIM);

    let content = column![
        heading,
        current_label,
        Space::with_height(8),
        row![text("Root").size(11).color(theme::TEXT_DIM)]
            .align_y(alignment::Vertical::Center),
        root_picker,
        Space::with_height(6),
        row![text("Mode").size(11).color(theme::TEXT_DIM)]
            .align_y(alignment::Vertical::Center),
        mode_picker,
        Space::with_height(10),
        clear_btn,
        Space::with_height(12),
        helper,
    ]
    .spacing(4)
    .padding(12);

    container(content)
        .width(Length::Fixed(PANEL_WIDTH))
        .height(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::PANEL)),
            border: iced::Border {
                color: theme::SEPARATOR,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}
