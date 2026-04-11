use iced::widget::{button, column, container, pick_list, row, slider, text, text_input, Space};
use iced::{alignment, Element, Length};

use resonance_audio::types::ClipId;

use crate::compose::drumroll::{
    AccentPattern, DrumrollMessage, DrumrollViewState, HumanizeScope,
};
use crate::compose::ComposeMessage;
use crate::message::*;
use crate::state::{InstrumentType, TrackState};
use crate::theme;

use super::super::instrument_panel::PANEL_WIDTH;

/// Right-side controls for the drumroll view. Renders instead of
/// `instrument_panel::view` when the Compose details panel focuses a drum
/// track.
pub fn view<'a>(
    state: &'a DrumrollViewState,
    track: &'a TrackState,
    clip_id: Option<ClipId>,
) -> Element<'a, Message> {
    let track_id = track.id;
    let selected_pad = state.selected_pad;
    let pad_name: String = selected_pad
        .and_then(|i| state.pad_map.get(i))
        .map(|p| p.name.to_string())
        .unwrap_or_else(|| "Click a pad row to select".to_string());

    let heading = text("Drum pattern").size(13).color(theme::ACCENT);
    let close_btn = button(text("Done").size(12))
        .on_press(Message::Compose(ComposeMessage::ClearInstrumentDetails))
        .padding([4, 10])
        .style(|_theme, status| theme::transport_button_style(status));

    let name_input = text_input("Name", &track.name)
        .on_input(move |s| Message::Track(TrackMessage::SetTrackName(track_id, s)))
        .size(12)
        .padding([4, 6])
        .width(Length::Fill);

    let type_picker = pick_list(
        InstrumentType::ALL.to_vec(),
        Some(track.instrument_type),
        move |ty| Message::Track(TrackMessage::SetInstrumentType(track_id, ty)),
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let steps_picker = pick_list(
        vec![4u32, 8, 16, 32],
        Some(state.steps_per_bar),
        |n| Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetStepsPerBar(n))),
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let velocity_slider = slider(0.0..=1.0, state.default_velocity, |v| {
        Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetDefaultVelocity(v)))
    })
    .step(0.01)
    .width(Length::Fill);

    let euclid_steps = text_input("Steps", &state.euclid_steps_input)
        .on_input(|s| Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetEuclidSteps(s))))
        .size(12)
        .padding([4, 6])
        .width(Length::Fill);
    let euclid_hits = text_input("Hits", &state.euclid_hits_input)
        .on_input(|s| Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetEuclidHits(s))))
        .size(12)
        .padding([4, 6])
        .width(Length::Fill);
    let euclid_rot = text_input("Rotation", &state.euclid_rotation_input)
        .on_input(|s| Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetEuclidRotation(s))))
        .size(12)
        .padding([4, 6])
        .width(Length::Fill);

    let can_apply = selected_pad.is_some() && clip_id.is_some();
    let apply_msg = if can_apply {
        Some(Message::Compose(ComposeMessage::Drumroll(
            DrumrollMessage::GenerateEuclideanPad {
                clip_id: clip_id.unwrap(),
                pad_index: selected_pad.unwrap(),
            },
        )))
    } else {
        None
    };
    let mut apply_btn = button(text("Apply euclidean").size(12))
        .padding([4, 10])
        .width(Length::Fill)
        .style(|_theme, status| theme::transport_button_style(status));
    if let Some(m) = apply_msg {
        apply_btn = apply_btn.on_press(m);
    }

    let clear_msg = if can_apply {
        Some(Message::Compose(ComposeMessage::Drumroll(
            DrumrollMessage::ClearPad {
                clip_id: clip_id.unwrap(),
                pad_index: selected_pad.unwrap(),
            },
        )))
    } else {
        None
    };
    let mut clear_btn = button(text("Clear pad").size(12))
        .padding([4, 10])
        .width(Length::Fill)
        .style(|_theme, status| theme::transport_button_style(status));
    if let Some(m) = clear_msg {
        clear_btn = clear_btn.on_press(m);
    }

    // --- Humanize block --------------------------------------------------
    let hum_vel_slider = slider(0.0..=1.0, state.humanize_velocity, |v| {
        Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetHumanizeVelocity(v)))
    })
    .step(0.01)
    .width(Length::Fill);
    let hum_timing_slider = slider(0.0..=1.0, state.humanize_timing, |v| {
        Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetHumanizeTiming(v)))
    })
    .step(0.01)
    .width(Length::Fill);
    let hum_swing_slider = slider(0.0..=1.0, state.humanize_swing, |v| {
        Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetHumanizeSwing(v)))
    })
    .step(0.01)
    .width(Length::Fill);
    let accent_picker = pick_list(
        AccentPattern::ALL.to_vec(),
        Some(state.humanize_accent),
        |p| {
            Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetHumanizeAccent(p)))
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);
    let accent_slider = slider(0.0..=1.0, state.humanize_accent_amount, |v| {
        Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetHumanizeAccentAmount(v)))
    })
    .step(0.01)
    .width(Length::Fill);
    let scope_picker = pick_list(
        HumanizeScope::ALL.to_vec(),
        Some(state.humanize_scope),
        |s| {
            Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetHumanizeScope(s)))
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let humanize_msg = clip_id.map(|cid| {
        Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::ApplyHumanize {
            clip_id: cid,
        }))
    });
    let mut humanize_btn = button(text("Humanize").size(12))
        .padding([4, 10])
        .width(Length::Fill)
        .style(|_theme, status| theme::transport_button_style(status));
    if let Some(m) = humanize_msg {
        humanize_btn = humanize_btn.on_press(m);
    }

    let content = column![
        row![heading, Space::with_width(Length::Fill), close_btn]
            .align_y(alignment::Vertical::Center),
        Space::with_height(10),
        text("Name").size(10).color(theme::TEXT_DIM),
        name_input,
        Space::with_height(6),
        text("Type").size(10).color(theme::TEXT_DIM),
        type_picker,
        Space::with_height(10),
        text("Selected pad").size(10).color(theme::TEXT_DIM),
        text(pad_name).size(13).color(theme::TEXT),
        Space::with_height(8),
        text(format!("Velocity: {:.2}", state.default_velocity))
            .size(10)
            .color(theme::TEXT_DIM),
        velocity_slider,
        Space::with_height(8),
        text("Steps per bar").size(10).color(theme::TEXT_DIM),
        steps_picker,
        Space::with_height(12),
        text("Euclidean").size(11).color(theme::ACCENT),
        Space::with_height(4),
        text("Steps").size(10).color(theme::TEXT_DIM),
        euclid_steps,
        text("Hits").size(10).color(theme::TEXT_DIM),
        euclid_hits,
        text("Rotation").size(10).color(theme::TEXT_DIM),
        euclid_rot,
        Space::with_height(6),
        apply_btn,
        Space::with_height(4),
        clear_btn,
        Space::with_height(14),
        text("Humanize").size(11).color(theme::ACCENT),
        Space::with_height(4),
        text(format!("Velocity jitter: {:.2}", state.humanize_velocity))
            .size(10)
            .color(theme::TEXT_DIM),
        hum_vel_slider,
        text(format!("Timing jitter: {:.2}", state.humanize_timing))
            .size(10)
            .color(theme::TEXT_DIM),
        hum_timing_slider,
        text(format!("Swing: {:.2}", state.humanize_swing))
            .size(10)
            .color(theme::TEXT_DIM),
        hum_swing_slider,
        Space::with_height(4),
        text("Accent pattern").size(10).color(theme::TEXT_DIM),
        accent_picker,
        text(format!("Accent amount: {:.2}", state.humanize_accent_amount))
            .size(10)
            .color(theme::TEXT_DIM),
        accent_slider,
        Space::with_height(4),
        text("Scope").size(10).color(theme::TEXT_DIM),
        scope_picker,
        Space::with_height(6),
        humanize_btn,
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
