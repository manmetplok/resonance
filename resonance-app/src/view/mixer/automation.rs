//! Per-channel automation controls on the mixer strips (architecture doc
//! #162 §3, todo #383 / A5).
//!
//! Each track / bus / master strip carries a compact "lane header": a
//! parameter picker that points an automation lane at any supported
//! target for that channel (its gain / pan / mute, or a CLAP param on one
//! of its plugin instances) and — once a lane exists — a per-lane Read
//! toggle plus a remove button. Selecting a target sends
//! [`AutomationMessage::AddLane`]; the engine echoes the lane back through
//! the one-way mirror, so the picker, toggle and the timeline canvas all
//! reflect the same [`AutomationState`].
//!
//! When Read is on during playback the channel's fader / pan knob is
//! tinted with the live automated value from
//! [`AutomationState::live_values`] (see [`live_value`]). The lookups here
//! never touch the engine — they read the app-side mirror only, per the
//! command/event boundary (doc #105).
//!
//! View-performance rules (MEMORY ui-work §11): the gain/pan/mute base
//! option list is channel-independent and cached once; per-plugin params
//! are appended only on channels that actually host plugins.

use std::rc::Rc;

use iced::widget::{button, column, container, pick_list, row, text, Space};
use iced::{alignment, Element, Length};

use crate::message::{AutomationMessage, Message};
use crate::state::{AutomationState, PluginSlotState};
use crate::theme;
use resonance_common::{AutomationLane, AutomationTarget};

/// The channel a strip's automation header belongs to. Resolves an
/// [`AutoChoice`] kind into the concrete [`AutomationTarget`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AutoChan {
    Track(u64),
    Bus(u64),
    Master,
}

/// A single pickable automation target shown in a strip's parameter
/// picker. Plugin params carry their own label; the built-in
/// gain/pan/mute kinds use a fixed one.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct AutoChoice {
    kind: AutoKind,
    label: Rc<str>,
}

#[derive(Debug, Clone, PartialEq)]
enum AutoKind {
    Gain,
    Pan,
    Mute,
    Param { instance: u64, param_id: u32 },
}

impl std::fmt::Display for AutoChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

impl AutoChoice {
    fn built_in(kind: AutoKind, label: &'static str) -> Self {
        Self {
            kind,
            label: Rc::from(label),
        }
    }
}

/// Channel-independent base options (gain / pan / mute), cached once per
/// thread. `pick_list` clones the `Rc<[_]>` per frame — a refcount bump,
/// no Vec rebuild — for every channel that hosts no plugins (the common
/// case). A `thread_local` (not a `static`) because `Rc` isn't `Sync`;
/// the view always runs on the GUI thread.
fn full_base() -> Rc<[AutoChoice]> {
    thread_local! {
        static BASE: Rc<[AutoChoice]> = Rc::from(vec![
            AutoChoice::built_in(AutoKind::Gain, "Volume"),
            AutoChoice::built_in(AutoKind::Pan, "Pan"),
            AutoChoice::built_in(AutoKind::Mute, "Mute"),
        ]);
    }
    BASE.with(Rc::clone)
}

/// Master strips automate gain only (no pan/mute on the master bus).
fn master_base() -> Rc<[AutoChoice]> {
    thread_local! {
        static BASE: Rc<[AutoChoice]> =
            Rc::from(vec![AutoChoice::built_in(AutoKind::Gain, "Volume")]);
    }
    BASE.with(Rc::clone)
}

/// Build the picker option list for `chan`: the cached gain/pan/mute base
/// plus one entry per exposed param on each hosted plugin instance.
fn choices_for(chan: AutoChan, plugins: &[PluginSlotState]) -> Vec<AutoChoice> {
    let base = match chan {
        AutoChan::Master => master_base(),
        _ => full_base(),
    };
    let mut out: Vec<AutoChoice> = base.to_vec();
    for slot in plugins {
        for param in &slot.params {
            out.push(AutoChoice {
                kind: AutoKind::Param {
                    instance: slot.instance_id,
                    param_id: param.id,
                },
                label: Rc::from(format!("{}: {}", slot.plugin_name, param.name)),
            });
        }
    }
    out
}

/// Resolve a picked [`AutoChoice`] into the concrete target for `chan`.
fn target_of(chan: AutoChan, kind: &AutoKind) -> Option<AutomationTarget> {
    Some(match (chan, kind) {
        (AutoChan::Track(id), AutoKind::Gain) => AutomationTarget::TrackGain(id),
        (AutoChan::Track(id), AutoKind::Pan) => AutomationTarget::TrackPan(id),
        (AutoChan::Track(id), AutoKind::Mute) => AutomationTarget::TrackMute(id),
        (AutoChan::Bus(id), AutoKind::Gain) => AutomationTarget::BusGain(id),
        (AutoChan::Bus(id), AutoKind::Pan) => AutomationTarget::BusPan(id),
        (AutoChan::Bus(id), AutoKind::Mute) => AutomationTarget::BusMute(id),
        (AutoChan::Master, AutoKind::Gain) => AutomationTarget::MasterGain,
        (_, AutoKind::Param { instance, param_id }) => AutomationTarget::PluginParam {
            instance: *instance,
            param_id: *param_id,
        },
        // Master has no pan/mute target; those choices are never built for
        // a master strip, so this arm is unreachable in practice.
        (AutoChan::Master, _) => return None,
    })
}

