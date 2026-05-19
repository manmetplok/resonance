//! Mixer Inspector panel — the right-side detail pane that shows the
//! currently selected track's signal, routing, and plugin chain. Hosts
//! the functional pickers that used to live on the strip itself: input
//! device + channel (audio) or MIDI in / channel and MIDI out / channel
//! (instruments), output destination (Master / Bus N), and an "+ FX"
//! picker for the chain.

use iced::widget::{column, container, pick_list, row, text, Space};
use iced::widget::text::Shaping;
use iced::{alignment, Element, Length};
use resonance_audio::types::{InputDeviceInfo, ScannedPlugin, TrackOutput, TrackType};

use crate::message::*;
use crate::state::TrackState;
use crate::theme;
use crate::util::format_pan;

use super::picks::{MidiChannelChoice, MidiPickerChoice, OutputChoice, PortChoice};
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

    // Fingerprint everything the inspector reads except the volatile
    // level fields (PEAK readout in SIGNAL stays accurate via the
    // per-frame `signal_group` outside this lazy block).
    let fp = inspector_fingerprint(r, selected);
    let body = iced::widget::lazy(fp, move |_: &u64| -> Element<'static, Message> {
        match selected {
            Some(track) => render_track_inner(r, track),
            None => render_empty(),
        }
    });

    container(body)
        .width(Length::Fixed(theme::INSPECTOR_WIDTH))
        .height(Length::Fill)
        .padding(18)
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

/// Hash every inspector-visible field except the live levels. The lazy
/// widget compares this across frames — when nothing has changed, the
/// cached widget tree is reused (which is the resize hot path).
fn inspector_fingerprint(
    r: &crate::Resonance,
    selected: Option<&TrackState>,
) -> u64 {
    use std::hash::{Hash, Hasher};
    use std::rc::Rc;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match selected {
        None => 0u8.hash(&mut h),
        Some(t) => {
            1u8.hash(&mut h);
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
            for p in &t.plugins {
                p.instance_id.hash(&mut h);
                p.plugin_name.hash(&mut h);
            }
        }
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

/// Build the inspector body assuming a track is selected. Returns a
/// `'static` element so the lazy widget can cache it across frames.
fn render_track_inner(r: &crate::Resonance, track: &TrackState) -> Element<'static, Message> {
    let header = column![
        text("INSPECTOR")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        Space::new().height(2),
        text(track.name.clone())
            .size(17)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::TEXT_1),
    ]
    .spacing(0);

    let scroll = iced::widget::scrollable(
        column![
            header,
            Space::new().height(16),
            signal_group(track),
            Space::new().height(16),
            routing_group(r, track),
            Space::new().height(16),
            chain_group(r, track),
        ]
        .spacing(0),
    )
    .height(Length::Fill);

    scroll.into()
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

fn signal_group(track: &TrackState) -> Element<'static, Message> {
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
        group_title("SIGNAL"),
        Space::new().height(8),
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

fn routing_group(r: &crate::Resonance, track: &TrackState) -> Element<'static, Message> {
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
        group_title("ROUTING"),
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
    let choices = r.view_caches.output_choices.clone();
    let selected = choices
        .iter()
        .find(|c| c.output == track.output)
        .cloned()
        .unwrap_or_else(|| choices[0].clone());

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

fn chain_group(r: &crate::Resonance, track: &TrackState) -> Element<'static, Message> {
    let mut col = column![group_title("CHAIN"), Space::new().height(8)].spacing(6);

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
// Group title with hairline below.
// ---------------------------------------------------------------------------

fn group_title(title: &'static str) -> Element<'static, Message> {
    column![
        text(title)
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
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
