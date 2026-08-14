//! `bus.*` control methods (ba doc #273, todo #1228).
//!
//! Busses are the stage between tracks and master: a group summed to one
//! point so it can be compressed, EQ'd and levelled as a unit. All of it
//! already existed (`BusMessage`, `update/bus.rs`, `ProjectState.busses`,
//! `AudioCommand::AddBus`) and was simply unreachable from the control
//! API, so no client could build a drum bus.
//!
//! Listing is deliberately absent: `song.summary` / `song.tracks` already
//! report busses as tracks with `kind: "bus"`, from a distinct id range.
//!
//! `bus.create` allocates the id APP-SIDE and passes it to the engine as
//! `id_hint`, because the plain `AddBus` path lets the engine allocate
//! and echo the id asynchronously — a reply that has to carry `bus_id`
//! cannot wait for that.

use crate::message::{BusMessage, Message};
use crate::state::BusState;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::bus::{
    self, CreateParams, CreateResult, DeleteParams, SetVolumeParams,
};
use resonance_control::methods::track;
use resonance_control::methods::mixer::{VOLUME_DB_MAX, VOLUME_DB_MIN};
use resonance_control::{Request, Response, RpcError};

use super::reply::{ack, not_found_bus, reject};

/// Handle a `bus.*` request, or `None` when `method` belongs to another
/// namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let out = match request.method.as_str() {
        bus::CREATE => create(app, request),
        bus::DELETE => delete(app, request),
        bus::SET_VOLUME => set_volume(app, request),
        bus::ADD_EFFECT => add_effect(app, request),
        bus::REMOVE_EFFECT => remove_effect(app, request),
        bus::MOVE_EFFECT => move_effect(app, request),
        bus::SET_FX_BYPASS => set_fx_bypass(app, request),
        bus::PLUGIN_PARAMS => plugin_params(app, request),
        bus::SET_PLUGIN_PARAM => set_plugin_param(app, request),
        _ => return None,
    };
    Some(out)
}

fn find_bus(app: &Resonance, id: u64) -> Option<&BusState> {
    app.registry.busses.iter().find(|b| b.id == id)
}

fn create(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: CreateParams = match super::optional_params(request) {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let name = match params.name {
        Some(name) if name.trim().is_empty() => {
            return reject(request, RpcError::invalid_params("bus name must not be empty"))
        }
        Some(name) => name,
        None => format!("Bus {}", app.registry.busses.len() + 1),
    };

    // App-side id so the reply can return it immediately; the engine
    // bumps its own allocator past any hint it receives.
    let bus_id = app.registry.allocate_return_bus_id();
    let task = super::run_via_update(
        app,
        Message::Bus(BusMessage::AddBusWithId { id: bus_id, name }),
    );
    let result = CreateResult {
        bus_id: resonance_control::ids::TrackId(bus_id),
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

fn delete(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: DeleteParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_bus(app, params.bus_id.0).is_none() {
        return not_found_bus(request, params.bus_id.0);
    }
    if !params.confirm {
        // `update/bus.rs` re-routes members to master rather than
        // silencing them, so say exactly that instead of implying loss.
        let members: Vec<&str> = app
            .registry
            .tracks
            .iter()
            .filter(|t| t.output == resonance_audio::types::TrackOutput::Bus(params.bus_id.0))
            .map(|t| t.name.as_str())
            .collect();
        return reject(
            request,
            RpcError::needs_confirmation(format!(
                "deleting bus {} re-routes {} track(s) to master ({}) and discards the bus's \
                 own level and effects; re-send with \"confirm\": true",
                params.bus_id,
                members.len(),
                if members.is_empty() {
                    "none".to_owned()
                } else {
                    members.join(", ")
                }
            )),
        );
    }
    let task = super::run_via_update(app, Message::Bus(BusMessage::RemoveBus(params.bus_id.0)));
    (ack(app, request), task)
}

fn set_volume(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetVolumeParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_bus(app, params.bus_id.0).is_none() {
        return not_found_bus(request, params.bus_id.0);
    }
    if !params.volume_db.is_finite() {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "volume_db must be finite (got {}); {VOLUME_DB_MIN} dB is the app's silence \
                 floor, not -inf",
                params.volume_db
            )),
        );
    }
    if !(VOLUME_DB_MIN..=VOLUME_DB_MAX).contains(&params.volume_db) {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "volume_db must be within {VOLUME_DB_MIN}..={VOLUME_DB_MAX} dB — the range the \
                 mixer fader spans — got {}",
                params.volume_db
            )),
        );
    }
    let task = super::run_via_update(
        app,
        Message::Bus(BusMessage::SetBusVolume(params.bus_id.0, params.volume_db)),
    );
    (ack(app, request), task)
}

