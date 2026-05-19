use iced::widget::{button, column, container, row, text, text_input, Space};
use iced::{alignment, Color, Element, Length};

use crate::compose::{
    ComposeMessage, ComposeState, EditSectionForm, NewSectionForm, SectionDefinitionState,
    SectionPlacementState,
};
use crate::message::{Message, ProjectIoMessage};
use crate::theme;

/// Fixed height of the Compose section bar. Tall enough for a three-line
/// chip (number / serif name / bar metadata) with breathing room.
const STRIP_HEIGHT: u16 = 76;

pub fn view(state: &ComposeState) -> Element<'_, Message> {
    let body: Element<'_, Message> = if let Some(form) = &state.edit_section_form {
        edit_form_row(form, state.selected_placement_id)
    } else if let Some(form) = &state.new_section_form {
        create_form_row(form)
    } else {
        chips_row(state)
    };

    container(body)
        .width(Length::Fill)
        .height(Length::Fixed(STRIP_HEIGHT as f32))
        .padding([8, 14])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

// ---------------------------------------------------------------------------
// Chips row (default state — no form open)
// ---------------------------------------------------------------------------

fn chips_row(state: &ComposeState) -> Element<'_, Message> {
    let sections_tag = container(
        text("SECTIONS")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
    )
    .padding([0, 4])
    .align_y(alignment::Vertical::Center)
    .height(Length::Fill);

    // Sections flex by length_bars; each chip carries its own intrinsic width
    // share via Length::FillPortion.
    let mut chips = row![].spacing(6).align_y(alignment::Vertical::Center);
    for (idx, placement) in state.placements.iter().enumerate() {
        let Some(def) = state.find_definition(placement.definition_id) else {
            continue;
        };
        let active = Some(placement.id) == state.selected_placement_id;
        chips = chips.push(section_chip(idx, placement, def, active));
    }

    let add_btn: Element<'_, Message> = button(
        row![
            text("+").size(13).color(theme::TEXT_3),
            text("Section").size(11).color(theme::TEXT_3),
        ]
        .spacing(5)
        .align_y(alignment::Vertical::Center),
    )
    .on_press(Message::Compose(ComposeMessage::OpenCreateSectionDialog))
    .padding([8, 12])
    .height(Length::Fill)
    .style(|_theme, status| {
        let bg = match status {
            button::Status::Hovered => theme::BG_2,
            button::Status::Pressed => theme::LINE_2,
            _ => Color::TRANSPARENT,
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: theme::TEXT_3,
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_LG.into(),
            },
            ..Default::default()
        }
    })
    .into();

    let mut tools = row![].spacing(4).align_y(alignment::Vertical::Center);
    if let Some(def) = selected_definition(state) {
        let edit_btn = button(text("Edit section").size(12))
            .on_press(Message::Compose(ComposeMessage::OpenEditSectionDialog {
                definition_id: def.id,
            }))
            .padding([6, 11])
            .style(|_theme, status| theme::ghost_button_style(status));
        tools = tools.push(edit_btn);
    }
    if !state.definitions.is_empty() {
        let export_btn = button(text("Export chords").size(12))
            .on_press(Message::ProjectIo(ProjectIoMessage::ExportChordSheet))
            .padding([6, 11])
            .style(|_theme, status| theme::ghost_button_style(status));
        tools = tools.push(export_btn);
    }

    row![
        sections_tag,
        Space::new().width(8),
        chips,
        Space::new().width(6),
        add_btn,
        Space::new().width(Length::Fill),
        tools,
    ]
    .spacing(0)
    .height(Length::Fill)
    .align_y(alignment::Vertical::Center)
    .into()
}

/// One section chip — numbered, italic-serif name, bar metadata, optional
/// EDITING pill on the active chip, with the section's color shown as a
/// small dot beside the name. Width flexes by `length_bars` so the chip
/// strip visually reflects the song layout.
fn section_chip<'a>(
    idx: usize,
    placement: &'a SectionPlacementState,
    definition: &'a SectionDefinitionState,
    active: bool,
) -> Element<'a, Message> {
    let placement_id = placement.id;
    let end_bar = placement.start_bar + definition.length_bars;
    let number = format!("{:02}", idx + 1);
    let bars_meta = format!(
        "{}\u{2013}{} \u{00b7} {} bars",
        placement.start_bar + 1,
        end_bar,
        definition.length_bars
    );

    let number_text = text(number)
        .size(10)
        .font(theme::MONO_FONT)
        .color(if active {
            theme::ACCENT_SOFT
        } else {
            theme::TEXT_3
        })
        .wrapping(iced::widget::text::Wrapping::None);

    let editing_pill: Element<'a, Message> = if active {
        container(
            text("EDITING")
                .size(8)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::ACCENT_SOFT),
        )
        .padding([1, 7])
        .style(theme::editing_pill_style)
        .into()
    } else {
        Space::new().width(0).into()
    };

    let top_row = row![number_text, Space::new().width(Length::Fill), editing_pill]
        .align_y(alignment::Vertical::Center)
        .spacing(6);

    let dot_color = Color::from_rgb(
        definition.color[0] as f32 / 255.0,
        definition.color[1] as f32 / 255.0,
        definition.color[2] as f32 / 255.0,
    );
    let color_dot = container(Space::new().width(Length::Fixed(6.0)).height(Length::Fixed(6.0)))
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(dot_color)),
            border: iced::Border {
                color: dot_color,
                width: 0.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        });

    let name_text = text(definition.name.clone())
        .size(14)
        .font(theme::SERIF_ITALIC_FONT)
        .color(if active {
            theme::TEXT_1
        } else {
            theme::TEXT_2
        })
        .wrapping(iced::widget::text::Wrapping::None);

    let name_row = row![color_dot, name_text]
        .align_y(alignment::Vertical::Center)
        .spacing(7);

    let meta_text = text(bars_meta)
        .size(10)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3)
        .wrapping(iced::widget::text::Wrapping::None);

    let content = column![top_row, name_row, meta_text]
        .spacing(3)
        .width(Length::Fill);

    let flex = definition.length_bars.max(1) as u16;
    button(content)
        .on_press(Message::Compose(ComposeMessage::SelectSectionPlacement {
            placement_id,
        }))
        .padding([6, 12])
        .width(Length::FillPortion(flex))
        .style(move |_theme, status| theme::section_chip_button_style(active, status))
        .into()
}

