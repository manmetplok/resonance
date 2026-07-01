//! Mixer Inspector panel — the right-side detail pane that shows the
//! currently selected track's signal, routing, and plugin chain. Hosts
//! the functional pickers that used to live on the strip itself: input
//! device + channel (audio) or MIDI in / channel and MIDI out / channel
//! (instruments), output destination (Master / Bus N), and an "+ FX"
//! picker for the chain.

use iced::widget::{button, column, container, pick_list, row, slider, text, Space};
use iced::widget::text::Shaping;
use iced::{alignment, Element, Length};
use resonance_audio::types::{InputDeviceInfo, ScannedPlugin, TrackOutput, TrackType};

use crate::message::*;
use crate::state::{
    ExternalInstrumentState, ExternalInstrumentStatus, MixerInspectorGroup, TrackState,
};
use crate::theme;
use crate::util::format_pan;
use crate::view::controls::collapse_caret;

use super::picks::{
    BankChoice, MidiChannelChoice, MidiPickerChoice, OutputChoice, PortChoice, ProgramChoice,
};
use crate::view::ui_caches::ChoiceList;

/// Combine the cached MIDI device list with a per-track override entry
/// for the rare case where the track's configured device is no longer
/// enumerated by the engine (controller unplugged). Normal path returns
/// the `Cached` variant — a cheap `Rc` clone with no allocation.
fn midi_choices_with_override(
    cached: &std::rc::Rc<[MidiPickerChoice]>,
    configured: Option<&str>,
    available: &[resonance_audio::MidiDeviceInfo],
) -> ChoiceList<MidiPickerChoice> {
    match configured.filter(|name| !available.iter().any(|d| d.name == *name)) {
        Some(stale) => {
            let mut v: Vec<MidiPickerChoice> = cached.iter().cloned().collect();
            v.push(MidiPickerChoice(Some(stale.to_string())));
            ChoiceList::Owned(v)
        }
        None => ChoiceList::Cached(cached.clone()),
    }
}

pub(super) fn view<'a>(r: &'a crate::Resonance) -> Element<'a, Message> {
    let selected_id = r.interaction.selected_track;
    let selected = selected_id.and_then(|id| r.registry.tracks.iter().find(|t| t.id == id));

    let body: Element<'a, Message> = match selected {
        Some(track) => {
            let signal_collapsed = r
                .mixer
                .collapsed_inspector_groups
                .contains(&MixerInspectorGroup::Signal);
            let routing_collapsed = r
                .mixer
                .collapsed_inspector_groups
                .contains(&MixerInspectorGroup::Routing);
            let chain_collapsed = r
                .mixer
                .collapsed_inspector_groups
                .contains(&MixerInspectorGroup::Chain);

            // An external-instrument track is identified purely by its
            // presence in the `external_instruments` map (todo #454) —
            // there's no track-type discriminant. When present, the
            // SIGNAL group reads "Signal · Return" (the metered signal is
            // the hardware return) and ROUTING becomes the External
            // Instrument group.
            let is_external = r.external_instruments.contains_key(&track.id);
            // Derived lifecycle status drives the inspector header badge,
            // the onboarding card, and the device-offline alert (todo
            // #459). Computed from the config + live device flags so it can
            // never drift out of sync with the state it renders.
            let ext_status = r
                .external_instruments
                .get(&track.id)
                .map(|ext| ext.status(track));

            // Title row: the track name, followed by the status badge on
            // external-instrument tracks (Unconfigured / Configuring /
            // Live / Offline), mirroring the prototype's inspector badge.
            let mut title_row = row![text(track.name.clone())
                .size(17)
                .font(theme::UI_FONT_MEDIUM)
                .color(theme::TEXT_1)]
            .spacing(0)
            .align_y(alignment::Vertical::Center);
            if let Some(status) = ext_status {
                title_row = title_row
                    .push(Space::new().width(8))
                    .push(status_badge(status));
            }
            let header = column![
                text("INSPECTOR")
                    .size(10)
                    .font(theme::UI_FONT_SEMIBOLD)
                    .color(theme::TEXT_3),
                Space::new().height(2),
                title_row,
            ]
            .spacing(0);

            // SIGNAL stays outside the lazy region: its PEAK tile reads
            // the per-tick track levels, which the fingerprint below
            // deliberately omits (see ui-work.md §11.2 — never key a
            // lazy region without the live data it renders).
            let signal = signal_group(track, signal_collapsed, is_external);

            // ROUTING + CHAIN only change on slow events (device lists,
            // routing edits, chain edits, collapse toggles) — all hashed
            // into the fingerprint, so the cached tree is reused across
            // audio ticks.
            let fp = inspector_fingerprint(r, track, routing_collapsed, chain_collapsed);
            let lazy_groups =
                iced::widget::lazy(fp, move |_: &u64| -> Element<'static, Message> {
                    let mut col = column![].spacing(0);
                    // Fresh external-instrument track (nothing paired yet):
                    // a dashed onboarding card walks the user through the
                    // four setup steps before the routing pickers (#459).
                    if ext_status == Some(ExternalInstrumentStatus::Unconfigured) {
                        col = col
                            .push(onboarding_card())
                            .push(Space::new().height(18));
                    }
                    col = col.push(routing_group(r, track, routing_collapsed));
                    col = col
                        .push(Space::new().height(18))
                        .push(chain_group(r, track, chain_collapsed));
                    col.into()
                });

            iced::widget::scrollable(
                column![
                    header,
                    Space::new().height(18),
                    signal,
                    Space::new().height(18),
                    lazy_groups,
                ]
                .spacing(0),
            )
            .height(Length::Fill)
            .into()
        }
        None => render_empty(),
    };

    container(body)
        .width(Length::Fixed(theme::INSPECTOR_WIDTH))
        .height(Length::Fill)
        .padding(26)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Hash every field the lazy ROUTING + CHAIN groups read. The lazy
