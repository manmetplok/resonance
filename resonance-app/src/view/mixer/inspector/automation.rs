//! AUTOMATION group — every automation lane on the inspected channel
//! (track, bus or master), one row each with its Read toggle, and the
//! `+ Add lane` picker (mixer-cleanup.md §3.4).
//!
//! It is emitters only: the rows and the picker raise the
//! `AutomationMessage`s the strip's lane header raises, and the option
//! list and target resolution come from the same functions
//! (`view::mixer::automation`), so a lane added here is the lane the
//! strip and the Arrange overlay show.
//!
//! Not built: the "show in Arrange" jump the spec lists per row. It
//! needs a navigation message that switches tab *and* points the
//! track's Arrange overlay at a specific lane; nothing does that today.

use std::hash::{Hash, Hasher};

use iced::widget::{button, column, container, row, text, Space};
use iced::{alignment, Element, Length};
use resonance_common::{AutomationTarget, DeviceParam};

use crate::message::{AutomationMessage, Message};
use crate::state::{MixerInspectorGroup, PluginSlotState};
use crate::theme;
use crate::view::mixer::automation::{self as lanes, AutoChan};

pub(super) fn automation_group(
    r: &crate::Resonance,
    chan: AutoChan,
    plugins: &[PluginSlotState],
    device_params: &[DeviceParam],
    collapsed: bool,
) -> Element<'static, Message> {
    let header =
        super::widgets::group_header("AUTOMATION", MixerInspectorGroup::Automation, collapsed);
    if collapsed {
        return header;
    }

    // 10px column spacing doubles as the title → first-row gap, like CHAIN.
    let mut col = column![header].spacing(10);
    let rows = lanes::lanes_for(&r.automation, chan, plugins, device_params);
    if rows.is_empty() {
        col = col.push(super::widgets::placeholder_row("No automation lanes"));
    } else {
        for (lane, label) in rows {
            col = col.push(lane_row(label, lane.target.clone(), lane.enabled));
        }
    }
    col.push(lanes::add_lane_picker(chan, plugins, device_params))
        .into()
}

/// Hash everything [`automation_group`] draws for `chan`: each lane's
/// target, label and Read state, and the picker's option source (the
/// plugin params and device params). Folded into the owner's inspector
/// fingerprint.
pub(super) fn hash_into<H: Hasher>(
    h: &mut H,
    r: &crate::Resonance,
    chan: AutoChan,
    plugins: &[PluginSlotState],
    device_params: &[DeviceParam],
) {
    for (lane, label) in lanes::lanes_for(&r.automation, chan, plugins, device_params) {
        lane.target.hash(h);
        lane.enabled.hash(h);
        label.hash(h);
    }
    for slot in plugins {
        slot.instance_id.hash(h);
        slot.plugin_name.hash(h);
        for p in &slot.params {
            p.id.hash(h);
            p.name.hash(h);
            p.hidden.hash(h);
        }
    }
    for p in device_params {
        p.id.hash(h);
        p.name.hash(h);
        p.group.hash(h);
    }
}

/// One lane: target name · READ · ✕.
fn lane_row(label: String, target: AutomationTarget, enabled: bool) -> Element<'static, Message> {
    let read_btn = button(
        text("READ")
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(if enabled { theme::WARM } else { theme::TEXT_3 }),
    )
    .on_press(Message::Automation(AutomationMessage::ToggleRead(
        target.clone(),
    )))
    .padding([2, 6])
    .style(move |_theme, status| theme::toggle_button_style(enabled, theme::WARM, true, status));

    let remove_btn = button(text("\u{2715}").size(10).color(theme::TEXT_3))
        .on_press(Message::Automation(AutomationMessage::RemoveLane(target)))
        .padding([2, 5])
        .style(|_theme, status| theme::small_button_style(status));

    let name_color = if enabled { theme::TEXT_1 } else { theme::TEXT_3 };
    container(
        row![
            text(label)
                .size(12)
                .color(name_color)
                .width(Length::Fill)
                .wrapping(iced::widget::text::Wrapping::None),
            Space::new().width(8),
            read_btn,
            Space::new().width(4),
            remove_btn,
        ]
        .align_y(alignment::Vertical::Center),
    )
    .padding([6, 10])
    .width(Length::Fill)
    .clip(true)
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
