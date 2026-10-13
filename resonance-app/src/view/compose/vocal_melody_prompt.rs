//! The "what about the vocal melodies?" question a chord or section-length
//! edit asks when it would rewrite a vocal lane's melody
//! (vocal-melody-on-chord-change). Same backdrop + centered-dialog pattern
//! as `confirm_delete_track`; Esc and the backdrop cancel, which undoes the
//! chord edit.

use iced::widget::{button, column, container, mouse_area, opaque, row, stack, text, Space};
use iced::{alignment, Element, Length};

use crate::compose::vocal_melody::{VocalMelodyChoice, VocalMelodyMsg, VocalMelodyPrompt};
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::theme::{self, fa};

fn msg(m: VocalMelodyMsg) -> Message {
    Message::Compose(ComposeMessage::VocalMelody(m))
}

/// The open question as a root overlay.
pub(crate) fn view(r: &crate::Resonance) -> Element<'_, Message> {
    let Some(prompt) = r.compose.vocal_melody.prompt.as_ref() else {
        return Space::new().into();
    };
    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.6,
                ))),
                ..Default::default()
            }),
    )
    .on_press(msg(VocalMelodyMsg::Cancel));

    let centered = container(opaque(dialog(prompt)))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);
    stack![backdrop, centered].into()
}

fn dialog(prompt: &VocalMelodyPrompt) -> Element<'_, Message> {
    let lanes = prompt.rows.len();
    let title = text("Keep the vocal melody?")
        .size(20)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1);
    let explanation = text(format!(
        "This change rewrites the chords under {} vocal lane{}. Choose what happens to each \
         melody \u{2014} the chord change and your choice undo as one step.",
        lanes,
        if lanes == 1 { "" } else { "s" }
    ))
    .size(13)
    .color(theme::TEXT_2);

    let mut rows = column![].spacing(8);
    for (index, row_state) in prompt.rows.iter().enumerate() {
        let origin = if row_state.hand_edited {
            "Notes edited by hand"
        } else {
            "Generated, untouched"
        };
        let label = column![
            text(format!("{} \u{00B7} {}", row_state.section_name, row_state.track_name))
                .size(13)
                .color(theme::TEXT_1),
            text(origin).size(11).color(theme::TEXT_3),
        ]
        .spacing(2);
        rows = rows.push(
            row![
                label,
                Space::new().width(Length::Fill),
                segmented(Some(row_state.choice), move |choice| {
                    VocalMelodyMsg::SetRowChoice { row: index, choice }
                }),
            ]
            .spacing(12)
            .align_y(alignment::Vertical::Center),
        );
    }

    let apply_all = row![
        text("Apply to all lanes").size(12).color(theme::TEXT_2),
        Space::new().width(Length::Fill),
        segmented(prompt.uniform_choice(), VocalMelodyMsg::SetAllChoices),
    ]
    .spacing(12)
    .align_y(alignment::Vertical::Center);

    let uniform = prompt.uniform_choice().is_some();
    let remember_icon = if prompt.remember {
        fa::CIRCLE
    } else {
        fa::CIRCLE_HOLLOW
    };
    let remember = button(
        row![
            theme::icon(remember_icon).size(13).color(if prompt.remember {
                theme::ACCENT
            } else {
                theme::TEXT_3
            }),
            text("Remember for this project").size(13).color(if uniform {
                theme::TEXT_1
            } else {
                theme::TEXT_3
            }),
        ]
        .spacing(10)
        .align_y(alignment::Vertical::Center),
    )
    .on_press_maybe(uniform.then(|| msg(VocalMelodyMsg::ToggleRemember)))
    .padding([6, 10])
    .style(|_theme, status| theme::ghost_button_style(status));
    let remember_hint = text(if uniform {
        "Later chord changes then skip this question. Settings \u{203A} Compose asks again."
    } else {
        "Give every lane the same choice to remember it."
    })
    .size(11)
    .color(theme::TEXT_3);

    let cancel = button(text("Cancel chord change").size(13).color(theme::TEXT_1))
        .on_press(msg(VocalMelodyMsg::Cancel))
        .padding([8, 18])
        .style(|_theme, status| theme::ghost_button_style(status));
    let apply = button(text("Apply").size(13).font(theme::UI_FONT_SEMIBOLD))
        .on_press(msg(VocalMelodyMsg::Apply))
        .padding([8, 18])
        .style(|_theme, status| theme::primary_button_style(status));
    let buttons = row![Space::new().width(Length::Fill), cancel, apply]
        .spacing(8)
        .align_y(alignment::Vertical::Center);

    let content = column![
        title,
        Space::new().height(8),
        explanation,
        Space::new().height(16),
        rows,
        Space::new().height(12),
        container(Space::new().height(1).width(Length::Fill)).style(theme::separator_bg),
        Space::new().height(12),
        apply_all,
        Space::new().height(8),
        remember,
        remember_hint,
        Space::new().height(18),
        buttons,
    ]
    .spacing(4)
    .padding(24)
    .width(600);

    container(content)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_XL.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Keep / Re-fit / Regenerate as three joined toggles, `selected` lit
/// (none when the rows differ).
fn segmented<'a>(
    selected: Option<VocalMelodyChoice>,
    on_pick: impl Fn(VocalMelodyChoice) -> VocalMelodyMsg,
) -> Element<'a, Message> {
    let mut seg = row![].spacing(4);
    for choice in VocalMelodyChoice::ALL {
        let active = selected == Some(choice);
        seg = seg.push(
            button(text(choice.label()).size(12))
                .on_press(msg(on_pick(choice)))
                .padding([4, 10])
                .style(move |_theme, status| {
                    theme::toggle_button_style(active, theme::ACCENT, true, status)
                }),
        );
    }
    seg.into()
}