/// widget compares this across frames — when nothing has changed, the
/// cached widget tree is reused (which is the resize hot path). The
/// live level fields are intentionally absent: the SIGNAL group renders
/// them per-frame *outside* the lazy region.
fn inspector_fingerprint(
    r: &crate::Resonance,
    t: &TrackState,
    routing_collapsed: bool,
    chain_collapsed: bool,
) -> u64 {
    use std::hash::{Hash, Hasher};
    use std::rc::Rc;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    routing_collapsed.hash(&mut h);
    chain_collapsed.hash(&mut h);
    t.id.hash(&mut h);
    t.name.hash(&mut h);
    t.track_type.hash(&mut h);
    t.sub_track.hash(&mut h);
    t.input_device_name.hash(&mut h);
    t.input_port_index.hash(&mut h);
    t.mono.hash(&mut h);
    t.midi_input_device.hash(&mut h);
    t.midi_input_channel.hash(&mut h);
    t.midi_output_device.hash(&mut h);
    t.midi_output_channel.hash(&mut h);
    t.output.hash(&mut h);
    t.pan.to_bits().hash(&mut h);
    // External-instrument fields the routing group reads: the monitor /
    // arm toggles, the patch + latency config, and the runtime offline
    // flags. Without these the lazy region wouldn't redraw when a patch
    // is picked or a device goes offline.
    t.monitor_enabled.hash(&mut h);
    t.record_armed.hash(&mut h);
    let is_external = r.external_instruments.contains_key(&t.id);
    is_external.hash(&mut h);
    if let Some(ext) = r.external_instruments.get(&t.id) {
        ext.bank.hash(&mut h);
        ext.program.hash(&mut h);
        ext.latency_offset_samples.hash(&mut h);
        ext.midi_out_offline.hash(&mut h);
        ext.return_input_offline.hash(&mut h);
        // The latency readout is in ms, derived from the sample rate.
        r.sample_rate.hash(&mut h);
    }
    for p in &t.plugins {
        p.instance_id.hash(&mut h);
        p.plugin_name.hash(&mut h);
    }
    // Cache pointers — when these Rcs are replaced, the inspector
    // needs to redraw with the new options.
    Rc::as_ptr(&r.view_caches.midi_input_choices).hash(&mut h);
    Rc::as_ptr(&r.view_caches.midi_output_choices).hash(&mut h);
    Rc::as_ptr(&r.view_caches.output_choices).hash(&mut h);
    Rc::as_ptr(&r.view_caches.fx_plugins).hash(&mut h);
    Rc::as_ptr(&r.view_caches.instrument_plugins).hash(&mut h);
    // Audio input picker uses `r.input_devices` directly (its options
    // include the per-device channel count, which the cached choice
    // lists above don't carry).
    r.input_devices.len().hash(&mut h);
    for d in &r.input_devices {
        d.name.hash(&mut h);
        d.channels.hash(&mut h);
    }
    // Live MIDI device list too — the audio block isn't a strict
    // function of the cached lists since the stale-override branch
    // peeks at `midi_input_devices` directly.
    r.midi_input_devices.len().hash(&mut h);
    r.midi_output_devices.len().hash(&mut h);
    h.finish()
}

fn render_empty() -> Element<'static, Message> {
    column![
        text("INSPECTOR")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        Space::new().height(8),
        text("Select a track or bus to view its routing and signal.")
            .size(12)
            .color(theme::TEXT_3),
    ]
    .spacing(0)
    .into()
}

// ---------------------------------------------------------------------------
// SIGNAL group — 2×2 stat tiles.
// ---------------------------------------------------------------------------

