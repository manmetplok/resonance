//! Drum-lane inspector body: per-pad mode picker, euclidean params,
//! humanize block.

use iced::widget::{button, column, pick_list, slider, text, text_input, Space};
use iced::{Element, Length};

use crate::compose::drumroll::DrumrollMessage;
use crate::compose::messages::LaneInspectorMsg;
use crate::compose::{
    ComposeMessage, DrumVoiceMode, DrumrollViewState, LaneGeneratorKind, SectionDefinitionState,
};
use crate::message::*;
use crate::state::TrackState;
use crate::theme;

/// Wrapper for drum voice mode in pick_list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrumModePick {
    Manual,
    Euclidean,
    Motif,
}

impl DrumModePick {
    const ALL: [DrumModePick; 3] = [
        DrumModePick::Manual,
        DrumModePick::Euclidean,
        DrumModePick::Motif,
    ];
}

impl std::fmt::Display for DrumModePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DrumModePick::Manual => "Manual",
            DrumModePick::Euclidean => "Euclidean",
            DrumModePick::Motif => "Motif",
        })
    }
}

pub(super) fn drum_body<'a>(
    definition: &'a SectionDefinitionState,
    track: &'a TrackState,
    drumroll_state: &'a DrumrollViewState,
    clip_id: Option<u64>,
) -> Element<'a, Message> {
    let definition_id = definition.id;
    let track_id = track.id;

    let heading = text(&track.name).size(13).color(theme::ACCENT);

    // Track name
    let name_input = text_input("Name", &track.name)
        .on_input(move |s| Message::Track(TrackMessage::SetTrackName(track_id, s)))
        .size(12)
        .padding([4, 6])
        .width(Length::Fill);

    // Steps per bar
    let steps_picker = pick_list(
        vec![4u32, 8, 16, 32],
        Some(drumroll_state.steps_per_bar),
        |n| Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetStepsPerBar(n))),
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    // Default velocity
    let vel_slider = slider(0.0..=1.0, drumroll_state.default_velocity, |v| {
        Message::Compose(ComposeMessage::Drumroll(
            DrumrollMessage::SetDefaultVelocity(v),
        ))
    })
    .step(0.01)
    .width(Length::Fill);

    // Per-pad euclidean section
    let selected_pad = drumroll_state.selected_pad;
    let pad_name: String = selected_pad
        .and_then(|i| drumroll_state.pad_map.get(i))
        .map(|p| p.name.to_string())
        .unwrap_or_else(|| "Click a pad row to select".to_string());

    // Get the drum lane config for this track
    let drum_config = definition
        .lane_generators
        .get(&track_id)
        .and_then(|cfg| match &cfg.kind {
            LaneGeneratorKind::Drum(dc) => Some(dc),
            _ => None,
        });

    let voice_mode = selected_pad.and_then(|pi| drum_config.and_then(|dc| dc.voices.get(&pi)));

    let current_mode_pick = match voice_mode {
        Some(DrumVoiceMode::Euclidean { .. }) => DrumModePick::Euclidean,
        Some(DrumVoiceMode::Motif) => DrumModePick::Motif,
        _ => DrumModePick::Manual,
    };

    let mode_picker_msg = selected_pad.map(|pad_index| {
        move |pick: DrumModePick| {
            let mode = match pick {
                DrumModePick::Manual => DrumVoiceMode::Manual,
                DrumModePick::Euclidean => DrumVoiceMode::Euclidean {
                    steps: 16,
                    hits: 4,
                    rotation: 0,
                },
                DrumModePick::Motif => DrumVoiceMode::Motif,
            };
            Message::Compose(ComposeMessage::LaneInspector {
                definition_id,
                track_id,
                msg: LaneInspectorMsg::SetDrumVoiceMode { pad_index, mode },
            })
        }
    });

    let mode_picker_el: Element<'a, Message> = if let Some(on_change) = mode_picker_msg {
        pick_list(
            DrumModePick::ALL.to_vec(),
            Some(current_mode_pick),
            on_change,
        )
        .text_size(12)
        .padding([4, 6])
        .width(Length::Fill)
        .into()
    } else {
        text("Select a pad first")
            .size(11)
            .color(theme::TEXT_DIM)
            .into()
    };

    // Euclidean params (if current voice is Euclidean)
    let euclid_controls: Element<'a, Message> = match (selected_pad, voice_mode) {
        (
            Some(pad_index),
            Some(DrumVoiceMode::Euclidean {
                steps,
                hits,
                rotation,
            }),
        ) => {
            let steps_input = text_input("Steps", &steps.to_string())
                .on_input(move |s| {
                    let val = s.parse::<u32>().unwrap_or(16).max(1);
                    Message::Compose(ComposeMessage::LaneInspector {
                        definition_id,
                        track_id,
                        msg: LaneInspectorMsg::SetDrumEuclidSteps {
                            pad_index,
                            steps: val,
                        },
                    })
                })
                .size(12)
                .padding([4, 6])
                .width(Length::Fill);
            let hits_input = text_input("Hits", &hits.to_string())
                .on_input(move |s| {
                    let val = s.parse::<u32>().unwrap_or(4);
                    Message::Compose(ComposeMessage::LaneInspector {
                        definition_id,
                        track_id,
                        msg: LaneInspectorMsg::SetDrumEuclidHits {
                            pad_index,
                            hits: val,
                        },
                    })
                })
                .size(12)
                .padding([4, 6])
                .width(Length::Fill);
            let rot_input = text_input("Rotation", &rotation.to_string())
                .on_input(move |s| {
                    let val = s.parse::<i32>().unwrap_or(0);
                    Message::Compose(ComposeMessage::LaneInspector {
                        definition_id,
                        track_id,
                        msg: LaneInspectorMsg::SetDrumEuclidRotation {
                            pad_index,
                            rotation: val,
                        },
                    })
                })
                .size(12)
                .padding([4, 6])
                .width(Length::Fill);

            // Apply button: generates euclidean pattern for this pad
            let can_apply = clip_id.is_some();
            let apply_msg = if can_apply {
                Some(Message::Compose(ComposeMessage::Drumroll(
                    DrumrollMessage::GenerateEuclideanPad {
                        clip_id: clip_id.unwrap(),
                        pad_index,
                    },
                )))
            } else {
                None
            };
            let mut apply_btn = button(text("Apply").size(12))
                .padding([4, 10])
                .width(Length::Fill)
                .style(|_theme, status| theme::transport_button_style(status));
            if let Some(m) = apply_msg {
                apply_btn = apply_btn.on_press(m);
            }

            column![
                text("Steps").size(10).color(theme::TEXT_DIM),
                steps_input,
                text("Hits").size(10).color(theme::TEXT_DIM),
                hits_input,
                text("Rotation").size(10).color(theme::TEXT_DIM),
                rot_input,
                Space::with_height(4),
                apply_btn,
            ]
            .spacing(2)
            .into()
        }
        (Some(pad_index), Some(DrumVoiceMode::Motif)) => {
            // Motif voices have no per-voice knobs — the rhythm comes
            // straight from the section's shared motif, so the Apply
            // button is the only control needed. Edits to the motif's
            // seed / complexity propagate via `propagate_motif_change`.
            let can_apply = clip_id.is_some();
            let apply_msg = if can_apply {
                Some(Message::Compose(ComposeMessage::Drumroll(
                    DrumrollMessage::GenerateMotifPad {
                        clip_id: clip_id.unwrap(),
                        pad_index,
                    },
                )))
            } else {
                None
            };
            let mut apply_btn = button(text("Apply").size(12))
                .padding([4, 10])
                .width(Length::Fill)
                .style(|_theme, status| theme::transport_button_style(status));
            if let Some(m) = apply_msg {
                apply_btn = apply_btn.on_press(m);
            }
            column![
                text("Plays the section's shared motif rhythm.")
                    .size(10)
                    .color(theme::TEXT_DIM),
                Space::with_height(4),
                apply_btn,
            ]
            .spacing(2)
            .into()
        }
        _ => Space::with_height(0).into(),
    };

    // Clear pad button
    let clear_msg = match (selected_pad, clip_id) {
        (Some(pad_index), Some(cid)) => Some(Message::Compose(ComposeMessage::Drumroll(
            DrumrollMessage::ClearPad {
                clip_id: cid,
                pad_index,
            },
        ))),
        _ => None,
    };
    let mut clear_btn = button(text("Clear pad").size(12))
        .padding([4, 10])
        .width(Length::Fill)
        .style(|_theme, status| theme::transport_button_style(status));
    if let Some(m) = clear_msg {
        clear_btn = clear_btn.on_press(m);
    }

    // Humanize section (kept from drumroll/controls.rs)
    let humanize = humanize_block(drumroll_state, clip_id);

    column![
        heading,
        Space::with_height(6),
        text("Name").size(10).color(theme::TEXT_DIM),
        name_input,
        Space::with_height(8),
        text("Steps per bar").size(10).color(theme::TEXT_DIM),
        steps_picker,
        text(format!("Velocity: {:.2}", drumroll_state.default_velocity))
            .size(10)
            .color(theme::TEXT_DIM),
        vel_slider,
        Space::with_height(10),
        text("Selected pad").size(10).color(theme::TEXT_DIM),
        text(pad_name.clone()).size(13).color(theme::TEXT),
        Space::with_height(4),
        text("Mode").size(10).color(theme::TEXT_DIM),
        mode_picker_el,
        Space::with_height(4),
        euclid_controls,
        Space::with_height(4),
        clear_btn,
        Space::with_height(12),
        humanize,
    ]
    .spacing(2)
    .into()
}

