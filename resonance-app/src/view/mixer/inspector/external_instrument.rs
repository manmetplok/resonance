//! External-instrument ROUTING group — replaces the generic ROUTING
//! fields for a track paired with a hardware synth (todo #454, doc #169).
//! Covers MIDI output, audio return, patch (Bank/Program), latency
//! compensation, return monitoring, and the offline alert.
//!
//! Widget helpers (`alert_action_button`, `pgnum_tile`, `preset_hint_chip`)
//! live in `widgets`. The audio-return device list helper (`return_device_choices`)
//! lives in `io`.

use iced::widget::{button, column, container, pick_list, row, slider, text, Space};
use iced::{alignment, Element, Length};

use crate::message::{ExternalInstrumentMessage, Message};
use crate::state::{ExternalInstrumentState, TrackState};
use crate::theme;
use crate::view::mixer::picks::{
    patch_choices, BankChoice, MidiChannelChoice, MidiPickerChoice, PortChoice, ProgramChoice,
};
use resonance_audio::types::InputDeviceInfo;

/// Fixed width of the numeric bank/program tile beside its picker in the
/// Patch card — the prototype's `grid-template-columns: 64px 1fr`.
const PATCH_TILE_COL: f32 = 64.0;

pub(super) fn external_instrument_group(
    r: &crate::Resonance,
    track: &TrackState,
    ext: &ExternalInstrumentState,
) -> Element<'static, Message> {
    // 8px inter-field gap matches the generic ROUTING group's rhythm
    // (see `routing_group`) so a normal track and an external one read
    // with the same vertical cadence.
    let mut col = column![
        super::widgets::group_header(
            "EXTERNAL INSTRUMENT",
            crate::state::MixerInspectorGroup::Routing,
            false,
        ),
        Space::new().height(10),
        ext_midi_output_block(r, track, ext),
        Space::new().height(8),
        ext_audio_return_block(r, track, ext),
        Space::new().height(8),
        ext_patch_block(r, track, ext),
        Space::new().height(8),
        ext_latency_block(r, track, ext),
        Space::new().height(8),
        ext_monitoring_block(track),
        Space::new().height(8),
        super::io::output_block(r, track),
        Space::new().height(10),
        disable_external_row(track.id),
    ]
    .spacing(0);

    // Device-offline alert — a configured MIDI-out or audio-return device
    // went away. The route is preserved (stale-override keeps it selected)
    // so a replug reconnects; the alert explains the outage and offers the
    // two recovery actions (todo #459, doc #169).
    if let Some(alert) = offline_alert(track, ext) {
        col = col.push(Space::new().height(12)).push(alert);
    }

    col.into()
}

/// Understated "Use built-in instrument" action that takes the track back
/// out of external mode. Styled like the other destructive-ish inspector
/// actions (dim text, hairline border, no fill) so it reads as a quiet exit
/// rather than a primary control — dispatching
/// `ExternalInstrumentMessage::Disable`, which drops the config and returns
/// the plain instrument ROUTING view (undo restores the map).
fn disable_external_row(track_id: resonance_audio::types::TrackId) -> Element<'static, Message> {
    button(
        text("Use built-in instrument")
            .size(11)
            .align_x(alignment::Horizontal::Center)
            .width(Length::Fill),
    )
    .padding([6, 0])
    .width(Length::Fill)
    .on_press(Message::ExternalInstrument(
        ExternalInstrumentMessage::Disable(track_id),
    ))
    .style(|_theme, status| {
        let hovered = matches!(status, button::Status::Hovered);
        button::Style {
            background: Some(iced::Background::Color(if hovered {
                theme::BG_2
            } else {
                theme::BG_1
            })),
            text_color: if hovered { theme::TEXT_2 } else { theme::TEXT_3 },
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        }
    })
    .into()
}