fn signal_group(
    track: &TrackState,
    collapsed: bool,
    is_external: bool,
) -> Element<'static, Message> {
    // External-instrument tracks meter the hardware *return*, so the
    // group reads "Signal · Return" per design doc #169.
    let title = if is_external {
        "SIGNAL · RETURN"
    } else {
        "SIGNAL"
    };
    if collapsed {
        return group_header(title, MixerInspectorGroup::Signal, true);
    }

    let peak = track.level_l.max(track.level_r);
    let peak_db = if peak < 1e-4 {
        "−∞ dB".to_string()
    } else {
        format!("{:+.1} dB", 20.0 * peak.log10())
    };

    let rms = "—".to_string();

    let pan = format_pan(track.pan).into_owned();
    let out = match track.output {
        TrackOutput::Master => "Master".to_string(),
        TrackOutput::Bus(_) => "Bus".to_string(),
    };

    let row1 = row![
        stat_tile("PEAK", peak_db),
        Space::new().width(10),
        stat_tile("RMS", rms),
    ]
    .align_y(alignment::Vertical::Center);
    let row2 = row![
        stat_tile("PAN", pan),
        Space::new().width(10),
        stat_tile("OUT", out),
    ]
    .align_y(alignment::Vertical::Center);

    column![
        group_header(title, MixerInspectorGroup::Signal, false),
        Space::new().height(10),
        row1,
        Space::new().height(10),
        row2,
    ]
    .spacing(0)
    .into()
}

fn stat_tile(label: &'static str, value: String) -> Element<'static, Message> {
    container(
        column![
            text(label)
                .size(9)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::TEXT_3),
            Space::new().height(3),
            text(value)
                .size(13)
                .font(theme::MONO_FONT)
                .color(theme::TEXT_1),
        ]
        .spacing(0),
    )
    .width(Length::FillPortion(1))
    .padding([8, 10])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    })
    .into()
}

// ---------------------------------------------------------------------------
// ROUTING group — functional pickers + read-only send rows.
// ---------------------------------------------------------------------------

fn routing_group(
    r: &crate::Resonance,
    track: &TrackState,
    collapsed: bool,
) -> Element<'static, Message> {
    // External-instrument tracks replace the generic input / MIDI-out
    // routing with the dedicated "External Instrument" group (doc #169).
    if let Some(ext) = r.external_instruments.get(&track.id) {
        if collapsed {
            return group_header("EXTERNAL INSTRUMENT", MixerInspectorGroup::Routing, true);
        }
        return external_instrument_group(r, track, ext);
    }

    if collapsed {
        return group_header("ROUTING", MixerInspectorGroup::Routing, true);
    }

    let input_block: Element<'static, Message> = match track.track_type {
        TrackType::Audio => audio_input_block(r, track),
        TrackType::Instrument | TrackType::Vocal => midi_input_block(r, track),
    };
    let output_block = output_block(r, track);
    let midi_out_block: Element<'static, Message> =
        if track.track_type.accepts_midi() && track.sub_track.is_none() {
            midi_output_block(r, track)
        } else {
            Space::new().height(0).into()
        };

    column![
        group_header("ROUTING", MixerInspectorGroup::Routing, false),
        Space::new().height(10),
        input_block,
        Space::new().height(8),
        output_block,
        Space::new().height(8),
        midi_out_block,
        Space::new().height(4),
        routing_row("Send A", "(none)", true),
        routing_row("Send B", "(none)", true),
    ]
    .spacing(0)
    .into()
}

// ---------------------------------------------------------------------------
// EXTERNAL INSTRUMENT group — replaces the generic ROUTING fields for a
// track that's been marked an external instrument (todo #454). Pairs a
// hardware MIDI output with an audio return, exposes patch (Bank/Program)
// + latency compensation, and reuses the per-track monitor/arm toggles.
// All controls dispatch `ExternalInstrumentMessage` variants from #454.
// Status badges, the onboarding card, and the device-offline alert are
// owned by todo #459; the auto-detect ping command is todo #453, so that
// button renders as a disabled affordance here.
// ---------------------------------------------------------------------------

/// Fixed width of the right-hand "aux" column in a two-column external
/// field (the MIDI channel / return-port picker) — the prototype's
/// `grid-template-columns: 1fr 92px`.
const EXT_AUX_COL: f32 = 92.0;
/// Fixed width of the numeric bank/program tile beside its picker in the
/// Patch card — the prototype's `grid-template-columns: 64px 1fr`.
const PATCH_TILE_COL: f32 = 64.0;

