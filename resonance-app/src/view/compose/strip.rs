use iced::widget::{button, container, row, text, text_input, Space};
use iced::{alignment, Element, Length};

use crate::compose::{ComposeMessage, ComposeState, EditSectionForm, NewSectionForm};
use crate::message::Message;
use crate::theme;

pub fn view(state: &ComposeState) -> Element<'_, Message> {
    let mut items: Vec<Element<'_, Message>> = Vec::new();
    items.push(Space::with_width(8).into());

    for placement in &state.placements {
        let Some(def) = state.find_definition(placement.definition_id) else {
            continue;
        };
        let active = Some(placement.id) == state.selected_placement_id;
        let end_bar = placement.start_bar + def.length_bars;
        let label = format!(
            "{}  {}\u{2013}{}",
            def.name,
            placement.start_bar + 1,
            end_bar
        );
        let color = def.color;
        let btn = button(text(label).size(12))
            .on_press(Message::Compose(ComposeMessage::SelectSectionPlacement {
                placement_id: placement.id,
            }))
            .padding([4, 10])
            .style(move |_theme, status| theme::section_button_style(active, color, status));
        items.push(btn.into());
    }

    // Order of precedence for the trailing control:
    //   1. edit form (takes priority, because it's the active UI)
    //   2. create form
    //   3. "+" / "Edit" trigger buttons
    if let Some(form) = &state.edit_section_form {
        items.push(edit_form(form, state.selected_placement_id));
    } else if let Some(form) = &state.new_section_form {
        items.push(create_form(form));
    } else {
        let add_btn = button(text("+").size(14))
            .on_press(Message::Compose(ComposeMessage::OpenCreateSectionDialog))
            .padding([4, 10])
            .style(|_theme, status| theme::transport_button_style(status));
        items.push(add_btn.into());

        // Edit trigger shown only when a placement is selected
        if let Some(def) = selected_definition(state) {
            let edit_btn = button(text("Edit section").size(12))
                .on_press(Message::Compose(ComposeMessage::OpenEditSectionDialog {
                    definition_id: def.id,
                }))
                .padding([4, 10])
                .style(|_theme, status| theme::transport_button_style(status));
            items.push(edit_btn.into());
        }
    }

    items.push(Space::with_width(Length::Fill).into());

    container(
        row(items)
            .spacing(6)
            .align_y(alignment::Vertical::Center)
            .height(40),
    )
    .width(Length::Fill)
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

fn selected_definition<'a>(
    state: &'a ComposeState,
) -> Option<&'a crate::compose::SectionDefinitionState> {
    state
        .selected_placement()
        .and_then(|p| state.find_definition(p.definition_id))
}

fn create_form<'a>(form: &'a NewSectionForm) -> Element<'a, Message> {
    let name_input = text_input("Section name", &form.name)
        .on_input(|s| Message::Compose(ComposeMessage::SetNewSectionName(s)))
        .on_submit(Message::Compose(ComposeMessage::ConfirmCreateSection))
        .size(12)
        .padding([4, 6])
        .width(140);

    let length_input = text_input("Bars", &form.length_input)
        .on_input(|s| Message::Compose(ComposeMessage::SetNewSectionLength(s)))
        .on_submit(Message::Compose(ComposeMessage::ConfirmCreateSection))
        .size(12)
        .padding([4, 6])
        .width(56);

    let confirm_btn = button(text("Create").size(12))
        .on_press(Message::Compose(ComposeMessage::ConfirmCreateSection))
        .padding([4, 10])
        .style(|_theme, status| theme::tab_button_style(true, status));

    let cancel_btn = button(text("Cancel").size(12))
        .on_press(Message::Compose(ComposeMessage::CancelCreateSectionDialog))
        .padding([4, 10])
        .style(|_theme, status| theme::transport_button_style(status));

    row![
        name_input,
        text("bars:").size(11).color(theme::TEXT_DIM),
        length_input,
        confirm_btn,
        cancel_btn,
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center)
    .into()
}

fn edit_form<'a>(
    form: &'a EditSectionForm,
    selected_placement_id: Option<u64>,
) -> Element<'a, Message> {
    let definition_id = form.definition_id;

    let name_input = text_input("Section name", &form.name)
        .on_input(|s| Message::Compose(ComposeMessage::SetEditSectionName(s)))
        .on_submit(Message::Compose(ComposeMessage::ConfirmEditSection))
        .size(12)
        .padding([4, 6])
        .width(140);

    let length_input = text_input("Bars", &form.length_input)
        .on_input(|s| Message::Compose(ComposeMessage::SetEditSectionLength(s)))
        .on_submit(Message::Compose(ComposeMessage::ConfirmEditSection))
        .size(12)
        .padding([4, 6])
        .width(56);

    let color_btn = button(text("Color").size(12))
        .on_press(Message::Compose(ComposeMessage::CycleSectionColor {
            definition_id,
        }))
        .padding([4, 10])
        .style(|_theme, status| theme::transport_button_style(status));

    let place_again_btn = button(text("Place again").size(12))
        .on_press(Message::Compose(ComposeMessage::PlaceSection {
            definition_id,
            // start_bar 0 is a hint — the handler will reject if it overlaps,
            // but the section-edit flow usually resolves this via dragging in
            // a follow-up. For now the user is expected to pick a bar that
            // doesn't collide; overlap errors surface in last_error.
            start_bar: 0,
        }))
        .padding([4, 10])
        .style(|_theme, status| theme::transport_button_style(status));

    let delete_placement_btn = button(text("Delete here").size(12))
        .on_press_maybe(selected_placement_id.map(|placement_id| {
            Message::Compose(ComposeMessage::DeleteSectionPlacement { placement_id })
        }))
        .padding([4, 10])
        .style(|_theme, status| theme::transport_button_style(status));

    let delete_definition_btn = button(text("Delete section").size(12))
        .on_press(Message::Compose(ComposeMessage::DeleteSectionDefinition {
            definition_id,
        }))
        .padding([4, 10])
        .style(|_theme, status| theme::record_armed_button_style(status));

    let save_btn = button(text("Save").size(12))
        .on_press(Message::Compose(ComposeMessage::ConfirmEditSection))
        .padding([4, 10])
        .style(|_theme, status| theme::tab_button_style(true, status));

    let cancel_btn = button(text("Cancel").size(12))
        .on_press(Message::Compose(ComposeMessage::CancelEditSectionDialog))
        .padding([4, 10])
        .style(|_theme, status| theme::transport_button_style(status));

    row![
        name_input,
        text("bars:").size(11).color(theme::TEXT_DIM),
        length_input,
        color_btn,
        place_again_btn,
        delete_placement_btn,
        delete_definition_btn,
        save_btn,
        cancel_btn,
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center)
    .into()
}