/// Inline BAD-pink alert shown when a configured device is offline. Returns
/// `None` when both endpoints are online. The title/body name the offline
/// endpoint (MIDI output takes precedence, matching the prototype), and the
/// two action buttons drive the recovery path: **Re-scan devices** re-checks
/// this track's endpoints against the live hardware (clearing offline and
/// restoring the live route when the device is back), and **Pick another
/// device…** refreshes the hardware lists so the pickers above offer a
/// working alternative.
fn offline_alert(
    track: &TrackState,
    ext: &ExternalInstrumentState,
) -> Option<Element<'static, Message>> {
    if !ext.midi_out_offline && !ext.return_input_offline {
        return None;
    }
    let track_id = track.id;
    let (title, device) = if ext.midi_out_offline {
        ("MIDI output unavailable", track.midi_output_device.clone())
    } else {
        ("Audio return unavailable", track.input_device_name.clone())
    };
    let device_name = device.unwrap_or_else(|| "The device".to_string());
    let body = format!(
        "\u{201c}{}\u{201d} isn't connected. Patch changes and automation can't \
         reach the synth, and the return input is silent. The route is kept, so \
         reconnecting restores it.",
        device_name
    );

    let rescan = super::widgets::alert_action_button(
        "Re-scan devices",
        Message::ExternalInstrument(ExternalInstrumentMessage::CheckDevices(track_id)),
    );
    let pick_another = super::widgets::alert_action_button(
        "Pick another device\u{2026}",
        Message::ExternalInstrument(ExternalInstrumentMessage::RescanDevices),
    );

    let inner = column![
        text(title).size(11).font(theme::UI_FONT_SEMIBOLD).color(theme::BAD),
        Space::new().height(4),
        text(body).size(11).color(theme::TEXT_1),
        Space::new().height(7),
        row![rescan, Space::new().width(7), pick_another]
            .align_y(alignment::Vertical::Center),
    ]
    .spacing(0);

    Some(
        container(inner)
            .width(Length::Fill)
            .padding([10, 12])
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::BAD_DIM)),
                border: iced::Border {
                    color: theme::BAD_LINE,
                    width: 1.0,
                    radius: theme::RADIUS_MD.into(),
                },
                ..Default::default()
            })
            .into(),
    )
}

/// MIDI Output — device + channel pickers (two-column). The device
/// picker carries the configured-but-offline device as a stale-override
/// entry so the route stays visible while the synth is unplugged.
fn ext_midi_output_block(
    r: &crate::Resonance,
    track: &TrackState,
    ext: &ExternalInstrumentState,
) -> Element<'static, Message> {
    let track_id = track.id;
    let out_choices = super::midi_choices_with_override(
        &r.view_caches.midi_output_choices,
        track.midi_output_device.as_deref(),
        &r.midi_output_devices,
    );
    let selected = MidiPickerChoice(track.midi_output_device.clone());
    let device_picker = pick_list(out_choices, Some(selected), move |choice| {
        Message::ExternalInstrument(ExternalInstrumentMessage::SetMidiOutDevice(track_id, choice.0))
    })
    .placeholder("(no MIDI out)")
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let channel_picker = pick_list(
        r.view_caches.output_channel_choices.clone(),
        Some(MidiChannelChoice(Some(track.midi_output_channel.unwrap_or(0)))),
        move |choice| {
            Message::ExternalInstrument(ExternalInstrumentMessage::SetMidiOutChannel(
                track_id, choice.0,
            ))
        },
    )
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    super::widgets::field2(
        "MIDI OUTPUT",
        ext.midi_out_offline,
        device_picker.into(),
        channel_picker.into(),
    )
}