fn external_instrument_group(
    r: &crate::Resonance,
    track: &TrackState,
    ext: &ExternalInstrumentState,
) -> Element<'static, Message> {
    // 8px inter-field gap matches the generic ROUTING group's rhythm
    // (see `routing_group`) so a normal track and an external one read
    // with the same vertical cadence.
    let mut col = column![
        group_header("EXTERNAL INSTRUMENT", MixerInspectorGroup::Routing, false),
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
        output_block(r, track),
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

    let rescan = alert_action_button(
        "Re-scan devices",
        Message::ExternalInstrument(ExternalInstrumentMessage::CheckDevices(track_id)),
    );
    let pick_another = alert_action_button(
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

/// A small recovery-action button used inside the offline alert (matching
/// the prototype's `.alert .fix button`).
fn alert_action_button(label: &'static str, msg: Message) -> Element<'static, Message> {
    button(text(label).size(10).color(theme::TEXT_2))
        .padding([4, 10])
        .on_press(msg)
        .style(|_theme, status| {
            let hovered = matches!(status, button::Status::Hovered);
            button::Style {
                background: Some(iced::Background::Color(if hovered {
                    theme::BG_2
                } else {
                    theme::BG_1
                })),
                text_color: theme::TEXT_2,
                border: iced::Border {
                    color: theme::LINE,
                    width: 1.0,
                    radius: theme::RADIUS_XS.into(),
                },
                ..Default::default()
            }
        })
        .into()
}

/// Dashed onboarding card shown for a fresh (Unconfigured) external
/// instrument track — an intro line plus four numbered setup steps (MIDI
/// out → return → patch → latency), mirroring the prototype's `.empty`
/// guidance block (todo #459, doc #169).
fn onboarding_card() -> Element<'static, Message> {
    let intro = text(
        "External instrument track. Pair a hardware synth's MIDI output with its \
         audio return so it plays and records in-line like a built-in instrument. \
         To set it up:",
    )
    .size(11)
    .color(theme::TEXT_2);

    let steps = column![
        onboarding_step(1, "Pick the synth's MIDI output device + channel below."),
        Space::new().height(9),
        onboarding_step(2, "Pick the audio return input the synth is wired into."),
        Space::new().height(9),
        onboarding_step(
            3,
            "Choose a patch (Bank + Program) — Resonance re-sends it on load & play.",
        ),
        Space::new().height(9),
        onboarding_step(
            4,
            "Dial in latency compensation so the return lines up with the grid.",
        ),
    ]
    .spacing(0);

    container(column![intro, Space::new().height(10), steps].spacing(0))
        .width(Length::Fill)
        .padding(12)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::ACCENT_LINE,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        })
        .into()
}

/// One numbered onboarding step: a lavender numbered chip beside its label.
fn onboarding_step(n: u8, label: &'static str) -> Element<'static, Message> {
    let chip = container(
        text(n.to_string())
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::ACCENT_SOFT),
    )
    .width(Length::Fixed(16.0))
    .height(Length::Fixed(16.0))
    .align_x(alignment::Horizontal::Center)
    .align_y(alignment::Vertical::Center)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::ACCENT_DIM)),
        border: iced::Border {
            color: theme::ACCENT_LINE,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    });

    row![
        chip,
        Space::new().width(9),
        text(label).size(11).color(theme::TEXT_2).width(Length::Fill),
    ]
    .align_y(alignment::Vertical::Top)
    .into()
}

