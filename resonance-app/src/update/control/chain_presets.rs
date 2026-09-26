//! `bus.*` / `master.*` plugin presets (ba todo #1333).
//!
//! The third surface's worth of the same three methods
//! `track.plugin_presets` / `.load_plugin_preset` / `.save_plugin_preset`
//! provide — everything past "which plugin instance is this?" is
//! [`super::plugin_presets`], shared with the track surface, so a preset
//! saved on a bus is the same file the plugin's own window lists.
//!
//! Bus and master share this module rather than one each because they
//! address a plugin identically: a flat insert chain over summed audio,
//! with no instrument slot, so an omitted `plugin_id` means "the first
//! plugin on the chain" on both. That is [`Chain`]'s whole job — the two
//! `try_handle`s parse their own params (a bus carries `bus_id`, master
//! is a singleton) and hand the rest here.

use crate::message::Message;
use crate::state::PluginSlotState;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::plugin_preset::PluginPresetSource;
use resonance_control::{Request, Response, RpcError};

use super::reply::{ack, no_bus, reject};
use super::{plugin_presets, view_model};

/// Which insert chain a request addresses.
#[derive(Debug, Clone, Copy)]
pub(super) enum Chain {
    Bus(u64),
    Master,
}

impl Chain {
    /// How to name this chain in an error, e.g. "bus 3" / "the master
    /// chain".
    fn describe(self) -> String {
        match self {
            Chain::Bus(id) => format!("bus {id}"),
            Chain::Master => "the master chain".to_owned(),
        }
    }

    /// The method prefix, so an error can point at the right list call.
    fn namespace(self) -> &'static str {
        match self {
            Chain::Bus(_) => "bus",
            Chain::Master => "master",
        }
    }

    fn slots(self, app: &Resonance) -> Result<&[PluginSlotState], RpcError> {
        match self {
            Chain::Bus(id) => app
                .registry
                .busses
                .iter()
                .find(|b| b.id == id)
                .map(|b| b.plugins.as_slice())
                .ok_or_else(|| no_bus(id)),
            Chain::Master => Ok(&app.master.plugins),
        }
    }
}

/// The chain as `slot:plugin_id` pairs, so a caller can correct an
/// address rather than guess again.
fn chain_description(slots: &[PluginSlotState]) -> String {
    slots
        .iter()
        .enumerate()
        .map(|(slot, p)| format!("{slot}:{}", p.clap_plugin_id))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The addressed plugin as `(clap id, mirrored params, instance id)`.
///
/// Shared by all three methods so they cannot disagree about which
/// plugin a given request means, and addressed exactly as
/// `*.set_plugin_param` addresses it.
fn resolve(
    app: &Resonance,
    chain: Chain,
    plugin_id: &Option<String>,
    occurrence: Option<u32>,
) -> Result<
    (
        String,
        Vec<resonance_control::methods::track::PluginParamView>,
        resonance_audio::types::PluginInstanceId,
    ),
    RpcError,
> {
    let slots = chain.slots(app)?;
    let occurrence = occurrence.unwrap_or(0);
    let slot = match plugin_id {
        Some(wanted) => slots
            .iter()
            .filter(|p| &p.clap_plugin_id == wanted)
            .nth(occurrence as usize),
        // No id names the first plugin on the chain — unambiguous on a
        // one-effect chain, which is the common case.
        None => slots.first(),
    };
    let Some(slot) = slot else {
        return Err(RpcError::not_found(match plugin_id {
            Some(wanted) => format!(
                "{} has no plugin {wanted:?} at occurrence {occurrence}; it carries [{}]",
                chain.describe(),
                chain_description(slots)
            ),
            None => format!(
                "{} carries no plugins; add one with {}.add_effect",
                chain.describe(),
                chain.namespace()
            ),
        }));
    };
    Ok((
        slot.clap_plugin_id.clone(),
        slot.params.iter().map(view_model::param_view).collect(),
        slot.instance_id,
    ))
}

/// `bus.plugin_presets` / `master.plugin_presets` — what this plugin can
/// recall. Read-only.
pub(super) fn view(
    app: &Resonance,
    request: &Request,
    chain: Chain,
    plugin_id: &Option<String>,
    occurrence: Option<u32>,
) -> (Response, Task<Message>) {
    let (clap_id, _, _) = match resolve(app, chain, plugin_id, occurrence) {
        Ok(resolved) => resolved,
        Err(e) => return reject(request, e),
    };
    (
        super::success(request, &plugin_presets::view(app, &clap_id)),
        Task::none(),
    )
}

/// `bus.load_plugin_preset` / `master.load_plugin_preset` — recall a
/// preset, as one undoable edit.
pub(super) fn load(
    app: &mut Resonance,
    request: &Request,
    chain: Chain,
    plugin_id: &Option<String>,
    occurrence: Option<u32>,
    preset: &str,
    source: Option<PluginPresetSource>,
) -> (Response, Task<Message>) {
    let (clap_id, mirrored, instance_id) = match resolve(app, chain, plugin_id, occurrence) {
        Ok(resolved) => resolved,
        Err(e) => return reject(request, e),
    };
    let message =
        match plugin_presets::load_message(app, &clap_id, instance_id, &mirrored, preset, source) {
            Ok(message) => message,
            Err(e) => return reject(request, e),
        };
    let task = super::run_via_update(app, message);
    (ack(app, request), task)
}

/// `bus.save_plugin_preset` / `master.save_plugin_preset` — capture this
/// plugin's current sound.
///
/// Answers as soon as the capture is armed, not when the file lands: the
/// plugin's state blob comes back from the engine a beat later and only
/// then is the preset written, because the plugin is the only thing that
/// knows edits made in its own window.
pub(super) fn save(
    app: &mut Resonance,
    request: &Request,
    chain: Chain,
    plugin_id: &Option<String>,
    occurrence: Option<u32>,
    name: &str,
    overwrite: bool,
) -> (Response, Task<Message>) {
    let (clap_id, _, instance_id) = match resolve(app, chain, plugin_id, occurrence) {
        Ok(resolved) => resolved,
        Err(e) => return reject(request, e),
    };
    if let Err(e) = plugin_presets::check_save(app, &clap_id, name, overwrite) {
        return reject(request, e);
    }

    app.presets.pending_plugin_preset_save = Some(crate::PendingPluginPresetSave {
        instance_id,
        clap_id,
        name: name.trim().to_string(),
    });
    let _ = app
        .engine
        .send(resonance_audio::types::AudioCommand::SavePluginState { instance_id });

    (ack(app, request), Task::none())
}