/// Audio Return — input device + input-channel pickers (two-column),
/// reusing the audio-input device list and `PortChoice` "In N/N+1"
/// labels. A configured-but-offline return device is kept selectable via
/// a synthesized entry so the route survives an unplug.
fn ext_audio_return_block(
    r: &crate::Resonance,
    track: &TrackState,
    ext: &ExternalInstrumentState,
) -> Element<'static, Message> {
    let track_id = track.id;
    let configured = track.input_device_name.as_deref();
    let choices = super::io::return_device_choices(&r.view_caches.input_devices, configured);
    let selected_device = configured.and_then(|name| {
        use std::borrow::Borrow;
        let slice: &[InputDeviceInfo] = choices.borrow();
        slice.iter().find(|d| d.name == name).cloned()
    });
    let device_channels = selected_device.as_ref().map(|d| d.channels).unwrap_or(0);

    let device_picker = pick_list(choices, selected_device, move |device: InputDeviceInfo| {
        Message::ExternalInstrument(ExternalInstrumentMessage::SetReturnDevice(
            track_id,
            Some(device.name),
        ))
    })
    .placeholder("(no input)")
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    // Build the channel (port) picker when the selected device exposes
    // channels — mirrors `audio_input_block`'s mono/stereo pairing.
    let channel_picker: Element<'static, Message> = if device_channels > 0 {
        let is_mono = track.mono;
        let last_valid_index = if is_mono {
            device_channels
        } else {
            device_channels.saturating_sub(1)
        };
        let ports: Vec<PortChoice> = (0..last_valid_index)
            .map(|i| PortChoice {
                index: i,
                mono: is_mono,
            })
            .collect();
        if ports.is_empty() {
            super::widgets::placeholder_pick("—")
        } else {
            let selected_port = PortChoice {
                index: track
                    .input_port_index
                    .min(last_valid_index.saturating_sub(1)),
                mono: is_mono,
            };
            pick_list(ports, Some(selected_port), move |choice: PortChoice| {
                Message::ExternalInstrument(ExternalInstrumentMessage::SetReturnPort(
                    track_id,
                    choice.index,
                ))
            })
            .text_size(12)
            .padding([5, 8])
            .width(Length::Fill)
            .into()
        }
    } else {
        super::widgets::placeholder_pick("—")
    };

    super::widgets::field2(
        "AUDIO RETURN",
        ext.return_input_offline,
        device_picker.into(),
        channel_picker,
    )
}