/// Inspector status badge for an external-instrument track — Unconfigured
/// (accent), Configuring (warm), Live (good), Offline (bad). Mirrors the
/// prototype's `.badge` pill styling (todo #459, doc #169).
fn status_badge(status: ExternalInstrumentStatus) -> Element<'static, Message> {
    let (label, fg, bg, line) = match status {
        ExternalInstrumentStatus::Unconfigured => (
            "Unconfigured",
            theme::ACCENT_SOFT,
            theme::ACCENT_DIM,
            theme::ACCENT_LINE,
        ),
        ExternalInstrumentStatus::Configuring => {
            ("Configuring", theme::WARM, theme::WARM_DIM, theme::WARM_LINE)
        }
        ExternalInstrumentStatus::Live => {
            ("Live", theme::GOOD, theme::GOOD_DIM, theme::GOOD_LINE)
        }
        ExternalInstrumentStatus::Offline => {
            ("Offline", theme::BAD, theme::BAD_DIM, theme::BAD_LINE)
        }
    };
    container(
        text(label)
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(fg),
    )
    .padding([2, 7])
    .style(move |_theme| container::Style {
        background: Some(iced::Background::Color(bg)),
        border: iced::Border {
            color: line,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    })
    .into()
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
    let out_choices = midi_choices_with_override(
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

    field2(
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
    let choices = return_device_choices(&r.view_caches.input_devices, configured);
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
            placeholder_pick("—")
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
        placeholder_pick("—")
    };

    field2(
        "AUDIO RETURN",
        ext.return_input_offline,
        device_picker.into(),
        channel_picker,
    )
}

/// Patch card — Bank (numeric tile + Bank picker → CC0/CC32) and Program
/// (numeric tile + Program picker → Program Change) rows, with a note that
/// the patch is re-sent on load and at transport start. The "Muse
/// preset →" affordance is a disabled hook for the device-preset epic
/// (#40); patch-pick flash/MIDI-dot animations need transient state that
/// #454 doesn't carry, so the card uses a static accent-lit style when a
/// patch is set.
fn ext_patch_block(
    r: &crate::Resonance,
    track: &TrackState,
    ext: &ExternalInstrumentState,
) -> Element<'static, Message> {
    let track_id = track.id;
    let has_patch = ext.bank.is_some() || ext.program.is_some();

    let bank_tile = pgnum_tile(match ext.bank {
        Some(bank) => format!("{:03}", bank),
        None => "—".to_string(),
    });
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

    let program_tile = pgnum_tile(match ext.program {
        Some(program) => format!("{:03}", program),
        None => "—".to_string(),
    });
    let program_picker = pick_list(
        r.view_caches.program_choices.clone(),
        Some(ProgramChoice(ext.program)),
        move |choice| {
            Message::ExternalInstrument(ExternalInstrumentMessage::SetProgram(track_id, choice.0))
        },
    )
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let note = text(
        "Sends Bank Select (CC0/CC32) + Program Change. Re-sent on project \
         load and at transport start so the synth is always in the right patch.",
    )
    .size(10)
    .color(theme::TEXT_3);

    let card_border = if has_patch {
        theme::ACCENT_LINE
    } else {
        theme::LINE
    };
    let card = container(
        column![
            patch_row(bank_tile, bank_picker.into()),
            Space::new().height(8),
            patch_row(program_tile, program_picker.into()),
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

    // The "Muse preset →" chip is a disabled affordance for epic #40 —
    // when device presets land, patch names replace the raw numbers.
    let label = row![
        text("PATCH")
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        Space::new().width(Length::Fill),
        preset_hint_chip(),
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

/// Compact zero-padded bank/program numeric readout tile (e.g. `031` /
/// `012`), mirroring the prototype's `.pgnum`.
fn pgnum_tile(value: String) -> Element<'static, Message> {
    container(
        text(value)
            .size(15)
            .font(theme::MONO_FONT)
            .color(theme::ACCENT_SOFT),
    )
    .width(Length::Fill)
    .padding([6, 0])
    .align_x(alignment::Horizontal::Center)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_1)),
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    })
    .into()
}

/// Disabled "Muse preset →" chip — the named-patch affordance reserved
/// for the device-preset epic (#40).
fn preset_hint_chip() -> Element<'static, Message> {
    container(
        text("Muse preset →")
            .size(8)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_4),
    )
    .padding([1, 5])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    })
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

    field(
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
    let mon = toggle_button(
        "Input monitor",
        track.monitor_enabled,
        theme::GOOD,
        theme::GOOD_DIM,
        Message::ExternalInstrument(ExternalInstrumentMessage::ToggleMonitor(track_id)),
    );
    let arm = toggle_button(
        "Record arm",
        track.record_armed,
        theme::BAD,
        theme::BAD_DIM,
        Message::ExternalInstrument(ExternalInstrumentMessage::ToggleRecordArm(track_id)),
    );
    field(
        "RETURN MONITORING",
        row![
            container(mon).width(Length::FillPortion(1)),
            Space::new().width(8),
            container(arm).width(Length::FillPortion(1)),
        ]
        .into(),
    )
}

/// A two-state toggle button: neutral when off, tinted with `on_color`
/// (text/border) over `on_bg` (fill) when on.
fn toggle_button(
    label: &'static str,
    on: bool,
    on_color: iced::Color,
    on_bg: iced::Color,
    msg: Message,
) -> Element<'static, Message> {
    button(
        text(label)
            .size(11)
            .align_x(alignment::Horizontal::Center)
            .width(Length::Fill),
    )
    .padding([7, 0])
    .width(Length::Fill)
    .on_press(msg)
    .style(move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered);
        let (bg, border, txt) = if on {
            (on_bg, on_color, on_color)
        } else if hovered {
            (theme::BG_3, theme::LINE, theme::TEXT_1)
        } else {
            (theme::BG_2, theme::LINE, theme::TEXT_3)
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: txt,
            border: iced::Border {
                color: border,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        }
    })
    .into()
}

