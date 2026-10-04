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

/// Text size of a slot line's plugin name: the 11 px floor
/// (ux-guidelines.md, Typography).
const SLOT_LINE_TEXT_SIZE: f32 = 11.0;

/// Text size of the FX header switch's label and the pan value: the
/// 11 px floor.
const STRIP_SMALL_TEXT_SIZE: f32 = 11.0;

/// Diameter of a slot line's state dot.
const SLOT_DOT: f32 = 6.0;

/// The text on a strip's slot line: the plugin name, ellipsised to
/// [`theme::MIXER_SLOT_LINE_CHARS`] so it never needs a second line.
/// [`slot_line_label_in`] takes another budget (the sub-track strip's).
///
/// A missing plugin is prefixed with a warning glyph and gets two fewer
/// characters of name to pay for it, so the line's width budget holds.
/// The glyph is in the *text* (not only the BAD-pink dot) so the state is
/// legible without relying on hue, and so a widget-tree test can read it.
#[cfg_attr(not(feature = "test-support"), allow(dead_code))]
pub(crate) fn slot_line_label(plugin_name: &str, missing: bool) -> String {
    slot_line_label_in(plugin_name, missing, theme::MIXER_SLOT_LINE_CHARS)
}

/// [`slot_line_label`] within a budget of `chars` characters.
pub(crate) fn slot_line_label_in(plugin_name: &str, missing: bool, chars: usize) -> String {
    if missing {
        format!("\u{26a0} {}", crate::util::short(plugin_name, chars - 2))
    } else {
        crate::util::short(plugin_name, chars)
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
    chars: usize,
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
        label: slot_line_label_in(&plugin.plugin_name, missing, chars),
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
    chars: usize,
) -> Element<'static, Message> {
    let look = slot_line_look(plugin, is_instrument_slot, chain_bypassed, focused, chars);
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

/// The dim "No instrument" line of an instrument track without an
/// instrument. A click selects the track and cues the inspector's
/// `+ Add instrument` picker (`ChainUiMessage::CueInstrumentPicker`):
/// iced cannot focus a `pick_list`, so the picker is drawn with an accent
/// border and a line saying what to do.
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
        .on_press(Message::Plugin(PluginMessage::ChainUi(
            ChainUiMessage::CueInstrumentPicker(track_id),
        )))
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

/// Where a chain's instrument sits, for the slot list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum InstrumentSlot {
    /// Not an instrument chain (audio track, bus, master, external
    /// instrument, sub-track): every slot is an effect line.
    None,
    /// An instrument track without an instrument.
    Empty(TrackId),
    /// The instrument is the chain's slot at this index
    /// (`plugin_chain::displayed_instrument_slot`).
    At(usize),
}

impl InstrumentSlot {
    /// What `track`'s strip shows: the instrument where the chain
    /// actually holds it (never "slot 0 because this is an instrument
    /// track"), "No instrument" for a plain instrument track without
    /// one, and plain effect lines everywhere else.
    pub(crate) fn of_track(r: &crate::Resonance, track: &crate::state::TrackState) -> Self {
        if let Some(i) = crate::plugin_chain::displayed_instrument_slot(r, track) {
            InstrumentSlot::At(i)
        } else if crate::plugin_chain::lacks_instrument(r, track) {
            InstrumentSlot::Empty(track.id)
        } else {
            InstrumentSlot::None
        }
    }
}

/// The slot list, in chain order. An instrument in slot 0 (the normal
/// shape) and the "No instrument" line sit fixed above the effects, with
/// a hairline under them; the effect lines are in a vertical scrollable
/// that absorbs the strip's slack, so a long chain scrolls and the fader
/// never moves. An instrument further down the chain (an effect was put
/// ahead of it) is drawn in its place among the effect lines, still with
/// its accent and hairline, so the strip shows the order the engine runs.
/// `chars` is the name budget of one line (the strip's width).
pub(super) fn slot_list(
    instrument: InstrumentSlot,
    plugins: &[PluginSlotState],
    chain_bypassed: bool,
    focused: Option<resonance_audio::types::PluginInstanceId>,
    chars: usize,
) -> Element<'static, Message> {
    let line = |index: usize, plugin: &PluginSlotState| {
        slot_line(
            plugin,
            instrument == InstrumentSlot::At(index),
            chain_bypassed,
            focused == Some(plugin.instance_id),
            chars,
        )
    };

    let mut list = column![]
        .spacing(3)
        .width(Length::Fill)
        .height(Length::Fill);
    let scrolled_from = match instrument {
        InstrumentSlot::Empty(track_id) => {
            list = list
                .push(empty_instrument_line(track_id))
                .push(instrument_divider());
            0
        }
        InstrumentSlot::At(0) if !plugins.is_empty() => {
            list = list.push(line(0, &plugins[0])).push(instrument_divider());
            1
        }
        _ => 0,
    };

    let mut fx_column = column![].spacing(1).width(Length::Fill);
    for (index, plugin) in plugins.iter().enumerate().skip(scrolled_from) {
        fx_column = fx_column.push(line(index, plugin));
        if instrument == InstrumentSlot::At(index) {
            fx_column = fx_column.push(instrument_divider());
        }
    }
    let fx_scroll = iced::widget::Scrollable::with_direction(
        fx_column,
        scrollable::Direction::Vertical(
            scrollable::Scrollbar::default().width(3).scroller_width(3),
        ),
    )
    .width(Length::Fill)
    .height(Length::Fill);

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
                .size(STRIP_SMALL_TEXT_SIZE)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(label_color),
            theme::icon(theme::fa::POWER_OFF)
                .size(STRIP_SMALL_TEXT_SIZE - 1.0)
                .color(power_color),
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
                .size(STRIP_SMALL_TEXT_SIZE)
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

/// The inline rename field (mixer-cleanup.md §2.3, §3.1;
/// `update::inline_rename`), drawn in place of a channel's name on the
/// one surface its rename is open on — a strip head or the inspector
/// header. The mouse area reports whether the pointer is over the field:
/// a press while it is not commits the rename.
pub(super) fn rename_field(placeholder: &str, buffer: &str, size: f32) -> Element<'static, Message> {
    mouse_area(
        iced::widget::text_input(placeholder, buffer)
            .id(crate::update::inline_rename::input_id())
            .on_input(|s| Message::Ui(UiMessage::RenameInput(s)))
            .on_submit(Message::Ui(UiMessage::CommitRename))
            .size(size)
            .padding([2, 4])
            .width(Length::Fill),
    )
    .on_enter(Message::Ui(UiMessage::RenameHovered(true)))
    .on_exit(Message::Ui(UiMessage::RenameHovered(false)))
    .into()
}
