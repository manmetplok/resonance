//! MIDI Learn on screen (doc #167, W1): the right-click wrapper every
//! mappable control wears, the learning outline and binding badge, the
//! floating MIDI menu, and the names the bindings list shows.
//!
//! A control is learnable when it is wrapped in [`learnable`]: a right
//! press opens the MIDI menu for its target ("Learn MIDI" / "Cancel
//! learn", "Clear MIDI binding"); while that target is armed the control
//! wears an accent outline. The fader also carries a badge naming its
//! control (`CC7`), or `LEARN` while armed.

use std::hash::{Hash, Hasher};

use iced::widget::{button, column, container, mouse_area, opaque, row, stack, text, Space};
use iced::{alignment, Element, Length};
use resonance_common::{MidiTarget, TransportAction};

use crate::message::{Message, MidiMapMessage};
use crate::state::{source_badge, source_label};
use crate::theme;
use crate::view::context_area::context_area;
use crate::Resonance;

/// Width of the floating MIDI menu.
const MENU_WIDTH: f32 = 230.0;

/// Wrap `content` as the learnable control for `target`: right-click
/// opens its MIDI menu, and it is outlined while learn is armed on it.
pub(crate) fn learnable(
    r: &Resonance,
    target: MidiTarget,
    content: impl Into<Element<'static, Message>>,
) -> Element<'static, Message> {
    let learning = r.devices.midi_map.learn_target == Some(target);
    let framed = container(content).style(move |_theme| container::Style {
        border: iced::Border {
            color: if learning {
                theme::ACCENT
            } else {
                iced::Color::TRANSPARENT
            },
            width: 1.0,
            radius: theme::RADIUS_XS.into(),
        },
        ..Default::default()
    });
    context_area(framed, move |p| {
        Message::MidiMap(MidiMapMessage::OpenMenu {
            target,
            x: p.x,
            y: p.y,
        })
    })
    .into()
}

/// The small pill a learnable control shows: `LEARN` (accent) while armed,
/// the bound control (`CC7`, `+1` more) otherwise, nothing when unbound —
/// an empty slot, so the tree keeps its shape.
pub(crate) fn badge(r: &Resonance, target: MidiTarget) -> Element<'static, Message> {
    let map = &r.devices.midi_map;
    let (label, color, edge) = if map.learn_target == Some(target) {
        ("LEARN".to_string(), theme::ACCENT_SOFT, theme::ACCENT_LINE)
    } else {
        let bound = map.for_target(target);
        let Some(first) = bound.first() else {
            return Space::new().width(0).height(0).into();
        };
        let mut label = source_badge(first.source);
        if bound.len() > 1 {
            label.push_str(&format!(" +{}", bound.len() - 1));
        }
        (label, theme::TEXT_2, theme::LINE)
    };
    container(text(label).size(9).font(theme::MONO_FONT).color(color))
        .padding([1, 4])
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_3)),
            border: iced::Border {
                color: edge,
                width: 1.0,
                radius: theme::RADIUS_XS.into(),
            },
            ..Default::default()
        })
        .into()
}

/// `content` with [`badge`] pinned to its top-right corner, without
/// moving anything (a stack layer, not a row).
pub(crate) fn with_badge(
    r: &Resonance,
    target: MidiTarget,
    content: Element<'static, Message>,
) -> Element<'static, Message> {
    let pinned = container(badge(r, target))
        .width(Length::Fill)
        .align_x(alignment::Horizontal::Right)
        .padding([0, 2]);
    stack![content, pinned].into()
}

/// Hash what [`learnable`] / [`badge`] draw for `target`, for the lazy
/// fingerprint of a region that holds one.
pub(crate) fn hash_target<H: Hasher>(h: &mut H, r: &Resonance, target: MidiTarget) {
    let map = &r.devices.midi_map;
    (map.learn_target == Some(target)).hash(h);
    for b in map.for_target(target) {
        b.id.hash(h);
        b.source.hash(h);
    }
}

