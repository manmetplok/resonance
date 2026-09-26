//! The half of `track/bus/master.replace_effect` that is the same on all
//! three surfaces (ba doc #275 P5, todo #1309).
//!
//! Each surface keeps its own slot addressing — a track resolves through
//! `track::chain`, a bus through `bus::resolve_bus_effect`, the master
//! through `master::resolve_master_effect`, and those already differ
//! (only a track has an instrument slot). What must NOT differ is
//! everything after the slot is known: which catalog entry the
//! replacement names, how a replace is refused, what the reply says, and
//! the fact that the mutation goes through `update()` so a remote
//! replace is undoable exactly like the mixer's.
//!
//! So the surfaces hand this module a resolved instance id and get back
//! the finished response.

use iced::Task;
use resonance_audio::types::{PluginInstanceId, ScannedPlugin};
use resonance_control::methods::track::{ReplaceEffectResult, ReplaceOutcome};
use resonance_control::{Request, Response, RpcError};

use crate::message::{Message, PluginMessage};
use crate::update::control::reply::{reject, success};
use crate::update::control::run_via_update;
use crate::update::plugin_replace::ReplaceKind;
use crate::Resonance;

/// Perform a replace on an already-resolved slot and build the reply.
///
/// `slot` is the chain position being replaced, echoed back so a caller
/// can confirm the position survived without a second round trip — the
/// one guarantee this method makes over remove-then-add.
pub(super) fn replace_resolved_slot(
    app: &mut Resonance,
    request: &Request,
    instance_id: PluginInstanceId,
    slot: u32,
    new_plugin_id: &str,
) -> (Response, Task<Message>) {
    let plugin = match catalog_entry(app, new_plugin_id) {
        Ok(plugin) => plugin,
        Err(error) => return reject(request, error),
    };

    // Decided before the dispatch, from the same rule the mutation
    // applies (`plugin_replace::classify`), so the reply describes what
    // actually happened rather than restating the rule a second time.
    let Some(kind) = crate::update::plugin_replace::classify(app, instance_id, new_plugin_id)
    else {
        return reject(
            request,
            RpcError::not_found(format!(
                "plugin at slot {slot} vanished between lookup and replace"
            )),
        );
    };

    // Nothing to dispatch for a slot that already carries this plugin,
    // loaded — and dispatching anyway would burn an undo entry on a
    // no-op.
    let task = if kind == ReplaceKind::AlreadyLoaded {
        Task::none()
    } else {
        run_via_update(
            app,
            Message::Plugin(PluginMessage::ReplacePlugin {
                instance_id,
                plugin,
            }),
        )
    };

    let result = ReplaceEffectResult {
        slot,
        plugin_id: new_plugin_id.to_owned(),
        outcome: match kind {
            ReplaceKind::Relocate => ReplaceOutcome::Relocated,
            ReplaceKind::Swap => ReplaceOutcome::Swapped,
            ReplaceKind::AlreadyLoaded => ReplaceOutcome::AlreadyLoaded,
        },
        revision: app.revision(),
    };
    (success(request, &result), task)
}

/// The catalog entry for `plugin_id`, or a rejection that lists what IS
/// installed.
///
/// The catalog is the only source: a replacement has to be a plugin this
/// machine can actually instantiate, and accepting an arbitrary id/path
/// pair would let a caller "fix" a missing plugin by pointing the slot
/// at a second thing that also does not load.
fn catalog_entry(app: &Resonance, plugin_id: &str) -> Result<ScannedPlugin, RpcError> {
    if plugin_id.is_empty() {
        return Err(RpcError::invalid_params(
            "new_plugin_id is required: name the plugin that takes the slot, \
             from plugins.catalog",
        ));
    }
    app.plugin_catalog.available_plugins
        .iter()
        .find(|p| p.clap_plugin_id == plugin_id)
        .cloned()
        .ok_or_else(|| {
            RpcError::not_found(format!(
                "no installed plugin has id {plugin_id:?}; plugins.catalog lists what is \
                 available. If this is the plugin that is MISSING, install it and call \
                 plugins.rescan first — replace_effect can only put a plugin this machine \
                 has in the slot."
            ))
        })
}
