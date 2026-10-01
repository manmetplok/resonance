//! The pieces every mixer strip is built from (mixer-cleanup.md §2, §5):
//! the FX header switch, the plugin slot lines, the centred pan block.
//!
//! **The strip shows state; the inspector edits.** A slot line answers
//! "what is on this channel and is it active?" — a state dot and the
//! plugin's name on one line — and offers exactly two gestures: a click
//! focuses the slot in the inspector's CHAIN (`PluginMessage::FocusSlot`)
//! and a double-click opens its window (`PluginMessage::OpenPluginWindow`).
//! Every structural edit (add, remove, reorder, bypass one slot, presets)
//! lives in the inspector.
//!
//! Everything here returns an owned (`'static`) element so the strip
//! bodies can build it inside their `lazy` regions, and reads only state
//! that `strip_fingerprint.rs` hashes.

use iced::widget::{button, column, container, mouse_area, row, scrollable, text, Space};
use iced::{alignment, Color, Element, Font, Length};
use resonance_audio::types::TrackId;

use crate::message::*;
use crate::state::PluginSlotState;
use crate::theme;

/// Text size of a slot line's plugin name.
const SLOT_LINE_TEXT_SIZE: f32 = 10.0;

/// Diameter of a slot line's state dot.
const SLOT_DOT: f32 = 6.0;

/// The text on a strip's slot line: the plugin name, ellipsised to
/// [`theme::MIXER_SLOT_LINE_CHARS`] so it never needs a second line.
///
/// A missing plugin is prefixed with a warning glyph and gets two fewer
/// characters of name to pay for it, so the line's width budget holds.
/// The glyph is in the *text* (not only the BAD-pink dot) so the state is
/// legible without relying on hue, and so a widget-tree test can read it.
pub(crate) fn slot_line_label(plugin_name: &str, missing: bool) -> String {
    if missing {
        format!(
            "\u{26a0} {}",
            crate::util::short(plugin_name, theme::MIXER_SLOT_LINE_CHARS - 2)
        )
    } else {
        crate::util::short(plugin_name, theme::MIXER_SLOT_LINE_CHARS)
    }
}

/// A slot line's state dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotDot {
    /// `●` — the plugin runs.
    Active,
    /// `○` — the slot is bypassed (its name dims too).
    Bypassed,
    /// BAD-pink `●` — the plugin is missing or unavailable.
    Missing,
}

/// Everything a slot line shows, decided in one place so a test can read
/// it back: `iced_test` sees a text's content but never its colour, and
/// the colours are the whole of the bypass / focus feedback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlotLineLook {
    pub label: String,
    pub dot: SlotDot,
    /// The name is drawn dim: the slot is bypassed, or the whole chain is.
    pub dimmed: bool,
    /// The slot is the inspector's focused slot (a subtle highlight).
    pub focused: bool,
    /// The instrument slot: accent colour, hairline under it.
    pub instrument: bool,
}

pub(crate) fn slot_line_look(
    plugin: &PluginSlotState,
    is_instrument_slot: bool,
    chain_bypassed: bool,
    focused: bool,
) -> SlotLineLook {
    let missing = plugin.availability.reason().is_some();
    let dot = if missing {
        SlotDot::Missing
    } else if plugin.bypassed {
        SlotDot::Bypassed
    } else {
        SlotDot::Active
    };
    SlotLineLook {
        label: slot_line_label(&plugin.plugin_name, missing),
        dot,
        dimmed: chain_bypassed || plugin.bypassed,
        focused,
        instrument: is_instrument_slot,
    }
}

/// One slot line: state dot + single-line name. Click focuses the slot,
/// double-click opens its window.
pub(super) fn slot_line(
    plugin: &PluginSlotState,
    is_instrument_slot: bool,
    chain_bypassed: bool,
    focused: bool,
) -> Element<'static, Message> {
    let look = slot_line_look(plugin, is_instrument_slot, chain_bypassed, focused);
    let pid = plugin.instance_id;

    let name_color = match (look.dot, look.dimmed, look.instrument, look.focused) {
        (SlotDot::Missing, _, _, _) => theme::BAD,
        (_, true, _, _) => theme::TEXT_3,
        (_, false, true, _) => theme::ACCENT_SOFT,
        (_, false, false, true) => theme::TEXT_1,
        (_, false, false, false) => theme::TEXT_2,
    };
    let line = row![
        state_dot(look.dot, chain_bypassed),
        container(
            text(look.label)
                .size(SLOT_LINE_TEXT_SIZE)
                .color(name_color)
                .wrapping(iced::widget::text::Wrapping::None),
        )
        .width(Length::Fill)
        .clip(true),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center);

    let body = container(line)
        .width(Length::Fill)
        .padding([3, 5])
        .style(move |_theme| {
            if focused {
                container::Style {
                    background: Some(iced::Background::Color(theme::ACCENT_DIM)),
                    border: iced::Border {
                        color: theme::ACCENT_LINE,
                        width: 1.0,
                        radius: theme::RADIUS_XS.into(),
                    },
                    ..Default::default()
                }
            } else {
                container::Style::default()
            }
        });

    // A mouse area, not a button: a button captures the press, and the
    // double-click would never be seen.
    mouse_area(body)
        .on_press(Message::Plugin(PluginMessage::FocusSlot(pid)))
        .on_double_click(Message::Plugin(PluginMessage::OpenPluginWindow(pid)))
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
}