/// Whether `target` drives one of `chan`'s parameters (its own gain/pan/
/// mute, or a CLAP param on a plugin instance the channel hosts).
fn belongs(target: AutomationTarget, chan: AutoChan, plugins: &[PluginSlotState]) -> bool {
    let hosts = |instance: u64| plugins.iter().any(|p| p.instance_id == instance);
    match (chan, target) {
        (AutoChan::Track(id), AutomationTarget::TrackGain(t))
        | (AutoChan::Track(id), AutomationTarget::TrackPan(t))
        | (AutoChan::Track(id), AutomationTarget::TrackMute(t)) => id == t,
        (AutoChan::Bus(id), AutomationTarget::BusGain(t))
        | (AutoChan::Bus(id), AutomationTarget::BusPan(t))
        | (AutoChan::Bus(id), AutomationTarget::BusMute(t)) => id == t,
        (AutoChan::Master, AutomationTarget::MasterGain) => true,
        (_, AutomationTarget::PluginParam { instance, .. }) => hosts(instance),
        _ => false,
    }
}

/// Stable ordering used to pick the single lane a strip header surfaces
/// when a channel automates several targets: gain, then pan, then mute,
/// then the lowest plugin-param id. Mirrors the timeline's
/// `target_priority` so the strip header and the timeline band agree on
/// which lane is "primary".
fn priority(target: AutomationTarget) -> u32 {
    match target {
        AutomationTarget::TrackGain(_)
        | AutomationTarget::BusGain(_)
        | AutomationTarget::MasterGain => 0,
        AutomationTarget::TrackPan(_) | AutomationTarget::BusPan(_) => 1,
        AutomationTarget::TrackMute(_) | AutomationTarget::BusMute(_) => 2,
        AutomationTarget::PluginParam { param_id, .. } => 10u32.saturating_add(param_id),
    }
}

/// The lane a strip's header surfaces for `chan` — the highest-priority
/// lane whose target belongs to the channel, or `None` when the channel
/// has no automation.
fn primary_lane<'a>(
    automation: &'a AutomationState,
    chan: AutoChan,
    plugins: &[PluginSlotState],
) -> Option<&'a AutomationLane> {
    automation
        .lanes
        .values()
        .filter(|lane| belongs(lane.target, chan, plugins))
        .min_by_key(|lane| priority(lane.target))
}

/// Live automated value (normalized `0.0..=1.0`) for `target`, or `None`
/// when the lane is absent, Read-disabled, or no throttled value has
/// arrived yet (i.e. playback isn't currently driving it). Thin wrapper
/// over [`AutomationState::live_value`] so the strips read the app-side
/// mirror through one entry point.
pub(super) fn live_value(
    automation: &AutomationState,
    target: AutomationTarget,
) -> Option<f32> {
    automation.live_value(target)
}

/// Short human label for a lane target, shown beside the Read toggle.
fn target_label(target: AutomationTarget) -> &'static str {
    match target {
        AutomationTarget::TrackGain(_)
        | AutomationTarget::BusGain(_)
        | AutomationTarget::MasterGain => "Volume",
        AutomationTarget::TrackPan(_) | AutomationTarget::BusPan(_) => "Pan",
        AutomationTarget::TrackMute(_) | AutomationTarget::BusMute(_) => "Mute",
        AutomationTarget::PluginParam { .. } => "Param",
    }
}

/// Build the compact automation lane header for a channel strip: the
/// parameter picker and, once a lane exists, its Read toggle + remove
/// button. Placed just above the pan/fader block on each strip.
pub(super) fn automation_header<'a>(
    automation: &AutomationState,
    chan: AutoChan,
    plugins: &[PluginSlotState],
) -> Element<'a, Message> {
    let options = choices_for(chan, plugins);
    let picker = pick_list(options, None::<AutoChoice>, move |choice: AutoChoice| {
        match target_of(chan, &choice.kind) {
            Some(target) => Message::Automation(AutomationMessage::AddLane(target)),
            // Unreachable for built choices; route to a harmless no-op by
            // re-adding nothing. AddLane on an existing lane is itself a
            // no-op, so reuse the master-gain target as a safe sink.
            None => Message::Automation(AutomationMessage::AddLane(AutomationTarget::MasterGain)),
        }
    })
    .placeholder("+ Automation")
    .text_size(10)
    .width(Length::Fill);

    let mut col = column![picker].spacing(3).width(Length::Fill);

    if let Some(lane) = primary_lane(automation, chan, plugins) {
        let target = lane.target;
        let enabled = lane.enabled;

        let name = text(target_label(target))
            .size(9)
            .color(theme::TEXT_2);

        let read_btn = button(
            text("READ")
                .size(8)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(if enabled { theme::WARM } else { theme::TEXT_3 }),
        )
        .on_press(Message::Automation(AutomationMessage::ToggleRead(target)))
        .padding([1, 5])
        .style(move |_theme, status| theme::toggle_button_style(enabled, theme::WARM, true, status));

        let remove_btn = button(text("\u{2715}").size(9).color(theme::TEXT_3))
            .on_press(Message::Automation(AutomationMessage::RemoveLane(target)))
            .padding([1, 4])
            .style(|_theme, status| theme::small_button_style(status));

        let lane_row = row![
            name,
            Space::new().width(Length::Fill),
            read_btn,
            remove_btn,
        ]
        .spacing(4)
        .align_y(alignment::Vertical::Center);

        col = col.push(lane_row);
    }

    container(col).width(Length::Fill).into()
}
