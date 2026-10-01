//! The mixer inspector's **SENDS** group — the aux-send controls (ba
//! todo #1310, design doc #172, finding P3 of the capability-vs-exposure
//! audit in doc #275).
//!
//! Everything behind aux sends already shipped: the engine's route model
//! and cyclic validation (#475), the tap-and-sum (#476), the app-side
//! [`MixerMessage`] variants + undo (#477), the engine-event mirror
//! (#478), persistence (#1269) and the `track.add_send` / `set_send` /
//! `remove_send` control methods (#1229). The only thing missing was a
//! human-reachable surface: ROUTING rendered two hardcoded read-only
//! rows, `Send A -> (none)` and `Send B -> (none)`, and nothing in the
//! whole view tree raised a single `MixerMessage`.
//!
//! So this module is **emitters only**. It adds no state, no message and
//! no handler; every control maps to a `MixerMessage` that already
//! existed, which is why a send made here reads back identically over
//! `song.tracks` and a send made over `track.add_send` shows up here.
//!
//! Per-send metering deliberately stays out (cancelled todo #481): a
//! send row shows what it routes, not how loud the tap currently is.

use iced::widget::{button, column, container, pick_list, row, slider, text, Space};
use iced::{alignment, Element, Length};
use resonance_audio::types::{AuxSend, BusId, SendId, SendSource, TrackId};

use crate::message::{Message, MixerMessage};
use crate::state::TrackState;
use crate::theme;
use crate::view::mixer::picks::{
    add_send_choices, send_dest_choices, AddSendChoice, SendDestChoice,
};

/// Slider travel for a send level, in dB. The engine clamps to
/// ±(120/24) dB, and `track.set_send` accepts that whole span, but a
/// 144 dB slider has no usable resolution — so the drag matches the
/// channel fader's range (`view::controls::fader`) and a level set
/// outside it over the control API is still shown verbatim in the
/// readout while the handle parks at the nearest end.
const SEND_DB_MIN: f32 = -60.0;
const SEND_DB_MAX: f32 = 6.0;

/// Every send tapped off `track`, in mirror order.
pub(crate) fn sends_for_track(
    r: &crate::Resonance,
    track_id: TrackId,
) -> impl Iterator<Item = &AuxSend> {
    r.aux
        .sends
        .iter()
        .filter(move |s| matches!(s.source, SendSource::Track(id) if id == track_id))
}

// ---------------------------------------------------------------------------
// Message constructors.
//
// Every affordance below is built from one of these, and the test hook
// `Resonance::test_send_affordances` hands back the very same values —
// so a test that presses a send control presses what the GUI presses,
// not a second copy of the wiring that can drift away from it.
// ---------------------------------------------------------------------------

/// Re-route an existing send into `bus_id`.
pub(crate) fn dest_message(send_id: SendId, bus_id: BusId) -> Message {
    Message::Mixer(MixerMessage::SetSendDest(send_id, bus_id))
}

/// Set an existing send's level, in dB.
pub(crate) fn level_message(send_id: SendId, level_db: f32) -> Message {
    Message::Mixer(MixerMessage::SetSendLevel(send_id, level_db))
}

/// Flip the pre/post-fader tap point.
pub(crate) fn tap_message(send_id: SendId) -> Message {
    Message::Mixer(MixerMessage::ToggleSendPreFader(send_id))
}

/// Enable / disable the send without disturbing its routing.
pub(crate) fn enable_message(send_id: SendId) -> Message {
    Message::Mixer(MixerMessage::ToggleSendEnabled(send_id))
}

/// Delete the send.
pub(crate) fn remove_message(send_id: SendId) -> Message {
    Message::Mixer(MixerMessage::RemoveSend(send_id))
}

/// What picking `choice` out of the "+ Add send" list does.
pub(crate) fn add_message(source: SendSource, choice: &AddSendChoice) -> Message {
    match choice {
        AddSendChoice::Bus { bus_id, .. } => Message::Mixer(MixerMessage::AddSend {
            source,
            dest: *bus_id,
        }),
        AddSendChoice::NewReturn => {
            Message::Mixer(MixerMessage::CreateReturnFromSend { source })
        }
    }
}

/// The destination options a send's re-route picker offers, and which
/// one it shows as selected.
pub(crate) fn dest_options(
    r: &crate::Resonance,
    send: &AuxSend,
) -> (Vec<SendDestChoice>, Option<SendDestChoice>) {
    let options = send_dest_choices(&r.registry.busses, send.dest);
    let selected = options.iter().find(|c| c.bus_id == send.dest).cloned();
    (options, selected)
}

/// The "+ Add send" options for `track_id`: every return bus it isn't
/// already feeding, then "New FX return…".
pub(crate) fn add_options(
    r: &crate::Resonance,
    track_id: TrackId,
) -> Vec<AddSendChoice> {
    let taken: Vec<BusId> = sends_for_track(r, track_id).map(|s| s.dest).collect();
    add_send_choices(&r.registry.busses, &taken)
}

/// The dB readout drawn beside a send's level slider.
pub(crate) fn level_readout(send: &AuxSend) -> String {
    format!("{:+.1} dB", send.level_db)
}

/// The label on a send's tap-point toggle.
pub(crate) fn tap_label(send: &AuxSend) -> &'static str {
    if send.pre_fader {
        "PRE"
    } else {
        "POST"
    }
}

