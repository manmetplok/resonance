//! Settings overlay (opened from the transport bar gear icon). Shows
//! project open / save / save-as buttons and global MIDI clock
//! settings.
use iced::widget::{button, column, container, mouse_area, opaque, pick_list, row, stack, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::theme::{self, fa};
use crate::view::mixer::picks::MidiPickerChoice;
use crate::view::ui_caches::midi_choices_with_override;
use crate::Resonance;

pub(crate) fn view_settings_overlay(r: &Resonance) -> Element<'_, Message> {
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
    .on_press(Message::Ui(UiMessage::CloseSettings));

    let title = text("Settings")
        .size(22)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1);

    let section = |label: &'static str| {
        text(label)
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3)
    };

    let open_btn = wide_button(
        fa::FOLDER_OPEN,
        "Open Project...",
        Message::ProjectIo(ProjectIoMessage::OpenProject),
    );
    let save_btn = wide_button(
        fa::FLOPPY_DISK,
        "Save Project",
        Message::ProjectIo(ProjectIoMessage::SaveProject),
    );
    let save_as_btn = wide_button(
        fa::FLOPPY_DISK,
        "Save Project As...",
        Message::ProjectIo(ProjectIoMessage::SaveProjectAs),
    );

    let close_btn = button(text("Close").size(13).color(theme::TEXT_1))
        .on_press(Message::Ui(UiMessage::CloseSettings))
        .padding([6, 14])
        .style(|_theme, status| theme::ghost_button_style(status));

    let send_toggle = toggle_button(
        "Send MIDI Clock",
        r.midi_clock_send_enabled,
        Message::Ui(UiMessage::ToggleMidiClockSend),
    );
    // Option lists come from `UiViewCaches` — the overlay redraws every
    // tick while open, and rebuilding these Vecs per frame was measured
    // per-view() work. The cache is rebuilt whenever the engine
    // re-enumerates MIDI devices; the override covers a configured
    // device that is currently unplugged.
    let send_choices = midi_choices_with_override(
        &r.view_caches.midi_output_choices,
        r.midi_clock_send_device.as_deref(),
        &r.midi_output_devices,
    );
    let send_picker = pick_list(
        send_choices,
        Some(MidiPickerChoice(r.midi_clock_send_device.clone())),
        |choice| Message::Ui(UiMessage::SetMidiClockSendDevice(choice.0)),
    )
    .placeholder("MIDI output port...")
    .text_size(12)
    .width(Length::Fill);

    let recv_toggle = toggle_button(
        "Receive MIDI Clock",
        r.midi_clock_recv_enabled,
        Message::Ui(UiMessage::ToggleMidiClockRecv),
    );
    let recv_choices = midi_choices_with_override(
        &r.view_caches.midi_input_choices,
        r.midi_clock_recv_device.as_deref(),
        &r.midi_input_devices,
    );
    let recv_picker = pick_list(
        recv_choices,
        Some(MidiPickerChoice(r.midi_clock_recv_device.clone())),
        |choice| Message::Ui(UiMessage::SetMidiClockRecvDevice(choice.0)),
    )
    .placeholder("MIDI input port...")
    .text_size(12)
    .width(Length::Fill);

    // Plugins: what the last scan found, and a way to look again
    // without restarting the app (ba todo #1307, finding X10). A plugin
    // installed while Resonance is open was invisible until now, because
    // the only scan ran at startup.
    let instrument_count = r
        .available_plugins
        .iter()
        .filter(|p| p.is_instrument)
        .count();
    let plugin_summary = text(format!(
        "{} plugin(s) available — {} instrument(s), {} effect(s)",
        r.available_plugins.len(),
        instrument_count,
        r.available_plugins.len() - instrument_count
    ))
    .size(11)
    .color(theme::TEXT_3);
    let rescan_btn = wide_button(
        fa::ARROW_ROTATE_LEFT,
        if r.plugin_scan_in_progress {
            "Scanning..."
        } else {
            "Rescan Plugins"
        },
        Message::Plugin(PluginMessage::RescanPlugins),
    );
    // Failures are shown, not swallowed: a `.clap` that refuses to load
    // is otherwise indistinguishable from one that was never installed.
    let mut plugins_section = column![
        section("Plugins"),
        Space::new().height(6),
        plugin_summary,
        rescan_btn,
    ]
    .spacing(6);
    for failure in &r.plugin_scan_failures {
        plugins_section = plugins_section.push(
            text(format!("Failed: {} — {}", failure.path, failure.reason))
                .size(10)
                .color(theme::BAD),
        );
    }

    // The way back to the load warning after it has been dismissed (ba
    // doc #275 P5, todo #1309). Scan failures above are about the
    // MACHINE's catalog; this is about THIS PROJECT's chains, and the
    // two are different problems that happen to share a section: a
    // plugin can be absent from the catalog and used by no project, or
    // present in the catalog and still refuse to instantiate.
    let missing_slots = r.missing_plugin_slots();
    if !missing_slots.is_empty() {
        plugins_section = plugins_section.push(
            button(
                text(format!(
                    "{} plugin slot(s) in this project could not be loaded \u{2014} review",
                    missing_slots.len()
                ))
                .size(10)
                .color(theme::BAD),
            )
            .on_press(Message::Ui(UiMessage::ShowMissingPlugins))
            .style(|_theme, status| theme::small_button_style(status))
            .padding([2, 0]),
        );
    }

    let follow_toggle = toggle_button(
        "Follow playhead",
        r.settings.arrange.follow_playhead,
        Message::Ui(UiMessage::ToggleFollowPlayhead),
    );

    let dialog_content = column![
        title,
        Space::new().height(16),
        section("Project"),
        Space::new().height(6),
        open_btn,
        save_btn,
        save_as_btn,
        Space::new().height(20),
        section("Arrange"),
        Space::new().height(6),
        follow_toggle,
        Space::new().height(20),
        plugins_section,
        Space::new().height(20),
        section("MIDI Clock"),
        Space::new().height(6),
        send_toggle,
        send_picker,
        Space::new().height(8),
        recv_toggle,
        recv_picker,
        Space::new().height(20),
        row![Space::new().width(Length::Fill), close_btn,],
    ]
    .spacing(6)
    .padding(24)
    .width(420);

    let dialog = container(dialog_content).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: theme::RADIUS_XL.into(),
        },
        ..Default::default()
    });

    let centered = container(opaque(dialog))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    stack![backdrop, centered].into()
}

fn wide_button<'a>(
    icon: char,
    label: &'a str,
    on_press: Message,
) -> iced::widget::Button<'a, Message> {
    button(
        row![
            theme::icon(icon).size(13).color(theme::TEXT_2),
            Space::new().width(10),
            text(label).size(13).color(theme::TEXT_1),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .on_press(on_press)
    .padding([8, 14])
    .width(Length::Fill)
    .style(|_theme, status| theme::ghost_button_style(status))
}

/// Pseudo-checkbox button: shows a filled/empty box icon plus a
/// label, fires the supplied message when clicked.
fn toggle_button<'a>(
    label: &'a str,
    enabled: bool,
    on_press: Message,
) -> iced::widget::Button<'a, Message> {
    let icon = if enabled {
        fa::CIRCLE
    } else {
        fa::CIRCLE_HOLLOW
    };
    let icon_color = if enabled {
        theme::ACCENT
    } else {
        theme::TEXT_3
    };
    button(
        row![
            theme::icon(icon).size(13).color(icon_color),
            Space::new().width(10),
            text(label).size(13).color(theme::TEXT_1),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .on_press(on_press)
    .padding([6, 10])
    .width(Length::Fill)
    .style(|_theme, status| theme::ghost_button_style(status))
}