/// The dim "No instrument" line of an instrument track with an empty
/// instrument slot. A click selects the track, which puts the
/// inspector's add picker (`+ Add instrument`) in front of the user.
pub(super) fn empty_instrument_line(track_id: TrackId) -> Element<'static, Message> {
    let line = row![
        container(Space::new().width(SLOT_DOT).height(SLOT_DOT)).style(|_theme| {
            container::Style {
                border: iced::Border {
                    color: theme::TEXT_4,
                    width: 1.0,
                    radius: (SLOT_DOT / 2.0).into(),
                },
                ..Default::default()
            }
        }),
        text("No instrument")
            .size(SLOT_LINE_TEXT_SIZE)
            .color(theme::TEXT_3)
            .wrapping(iced::widget::text::Wrapping::None),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center);
    mouse_area(container(line).width(Length::Fill).padding([3, 5]))
        .on_press(Message::Ui(UiMessage::SelectTrack(Some(track_id))))
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
}

fn state_dot(dot: SlotDot, chain_bypassed: bool) -> Element<'static, Message> {
    let fade = |c: Color| {
        if chain_bypassed {
            Color { a: c.a * 0.45, ..c }
        } else {
            c
        }
    };
    let (fill, ring) = match dot {
        SlotDot::Active => (Some(fade(theme::GOOD)), fade(theme::GOOD)),
        SlotDot::Bypassed => (None, fade(theme::TEXT_3)),
        SlotDot::Missing => (Some(theme::BAD), theme::BAD),
    };
    container(Space::new().width(SLOT_DOT).height(SLOT_DOT))
        .style(move |_theme| container::Style {
            background: fill.map(iced::Background::Color),
            border: iced::Border {
                color: ring,
                width: 1.0,
                radius: (SLOT_DOT / 2.0).into(),
            },
            ..Default::default()
        })
        .into()
}

/// The hairline under an instrument line (Q14).
fn instrument_divider() -> Element<'static, Message> {
    container(Space::new().width(Length::Fill).height(1))
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::ACCENT_LINE)),
            ..Default::default()
        })
        .into()
}

/// What the slot list's fixed instrument section shows.
pub(super) enum InstrumentSlot<'a> {
    /// Not an instrument chain (audio track, bus, master, external
    /// instrument): every slot is an effect line.
    None,
    /// An instrument track with an empty instrument slot.
    Empty(TrackId),
    /// An instrument track's slot 0.
    Filled(&'a PluginSlotState),
}

/// The slot list: the instrument line (fixed, hairline under it) and the
/// effect lines in a vertical scrollable that absorbs the strip's slack,
/// so a long chain scrolls and the fader never moves.
pub(super) fn slot_list(
    instrument: InstrumentSlot<'_>,
    effects: &[PluginSlotState],
    chain_bypassed: bool,
    focused: Option<resonance_audio::types::PluginInstanceId>,
) -> Element<'static, Message> {
    let mut fx_column = column![].spacing(1).width(Length::Fill);
    for plugin in effects {
        fx_column = fx_column.push(slot_line(
            plugin,
            false,
            chain_bypassed,
            focused == Some(plugin.instance_id),
        ));
    }
    let fx_scroll = iced::widget::Scrollable::with_direction(
        fx_column,
        scrollable::Direction::Vertical(
            scrollable::Scrollbar::default().width(3).scroller_width(3),
        ),
    )
    .width(Length::Fill)
    .height(Length::Fill);

    let mut list = column![]
        .spacing(3)
        .width(Length::Fill)
        .height(Length::Fill);
    match instrument {
        InstrumentSlot::None => {}
        InstrumentSlot::Empty(track_id) => {
            list = list
                .push(empty_instrument_line(track_id))
                .push(instrument_divider());
        }
        InstrumentSlot::Filled(plugin) => {
            list = list
                .push(slot_line(
                    plugin,
                    true,
                    chain_bypassed,
                    focused == Some(plugin.instance_id),
                ))
                .push(instrument_divider());
        }
    }
    list.push(fx_scroll).into()
}

/// The FX header line: the chain's on/off switch (Q6), then a hairline.
/// `bypassed` is the whole chain's bypass; `toggle` flips it.
pub(super) fn fx_header(bypassed: bool, toggle: Message) -> Element<'static, Message> {
    let (label_color, power_color) = if bypassed {
        (theme::TEXT_3, theme::TEXT_4)
    } else {
        (theme::TEXT_2, theme::GOOD)
    };
    let switch = button(
        row![
            text("FX")
                .size(9)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(label_color),
            theme::icon(theme::fa::POWER_OFF).size(9).color(power_color),
        ]
        .spacing(5)
        .align_y(alignment::Vertical::Center),
    )
    .on_press(toggle)
    .padding([2, 5])
    .style(|_theme, status| theme::small_button_style(status));
    row![
        switch,
        container(Space::new().width(Length::Fill).height(1))
            .width(Length::Fill)
            .style(theme::separator_bg),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center)
    .into()
}

/// The centred pan block (Q7): the knob, its value under it, no label.
pub(super) fn pan_block(knob: Element<'static, Message>, pan: f32) -> Element<'static, Message> {
    column![
        container(knob).width(Length::Fill).center_x(Length::Fill),
        container(
            text(crate::util::format_pan(pan))
                .size(9)
                .font(Font::MONOSPACE)
                .color(theme::TEXT_2),
        )
        .width(Length::Fill)
        .center_x(Length::Fill),
    ]
    .spacing(2)
    .width(Length::Fill)
    .into()
}

/// A strip head's colour band: the owner's identity colour on the left
/// edge of the head (mixer-cleanup.md §2.3, §6).
pub(super) fn color_band(color: Color) -> Element<'static, Message> {
    container(Space::new().width(theme::TRACK_COLOR_BAND_WIDTH).height(22))
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(color)),
            border: iced::Border {
                radius: 2.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}
