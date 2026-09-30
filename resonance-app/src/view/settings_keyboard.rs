//! Preferences › Keyboard (command-palette.md §8, the epic #58 prototype's
//! rebinding panel): the DAW preset and what it leaves unbound, a filter,
//! every command grouped by category with all its chords, an "edited" pill
//! where it differs from the preset, Rebind / Reset, and a conflict banner
//! naming the current owner (whose chord takes the conflict ring).
//!
//! The command list is `lazy`, keyed on everything it shows, so it is
//! rebuilt only when the filter, the keymap or the rebind state changes.

use iced::widget::{button, column, container, lazy, pick_list, row, scrollable, text, text_input, Space};
use iced::{alignment, Element, Length};

use crate::commands::{fuzzy_match, CommandCategory, CommandId, KeyChord, KeymapPreset, Platform};
use crate::message::{Message, UiMessage};
use crate::theme::{self, KeycapTone};
use crate::update::keymap::{preset_of, KeymapConflict, KeymapMsg};
use crate::Resonance;

fn msg(m: KeymapMsg) -> Message {
    Message::Ui(UiMessage::Keymap(m))
}

fn chord_caps(chord: KeyChord, tone: KeycapTone) -> Element<'static, Message> {
    let caps = chord.keycaps(Platform::current());
    let labels: Vec<&str> = caps.iter().map(String::as_str).collect();
    theme::keycap_row(&labels, tone).into()
}

fn small_button(label: &'static str, on_press: Message) -> Element<'static, Message> {
    button(text(label).size(11).color(theme::ACCENT_SOFT))
        .on_press(on_press)
        .padding([4, 9])
        .style(|_theme, status| theme::ghost_button_style(status))
        .into()
}

/// The panel's width, shared with the Settings card so it doesn't jump.
pub(crate) const PAGE_WIDTH: f32 = 700.0;

pub(crate) fn view_keyboard_page(r: &Resonance) -> Element<'_, Message> {
    let editor = &r.ui.keymap_editor;
    let preset = preset_of(&r.settings.keymap);

    let title = text("Keyboard")
        .size(22)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1);
    let preset_picker = pick_list(KeymapPreset::ALL, Some(preset), |p| msg(KeymapMsg::SetPreset(p)))
        .text_size(12)
        .width(180);
    let reset_all = button(text("Reset all").size(12).color(theme::TEXT_2))
        .on_press(msg(KeymapMsg::ResetAll))
        .padding([7, 12])
        .style(|_theme, status| theme::ghost_button_style(status));
    let header = row![
        title,
        Space::new().width(Length::Fill),
        text("PRESET").size(10).font(theme::UI_FONT_SEMIBOLD).color(theme::TEXT_3),
        preset_picker,
        reset_all,
    ]
    .spacing(10)
    .align_y(alignment::Vertical::Center);

    let filter = text_input("Filter shortcuts…", &editor.filter)
        .on_input(|s| msg(KeymapMsg::Filter(s)))
        .size(13)
        .padding([8, 12]);

    let mut page = column![header, filter].spacing(12);

    if !editor.unbound_by_preset.is_empty() {
        let names: Vec<&str> = editor.unbound_by_preset.iter().map(|c| c.display_name()).collect();
        page = page.push(
            container(
                column![
                    text(format!("{} takes some default chords", preset.display_name()))
                        .size(12)
                        .color(theme::TEXT_1),
                    text(format!("Left unbound: {}", names.join(", ")))
                        .size(11)
                        .color(theme::TEXT_3),
                ]
                .spacing(2),
            )
            .padding([10, 14])
            .width(Length::Fill)
            .style(theme::edited_pill_style),
        );
    }

    if let Some(conflict) = editor.conflict {
        let banner = container(
            row![
                column![
                    text(format!(
                        "{} is already {}",
                        conflict.chord.format_for_platform(),
                        conflict.owner.display_name()
                    ))
                    .size(12)
                    .color(theme::TEXT_1),
                    text(format!(
                        "Replacing gives it to {} and takes it from {}.",
                        conflict.command.display_name(),
                        conflict.owner.display_name()
                    ))
                    .size(11)
                    .color(theme::TEXT_3),
                ]
                .spacing(2),
                Space::new().width(Length::Fill),
                small_button("Cancel", msg(KeymapMsg::CancelRebind)),
                small_button("Replace", msg(KeymapMsg::ConfirmConflict)),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
        )
        .padding([10, 14])
        .style(theme::conflict_ring_style);
        page = page.push(banner);
    }

    let key = (
        editor.filter.clone(),
        editor.capturing,
        editor.conflict,
        r.settings.keymap.clone(),
    );
    let list = lazy(key, move |_| command_list(r));

    page.push(scrollable(list).height(420)).width(PAGE_WIDTH).into()
}