// ---------------------------------------------------------------------------
// The bus insert chain (ba doc #273, todo #1237)
// ---------------------------------------------------------------------------
//
// This is what makes a group actually mixable rather than merely
// summable. `render_core` already runs a bus's plugin chain over the
// accumulated buffer, with bus PDC, fader and the master sum after it,
// and sub-tracks already honour `TrackOutput::Bus` — so routing a kit's
// taps to a bus and inserting there needs ZERO engine DSP work. It
// worked in the GUI all along and was unreachable over the control API
// only because no method could put a plugin on a bus.
//
// Why that matters: a kit that peaks near clipping while no individual
// tap exceeds ~-8 dBFS can only be controlled on the SUM. A compressor
// inserted on each contributing track never sees the peak, because no
// single source makes it — kick plus snare plus crash landing together
// does.
//
// Unlike a track's, a bus chain is a plain `Vec<PluginInstanceId>`
// engine-side, and it has no structural slot 0: every entry is an
// effect over the group sum.

/// The bus's chain as wire entries, in processing order, each tagged
/// with its occurrence among same-id siblings.
///
/// Deliberately the same [`track::PluginParamsEntry`] shape
/// `track.plugin_params` returns, so a client reads a bus chain with the
/// code it already has. `kind` is always `Effect`: a bus has no
/// instrument slot.
fn bus_plugin_entries(bus: &BusState) -> Vec<track::PluginParamsEntry> {
    let mut seen: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    bus.plugins
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let occurrence = seen
                .entry(p.clap_plugin_id.as_str())
                .and_modify(|n| *n += 1)
                .or_insert(0);
            track::PluginParamsEntry {
                plugin_id: p.clap_plugin_id.clone(),
                name: p.plugin_name.clone(),
                slot: i as u32,
                occurrence: *occurrence,
                kind: track::PluginKind::Effect,
                params: p
                    .params
                    .iter()
                    .map(|param| track::PluginParamView {
                        id: param.id,
                        name: param.name.clone(),
                        value: param.current_value,
                        min: param.min_value,
                        max: param.max_value,
                        default: param.default_value,
                    })
                    .collect(),
            }
        })
        .collect()
}