/// Build the audio-return device option list, appending a synthesized
/// entry for a configured-but-unenumerated device (offline / replug
/// pending) so the route stays selected. Mirrors
/// `midi_choices_with_override` for the audio-input device list.
fn return_device_choices(
    cached: &std::rc::Rc<[InputDeviceInfo]>,
    configured: Option<&str>,
) -> ChoiceList<InputDeviceInfo> {
    match configured.filter(|name| !cached.iter().any(|d| &d.name == name)) {
        Some(stale) => {
            let mut v: Vec<InputDeviceInfo> = cached.iter().cloned().collect();
            v.push(InputDeviceInfo {
                name: stale.to_string(),
                description: stale.to_string(),
                // Channels are unknown while the device is gone; assume a
                // stereo pair so the port picker still offers "In 1/2".
                channels: 2,
            });
            ChoiceList::Owned(v)
        }
        None => ChoiceList::Cached(cached.clone()),
    }
}

/// A non-interactive picker-shaped placeholder used where a second
/// two-column field has no live control yet (e.g. no return channels).
fn placeholder_pick(value: &'static str) -> Element<'static, Message> {
    container(text(value).size(12).color(theme::TEXT_4))
        .width(Length::Fill)
        .padding([5, 8])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Two-column field: a label (with an optional BAD-pink "offline" tag)
/// above a row of two pickers (`device` on the left grows, `aux` on the
/// right is a fixed-narrow column), matching the prototype's `.two`
/// grid (`1fr 92px`).
fn field2(
    label: &'static str,
    offline: bool,
    device: Element<'static, Message>,
    aux: Element<'static, Message>,
) -> Element<'static, Message> {
    let mut label_row = row![text(label)
        .size(9)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3)]
    .align_y(alignment::Vertical::Center);
    if offline {
        label_row = label_row
            .push(Space::new().width(6))
            .push(text("offline").size(8).color(theme::BAD));
    }
    column![
        label_row,
        Space::new().height(4),
        row![
            container(device).width(Length::Fill),
            Space::new().width(8),
            container(aux).width(Length::Fixed(EXT_AUX_COL)),
        ]
        .align_y(alignment::Vertical::Center),
    ]
    .spacing(0)
    .into()
}

/// Stacked field label + picker block used inside the ROUTING group.
fn field(label: &'static str, picker: Element<'static, Message>) -> Element<'static, Message> {
    column![
        text(label)
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        Space::new().height(4),
        picker,
    ]
    .spacing(0)
    .into()
}

fn audio_input_block(
    r: &crate::Resonance,
    track: &TrackState,
) -> Element<'static, Message> {
    let track_id = track.id;
    let selected_device = track
        .input_device_name
        .as_ref()
        .and_then(|name| r.input_devices.iter().find(|d| &d.name == name))
        .cloned();
    let device_channels = selected_device.as_ref().map(|d| d.channels).unwrap_or(0);

    let device_picker = pick_list(
        // Cached `Rc<[InputDeviceInfo]>` — clones are cheap (refcount).
        // Rebuilt by `engine_events::transport::input_devices_listed`
        // only when the engine re-enumerates devices.
        r.view_caches.input_devices.clone(),
        selected_device,
        move |device: InputDeviceInfo| {
            Message::Track(TrackMessage::SetTrackInputDevice(track_id, Some(device.name)))
        },
    )
    .placeholder("(no input)")
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let mut col = column![
        field("INPUT DEVICE", device_picker.into()),
    ]
    .spacing(0);

    if device_channels > 0 {
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
        if !ports.is_empty() {
            let selected_port = PortChoice {
                index: track
                    .input_port_index
                    .min(last_valid_index.saturating_sub(1)),
                mono: is_mono,
            };
            let port_picker = pick_list(ports, Some(selected_port), move |choice: PortChoice| {
                Message::Track(TrackMessage::SetTrackInputPort(track_id, choice.index))
            })
            .text_size(12)
            .padding([5, 8])
            .width(Length::Fill);
            col = col
                .push(Space::new().height(8))
                .push(field("INPUT CHANNEL", port_picker.into()));
        }
    }
    col.into()
}

fn midi_input_block(
    r: &crate::Resonance,
    track: &TrackState,
) -> Element<'static, Message> {
    let track_id = track.id;
    // Pull the cached "(None) + every device" list off Resonance; only
    // synthesize a one-off Vec when the track is bound to a configured
    // device that's not currently enumerated (controller unplugged).
    let in_choices = midi_choices_with_override(
        &r.view_caches.midi_input_choices,
        track.midi_input_device.as_deref(),
        &r.midi_input_devices,
    );
    let in_selected = MidiPickerChoice(track.midi_input_device.clone());
    let in_picker = pick_list(in_choices, Some(in_selected), move |choice| {
        Message::Track(TrackMessage::SetTrackMidiInputDevice(track_id, choice.0))
    })
    .placeholder("(no MIDI in)")
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let mut col = column![field("MIDI INPUT", in_picker.into())].spacing(0);

    if track.midi_input_device.is_some() {
        let in_ch_picker = pick_list(
            r.view_caches.input_channel_choices.clone(),
            Some(MidiChannelChoice(track.midi_input_channel)),
            move |choice| {
                Message::Track(TrackMessage::SetTrackMidiInputChannel(track_id, choice.0))
            },
        )
        .text_size(12)
        .padding([5, 8])
        .width(Length::Fill);
        col = col
            .push(Space::new().height(8))
            .push(field("MIDI IN CHANNEL", in_ch_picker.into()));
    }
    col.into()
}