fn command_list(r: &Resonance) -> Element<'static, Message> {
    let editor = &r.ui.keymap_editor;
    let mut list = column![].spacing(2);
    for category in CommandCategory::ALL {
        let rows: Vec<CommandId> = CommandId::ALL
            .iter()
            .copied()
            .filter(|c| c.category() == category)
            .filter(|c| {
                editor.filter.trim().is_empty()
                    || fuzzy_match(&editor.filter, c.display_name()).is_some()
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        list = list.push(
            container(
                text(format!("{}  {}", category.display_name().to_uppercase(), rows.len()))
                    .size(10)
                    .font(theme::UI_FONT_SEMIBOLD)
                    .color(theme::TEXT_3),
            )
            .padding(iced::Padding {
                top: 14.0,
                bottom: 6.0,
                left: 12.0,
                right: 12.0,
            }),
        );
        for id in rows {
            list = list.push(binding_row(r, id, editor.conflict));
        }
    }
    list.into()
}

fn binding_row(r: &Resonance, id: CommandId, conflict: Option<KeymapConflict>) -> Element<'static, Message> {
    let editor = &r.ui.keymap_editor;
    let chords: Vec<KeyChord> = r.ui.keymap.chords_for(id).collect();
    let edited = chords != editor.baseline.chords_for(id).collect::<Vec<_>>();
    let capturing = editor.capturing == Some(id);
    let unbound_by_preset = chords.is_empty() && editor.unbound_by_preset.contains(&id);
    // The ring goes on the chord being fought over, in its owner's row.
    let contested = conflict.filter(|c| c.owner == id).map(|c| c.chord);

    let mut line = row![text(id.display_name()).size(13).color(theme::TEXT_1)]
        .spacing(10)
        .align_y(alignment::Vertical::Center);
    if edited {
        line = line.push(
            container(text("edited").size(10).color(theme::WARM))
                .padding([1, 7])
                .style(theme::edited_pill_style),
        );
    }
    if unbound_by_preset {
        line = line.push(text("unbound by preset").size(10).color(theme::TEXT_3));
    }
    line = line.push(Space::new().width(Length::Fill));

    if capturing {
        line = line.push(
            container(
                text("Press a key…  Esc cancels")
                    .size(12)
                    .font(theme::MONO_FONT)
                    .color(theme::ACCENT_SOFT),
            )
            .padding([4, 12])
            .style(theme::active_row_style),
        );
    } else {
        if chords.is_empty() {
            line = line.push(text("—").size(12).color(theme::TEXT_4));
        }
        // Every chord, primary first; alternates dimmer behind a "·".
        for (i, &chord) in chords.iter().enumerate() {
            if i > 0 {
                line = line.push(text("·").size(12).color(theme::TEXT_4));
            }
            let tone = if contested == Some(chord) {
                KeycapTone::Conflict
            } else {
                KeycapTone::Neutral
            };
            line = line.push(chord_caps(chord, tone));
        }
        line = line.push(small_button("Rebind", msg(KeymapMsg::BeginRebind(id))));
        if edited {
            line = line.push(small_button("Reset", msg(KeymapMsg::Reset(id))));
        }
    }

    let body = container(line).padding([7, 12]).width(Length::Fill);
    if capturing {
        body.style(theme::active_row_style).into()
    } else {
        body.into()
    }
}