/// Patch card — a Device-preset picker (epic #40) on top, then Bank (numeric
/// tile + Bank picker → CC0/CC32) and Program (numeric tile + Program picker
/// → Program Change) rows, with a note that the patch is re-sent on load and
/// at transport start. Selecting a preset stores its `device_id` and dispatches
/// `SetTrackDeviceParams`; the "<model> preset →" chip lights when a device is
/// selected. The card uses a static accent-lit style when a device or patch is
/// set (patch-pick flash/MIDI-dot animations need transient state #454 doesn't
/// carry).
fn ext_patch_block(
    r: &crate::Resonance,
    track: &TrackState,
    ext: &ExternalInstrumentState,
) -> Element<'static, Message> {
    let track_id = track.id;
    let has_patch = ext.bank.is_some() || ext.program.is_some();

    // Device-preset picker, fed by the registry-backed cached options. The
    // selected value is the matching cached choice (so its label renders on
    // the closed picker); a `None` id is the "(no device)" clear entry.
    let selected_device = r
        .view_caches
        .device_choices
        .iter()
        .find(|c| c.id == ext.device_id)
        .cloned();
    let has_device = ext.device_id.is_some();
    // The chip names the device only for a real selection — the "(no device)"
    // clear entry (id `None`) must read as the inactive hint, not its label.
    let selected_device_label = selected_device
        .as_ref()
        .filter(|c| c.id.is_some())
        .map(|c| c.label.clone());
    let device_picker = pick_list(
        r.view_caches.device_choices.clone(),
        selected_device,
        move |choice| {
            Message::ExternalInstrument(ExternalInstrumentMessage::SetDevice(
                track_id, choice.id,
            ))
        },
    )
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);
    let device_label = text("DEVICE PRESET")
        .size(9)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3);

    // User-definitions helpers: "Reveal folder" opens the user definitions
    // directory in the OS file manager so the user can drop in or edit a
    // `.json` definition file. "Re-scan" rebuilds the registry immediately so
    // the new file appears in the picker above without restarting.
    let reveal_btn = super::widgets::alert_action_button(
        "\u{f07b} Reveal folder",
        Message::ExternalInstrument(ExternalInstrumentMessage::RevealUserDefinitionsFolder),
    );
    let rescan_btn = super::widgets::alert_action_button(
        "\u{f021} Re-scan",
        Message::ExternalInstrument(ExternalInstrumentMessage::RescanDefinitions),
    );
    let def_actions = row![
        reveal_btn,
        Space::new().width(6),
        rescan_btn,
    ]
    .align_y(alignment::Vertical::Center);

    let bank_tile = super::widgets::pgnum_tile(match ext.bank {
        Some(bank) => format!("{:03}", bank),
        None => "—".to_string(),
    });
    let program_tile = super::widgets::pgnum_tile(match ext.program {
        Some(program) => format!("{:03}", program),
        None => "—".to_string(),
    });

    // When the selected preset resolves to a definition that ships named
    // patches, the two numeric Bank/Program pickers give way to a single
    // patch-by-name picker (grouped by bank/category); a pick resolves to the
    // entry's bank_msb/lsb + program and drives the same Bank Select + Program
    // Change path (doc #201 §5). With no device — or a device that has no
    // patch list — we fall back to the numeric pickers.
    let selected_def = ext
        .device_id
        .as_ref()
        .and_then(|id| r.device_registry.get(id));
    let named_patches = selected_def.map(|d| !d.patches.is_empty()).unwrap_or(false);

    let patch_section: Element<'static, Message> = if let Some(def) =
        selected_def.filter(|_| named_patches)
    {
        let choices = patch_choices(def);
        // Highlight the entry whose resolved bank/program matches the track's
        // current patch; when nothing matches (a custom bank/program not in
        // the definition) the picker shows the "(no patch)" clear entry.
        let selected = choices
            .iter()
            .find(|c| c.bank == ext.bank && c.program == ext.program)
            .cloned();
        // The group of the currently-selected named patch, shown as a small
        // read-out beside the program tile so the bank/category is legible on
        // the closed card.
        let selected_group = selected
            .as_ref()
            .and_then(|c| c.group.clone())
            .unwrap_or_else(|| "Named patches".to_string());
        let patch_picker = pick_list(choices, selected, move |choice| {
            Message::ExternalInstrument(ExternalInstrumentMessage::SetPatch(
                track_id,
                choice.bank,
                choice.program,
            ))
        })
        .text_size(12)
        .padding([5, 8])
        .width(Length::Fill);
        let group_readout = text(selected_group)
            .size(11)
            .color(theme::TEXT_2)
            .shaping(iced::widget::text::Shaping::Advanced);
        column![
            patch_row(bank_tile, patch_picker.into()),
            Space::new().height(8),
            patch_row(
                program_tile,
                container(group_readout)
                    .align_y(alignment::Vertical::Center)
                    .into()
            ),
        ]
        .spacing(0)
        .into()
    } else {
        let bank_picker = pick_list(
            r.view_caches.bank_choices.clone(),
            Some(BankChoice(ext.bank)),
            move |choice| {
                Message::ExternalInstrument(ExternalInstrumentMessage::SetBank(track_id, choice.0))
            },
        )
        .text_size(12)
        .padding([5, 8])
        .width(Length::Fill);
        let program_picker = pick_list(
            r.view_caches.program_choices.clone(),
            Some(ProgramChoice(ext.program)),
            move |choice| {
                Message::ExternalInstrument(ExternalInstrumentMessage::SetProgram(
                    track_id, choice.0,
                ))
            },
        )
        .text_size(12)
        .padding([5, 8])
        .width(Length::Fill);
        column![
            patch_row(bank_tile, bank_picker.into()),
            Space::new().height(8),
            patch_row(program_tile, program_picker.into()),
        ]
        .spacing(0)
        .into()
    };

    let note = text(
        "Sends Bank Select (CC0/CC32) + Program Change. Re-sent on project \
         load and at transport start so the synth is always in the right patch.",
    )
    .size(10)
    .color(theme::TEXT_3);

    let card_border = if has_patch || has_device {
        theme::ACCENT_LINE
    } else {
        theme::LINE
    };
    let card = container(
        column![
            device_label,
            Space::new().height(6),
            device_picker,
            Space::new().height(6),
            def_actions,
            Space::new().height(10),
            patch_section,
            Space::new().height(8),
            note,
        ]
        .spacing(0),
    )
    .width(Length::Fill)
    .padding(10)
    .style(move |_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: card_border,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    });

    // The preset chip lights (accent) and names the selected device once a
    // preset is chosen (epic #40); until then it reads as a dim hint.
    let label = row![
        text("PATCH")
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        Space::new().width(Length::Fill),
        super::widgets::preset_hint_chip(selected_device_label),
    ]
    .align_y(alignment::Vertical::Center);

    column![label, Space::new().height(6), card].spacing(0).into()
}

