//! Per-channel automation support for the mixer (architecture doc #162
//! §3, todo #383 / A5).
//!
//! The inspector's AUTOMATION group (mixer-cleanup.md §3.4) lists a
//! channel's lanes and offers a parameter picker that points a lane at
//! any supported target for that channel (its gain / pan / mute, a CLAP
//! param on one of its plugin instances, or a named device param).
//! Selecting a target sends [`AutomationMessage::AddLane`]; the engine
//! echoes the lane back through the one-way mirror, so the picker, the
//! Read toggles and the timeline canvas all reflect the same
//! [`AutomationState`].
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

use iced::widget::pick_list;
use iced::{Element, Length};

use crate::message::{AutomationMessage, Message};
use crate::state::{AutomationState, PluginSlotState};
use resonance_common::{AutomationLane, AutomationTarget, DeviceParam};

/// The channel a strip's automation header belongs to. Resolves an
/// [`AutoChoice`] kind into the concrete [`AutomationTarget`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoChan {
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
    /// A named parameter on the track's selected external-instrument device
    /// definition (epic #40, doc #201 §5). Only ever built for a
    /// [`AutoChan::Track`] whose track has a device preset selected; resolves
    /// to [`AutomationTarget::DeviceParam`]. `param_id` is the
    /// [`DeviceParam::id`].
    Device { param_id: String },
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

/// Build the picker option list for `chan`: the cached gain/pan/mute base,
/// one entry per exposed param on each hosted plugin instance, and — for a
/// track with an external-instrument device preset selected — one entry per
/// named [`DeviceParam`], clustered by [`DeviceParam::group`] (epic #40, doc
/// #201 §5). `device_params` is empty when no preset is selected (or the
/// channel is a bus/master), so the picker hides device params exactly then.
fn choices_for(
    chan: AutoChan,
    plugins: &[PluginSlotState],
    device_params: &[DeviceParam],
) -> Vec<AutoChoice> {
    let base = match chan {
        AutoChan::Master => master_base(),
        _ => full_base(),
    };
    let mut out: Vec<AutoChoice> = base.to_vec();
    for (index, slot) in plugins.iter().enumerate() {
        let slot_name = slot_label(plugins, index);
        for param in &slot.params {
            // CLAP's IS_HIDDEN: the plugin asks that this parameter not
            // be presented as a control. It stays in the app's mirror
            // because it is still automatable and still saved (ba todo
            // #1290) — it just doesn't belong in a human's picker.
            if param.hidden {
                continue;
            }
            // CLAP's IS_AUTOMATABLE unset: the plugin offers no lane for
            // it (the drums' kit selector, a read-only output).
            if !param.automatable {
                continue;
            }
            out.push(AutoChoice {
                kind: AutoKind::Param {
                    instance: slot.instance_id,
                    param_id: param.id,
                },
                label: Rc::from(format!("{}: {}", slot_name, param.name)),
            });
        }
    }
    // Device params, clustered by group so related controls (Filter,
    // Envelope, …) sit together in the dropdown. The group prefixes the
    // label ("Filter: Cutoff") the same way plugin params prefix with the
    // plugin name; ungrouped params show their bare name. The definition
    // lists params already grouped, so file order preserves the clustering.
    for param in device_params {
        let label = match &param.group {
            Some(group) => format!("{}: {}", group, param.name),
            None => param.name.clone(),
        };
        out.push(AutoChoice {
            kind: AutoKind::Device {
                param_id: param.id.clone(),
            },
            label: Rc::from(label),
        });
    }
    out
}

/// Test-only: the ordered picker option labels a *track* strip would show,
/// given its hosted `plugins` and any resolved `device_params`. Mirrors
/// [`choices_for`] exactly (a closed `pick_list` renders only its
/// placeholder, so an integration test can't read the dropdown items off the
/// rendered tree) — used by `tests/automation_device_params.rs` to assert the
/// named device params appear, grouped, only when a preset is selected.
#[doc(hidden)]
pub(crate) fn track_choice_labels(
    track_id: u64,
    plugins: &[PluginSlotState],
    device_params: &[DeviceParam],
) -> Vec<String> {
    choices_for(AutoChan::Track(track_id), plugins, device_params)
        .into_iter()
        .map(|c| c.label.to_string())
        .collect()
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
        // Device params only exist on a track channel (they're built solely
        // for `AutoChan::Track` with a selected preset). The lane addresses
        // the track directly.
        (AutoChan::Track(id), AutoKind::Device { param_id }) => AutomationTarget::DeviceParam {
            track: id,
            param_id: param_id.clone(),
        },
        // Master has no pan/mute target, and no channel other than a track
        // builds a device choice; those choices are never built here, so
        // these arms are unreachable in practice.
        (AutoChan::Master, _) | (_, AutoKind::Device { .. }) => return None,
    })
}

