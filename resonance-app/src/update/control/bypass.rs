//! Per-slot bypass over the control API (ba doc #275 finding X3, todo
//! #1305), shared by the track, bus and master surfaces.
//!
//! Two different things are called "bypass" and they are independent:
//!
//! * `<surface>.set_fx_bypass` mutes a WHOLE chain at once. It is the
//!   button the mixer strip has always had.
//! * `<surface>.set_plugin_bypass` takes ONE slot out of the signal path
//!   and leaves the rest running. The chain remembers its per-slot flags
//!   across a chain bypass, so re-engaging the chain restores the mix
//!   rather than switching everything on.
//!
//! Both SET rather than toggle. The app's own messages toggle, which is
//! right for a button and wrong for a wire: a client that retried a
//! request whose reply it never saw would flip the state back. So every
//! handler here reads the mirrored value first and dispatches only a real
//! change — except the per-slot path, which is a SET message in its own
//! right and is safe to send unconditionally.

use resonance_audio::types::PluginInstanceId;
use resonance_control::{Request, Response, RpcError};

use super::reply::{ack, reject};
use crate::state::PluginSlotState;

/// Resolve `(plugin_id, occurrence)` against one chain.
///
/// `default_slot` decides what an omitted `plugin_id` means, which
/// differs per surface for the same reason `set_plugin_param` differs: a
/// track's unnamed plugin is its instrument (the thing you meant), a
/// bus's or the master's is the first insert (there is no instrument).
pub(super) fn resolve_slot(
    chain: &[PluginSlotState],
    plugin_id: Option<&str>,
    occurrence: Option<u32>,
    default_slot: impl Fn(&[PluginSlotState]) -> Option<PluginInstanceId>,
    host: &str,
) -> Result<PluginInstanceId, RpcError> {
    let Some(wanted) = plugin_id else {
        return default_slot(chain).ok_or_else(|| {
            RpcError::not_found(format!(
                "{host} carries no plugin to bypass [{}]",
                chain_description(chain)
            ))
        });
    };
    let occurrence = occurrence.unwrap_or(0);
    chain
        .iter()
        .filter(|p| p.clap_plugin_id == wanted)
        .nth(occurrence as usize)
        .map(|p| p.instance_id)
        .ok_or_else(|| {
            RpcError::not_found(format!(
                "{host} has no plugin {wanted:?} at occurrence {occurrence}; it carries [{}]",
                chain_description(chain)
            ))
        })
}

/// The chain as a readable list, for an error a caller can act on.
fn chain_description(chain: &[PluginSlotState]) -> String {
    chain
        .iter()
        .map(|p| p.clap_plugin_id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The instrument slot, else the first — a track's `plugin_id`-less
/// default.
///
/// `instrument_slot` is the same resolver `plugin_entries` tags entries
/// with, so "the unnamed plugin" means the same thing here as it does in
/// `track.plugin_params` and `track.set_plugin_param`. Falls back to the
/// first slot on an effect-only track, where there is no instrument but
/// there is still an obvious "the plugin".
pub(super) fn instrument_or_first(
    app: &crate::Resonance,
    t: &crate::state::TrackState,
) -> Option<PluginInstanceId> {
    crate::plugin_chain::instrument_slot(app, t)
        .and_then(|i| t.plugins.get(i))
        .or_else(|| t.plugins.first())
        .map(|p| p.instance_id)
}

/// The first slot — a bus's or the master's `plugin_id`-less default.
pub(super) fn first_slot(chain: &[PluginSlotState]) -> Option<PluginInstanceId> {
    chain.first().map(|p| p.instance_id)
}

/// Turn a resolved slot into the app message that changes it.
///
/// One message for all three surfaces: `PluginMessage::SetPluginBypass`
/// addresses by instance id, which is already unique across every chain,
/// so the surface cannot be got wrong on the way through.
pub(super) fn message(instance_id: PluginInstanceId, bypassed: bool) -> crate::message::Message {
    crate::message::Message::Plugin(crate::message::PluginMessage::SetPluginBypass {
        instance_id,
        bypassed,
    })
}

/// Shared tail of the three `set_plugin_bypass` handlers: resolve, then
/// dispatch. Split out so a surface cannot quietly grow its own rule for
/// what an unknown plugin means.
pub(super) fn run(
    app: &mut crate::Resonance,
    request: &Request,
    chain: &[PluginSlotState],
    plugin_id: Option<&str>,
    occurrence: Option<u32>,
    bypassed: bool,
    default_slot: impl Fn(&[PluginSlotState]) -> Option<PluginInstanceId>,
    host: &str,
) -> (Response, iced::Task<crate::message::Message>) {
    match resolve_slot(chain, plugin_id, occurrence, default_slot, host) {
        Ok(instance_id) => {
            let task = super::run_via_update(app, message(instance_id, bypassed));
            (ack(app, request), task)
        }
        Err(e) => reject(request, e),
    }
}
