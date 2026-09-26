//! Audio-input, MIDI-input, MIDI-output, and audio-output picker blocks
//! for the generic (non-external-instrument) ROUTING group.

use iced::widget::{column, pick_list, Space};
use iced::widget::text::Shaping;
use iced::{Element, Length};
use resonance_audio::types::{InputDeviceInfo, TrackOutput};

use crate::message::{Message, TrackMessage};
use crate::state::TrackState;
use crate::view::ui_caches::ChoiceList;
use crate::view::mixer::picks::{
    MidiChannelChoice, MidiPickerChoice, OutputChoice, PortChoice,
};

pub(super) fn audio_input_block(
    r: &crate::Resonance,
    track: &TrackState,
) -> Element<'static, Message> {
    let track_id = track.id;
    let selected_device = track
        .input_device_name
        .as_ref()
        .and_then(|name| r.devices.input.devices.iter().find(|d| &d.name == name))
        .cloned();
    let device_channels = selected_device.as_ref().map(|d| d.channels).unwrap_or(0);

    let device_picker = pick_list(
        // Cached `Rc<[InputDeviceInfo]>` — clones are cheap (refcount).
        // Rebuilt by `engine_events::transport::input_devices_listed`
        // only when the engine re-enumerates devices.
        r.ui.view_caches.input_devices.clone(),
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
        super::widgets::field("INPUT DEVICE", device_picker.into()),
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
                .push(super::widgets::field("INPUT CHANNEL", port_picker.into()));
        }
    }
    col.into()
}

pub(super) fn midi_input_block(
    r: &crate::Resonance,
    track: &TrackState,
) -> Element<'static, Message> {
    let track_id = track.id;
    // Pull the cached "(None) + every device" list off Resonance; only
    // synthesize a one-off Vec when the track is bound to a configured
    // device that's not currently enumerated (controller unplugged).
    let in_choices = super::midi_choices_with_override(
        &r.ui.view_caches.midi_input_choices,
        track.midi_input_device.as_deref(),
        &r.devices.midi.midi_input_devices,
    );
    let in_selected = MidiPickerChoice(track.midi_input_device.clone());
    let in_picker = pick_list(in_choices, Some(in_selected), move |choice| {
        Message::Track(TrackMessage::SetTrackMidiInputDevice(track_id, choice.0))
    })
    .placeholder("(no MIDI in)")
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let mut col = column![super::widgets::field("MIDI INPUT", in_picker.into())].spacing(0);

    if track.midi_input_device.is_some() {
        let in_ch_picker = pick_list(
            r.ui.view_caches.input_channel_choices.clone(),
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
            .push(super::widgets::field("MIDI IN CHANNEL", in_ch_picker.into()));
    }
    col.into()
}

pub(super) fn midi_output_block(
    r: &crate::Resonance,
    track: &TrackState,
) -> Element<'static, Message> {
    let track_id = track.id;
    let out_choices = super::midi_choices_with_override(
        &r.ui.view_caches.midi_output_choices,
        track.midi_output_device.as_deref(),
        &r.devices.midi.midi_output_devices,
    );
    let out_selected = MidiPickerChoice(track.midi_output_device.clone());
    let out_picker = pick_list(out_choices, Some(out_selected), move |choice| {
        Message::Track(TrackMessage::SetTrackMidiOutputDevice(track_id, choice.0))
    })
    .placeholder("(no MIDI out)")
    .text_size(12)
    .padding([5, 8])
    .width(Length::Fill);

    let mut col = column![super::widgets::field("MIDI OUTPUT", out_picker.into())].spacing(0);

    if track.midi_output_device.is_some() {
        let selected = MidiChannelChoice(Some(track.midi_output_channel.unwrap_or(0)));
        let out_ch_picker = pick_list(
            r.ui.view_caches.output_channel_choices.clone(),
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
            .push(super::widgets::field("MIDI OUT CHANNEL", out_ch_picker.into()));
    }
    col.into()
}

pub(super) fn output_block(
    r: &crate::Resonance,
    track: &TrackState,
) -> Element<'static, Message> {
    let track_id = track.id;
    let cached = r.ui.view_caches.output_choices.clone();
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

    super::widgets::field("OUTPUT", picker.into())
}

/// Build the audio-return device option list for an external-instrument
/// track, appending a synthesized entry for a configured-but-unenumerated
/// device (offline / replug pending) so the route stays selected. Mirrors
/// `midi_choices_with_override` for the audio-input device list.
pub(super) fn return_device_choices(
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