// ---------------------------------------------------------------------------
// Forms (replace the chips row when active)
// ---------------------------------------------------------------------------

fn create_form_row(form: &NewSectionForm) -> Element<'_, Message> {
    let name_input = text_input("Section name", &form.name)
        .on_input(|s| Message::Compose(ComposeMessage::SetNewSectionName(s)))
        .on_submit(Message::Compose(ComposeMessage::ConfirmCreateSection))
        .size(12)
        .padding([4, 6])
        .width(160);

    let length_input = text_input("Bars", &form.length_input)
        .on_input(|s| Message::Compose(ComposeMessage::SetNewSectionLength(s)))
        .on_submit(Message::Compose(ComposeMessage::ConfirmCreateSection))
        .size(12)
        .padding([4, 6])
        .width(64);

    let confirm_btn = button(text("Create").size(12))
        .on_press(Message::Compose(ComposeMessage::ConfirmCreateSection))
        .padding([6, 12])
        .style(|_theme, status| theme::primary_button_style(status));

    let cancel_btn = button(text("Cancel").size(12))
        .on_press(Message::Compose(ComposeMessage::CancelCreateSectionDialog))
        .padding([6, 12])
        .style(|_theme, status| theme::ghost_button_style(status));

    row![
        text("NEW SECTION")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::ACCENT_SOFT),
        Space::new().width(12),
        name_input,
        text("bars:").size(11).color(theme::TEXT_DIM),
        length_input,
        confirm_btn,
        cancel_btn,
        Space::new().width(Length::Fill),
    ]
    .spacing(8)
    .height(Length::Fill)
    .align_y(alignment::Vertical::Center)
    .into()
}

fn edit_form_row(
    form: &EditSectionForm,
    selected_placement_id: Option<u64>,
) -> Element<'_, Message> {
    let definition_id = form.definition_id;

    let name_input = text_input("Section name", &form.name)
        .on_input(|s| Message::Compose(ComposeMessage::SetEditSectionName(s)))
        .on_submit(Message::Compose(ComposeMessage::ConfirmEditSection))
        .size(12)
        .padding([4, 6])
        .width(160);

    let length_input = text_input("Bars", &form.length_input)
        .on_input(|s| Message::Compose(ComposeMessage::SetEditSectionLength(s)))
        .on_submit(Message::Compose(ComposeMessage::ConfirmEditSection))
        .size(12)
        .padding([4, 6])
        .width(64);

    let color_btn = button(text("Color").size(12))
        .on_press(Message::Compose(ComposeMessage::CycleSectionColor {
            definition_id,
        }))
        .padding([6, 11])
        .style(|_theme, status| theme::ghost_button_style(status));

    let place_again_btn = button(text("Place again").size(12))
        .on_press(Message::Compose(ComposeMessage::PlaceSection {
            definition_id,
            start_bar: 0,
        }))
        .padding([6, 11])
        .style(|_theme, status| theme::ghost_button_style(status));

    let delete_placement_btn = button(text("Delete here").size(12))
        .on_press_maybe(selected_placement_id.map(|placement_id| {
            Message::Compose(ComposeMessage::DeleteSectionPlacement { placement_id })
        }))
        .padding([6, 11])
        .style(|_theme, status| theme::ghost_button_style(status));

    let delete_definition_btn = button(text("Delete section").size(12))
        .on_press(Message::Compose(ComposeMessage::DeleteSectionDefinition {
            definition_id,
        }))
        .padding([6, 11])
        .style(|_theme, status| theme::destructive_button_style(status));

    let save_btn = button(text("Save").size(12))
        .on_press(Message::Compose(ComposeMessage::ConfirmEditSection))
        .padding([6, 12])
        .style(|_theme, status| theme::primary_button_style(status));

    let cancel_btn = button(text("Cancel").size(12))
        .on_press(Message::Compose(ComposeMessage::CancelEditSectionDialog))
        .padding([6, 11])
        .style(|_theme, status| theme::ghost_button_style(status));

    row![
        text("EDIT SECTION")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::ACCENT_SOFT),
        Space::new().width(12),
        name_input,
        text("bars:").size(11).color(theme::TEXT_DIM),
        length_input,
        color_btn,
        place_again_btn,
        delete_placement_btn,
        delete_definition_btn,
        save_btn,
        cancel_btn,
        Space::new().width(Length::Fill),
    ]
    .spacing(6)
    .height(Length::Fill)
    .align_y(alignment::Vertical::Center)
    .into()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn selected_definition(state: &ComposeState) -> Option<&SectionDefinitionState> {
    state
        .selected_placement()
        .and_then(|p| state.find_definition(p.definition_id))
}
