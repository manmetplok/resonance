//! Settings › MIDI (doc #167, W1): which input port is the control
//! surface, every MIDI Learn binding in the project with a per-row clear
//! and Clear all, and the controller-map presets (save the project's
//! bindings under a name, load or delete one).
//!
//! The port is this machine's (settings.json); the bindings and a loaded
//! map are the project's, undoable like any edit.

use std::hash::{Hash, Hasher};

use iced::widget::{button, column, container, lazy, pick_list, row, scrollable, text, text_input, Space};
use iced::{alignment, Element, Length};

use crate::message::{Message, MidiMapMessage};
use crate::state::source_label;
use crate::state::ui_caches::midi_choices_with_override;
use crate::theme;
use crate::view::midi_learn::target_label;
use crate::view::mixer::picks::MidiPickerChoice;
use crate::Resonance;

fn msg(m: MidiMapMessage) -> Message {
    Message::MidiMap(m)
}

fn section(label: &'static str) -> Element<'static, Message> {
    text(label)
        .size(10)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3)
        .into()
}

fn note(label: impl Into<String>) -> Element<'static, Message> {
    text(label.into()).size(11).color(theme::TEXT_3).into()
}

fn small_button(label: &'static str, on_press: Option<Message>) -> Element<'static, Message> {
    let color = if on_press.is_some() {
        theme::ACCENT_SOFT
    } else {
        theme::TEXT_4
    };
    let mut b = button(text(label).size(11).color(color))
        .padding([4, 9])
        .style(|_theme, status| theme::ghost_button_style(status));
    if let Some(m) = on_press {
        b = b.on_press(m);
    }
    b.into()
}

/// One bindings-list row as drawn: id, target name, control name.
type Row = (resonance_common::BindingId, String, String);

pub(crate) fn view_midi_page(r: &Resonance, width: f32) -> Element<'_, Message> {
    let map = &r.devices.midi_map;
    let title = text("MIDI")
        .size(22)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1);

    // ---- Control surface ----
    let configured = r.settings.midi.control_surface_input.as_deref();
    let choices = midi_choices_with_override(
        &r.ui.view_caches.midi_input_choices,
        configured,
        &r.devices.midi.midi_input_devices,
    );
    let port = pick_list(
        choices,
        Some(MidiPickerChoice(configured.map(str::to_string))),
        |c| msg(MidiMapMessage::SetControlSurfaceInput(c.0)),
    )
    .placeholder("MIDI input port...")
    .text_size(12)
    .width(Length::Fill);
    let unplugged = configured
        .filter(|name| !r.devices.midi.midi_input_devices.iter().any(|d| d.name == *name))
        .map(|name| note(format!("{name} is not connected; it is used as soon as it is.")));

    // ---- Learn state ----
    let learning: Element<'_, Message> = match map.learn_target {
        Some(target) => container(
            row![
                text(format!(
                    "Listening: move a control for {}\u{2026}",
                    target_label(r, target)
                ))
                .size(11)
                .color(theme::ACCENT_SOFT)
                .width(Length::Fill),
                small_button("Cancel", Some(msg(MidiMapMessage::CancelLearn))),
            ]
            .align_y(alignment::Vertical::Center),
        )
        .padding([6, 8])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::ACCENT_DIM)),
            border: iced::Border {
                color: theme::ACCENT_LINE,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into(),
        // An empty slot: the page keeps its shape when learn arms.
        None => Space::new().height(0).into(),
    };

    // ---- Bindings ----
    // The labels are cheap; building the rows is what the lazy region
    // saves while the overlay redraws.
    let rows: Vec<Row> = map
        .sorted()
        .into_iter()
        .map(|b| (b.id, target_label(r, b.target), source_label(b.source)))
        .collect();
    let fp = {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        rows.hash(&mut h);
        h.finish()
    };
    let count = rows.len();
    let list = lazy(fp, move |_: &u64| -> Element<'static, Message> {
        if rows.is_empty() {
            return note(
                "No MIDI bindings yet. Right-click a fader, pan knob, M or S in the \
                 mixer (or use TRACK \u{203a} Learn MIDI for\u{2026} in the inspector), \
                 choose Learn MIDI, then move a control on the control surface.",
            );
        }
        let mut col = column![].spacing(2);
        for (id, target, source) in &rows {
            col = col.push(
                row![
                    text(target.clone()).size(12).color(theme::TEXT_1).width(Length::Fill),
                    text(source.clone())
                        .size(11)
                        .font(theme::MONO_FONT)
                        .color(theme::TEXT_3),
                    button(theme::icon(theme::fa::TRASH).size(10).color(theme::TEXT_3))
                        .on_press(Message::MidiMap(MidiMapMessage::Clear(*id)))
                        .padding([3, 6])
                        .style(|_theme, status| theme::small_button_style(status)),
                ]
                .spacing(10)
                .padding([2, 4])
                .align_y(alignment::Vertical::Center),
            );
        }
        scrollable(col).height(Length::Shrink).into()
    });
    let bindings_header = row![
        section("BINDINGS"),
        text(format!("{count}")).size(10).color(theme::TEXT_3),
        Space::new().width(Length::Fill),
        small_button(
            "Clear all",
            (count > 0).then(|| msg(MidiMapMessage::ClearAll)),
        ),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    // ---- Controller maps ----
    let mut maps = column![].spacing(2);
    for m in &map.saved_maps {
        maps = maps.push(
            row![
                text(m.name.clone()).size(12).color(theme::TEXT_1).width(Length::Fill),
                text(format!("{} binding(s)", m.bindings.len()))
                    .size(11)
                    .color(theme::TEXT_3),
                small_button(
                    "Load",
                    Some(msg(MidiMapMessage::LoadControllerMap(m.name.clone()))),
                ),
                small_button(
                    "Delete",
                    Some(msg(MidiMapMessage::DeleteControllerMap(m.name.clone()))),
                ),
            ]
            .spacing(8)
            .padding([2, 4])
            .align_y(alignment::Vertical::Center),
        );
    }
    let can_save = !map.map_name.trim().is_empty();
    let save_row = row![
        text_input("Map name", &map.map_name)
            .on_input(|s| msg(MidiMapMessage::SetMapName(s)))
            .on_submit(msg(MidiMapMessage::SaveControllerMap))
            .size(12)
            .padding([6, 8])
            .width(Length::Fill),
        small_button(
            "Save bindings as map",
            can_save.then(|| msg(MidiMapMessage::SaveControllerMap)),
        ),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let mut page = column![
        title,
        Space::new().height(16),
        section("CONTROL SURFACE"),
        Space::new().height(2),
        note("The MIDI input port MIDI Learn listens to. This machine's setting, not the project's."),
        port,
    ]
    .spacing(6)
    .width(width);
    if let Some(n) = unplugged {
        page = page.push(n);
    }
    page.push(Space::new().height(16))
        .push(bindings_header)
        .push(learning)
        .push(container(list).max_height(260.0))
        .push(Space::new().height(16))
        .push(section("CONTROLLER MAPS"))
        .push(note(
            "A map is a saved set of bindings. Loading one replaces this project's \
             bindings. Maps name tracks and plugins by id, so they fit projects built \
             the same way; transport bindings fit any project.",
        ))
        .push(maps)
        .push(save_row)
        .into()
}
