//! Export modal (design doc #155, todo #324; wired by code review
//! ARCH2-01).
//!
//! One overlay with two mode tabs (Audio stems / MIDI) and a shared
//! footer. The Audio-stems tab has its source checklist, range toggle
//! and destination folder; Export renders through
//! `AudioCommand::ExportStems`, and the body follows the render phases
//! (Rendering → Done / Error / Cancelled). The MIDI tab has no exporter
//! yet, so it shows a note and its action stays disabled.
//!
//! Same backdrop + centered-dialog pattern as `bounce_dialog.rs`.
use iced::widget::{
    button, checkbox, column, container, mouse_area, opaque, row, scrollable, stack, text, Space,
};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::state::{ExportDialogState, ExportMode, ExportPhase, ExportRange, ExportSource};
use crate::theme;
use crate::Resonance;

pub(crate) fn view_export_dialog_overlay<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let Some(dialog) = r.modals.export_dialog.as_ref() else {
        return Space::new().width(Length::Fixed(0.0)).height(Length::Fixed(0.0)).into();
    };

    // The backdrop dismisses the modal, except mid-render: the handler
    // ignores `Close` then, so the export must be stopped explicitly.
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
    .on_press(Message::Export(ExportMessage::Close));

    let title = text("Export")
        .size(20)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1);

    // Mode tabs — segmented toggle mirroring the bounce dialog's
    // stereo/mono buttons. The active tab gets the accent border so the
    // current mode reads at a glance.
    let editable = matches!(dialog.phase, ExportPhase::Setup);
    let tab = |label: &'static str, mode: ExportMode| {
        let selected = dialog.mode == mode;
        let mut b = button(
            text(label)
                .size(13)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(if selected { theme::TEXT_1 } else { theme::TEXT_2 }),
        )
        .padding([6, 16])
        .style(move |_t, status| {
            let mut s = theme::transport_button_style(status);
            if selected {
                s.border.color = theme::ACCENT;
                s.background = Some(iced::Background::Color(theme::ACCENT_DIM));
            }
            s
        });
        if !selected && editable {
            b = b.on_press(Message::Export(ExportMessage::SetMode(mode)));
        }
        b
    };
    let tabs = row![
        tab("Audio stems", ExportMode::AudioStems),
        tab("MIDI", ExportMode::Midi),
    ]
    .spacing(8);

    // Per-tab body: the editable Setup controls, or the render phase.
    let body_content: Element<'a, Message> = match (&dialog.phase, dialog.mode) {
        (ExportPhase::Setup, ExportMode::AudioStems) => stems_setup_body(r, dialog),
        (ExportPhase::Setup, ExportMode::Midi) => text(
            "MIDI file export is not available yet; switch to Audio stems to render WAVs.",
        )
        .size(12)
        .color(theme::TEXT_3)
        .into(),
        (phase, _) => phase_body(phase),
    };
    let body = container(body_content)
        .width(Length::Fill)
        .height(Length::Fixed(260.0))
        .padding(16)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::LINE_2)),
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        });

    // Footer: live count on the left, Cancel + primary action on the
    // right. The primary label and count noun depend on the mode.
    let count = dialog.selected_count();
    let (count_label, action_label) = match dialog.mode {
        ExportMode::AudioStems => (
            format!("{count} selected"),
            if count == 0 {
                "Export stems".to_string()
            } else {
                format!("Export {count} stems")
            },
        ),
        ExportMode::Midi => (
            format!("{count} selected"),
            if count == 0 {
                "Export MIDI".to_string()
            } else {
                format!("Export {count} MIDI files")
            },
        ),
    };

    let count_text = text(count_label).size(12).color(theme::TEXT_2);

    let (cancel_label, cancel_msg) = match &dialog.phase {
        ExportPhase::Setup => ("Cancel", Some(ExportMessage::Close)),
        ExportPhase::Rendering { .. } => (
            "Stop export",
            (!dialog.cancel_requested).then_some(ExportMessage::CancelRender),
        ),
        ExportPhase::Done(_) | ExportPhase::Error { .. } | ExportPhase::Cancelled(_) => {
            ("Close", Some(ExportMessage::Close))
        }
    };
    let cancel_btn = button(text(cancel_label).size(13).color(theme::TEXT_1))
        .on_press_maybe(cancel_msg.map(Message::Export))
        .padding([8, 18])
        .style(|_theme, status| theme::ghost_button_style(status));

    let can_export = dialog.can_export();
    let action_btn = button(
        text(action_label)
            .size(13)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(if can_export { theme::BG_0 } else { theme::TEXT_3 }),
    )
    .padding([8, 18])
    .style(move |_theme, status| {
        if can_export {
            theme::primary_button_style(status)
        } else {
            theme::ghost_button_style(status)
        }
    })
    .on_press_maybe(can_export.then_some(Message::Export(ExportMessage::Confirm)));

    let footer = row![
        count_text,
        Space::new().width(Length::Fill),
        cancel_btn,
        action_btn,
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);

    let dialog_content = column![
        title,
        Space::new().height(14),
        tabs,
        Space::new().height(14),
        body,
        Space::new().height(20),
        footer,
    ]
    .spacing(0)
    .padding(24)
    .width(560);

    let dialog_box = container(dialog_content).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: theme::RADIUS_XL.into(),
        },
        ..Default::default()
    });

    let centered = container(opaque(dialog_box))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    stack![backdrop, centered].into()
}