fn humanize_block<'a>(state: &'a DrumrollViewState, clip_id: Option<u64>) -> Element<'a, Message> {
    use crate::compose::drumroll::{AccentPattern, HumanizeScope};

    let hum_vel_slider = slider(0.0..=1.0, state.humanize_velocity, |v| {
        Message::Compose(ComposeMessage::Drumroll(
            DrumrollMessage::SetHumanizeVelocity(v),
        ))
    })
    .step(0.01)
    .width(Length::Fill);

    let hum_timing_slider = slider(0.0..=1.0, state.humanize_timing, |v| {
        Message::Compose(ComposeMessage::Drumroll(
            DrumrollMessage::SetHumanizeTiming(v),
        ))
    })
    .step(0.01)
    .width(Length::Fill);

    let hum_swing_slider = slider(0.0..=1.0, state.humanize_swing, |v| {
        Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetHumanizeSwing(
            v,
        )))
    })
    .step(0.01)
    .width(Length::Fill);

    let accent_picker = pick_list(
        AccentPattern::ALL.to_vec(),
        Some(state.humanize_accent),
        |p| {
            Message::Compose(ComposeMessage::Drumroll(
                DrumrollMessage::SetHumanizeAccent(p),
            ))
        },
    )
    .text_size(12)
    .padding([4, 6])
    .width(Length::Fill);

    let accent_slider = slider(0.0..=1.0, state.humanize_accent_amount, |v| {
        Message::Compose(ComposeMessage::Drumroll(
            DrumrollMessage::SetHumanizeAccentAmount(v),
        ))
    })
    .step(0.01)
    .width(Length::Fill);

    let scope_picker = pick_list(
        HumanizeScope::ALL.to_vec(),
        Some(state.humanize_scope),
        |s| {
            Message::Compose(ComposeMessage::Drumroll(DrumrollMessage::SetHumanizeScope(
                s,
            )))
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

    column![
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
        text(format!(
            "Accent amount: {:.2}",
            state.humanize_accent_amount
        ))
        .size(10)
        .color(theme::TEXT_DIM),
        accent_slider,
        Space::with_height(4),
        text("Scope").size(10).color(theme::TEXT_DIM),
        scope_picker,
        Space::with_height(6),
        humanize_btn,
    ]
    .spacing(2)
    .into()
}