fn midi_output_block(
    r: &crate::Resonance,
    track: &TrackState,
) -> Element<'static, Message> {
    let track_id = track.id;
    let out_choices = midi_choices_with_override(
        &r.view_caches.midi_output_choices,
        track.midi_output_device.as_deref(),
        &r.midi_output_devices,
    );
    let out_selected = MidiPickerChoice(track.midi_output_device.clone());
    let out_picker = pick_list(out_choices, Some(out_selected), move |choice| {
        Message::Track(TrackMessage::SetTrackMidiOutputDevice(track_id, choice.0))
    })
    .placeholder("(no MIDI out)")
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let mut col = column![field("MIDI OUTPUT", out_picker.into())].spacing(0);

    if track.midi_output_device.is_some() {
        let selected = MidiChannelChoice(Some(track.midi_output_channel.unwrap_or(0)));
        let out_ch_picker = pick_list(
            r.view_caches.output_channel_choices.clone(),
            Some(selected),
            move |choice| {
                Message::Track(TrackMessage::SetTrackMidiOutputChannel(track_id, choice.0))
            },
        )
        .text_size(12)
        .padding([5, 8])
        .width(Length::Fill);
        col = col
            .push(Space::new().height(8))
            .push(field("MIDI OUT CHANNEL", out_ch_picker.into()));
    }
    col.into()
}

fn output_block(r: &crate::Resonance, track: &TrackState) -> Element<'static, Message> {
    let track_id = track.id;
    let cached = r.view_caches.output_choices.clone();
    // `cached` is normally seeded with at least a Master entry, but
    // defend against any future code path that clears it (or a window
    // between project-load and `rebuild_output`) and against a track
    // routed to a bus that's not in the cached list (e.g. mid-replay).
    // The previous `choices[0]` fallback panicked when the cache was
    // empty (`index out of bounds: the len is 0 but the index is 0`)
    // on a fresh project where `rebuild_output` had never fired.
    let (choices, selected) = match cached.iter().find(|c| c.output == track.output).cloned() {
        Some(c) => (ChoiceList::Cached(cached), c),
        None => {
            // Track's output not in the cached list (or list is empty).
            // Synthesize a label and append/prepend it to a one-shot
            // owned list so the picker shows the track's actual routing
            // without panicking.
            use crate::theme::fa;
            let label = match track.output {
                TrackOutput::Master => format!("{} Master", fa::ARROW_RIGHT),
                TrackOutput::Bus(bus_id) => {
                    let name = r
                        .registry
                        .busses
                        .iter()
                        .find(|b| b.id == bus_id)
                        .map(|b| b.name.clone())
                        .unwrap_or_else(|| format!("Bus {}", bus_id));
                    format!("{} {}", fa::ARROW_RIGHT, name)
                }
            };
            let fallback = OutputChoice { label, output: track.output };
            let mut owned: Vec<OutputChoice> = cached.iter().cloned().collect();
            // Insert the fallback so the picker has something selectable;
            // put it first so it's the obvious entry if the cache really
            // is empty.
            owned.insert(0, fallback.clone());
            (ChoiceList::Owned(owned), fallback)
        }
    };

    let picker = pick_list(choices, Some(selected), move |choice: OutputChoice| {
        Message::Track(TrackMessage::SetTrackOutput(track_id, choice.output))
    })
    .text_size(12)
    .text_shaping(Shaping::Advanced)
    .padding([5, 8])
    .width(Length::Fill);

    field("OUTPUT", picker.into())
}

/// Read-only routing row used for Send A/B placeholders.
fn routing_row(label: &'static str, value: &'static str, muted: bool) -> Element<'static, Message> {
    let value_color = if muted { theme::TEXT_4 } else { theme::TEXT_1 };
    let r_row = row![
        text(label).size(11).color(theme::TEXT_3),
        Space::new().width(Length::Fill),
        text(value).size(12).font(theme::MONO_FONT).color(value_color),
    ]
    .align_y(alignment::Vertical::Center)
    .padding([6, 0]);

    column![
        r_row,
        container(Space::new().width(Length::Fill))
            .height(1)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::LINE_2)),
                ..Default::default()
            }),
    ]
    .spacing(0)
    .into()
}

