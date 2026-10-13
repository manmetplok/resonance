//! The Voice & delivery card's Voice preset block
//! (`docs/features/compose/voice-presets.md`): a picker over the cached
//! preset list (built-ins best first, then the user's), the preset the lane
//! matches — or "Custom" — with its score as stars and its note, Save…,
//! and Rename… / Delete on a user preset. A preset on a bank that is not
//! installed says so and points at Library…; picking it still applies.

use iced::widget::{button, column, pick_list, row, text, text_input, Space};
use iced::{Element, Length};

use resonance_audio::types::TrackId;
use resonance_music_theory::VocalParams;

use super::super::common::dim_label;
use crate::message::*;
use crate::state::picks::VoicePresetChoice;
use crate::state::ui_caches::{ChoiceList, VocalRailContext};
use crate::state::voice_presets::PromptMode;
use crate::theme;

fn msg(m: VoicePresetMessage) -> Message {
    Message::VoicePreset(m)
}

fn small_button<'a>(label: &'a str, on_press: Message) -> Element<'a, Message> {
    button(text(label).size(11).color(theme::TEXT))
        .on_press(on_press)
        .style(|_theme, status| theme::ghost_button_style(status))
        .padding([4, 8])
        .into()
}

/// The score as five icon-font stars, the filled ones warm.
fn stars<'a>(score: u8) -> Element<'a, Message> {
    let mut r = row![].spacing(1);
    for i in 1..=5u8 {
        r = r.push(
            text(String::from(theme::fa::STAR))
                .font(theme::ICON_FONT)
                .size(9)
                .color(if i <= score {
                    theme::WARM
                } else {
                    theme::TEXT_4
                }),
        );
    }
    r.into()
}

pub(super) fn preset_block<'a>(
    definition_id: u64,
    track_id: TrackId,
    params: &VocalParams,
    vocal: &VocalRailContext,
) -> Element<'a, Message> {
    let matched: Option<VoicePresetChoice> = vocal
        .voice_presets
        .iter()
        .find(|c| c.settings.matches(params))
        .cloned();
    let picker = pick_list(
        ChoiceList::Cached(vocal.voice_presets.clone()),
        matched.clone(),
        move |choice: VoicePresetChoice| {
            msg(VoicePresetMessage::Pick {
                definition_id,
                track_id,
                id: choice.id.to_string(),
            })
        },
    )
    .placeholder("Custom")
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);
    let save = small_button(
        "Save\u{2026}",
        msg(VoicePresetMessage::OpenSave {
            definition_id,
            track_id,
        }),
    );
    // Save… sits on the label's row so the picker gets the card's full
    // width: `5/5 · Keiro Revenant — Aggressive tenor` needs it.
    let mut block = column![
        row![
            dim_label("Voice preset"),
            Space::new().width(Length::Fill),
            save
        ]
        .align_y(iced::alignment::Vertical::Center),
        Space::new().height(2),
        picker,
    ]
    .spacing(0);

    // What the matched preset is: its score and note, and for a user
    // preset its Rename / Delete.
    if let Some(choice) = &matched {
        if choice.score.is_some() || !choice.description.is_empty() {
            let mut about = row![].spacing(6).align_y(iced::alignment::Vertical::Center);
            if let Some(score) = choice.score {
                about = about.push(stars(score));
            }
            about = about.push(
                text(choice.description.to_string())
                    .size(10)
                    .font(theme::SERIF_ITALIC_FONT)
                    .color(theme::TEXT_3)
                    .width(Length::Fill),
            );
            block = block.push(Space::new().height(4)).push(about);
        }
        if !choice.builtin {
            let name = choice.name.to_string();
            let armed = vocal.preset_ui.confirm_delete.as_deref() == Some(name.as_str());
            let delete = button(
                text(if armed {
                    "Delete? Press again"
                } else {
                    "Delete"
                })
                .size(11)
                .color(if armed { theme::BAD } else { theme::TEXT }),
            )
            .on_press(msg(VoicePresetMessage::Delete(name.clone())))
            .style(|_theme, status| theme::ghost_button_style(status))
            .padding([4, 8]);
            block = block.push(Space::new().height(4)).push(
                row![
                    small_button(
                        "Rename\u{2026}",
                        msg(VoicePresetMessage::OpenRename {
                            definition_id,
                            track_id,
                            name,
                        }),
                    ),
                    delete,
                ]
                .spacing(4),
            );
        }
    }

    // A lane on a bank that is not installed: the preset (or the bank
    // picked by hand) still applies; say what to install.
    let bank = params.voicebank.as_str();
    if !vocal.voicebanks.iter().any(|c| &*c.id == bank) {
        block = block.push(Space::new().height(4)).push(
            text(format!(
                "{bank} is not installed \u{2014} Library\u{2026} downloads it; the lane renders once it is"
            ))
            .size(10)
            .font(theme::SERIF_ITALIC_FONT)
            .color(theme::WARM),
        );
    }

    if let Some(prompt) = &vocal.preset_ui.prompt {
        let taken = vocal
            .voice_presets
            .iter()
            .any(|c| !c.builtin && *c.name == *prompt.name.trim());
        let renaming_itself =
            matches!(&prompt.mode, PromptMode::Rename(from) if from.trim() == prompt.name.trim());
        let confirm_label = match (&prompt.mode, taken && !renaming_itself) {
            (PromptMode::Save, true) => "Overwrite",
            (PromptMode::Save, false) => "Save",
            (PromptMode::Rename(_), _) => "Rename",
        };
        let input = text_input("Preset name", &prompt.name)
            .on_input(|s| msg(VoicePresetMessage::NameChanged(s)))
            .on_submit(msg(VoicePresetMessage::Confirm))
            .size(12)
            .padding([4, 6])
            .width(Length::Fill);
        block = block.push(Space::new().height(6)).push(
            row![
                input,
                small_button(confirm_label, msg(VoicePresetMessage::Confirm)),
                small_button("Cancel", msg(VoicePresetMessage::Cancel)),
            ]
            .spacing(4)
            .align_y(iced::alignment::Vertical::Center),
        );
        if taken && !renaming_itself && prompt.mode == PromptMode::Save {
            block = block.push(Space::new().height(2)).push(
                text(format!(
                    "Replaces the voice preset \u{201C}{}\u{201D}",
                    prompt.name.trim()
                ))
                .size(10)
                .color(theme::WARM),
            );
        }
    }
    if let Some(error) = &vocal.preset_ui.error {
        block = block
            .push(Space::new().height(2))
            .push(text(error.clone()).size(10).color(theme::BAD));
    }
    block.into()
}