/// The chain as `slot:plugin_id` pairs, for error messages that let the
/// caller correct an address rather than guess again.
fn chain_description(entries: &[track::PluginParamsEntry]) -> String {
    entries
        .iter()
        .map(|e| format!("{}:{}", e.slot, e.plugin_id))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolve "which plugin on this bus" from the slot-or-(id, occurrence)
/// pair every chain method accepts. `verb` only shapes the wording.
fn resolve_bus_effect(
    bus: &BusState,
    slot: Option<u32>,
    plugin_id: Option<&str>,
    occurrence: Option<u32>,
    verb: &str,
) -> Result<(track::PluginParamsEntry, resonance_audio::types::PluginInstanceId), RpcError> {
    let entries = bus_plugin_entries(bus);
    let entry = match (slot, plugin_id) {
        (Some(_), Some(_)) => {
            return Err(RpcError::invalid_params(
                "address the effect by slot OR by plugin_id (+ occurrence), not both",
            ))
        }
        (None, None) => {
            return Err(RpcError::invalid_params(format!(
                "name the effect to {verb}: slot, or plugin_id (+ occurrence). Bus {} carries \
                 [{}]",
                bus.id,
                chain_description(&entries)
            )))
        }
        (Some(slot), None) => match entries.iter().find(|e| e.slot == slot) {
            Some(entry) => entry.clone(),
            None => {
                return Err(RpcError::not_found(format!(
                    "bus {} has no plugin at slot {slot}; it carries [{}]",
                    bus.id,
                    chain_description(&entries)
                )))
            }
        },
        (None, Some(wanted)) => {
            let occurrence = occurrence.unwrap_or(0);
            match entries
                .iter()
                .find(|e| e.plugin_id == wanted && e.occurrence == occurrence)
            {
                Some(entry) => entry.clone(),
                None => {
                    return Err(RpcError::not_found(format!(
                        "bus {} has no plugin {wanted:?} at occurrence {occurrence}; it carries \
                         [{}]",
                        bus.id,
                        chain_description(&entries)
                    )))
                }
            }
        }
    };
    let instance_id = bus
        .plugins
        .iter()
        .filter(|p| p.clap_plugin_id == entry.plugin_id)
        .nth(entry.occurrence as usize)
        .map(|p| p.instance_id)
        .ok_or_else(|| {
            RpcError::not_found(format!(
                "plugin {:?} vanished from bus {} between lookup and {verb}",
                entry.plugin_id, bus.id
            ))
        })?;
    Ok((entry, instance_id))
}

/// `bus.add_effect` — put an effect on the group sum.
fn add_effect(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: bus::AddEffectParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(b) = find_bus(app, params.bus_id.0) else {
        return not_found_bus(request, params.bus_id.0);
    };
    let (slot, occurrence) = (
        b.plugins.len() as u32,
        b.plugins
            .iter()
            .filter(|p| p.clap_plugin_id == params.plugin_id)
            .count() as u32,
    );

    let Some(plugin) = app
        .available_plugins
        .iter()
        .find(|p| p.clap_plugin_id == params.plugin_id)
        .cloned()
    else {
        let valid: Vec<&str> = app
            .available_plugins
            .iter()
            .filter(|p| !p.is_instrument)
            .map(|p| p.clap_plugin_id.as_str())
            .collect();
        // An empty catalog is nearly always an unbuilt checkout rather
        // than a wrong id — the first-party plugins are CLAP bundles the
        // scanner only sees once they are built (ba doc #270 §1).
        let hint = if valid.is_empty() {
            " — the catalog holds no effects at all, which usually means the first-party \
             plugins have not been built; run scripts/bundle.sh and rescan"
        } else {
            ""
        };
        return reject(
            request,
            RpcError::not_found(format!(
                "unknown plugin id {:?}; valid effect ids: {:?}{}",
                params.plugin_id, valid, hint
            )),
        );
    };
    if plugin.is_instrument {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "plugin {:?} is an instrument; a bus chain is handed the summed audio of its \
                 members and has no notes to play — put instruments on a track with \
                 track.add_instrument",
                params.plugin_id
            )),
        );
    }

    // Same synchronous commit as `track.add_effect` (todo #1234): the id
    // is allocated app-side and the slot mirrored at dispatch, so
    // `bus.plugin_params` and `bus.set_plugin_param` can address the
    // plugin in the same cycle as this reply instead of racing the
    // engine's `BusPluginAdded` echo.
    let instance_id = app.allocate_control_plugin_id();
    let task = super::run_via_update(
        app,
        Message::Bus(BusMessage::AddPluginToBusWithId {
            bus_id: params.bus_id.0,
            instance_id,
            plugin,
        }),
    );
    let result = track::AddPluginResult {
        plugin_id: params.plugin_id,
        occurrence,
        slot,
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

/// `bus.remove_effect` — take one effect off the group.
fn remove_effect(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: bus::RemoveEffectParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(b) = find_bus(app, params.bus_id.0) else {
        return not_found_bus(request, params.bus_id.0);
    };
    let instance_id = match resolve_bus_effect(
        b,
        params.slot,
        params.plugin_id.as_deref(),
        params.occurrence,
        "remove",
    ) {
        Ok((_, id)) => id,
        Err(error) => return reject(request, error),
    };
    let task = super::run_via_update(
        app,
        Message::Bus(BusMessage::RemovePluginFromBus(params.bus_id.0, instance_id)),
    );
    (ack(app, request), task)
}

/// `bus.move_effect` — reorder the group's chain.
fn move_effect(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: bus::MoveEffectParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(b) = find_bus(app, params.bus_id.0) else {
        return not_found_bus(request, params.bus_id.0);
    };
    let (entry, instance_id) = match resolve_bus_effect(
        b,
        params.slot,
        params.plugin_id.as_deref(),
        params.occurrence,
        "move",
    ) {
        Ok(found) => found,
        Err(error) => return reject(request, error),
    };
    // Clamp rather than error: naming a slot past the end means "the
    // end", and the engine clamps identically so the echo agrees.
    let to_slot = params.to_slot.min(b.plugins.len().saturating_sub(1) as u32);
    if to_slot == entry.slot {
        return (ack(app, request), Task::none());
    }
    let task = super::run_via_update(
        app,
        Message::Bus(BusMessage::MovePluginInBus {
            bus_id: params.bus_id.0,
            instance_id,
            to_index: to_slot as usize,
        }),
    );
    (ack(app, request), task)
}

/// `bus.set_fx_bypass` — SET, not toggle.
///
/// `BusMessage::ToggleBusFxBypass` flips, so dispatching it
/// unconditionally would turn a bypassed chain back ON when a client
/// retried a request whose reply it never saw. Read the mirrored state
/// and only dispatch when it actually has to change; setting the state
/// it is already in is a no-op with no undo entry.
fn set_fx_bypass(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: bus::SetFxBypassParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(b) = find_bus(app, params.bus_id.0) else {
        return not_found_bus(request, params.bus_id.0);
    };
    if b.fx_bypassed == params.bypassed {
        return (ack(app, request), Task::none());
    }
    let task = super::run_via_update(
        app,
        Message::Bus(BusMessage::ToggleBusFxBypass(params.bus_id.0)),
    );
    (ack(app, request), task)
}

/// `bus.plugin_params` — read the bus's chain, in the same shape
/// `track.plugin_params` reports a track's.
///
/// Read-only but project-requiring, so it deliberately stays OUT of
/// `is_read_only_method`: with no project open the honest answer is
/// `busy`, not an empty chain.
fn plugin_params(app: &Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: bus::PluginParamsParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(b) = find_bus(app, params.bus_id.0) else {
        return not_found_bus(request, params.bus_id.0);
    };
    let entries = bus_plugin_entries(b);
    let plugins = match &params.plugin_id {
        None => entries,
        Some(wanted) => {
            let occurrence = params.occurrence.unwrap_or(0);
            let matched: Vec<track::PluginParamsEntry> = entries
                .iter()
                .filter(|e| &e.plugin_id == wanted && e.occurrence == occurrence)
                .cloned()
                .collect();
            if matched.is_empty() {
                return reject(
                    request,
                    RpcError::not_found(format!(
                        "bus {} has no plugin {wanted:?} at occurrence {occurrence}; it carries \
                         [{}]",
                        b.id,
                        chain_description(&entries)
                    )),
                );
            }
            matched
        }
    };
    let result = bus::PluginParamsView {
        bus_id: params.bus_id,
        plugins,
        revision: app.revision(),
    };
    (super::success(request, &result), Task::none())
}

/// `bus.set_plugin_param` — configure a plugin on the group sum.
///
/// A separate method rather than an overload of
/// `track.set_plugin_param`: see [`bus::SetPluginParamParams`] for why.
fn set_plugin_param(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: bus::SetPluginParamParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if !params.value.is_finite() {
        return reject(
            request,
            RpcError::invalid_params(format!("value must be finite (got {})", params.value)),
        );
    }
    let Some(b) = find_bus(app, params.bus_id.0) else {
        return not_found_bus(request, params.bus_id.0);
    };
    let entries = bus_plugin_entries(b);
    let occurrence = params.occurrence.unwrap_or(0);
    let entry = match &params.plugin_id {
        Some(id) => entries
            .iter()
            .find(|e| &e.plugin_id == id && e.occurrence == occurrence),
        // No id names the first plugin on the chain — unambiguous on a
        // one-effect bus, which is the common case.
        None => entries.first(),
    };
    let Some(entry) = entry else {
        return reject(
            request,
            RpcError::not_found(match &params.plugin_id {
                Some(id) => format!(
                    "bus {} has no plugin {id:?} at occurrence {occurrence}; it carries [{}]",
                    b.id,
                    chain_description(&entries)
                ),
                None => format!(
                    "bus {} carries no plugins; add one with bus.add_effect",
                    b.id
                ),
            }),
        );
    };

    // Same initializing window `track.set_plugin_param` names (todo
    // #1234): the slot is mirrored at dispatch, the parameter list
    // arrives with the engine echo.
    if entry.params.is_empty() {
        return reject(
            request,
            RpcError::busy(format!(
                "plugin {:?} is on bus {} but is still initializing — its parameter list \
                 arrives with the engine echo, usually within a frame. Retry, or read \
                 bus.plugin_params until its params array is non-empty. (A plugin that \
                 genuinely exposes no parameters reports the same empty list.)",
                entry.plugin_id, b.id
            )),
        );
    }

    let wanted = params.param.trim();
    let param = entry
        .params
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(wanted))
        .or_else(|| {
            wanted
                .parse::<u32>()
                .ok()
                .and_then(|id| entry.params.iter().find(|p| p.id == id))
        });
    let Some(param) = param else {
        let known: Vec<&str> = entry.params.iter().map(|p| p.name.as_str()).collect();
        return reject(
            request,
            RpcError::not_found(format!(
                "plugin {:?} has no parameter {wanted:?} (has: [{}])",
                entry.plugin_id,
                known.join(", ")
            )),
        );
    };

    // Shared with `track.set_plugin_param` so the f32-declared-bounds
    // tolerance (todo #1235) behaves identically on both surfaces.
    let Some(value) =
        super::track::clamp_within_tolerance(params.value, param.min, param.max)
    else {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "{} must be within {}..={} (got {})",
                param.name, param.min, param.max, params.value
            )),
        );
    };

    let Some(instance_id) = b
        .plugins
        .iter()
        .filter(|p| p.clap_plugin_id == entry.plugin_id)
        .nth(entry.occurrence as usize)
        .map(|p| p.instance_id)
    else {
        return reject(
            request,
            RpcError::not_found(format!(
                "plugin {:?} vanished from bus {} between lookup and set",
                entry.plugin_id, b.id
            )),
        );
    };
    let param_id = param.id;
    let task = super::run_via_update(
        app,
        Message::Plugin(crate::message::PluginMessage::SetPluginParam(
            instance_id,
            param_id,
            value,
        )),
    );
    (ack(app, request), task)
}