/// The Audio-stems Setup body: the source checklist (master, busses,
/// top-level tracks — a track's stem carries its sub-tracks), the range
/// toggle and the destination folder.
fn stems_setup_body<'a>(r: &'a Resonance, dialog: &'a ExportDialogState) -> Element<'a, Message> {
    let source_row = |source: ExportSource, label: String| -> Element<'a, Message> {
        checkbox(dialog.selected_sources.contains(&source))
            .label(label)
            .text_size(12)
            .size(14)
            .on_toggle(move |_| Message::Export(ExportMessage::ToggleSource(source)))
            .into()
    };
    let mut sources = column![source_row(ExportSource::Master, "Master".to_owned())].spacing(6);
    let mut busses: Vec<_> = r.registry.busses.iter().collect();
    busses.sort_by_key(|b| b.order);
    for bus in busses {
        sources = sources.push(source_row(
            ExportSource::Bus(bus.id),
            format!("Bus: {}", bus.name),
        ));
    }
    let mut tracks: Vec<_> = r
        .registry
        .tracks
        .iter()
        .filter(|t| t.sub_track.is_none())
        .collect();
    tracks.sort_by_key(|t| t.order);
    for track in tracks {
        sources = sources.push(source_row(ExportSource::Track(track.id), track.name.clone()));
    }

    let range_btn = |label: &'static str, range: ExportRange| {
        let selected = dialog.range == range;
        button(
            text(label)
                .size(11)
                .color(if selected { theme::TEXT_1 } else { theme::TEXT_2 }),
        )
        .padding([4, 10])
        .on_press(Message::Export(ExportMessage::SetRange(range)))
        .style(move |_t, status| theme::toggle_button_style(selected, theme::ACCENT, true, status))
    };
    let range_row = row![
        text("Range").size(11).color(theme::TEXT_3),
        range_btn("Whole project", ExportRange::WholeProject),
        range_btn("Loop range", ExportRange::LoopOrSelection),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center);

    let destination = dialog
        .destination
        .as_ref()
        .map_or_else(|| "No folder chosen".to_owned(), |p| p.display().to_string());
    let dest_row = row![
        text("To").size(11).color(theme::TEXT_3),
        text(destination).size(11).color(theme::TEXT_2).width(Length::Fill),
        button(text("Choose folder…").size(11).color(theme::TEXT_1))
            .padding([4, 10])
            .on_press(Message::Export(ExportMessage::ChooseDestination))
            .style(|_theme, status| theme::ghost_button_style(status)),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center);

    column![
        scrollable(sources.width(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill),
        range_row,
        dest_row
    ]
        .spacing(10)
        .into()
}

/// The body for a render phase: progress, the written files, the error.
fn phase_body<'a>(phase: &'a ExportPhase) -> Element<'a, Message> {
    let line = |s: String, color| text(s).size(12).color(color);
    match phase {
        ExportPhase::Setup => Space::new().into(),
        ExportPhase::Rendering { done, total } => column![
            line(
                format!("Rendering stem {} of {total}…", (done + 1).min(*total)),
                theme::TEXT_1
            ),
            line(format!("{done} of {total} written"), theme::TEXT_3),
        ]
        .spacing(6)
        .into(),
        ExportPhase::Done(files) => files_body(
            line(format!("{} stems written.", files.len()), theme::TEXT_1),
            files,
        ),
        ExportPhase::Cancelled(files) => files_body(
            line(
                format!("Export stopped; {} stems were written.", files.len()),
                theme::TEXT_1,
            ),
            files,
        ),
        ExportPhase::Error {
            written,
            message,
            remaining,
        } => files_body(
            column![
                line(format!("Export failed ({remaining} not written):"), theme::TEXT_1),
                line(message.clone(), theme::TEXT_2),
            ]
            .spacing(4),
            written,
        ),
    }
}

/// A heading over the list of files written.
fn files_body<'a>(
    heading: impl Into<Element<'a, Message>>,
    files: &'a [std::path::PathBuf],
) -> Element<'a, Message> {
    let list = files.iter().fold(column![].spacing(2), |col, f| {
        col.push(text(f.display().to_string()).size(11).color(theme::TEXT_3))
    });
    column![heading.into(), scrollable(list).height(Length::Fill)]
        .spacing(8)
        .into()
}