// ---------------------------------------------------------------------------
// CHAIN group — plugin rows + a functional "+ Add to chain" picker.
// ---------------------------------------------------------------------------

fn chain_group(
    r: &crate::Resonance,
    track: &TrackState,
    collapsed: bool,
) -> Element<'static, Message> {
    if collapsed {
        return group_header("CHAIN", MixerInspectorGroup::Chain, true);
    }

    // 10px column spacing doubles as the title → first-row gap, so no
    // explicit spacer is needed after the group title.
    let mut col = column![group_header("CHAIN", MixerInspectorGroup::Chain, false)].spacing(10);

    // Instrument tracks render the instrument slot (plugin index 0) plus
    // any FX rows after it. Audio tracks render every plugin as an FX
    // row. Both end with the "+ FX" picker.
    let is_instrument = track.track_type == TrackType::Instrument;
    if track.plugins.is_empty() {
        col = col.push(
            container(text("Empty chain").size(11).color(theme::TEXT_3))
                .padding([8, 10])
                .width(Length::Fill)
                .style(|_theme| container::Style {
                    background: Some(iced::Background::Color(theme::BG_2)),
                    border: iced::Border {
                        color: theme::LINE_2,
                        width: 1.0,
                        radius: theme::RADIUS_MD.into(),
                    },
                    ..Default::default()
                }),
        );
    } else {
        for (i, plugin) in track.plugins.iter().enumerate() {
            let is_instrument_slot = is_instrument && i == 0;
            col = col.push(chain_row(&plugin.plugin_name, is_instrument_slot));
        }
    }

    // Functional add-plugin picker. Instrument tracks with an empty
    // chain get the instrument picker first; everyone else gets the FX
    // picker. Skipped when no plugins have been scanned yet. Options
    // come from `view_caches.{fx,instrument}_plugins` — Rc clones, not
    // a per-frame filter pass.
    let needs_instrument =
        is_instrument && track.plugins.is_empty() && track.sub_track.is_none();
    let candidates = if needs_instrument {
        r.view_caches.instrument_plugins.clone()
    } else {
        r.view_caches.fx_plugins.clone()
    };
    if !candidates.is_empty() {
        let track_id = track.id;
        let placeholder = if needs_instrument {
            "+ Add instrument"
        } else {
            "+ Add to chain"
        };
        let picker = pick_list(
            candidates,
            None::<ScannedPlugin>,
            move |plugin: ScannedPlugin| {
                Message::Plugin(PluginMessage::AddPluginToTrack(track_id, plugin))
            },
        )
        .placeholder(placeholder)
        .text_size(12)
        .padding([8, 10])
        .width(Length::Fill);
        col = col.push(picker);
    }

    col.into()
}

fn chain_row(name: &str, is_instrument_slot: bool) -> Element<'static, Message> {
    let bullet_color = if is_instrument_slot {
        theme::ACCENT_SOFT
    } else {
        theme::ACCENT
    };
    let bullet = container(Space::new().width(0))
        .width(6)
        .height(6)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(bullet_color)),
            border: iced::Border {
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        });
    let label_color = if is_instrument_slot {
        theme::ACCENT_SOFT
    } else {
        theme::TEXT_1
    };
    container(
        row![
            bullet,
            Space::new().width(8),
            text(name.to_string()).size(12).color(label_color),
            Space::new().width(Length::Fill),
            text("BYP")
                .size(9)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::TEXT_3),
        ]
        .align_y(alignment::Vertical::Center),
    )
    .padding([8, 10])
    .width(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    })
    .into()
}

// ---------------------------------------------------------------------------
// Group header — clickable collapse row (title left, caret right) with a
// hairline below. Clicking anywhere on the row folds / unfolds the group.
// ---------------------------------------------------------------------------

fn group_header(
    title: &'static str,
    group: MixerInspectorGroup,
    collapsed: bool,
) -> Element<'static, Message> {
    let head = iced::widget::button(
        row![
            text(title)
                .size(10)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::TEXT_3),
            Space::new().width(Length::Fill),
            collapse_caret(!collapsed),
        ]
        .align_y(alignment::Vertical::Center)
        .width(Length::Fill),
    )
    .padding([2, 0])
    .width(Length::Fill)
    .style(|_theme, status| theme::small_button_style(status))
    .on_press(Message::Ui(UiMessage::ToggleMixerInspectorGroup(group)));

    column![
        head,
        Space::new().height(4),
        container(Space::new().width(Length::Fill))
            .height(1)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::LINE_2)),
                ..Default::default()
            }),
    ]
    .spacing(0)
    .into()
}