/// One Patch-card row: a fixed-width numeric tile beside its picker.
fn patch_row(
    tile: Element<'static, Message>,
    picker: Element<'static, Message>,
) -> Element<'static, Message> {
    row![
        container(tile).width(Length::Fixed(PATCH_TILE_COL)),
        Space::new().width(8),
        container(picker).width(Length::Fill),
    ]
    .align_y(alignment::Vertical::Center)
    .into()
}

/// Latency Compensation — ms + sample readout, a manual offset slider,
/// and a disabled Auto-detect (ping) button (the ping command itself is
/// todo #453).
fn ext_latency_block(
    r: &crate::Resonance,
    track: &TrackState,
    ext: &ExternalInstrumentState,
) -> Element<'static, Message> {
    let track_id = track.id;
    let sample_rate = r.sample_rate.max(1) as f32;
    let samples = ext.latency_offset_samples;
    let ms = samples as f32 / sample_rate * 1000.0;

    let readout = row![
        text(format!("{:.1}", ms))
            .size(15)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_1),
        Space::new().width(4),
        text("ms").size(10).color(theme::TEXT_3),
        Space::new().width(Length::Fill),
        text(format!("{} smp @ {:.1}k", samples, sample_rate / 1000.0))
            .size(10)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_3),
    ]
    .align_y(alignment::Vertical::Center);

    // Manual offset slider in milliseconds (0..40 ms, matching the
    // prototype range); converted to samples for the engine message.
    let slider_widget = slider(0.0..=40.0f32, ms.clamp(0.0, 40.0), move |new_ms| {
        let new_samples = (new_ms / 1000.0 * sample_rate).round() as i64;
        Message::ExternalInstrument(ExternalInstrumentMessage::SetLatencyOffset(
            track_id,
            new_samples,
        ))
    })
    .step(0.1f32)
    .width(Length::Fill);

    // Disabled until the auto-detect ping command lands (#453). It has
    // no `on_press`, and a flat hover-less style so it never implies it
    // is clickable.
    let ping_button = button(
        text("Auto-detect (ping)")
            .size(11)
            .color(theme::TEXT_4)
            .align_x(alignment::Horizontal::Center)
            .width(Length::Fill),
    )
    .padding([6, 0])
    .width(Length::Fill)
    .style(|_theme, _status| button::Style {
        background: Some(iced::Background::Color(theme::BG_1)),
        text_color: theme::TEXT_4,
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    });

    let box_inner = column![
        readout,
        Space::new().height(10),
        slider_widget,
        Space::new().height(10),
        ping_button,
    ]
    .spacing(0);

    super::widgets::field(
        "LATENCY COMPENSATION",
        container(box_inner)
            .width(Length::Fill)
            .padding(10)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::BG_2)),
                border: iced::Border {
                    color: theme::LINE,
                    width: 1.0,
                    radius: theme::RADIUS_MD.into(),
                },
                ..Default::default()
            })
            .into(),
    )
}

/// Return Monitoring — Input monitor (mint when on) + Record arm
/// (BAD-pink when armed) toggles, sharing the track's per-track capture
/// state with the strip buttons (todo #458).
fn ext_monitoring_block(track: &TrackState) -> Element<'static, Message> {
    let track_id = track.id;
    let mon = super::widgets::toggle_button(
        "Input monitor",
        track.monitor_enabled,
        theme::GOOD,
        theme::GOOD_DIM,
        Message::ExternalInstrument(ExternalInstrumentMessage::ToggleMonitor(track_id)),
    );
    let arm = super::widgets::toggle_button(
        "Record arm",
        track.record_armed,
        theme::BAD,
        theme::BAD_DIM,
        Message::ExternalInstrument(ExternalInstrumentMessage::ToggleRecordArm(track_id)),
    );
    super::widgets::field(
        "RETURN MONITORING",
        row![
            container(mon).width(Length::FillPortion(1)),
            Space::new().width(8),
            container(arm).width(Length::FillPortion(1)),
        ]
        .into(),
    )
}