/// The SENDS group: its collapsible header over [`sends_block`]. Lifted
/// out of ROUTING into a group of its own (mixer-cleanup.md §3.1).
pub(super) fn sends_group(
    r: &crate::Resonance,
    track: &TrackState,
    collapsed: bool,
) -> Element<'static, Message> {
    let header = super::widgets::group_header(
        "SENDS",
        crate::state::MixerInspectorGroup::Sends,
        collapsed,
    );
    if collapsed {
        return header;
    }
    column![header, Space::new().height(10), sends_block(r, track)]
        .spacing(0)
        .into()
}

/// The SENDS body: one slot per live send, the "+ Add send" picker, and
/// the engine's most recent rejection note when it concerns this track.
fn sends_block(r: &crate::Resonance, track: &TrackState) -> Element<'static, Message> {
    let source = SendSource::Track(track.id);
    let sends: Vec<AuxSend> = sends_for_track(r, track.id).copied().collect();

    let mut col = column![].spacing(0);

    if sends.is_empty() {
        col = col.push(empty_sends_row());
    } else {
        for (i, send) in sends.iter().enumerate() {
            if i > 0 {
                col = col.push(Space::new().height(6));
            }
            col = col.push(send_slot(r, send));
        }
    }

    // Add affordance. `add_send_choices` always yields at least the
    // "New FX return…" entry, so the picker is never dead — a project
    // with no busses at all can still grow its first reverb send.
    let choices = add_options(r, track.id);
    let add_picker = pick_list(choices, None::<AddSendChoice>, move |choice| {
        add_message(source, &choice)
    })
    .placeholder("+ Add send")
    .text_size(12)
    .text_shaping(iced::widget::text::Shaping::Advanced)
    .padding([6, 8])
    .width(Length::Fill);
    col = col.push(Space::new().height(8)).push(add_picker);

    // A route the engine refused never becomes a slot above, so without
    // this the gesture would look like it simply did nothing.
    if let Some(rejection) = r.aux.last_rejection.as_ref() {
        if rejection.source == source {
            col = col
                .push(Space::new().height(6))
                .push(rejection_note(&rejection.reason));
        }
    }

    container(col)
        .width(Length::Fill)
        .into()
}

/// One send: destination picker on top, then level slider + readout,
/// then the PRE/POST, ON and remove affordances.
fn send_slot(r: &crate::Resonance, send: &AuxSend) -> Element<'static, Message> {
    let id = send.id;

    let (options, selected) = dest_options(r, send);
    let dest_picker = pick_list(options, selected, move |choice| {
        dest_message(id, choice.bus_id)
    })
    .text_size(12)
    .text_shaping(iced::widget::text::Shaping::Advanced)
    .padding([5, 8])
    .width(Length::Fill);

    // The handle parks at the nearest end for an out-of-range level; the
    // readout beside it always reports the mirrored value.
    let level_slider = slider(
        SEND_DB_MIN..=SEND_DB_MAX,
        send.level_db.clamp(SEND_DB_MIN, SEND_DB_MAX),
        move |db| level_message(id, db),
    )
    .step(0.5f32)
    .width(Length::Fill);

    let readout = text(level_readout(send))
        .size(11)
        .font(theme::MONO_FONT)
        .color(if send.enabled {
            theme::TEXT_1
        } else {
            theme::TEXT_3
        })
        .width(Length::Fixed(56.0))
        .align_x(alignment::Horizontal::Right);

    let tap_toggle = super::widgets::toggle_button(
        tap_label(send),
        send.pre_fader,
        theme::WARM,
        theme::WARM_DIM,
        tap_message(id),
    );
    let enable_toggle = super::widgets::toggle_button(
        "ON",
        send.enabled,
        theme::GOOD,
        theme::GOOD_DIM,
        enable_message(id),
    );

    let controls = row![
        container(tap_toggle).width(Length::FillPortion(1)),
        Space::new().width(6),
        container(enable_toggle).width(Length::FillPortion(1)),
        Space::new().width(6),
        remove_button(id),
    ]
    .align_y(alignment::Vertical::Center);

    container(
        column![
            dest_picker,
            Space::new().height(6),
            row![level_slider, Space::new().width(8), readout]
                .align_y(alignment::Vertical::Center),
            Space::new().height(6),
            controls,
        ]
        .spacing(0),
    )
    .padding(8)
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

/// Trash affordance that deletes the send outright
/// (`MixerMessage::RemoveSend`).
fn remove_button(send_id: SendId) -> Element<'static, Message> {
    button(
        text(theme::fa::TRASH)
            .font(theme::ICON_FONT)
            .size(10)
            .align_x(alignment::Horizontal::Center)
            .width(Length::Fill),
    )
    .padding([7, 0])
    .width(Length::Fixed(34.0))
    .on_press(remove_message(send_id))
    .style(|_theme, status| {
        let hovered = matches!(status, button::Status::Hovered);
        let (bg, border, txt) = if hovered {
            (theme::BAD_DIM, theme::BAD_LINE, theme::BAD)
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

/// Shown in place of the slots when the track feeds nothing.
fn empty_sends_row() -> Element<'static, Message> {
    container(text("No sends").size(11).color(theme::TEXT_4))
        .width(Length::Fill)
        .padding([6, 8])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into()
}

/// The engine's plain-language reason for refusing a route, in the BAD
/// tint, directly under the picker that raised it.
fn rejection_note(reason: &str) -> Element<'static, Message> {
    container(text(reason.to_string()).size(10).color(theme::BAD))
        .width(Length::Fill)
        .padding([5, 8])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BAD_DIM)),
            border: iced::Border {
                color: theme::BAD_LINE,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into()
}