/// Whether `target` drives one of `chan`'s parameters (its own gain/pan/
/// mute, or a CLAP param on a plugin instance the channel hosts).
fn belongs(target: &AutomationTarget, chan: AutoChan, plugins: &[PluginSlotState]) -> bool {
    let hosts = |instance: u64| plugins.iter().any(|p| p.instance_id == instance);
    match (chan, target) {
        (AutoChan::Track(id), AutomationTarget::TrackGain(t))
        | (AutoChan::Track(id), AutomationTarget::TrackPan(t))
        | (AutoChan::Track(id), AutomationTarget::TrackMute(t)) => id == *t,
        (AutoChan::Bus(id), AutomationTarget::BusGain(t))
        | (AutoChan::Bus(id), AutomationTarget::BusPan(t))
        | (AutoChan::Bus(id), AutomationTarget::BusMute(t)) => id == *t,
        (AutoChan::Master, AutomationTarget::MasterGain) => true,
        (_, AutomationTarget::PluginParam { instance, .. }) => hosts(*instance),
        (AutoChan::Track(id), AutomationTarget::DeviceParam { track, .. }) => id == *track,
        _ => false,
    }
}

/// Stable ordering used to pick the single lane a strip header surfaces
/// when a channel automates several targets: gain, then pan, then mute,
/// then the lowest plugin-param id. Mirrors the timeline's
/// `target_priority` so the strip header and the timeline band agree on
/// which lane is "primary".
fn priority(target: &AutomationTarget) -> u32 {
    match target {
        AutomationTarget::TrackGain(_)
        | AutomationTarget::BusGain(_)
        | AutomationTarget::MasterGain => 0,
        AutomationTarget::TrackPan(_) | AutomationTarget::BusPan(_) => 1,
        AutomationTarget::TrackMute(_) | AutomationTarget::BusMute(_) => 2,
        // Device params sit below plugin params; several device lanes tie at
        // this tier, and `min_by_key` surfaces one of them as the header's
        // primary lane (the exact one among ties is unimportant).
        AutomationTarget::DeviceParam { .. } => 5,
        AutomationTarget::PluginParam { param_id, .. } => 10u32.saturating_add(*param_id),
    }
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

/// Short human label for a lane target, shown beside the Read toggle. A
/// `DeviceParam` lane resolves to its named [`DeviceParam::name`] from
/// `device_params` (the track's selected device definition), falling back to
/// a generic label if the param id is unknown (e.g. the preset changed while
/// a lane survives).
fn target_label(target: &AutomationTarget, device_params: &[DeviceParam]) -> String {
    match target {
        AutomationTarget::TrackGain(_)
        | AutomationTarget::BusGain(_)
        | AutomationTarget::MasterGain => "Volume".to_string(),
        AutomationTarget::TrackPan(_) | AutomationTarget::BusPan(_) => "Pan".to_string(),
        AutomationTarget::TrackMute(_) | AutomationTarget::BusMute(_) => "Mute".to_string(),
        AutomationTarget::PluginParam { .. } => "Param".to_string(),
        AutomationTarget::DeviceParam { param_id, .. } => device_params
            .iter()
            .find(|p| &p.id == param_id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Device".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Inspector AUTOMATION group support (mixer-cleanup.md §3.4).
//
// The inspector lists *every* lane on a channel and offers the
// `+ Add lane` options (the strips' own lane header left them in
// mixer-cleanup.md §2.2). These helpers keep one option source and one
// target resolution, so a lane added from any surface is the same
// `AutomationMessage::AddLane`.
// ---------------------------------------------------------------------------

/// The message picking `choice` on `chan` raises.
fn add_lane_message(chan: AutoChan, choice: &AutoChoice) -> Message {
    match target_of(chan, &choice.kind) {
        Some(target) => Message::Automation(AutomationMessage::AddLane(target)),
        // Unreachable for built choices; `AddLane` on an existing lane
        // is a no-op, so the master-gain target is a safe sink.
        None => Message::Automation(AutomationMessage::AddLane(AutomationTarget::MasterGain)),
    }
}

/// The inspector's `+ Add lane` picker for `chan`: the options the strip
/// header offers (gain / pan / mute, plugin params, device params).
pub(super) fn add_lane_picker(
    chan: AutoChan,
    plugins: &[PluginSlotState],
    device_params: &[DeviceParam],
) -> Element<'static, Message> {
    let options = choices_for(chan, plugins, device_params);
    pick_list(options, None::<AutoChoice>, move |choice: AutoChoice| {
        add_lane_message(chan, &choice)
    })
    .placeholder("+ Add lane")
    .text_size(12)
    .padding([8, 10])
    .width(Length::Fill)
    .into()
}

/// Test-only: the `AddLane` message the inspector picker raises for the
/// option labelled `label` on `chan`, or `None` when no option carries
/// that label. A closed `pick_list` renders only its placeholder, so a
/// test can't click an option; this resolves one exactly as the picker's
/// `on_select` does.
#[doc(hidden)]
pub(crate) fn add_lane_message_for_label(
    chan: AutoChan,
    plugins: &[PluginSlotState],
    device_params: &[DeviceParam],
    label: &str,
) -> Option<Message> {
    choices_for(chan, plugins, device_params)
        .into_iter()
        .find(|c| &*c.label == label)
        .map(|c| add_lane_message(chan, &c))
}

/// Every lane whose target belongs to `chan`, with its label, in the
/// order the inspector lists them: gain, pan, mute, device params, then
/// plugin params grouped per plugin — by chain slot, then parameter id —
/// so one plugin's lanes sit together in chain order. Remaining ties
/// break by label then lane id so the list never reshuffles between
/// frames — `lanes` is a `HashMap`.
pub(super) fn lanes_for<'a>(
    automation: &'a AutomationState,
    chan: AutoChan,
    plugins: &[PluginSlotState],
    device_params: &[DeviceParam],
) -> Vec<(&'a AutomationLane, String)> {
    let mut lanes: Vec<(&AutomationLane, String)> = automation
        .lanes
        .values()
        .filter(|lane| belongs(&lane.target, chan, plugins))
        .map(|lane| (lane, lane_label(&lane.target, plugins, device_params)))
        .collect();
    lanes.sort_by(|(a, la), (b, lb)| {
        list_order(&a.target, plugins)
            .cmp(&list_order(&b.target, plugins))
            .then_with(|| la.cmp(lb))
            .then_with(|| a.id.cmp(&b.id))
    });
    lanes
}

/// The lanes [`lanes_for`] lists for `chan`, unsorted and unlabelled —
/// the cheap walk the inspector's fingerprint hashes.
pub(super) fn owned_lanes<'a>(
    automation: &'a AutomationState,
    chan: AutoChan,
    plugins: &'a [PluginSlotState],
) -> impl Iterator<Item = &'a AutomationLane> + 'a {
    automation
        .lanes
        .values()
        .filter(move |lane| belongs(&lane.target, chan, plugins))
}

/// The inspector list's sort key for a lane: the built-in tiers of
/// [`priority`] first, then plugin params by (chain slot, param id).
fn list_order(target: &AutomationTarget, plugins: &[PluginSlotState]) -> (u32, usize, u32) {
    match target {
        AutomationTarget::PluginParam { instance, param_id } => {
            let slot = plugins
                .iter()
                .position(|p| p.instance_id == *instance)
                .unwrap_or(usize::MAX);
            (10, slot, *param_id)
        }
        other => (priority(other), 0, 0),
    }
}

/// The name a chain slot goes by in a lane label: the plugin's name, with
/// its ordinal among same-named slots ("Comp #2") when the chain holds
/// more than one instance of it — otherwise two instances' lanes would
/// read identically.
pub(crate) fn slot_label(plugins: &[PluginSlotState], index: usize) -> String {
    let name = &plugins[index].plugin_name;
    let same = |p: &&PluginSlotState| p.plugin_name == *name;
    if plugins.iter().filter(same).count() < 2 {
        return name.clone();
    }
    let ordinal = plugins[..=index].iter().filter(same).count();
    format!("{name} #{ordinal}")
}

/// The full human label of a lane target: [`target_label`], except that
/// a plugin-param lane names its plugin and parameter (the picker's
/// `"<plugin>: <param>"` label, with the slot ordinal of
/// [`slot_label`]) rather than a bare "Param" — the inspector lists
/// several lanes, and "Param" twice says nothing.
pub(super) fn lane_label(
    target: &AutomationTarget,
    plugins: &[PluginSlotState],
    device_params: &[DeviceParam],
) -> String {
    if let AutomationTarget::PluginParam { instance, param_id } = target {
        if let Some(index) = plugins.iter().position(|p| p.instance_id == *instance) {
            let slot = &plugins[index];
            let slot_name = slot_label(plugins, index);
            return match slot.params.iter().find(|p| p.id == *param_id) {
                Some(param) => format!("{}: {}", slot_name, param.name),
                None => format!("{}: #{}", slot_name, param_id),
            };
        }
    }
    target_label(target, device_params)
}