/// What a target is called in the menu header and the bindings lists:
/// `Bass · Volume`, `Bass · Send → FX 1`, `EQ · Gain`, `Transport · Play`.
/// A target whose track / plugin is gone says so instead of guessing.
pub(crate) fn target_label(r: &Resonance, target: MidiTarget) -> String {
    let track_name = |id| {
        r.registry
            .tracks
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.name.clone())
    };
    let on_track = |id, what: &str| match track_name(id) {
        Some(name) => format!("{name} \u{b7} {what}"),
        None => format!("Deleted track \u{b7} {what}"),
    };
    match target {
        MidiTarget::TrackVolume(id) => on_track(id, "Volume"),
        MidiTarget::TrackPan(id) => on_track(id, "Pan"),
        MidiTarget::TrackMute(id) => on_track(id, "Mute"),
        MidiTarget::TrackSolo(id) => on_track(id, "Solo"),
        MidiTarget::SendLevel { track, send } => {
            let dest = r
                .aux
                .sends
                .iter()
                .find(|s| s.id == send.0)
                .and_then(|s| r.registry.busses.iter().find(|b| b.id == s.dest))
                .map(|b| b.name.clone())
                .unwrap_or_else(|| "deleted send".to_string());
            on_track(track, &format!("Send \u{2192} {dest}"))
        }
        MidiTarget::PluginParam { instance, param_id } => match r.plugin_slot(instance) {
            Some(p) => {
                let param = p
                    .params
                    .iter()
                    .find(|pp| pp.id == param_id)
                    .map(|pp| pp.name.clone())
                    .unwrap_or_else(|| format!("param {param_id}"));
                format!("{} \u{b7} {param}", p.plugin_name)
            }
            None => format!("Deleted plugin \u{b7} param {param_id}"),
        },
        MidiTarget::Transport(action) => format!(
            "Transport \u{b7} {}",
            match action {
                TransportAction::Play => "Play",
                TransportAction::Stop => "Stop",
                TransportAction::Record => "Record",
                TransportAction::LoopToggle => "Loop",
            }
        ),
    }
}

/// One row of the MIDI menu, in the track menu's style.
fn menu_item(label: String, msg: Message, enabled: bool) -> Element<'static, Message> {
    let color = if enabled { theme::TEXT_1 } else { theme::TEXT_4 };
    let mut b = button(text(label).size(12).color(color))
        .width(Length::Fill)
        .padding([5, 10])
        .style(|_theme, status| theme::transport_button_style(status));
    if enabled {
        b = b.on_press(msg);
    }
    b.into()
}

/// The floating MIDI menu, under the pointer where the control was
/// right-clicked (kept inside the window). Click-away closes it.
pub(crate) fn view_midi_menu_overlay(r: &Resonance) -> Element<'_, Message> {
    let Some(menu) = r.devices.midi_map.menu else {
        return Space::new().into();
    };
    let target = menu.target;
    let map = &r.devices.midi_map;
    let learning = map.learn_target == Some(target);
    let bound = map.for_target(target);

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::MidiMap(MidiMapMessage::CloseMenu))
    .on_right_press(Message::MidiMap(MidiMapMessage::CloseMenu));

    let header = container(
        column![
            text("MIDI")
                .size(9)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::TEXT_3),
            text(target_label(r, target)).size(12).color(theme::TEXT_2),
        ]
        .spacing(2),
    )
    .padding([6, 10]);

    let mut col = column![header].spacing(1).width(MENU_WIDTH);
    let learn_label = if learning {
        "Cancel learn".to_string()
    } else if bound.is_empty() {
        "Learn MIDI".to_string()
    } else {
        "Learn MIDI (replace)".to_string()
    };
    col = col.push(menu_item(
        learn_label,
        Message::MidiMap(MidiMapMessage::Learn(target)),
        true,
    ));
    for b in &bound {
        col = col.push(
            container(text(source_label(b.source)).size(11).font(theme::MONO_FONT).color(theme::TEXT_3))
                .padding([2, 10]),
        );
    }
    col = col.push(menu_item(
        "Clear MIDI binding".to_string(),
        Message::MidiMap(MidiMapMessage::ClearTarget(target)),
        !bound.is_empty(),
    ));

    let menu_box = container(opaque(col)).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    });

    // Keep the menu on screen: flip it left / up near the window edge.
    let window = r.ui.window_size;
    let menu_height = 110.0 + 18.0 * bound.len() as f32;
    let x = if window.width > 0.0 && menu.x + MENU_WIDTH > window.width {
        (menu.x - MENU_WIDTH).max(0.0)
    } else {
        menu.x
    };
    let y = if window.height > 0.0 && menu.y + menu_height > window.height {
        (menu.y - menu_height).max(0.0)
    } else {
        menu.y
    };
    let positioned = container(row![menu_box])
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(iced::Padding {
            top: y,
            right: 0.0,
            bottom: 0.0,
            left: x,
        });

    stack![backdrop, positioned].into()
}
