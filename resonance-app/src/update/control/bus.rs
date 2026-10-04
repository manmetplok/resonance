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
//! `bus.create` allocates the id APP-SIDE (ARCH-04 D-3: the app is the
//! only bus-id allocator, so every `AddBus` — including the plain GUI
//! one — carries a concrete id now) so the reply can return `bus_id`
//! immediately rather than waiting on the engine's asynchronous
//! `BusAdded` echo.

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

use super::chain_presets::{self, Chain};
use super::effect_addressing::{self, ChainWording};
use super::plugin_target::{self, ChainOwner};
use super::reply::{ack, not_found_bus, reject};
use super::sidechain;
use super::view_model::bus_plugin_entries;

/// Handle a `bus.*` request, or `None` when `method` belongs to another
/// namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let out = match request.method.as_str() {
        bus::CREATE => create(app, request),
        bus::DELETE => delete(app, request),
        bus::RENAME => rename(app, request),
        bus::SET_VOLUME => set_volume(app, request),
        bus::ADD_EFFECT => add_effect(app, request),
        bus::REMOVE_EFFECT => remove_effect(app, request),
        bus::MOVE_EFFECT => move_effect(app, request),
        bus::REPLACE_EFFECT => replace_effect(app, request),
        bus::SET_FX_BYPASS => set_fx_bypass(app, request),
        bus::PLUGIN_PARAMS => plugin_params(app, request),
        bus::SET_PLUGIN_PARAM => set_plugin_param(app, request),
        bus::SET_PLUGIN_BYPASS => set_plugin_bypass(app, request),
        bus::PLUGIN_PRESETS => plugin_presets(app, request),
        bus::LOAD_PLUGIN_PRESET => load_plugin_preset(app, request),
        bus::SAVE_PLUGIN_PRESET => save_plugin_preset(app, request),
        bus::SET_SIDECHAIN => set_sidechain(app, request),
        bus::CLEAR_SIDECHAIN => clear_sidechain(app, request),
        _ => return None,
    };
    Some(out)
}

/// `bus.set_sidechain` — key a plugin on this bus's chain from another
/// track or bus (ba doc #275 P4, todo #1311).
///
/// The bus chain is where a keyed ducker usually belongs: the point of
/// ducking is to move a whole group out of the way of one hit, and the
/// group only exists at the bus. Until this method existed the control
/// API could not express that at all — `track.set_sidechain` is keyed on
/// `track_id` — even though the engine's route table has always been
/// keyed by plugin instance and would have accepted it.
fn set_sidechain(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: bus::SetSidechainParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(b) = find_bus(app, params.bus_id.0) else {
        return not_found_bus(request, params.bus_id.0);
    };
    let chain = b.plugins.clone();
    let host = format!("bus {}", params.bus_id.0);

    let source = match sidechain::resolve_key_source(
        app,
        bus::SET_SIDECHAIN,
        params.source_track_id,
        params.source_bus_id,
    ) {
        Ok(source) => source,
        Err(e) => return reject(request, e),
    };
    let instance_id = match sidechain::resolve_chain_target(
        &chain,
        params.plugin_id.as_deref(),
        params.occurrence,
        &host,
    ) {
        Ok(id) => id,
        Err(e) => return reject(request, e),
    };
    if let Err(e) = sidechain::require_key_port(&chain, instance_id, &host) {
        return reject(request, e);
    }

    let task = super::run_via_update(
        app,
        sidechain::route_message(instance_id, Some(source), params.enabled),
    );
    (ack(app, request), task)
}

/// `bus.clear_sidechain` — drop a bus plugin's key route so its detector
/// goes back to the bus's own signal.
fn clear_sidechain(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: bus::ClearSidechainParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(b) = find_bus(app, params.bus_id.0) else {
        return not_found_bus(request, params.bus_id.0);
    };
    let chain = b.plugins.clone();
    let instance_id = match sidechain::resolve_chain_clear_target(
        app,
        &chain,
        params.plugin_id.as_deref(),
        params.occurrence,
        &format!("bus {}", params.bus_id.0),
    ) {
        Ok(id) => id,
        Err(e) => return reject(request, e),
    };
    let task = super::run_via_update(app, sidechain::route_message(instance_id, None, false));
    (ack(app, request), task)
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
        // Smallest free "Bus N": len()+1 collides after a deletion (delete
        // "Bus 1" of two, create → a second "Bus 2").
        None => {
            let taken: Vec<&str> = app.registry.busses.iter().map(|b| b.name.as_str()).collect();
            (1..)
                .map(|n| format!("Bus {n}"))
                .find(|candidate| !taken.iter().any(|t| t == candidate))
                .expect("unbounded counter always finds a free name")
        }
    };

    // App-side id so the reply can return it immediately (ARCH-04 D-3: the
    // app is the only bus-id allocator left, GUI adds included).
    let bus_id = app.registry.allocate_bus_id();
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

/// `bus.rename` — the bus twin of `track.rename`: same refusals (an
/// unknown id is `not_found`, an empty name `invalid_params`), and the
/// same message the strip / inspector inline rename sends, so it is one
/// undo step. The name is trimmed; renaming a bus to the name it already
/// has is acknowledged and changes nothing (no undo step, no dirty).
fn rename(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: bus::RenameParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(current) = find_bus(app, params.bus_id.0).map(|b| b.name.clone()) else {
        return not_found_bus(request, params.bus_id.0);
    };
    let name = params.name.trim();
    if name.is_empty() {
        return reject(request, RpcError::invalid_params("bus name must not be empty"));
    }
    if name == current {
        return (ack(app, request), Task::none());
    }
    let task = super::run_via_update(
        app,
        Message::Bus(BusMessage::RenameBus(params.bus_id.0, name.to_owned())),
    );
    (ack(app, request), task)
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

/// Bus's [`ChainWording`]: a bus chain has no instrument slot — every
/// entry [`bus_plugin_entries`] builds is `PluginKind::Effect` — so
/// [`ChainWording::instrument_refusal`] never actually fires; it's
/// implemented anyway to satisfy the shared trait.
struct BusWording<'a> {
    bus: &'a BusState,
}

impl ChainWording for BusWording<'_> {
    fn no_address(&self, verb: &str, listing: &str) -> String {
        format!(
            "name the effect to {verb}: slot, or plugin_id (+ occurrence). Bus {} carries \
             [{listing}]",
            self.bus.id
        )
    }

    fn slot_not_found(&self, slot: u32, listing: &str) -> String {
        format!(
            "bus {} has no plugin at slot {slot}; it carries [{listing}]",
            self.bus.id
        )
    }

    fn id_not_found(&self, plugin_id: &str, occurrence: u32, listing: &str) -> RpcError {
        RpcError::not_found(format!(
            "bus {} has no plugin {plugin_id:?} at occurrence {occurrence}; it carries \
             [{listing}]",
            self.bus.id
        ))
    }

    fn instrument_refusal(&self, _entry: &track::PluginParamsEntry, _verb: &str) -> Option<String> {
        None
    }

    fn vanished(&self, plugin_id: &str, verb: &str) -> String {
        format!(
            "plugin {plugin_id:?} vanished from bus {} between lookup and {verb}",
            self.bus.id
        )
    }
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
        .plugin_catalog
        .available_plugins
        .iter()
        .find(|p| p.clap_plugin_id == params.plugin_id)
        .cloned()
    else {
        let valid: Vec<&str> = app
            .plugin_catalog
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

    // A preset to load onto it, checked before anything is added.
    let preset = match params.preset.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => match crate::update::control::plugin_presets::resolve_add_preset(
            app,
            &params.plugin_id,
            p,
        ) {
            Ok(found) => Some(found),
            Err(e) => return reject(request, e),
        },
        None => None,
    };

    // Same synchronous commit as `track.add_effect` (todo #1234): the id
    // is allocated app-side and the slot mirrored at dispatch, so
    // `bus.plugin_params` and `bus.set_plugin_param` can address the
    // plugin in the same cycle as this reply instead of racing the
    // engine's `PluginAdded` echo.
    let instance_id = app.allocate_plugin_id();
    if let Some(found) = &preset {
        crate::update::control::plugin_presets::park_add_preset(
            app,
            instance_id,
            &params.plugin_id,
            found,
        );
    }
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
    let entries = bus_plugin_entries(b);
    let instance_id = match effect_addressing::resolve_effect(
        &entries,
        params.slot,
        params.plugin_id.as_deref(),
        params.occurrence,
        "remove",
        &BusWording { bus: b },
        |id, occurrence| effect_addressing::instance_at(&b.plugins, id, occurrence),
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

/// `bus.replace_effect` — put a different plugin in one of the group's
/// chain slots, keeping its position (ba doc #275 P5, todo #1309).
///
/// Addressing runs through the shared resolver
/// ([`effect_addressing::resolve_effect`]) with this surface's own
/// [`BusWording`]; everything after the slot is resolved is shared with
/// the track and master surfaces.
fn replace_effect(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: bus::ReplaceEffectParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(b) = find_bus(app, params.bus_id.0) else {
        return not_found_bus(request, params.bus_id.0);
    };
    let entries = bus_plugin_entries(b);
    let (entry, instance_id) = match effect_addressing::resolve_effect(
        &entries,
        params.slot,
        params.plugin_id.as_deref(),
        params.occurrence,
        "replace",
        &BusWording { bus: b },
        |id, occurrence| effect_addressing::instance_at(&b.plugins, id, occurrence),
    ) {
        Ok(found) => found,
        Err(error) => return reject(request, error),
    };
    super::replace::replace_resolved_slot(
        app,
        request,
        instance_id,
        entry.slot,
        &params.new_plugin_id,
    )
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
    let entries = bus_plugin_entries(b);
    let (entry, instance_id) = match effect_addressing::resolve_effect(
        &entries,
        params.slot,
        params.plugin_id.as_deref(),
        params.occurrence,
        "move",
        &BusWording { bus: b },
        |id, occurrence| effect_addressing::instance_at(&b.plugins, id, occurrence),
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
/// `bypasses_mutation_gate`: with no project open the honest answer is
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
                        effect_addressing::chain_description(&entries)
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
    // The addressing every chain shares (plugin_target.rs): no id names
    // the first plugin on the chain — unambiguous on a one-effect chain,
    // the common case — and a plugin whose parameter list has not arrived
    // yet answers `busy` (todo #1234).
    let (target, param) = match plugin_target::resolve_plugin_param(
        app,
        ChainOwner::Bus(params.bus_id.0),
        params.plugin_id.as_deref(),
        params.occurrence,
        &params.param,
    ) {
        Ok(found) => found,
        Err(e) => return reject(request, e),
    };

    // Shared with `track.set_plugin_param` so the f32-declared-bounds
    // tolerance (todo #1235) and choice-label resolution (todo #1290)
    // behave identically on all three chains.
    let value = match super::track::resolve_param_value_on(
        app,
        request,
        &target,
        &param,
        &params.value,
    ) {
        Ok(super::track::ParamValueOutcome::Value(value)) => value,
        Ok(super::track::ParamValueOutcome::Deferred) => return super::track::deferred_reply(request),
        Err(e) => return reject(request, e),
    };
    let instance_id = target.instance_id;
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

// ---------------------------------------------------------------------------
// Plugin presets on a bus chain (ba todo #1333)
// ---------------------------------------------------------------------------
//
// Thin parameter-shaped wrappers: a bus addresses a plugin exactly as the
// master chain does, so the handlers themselves live in
// [`super::chain_presets`], and the preset machinery under those is
// [`super::plugin_presets`], shared with the track surface — which is why
// a preset saved here is the same file the plugin's own window lists.

fn plugin_presets(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: bus::PluginPresetsParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    chain_presets::view(
        app,
        request,
        Chain::Bus(params.bus_id.0),
        &params.plugin_id,
        params.occurrence,
        &params.filter,
    )
}

fn load_plugin_preset(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: bus::LoadPluginPresetParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    chain_presets::load(
        app,
        request,
        Chain::Bus(params.bus_id.0),
        &params.plugin_id,
        params.occurrence,
        super::plugin_presets::LoadArgs {
            preset: &params.preset,
            preset_id: params.preset_id.as_deref(),
            source: params.source,
            extra: params.extra.unwrap_or(true),
        },
    )
}

fn save_plugin_preset(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: bus::SavePluginPresetParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    chain_presets::save(
        app,
        request,
        Chain::Bus(params.bus_id.0),
        &params.plugin_id,
        params.occurrence,
        super::plugin_presets::SaveArgs {
            name: params.name,
            overwrite: params.overwrite,
            meta: params.meta,
            favorite: params.favorite,
            overwrite_id: params.overwrite_id,
        },
    )
}

/// `bus.set_plugin_bypass` — one slot on a bus's chain.
fn set_plugin_bypass(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: bus::SetPluginBypassParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(b) = find_bus(app, params.bus_id.0) else {
        return not_found_bus(request, params.bus_id.0);
    };
    let host = format!("bus {}", b.id);
    let chain = b.plugins.clone();
    super::bypass::run(
        app,
        request,
        &chain,
        params.plugin_id.as_deref(),
        params.occurrence,
        params.bypassed,
        super::bypass::first_slot,
        &host,
    )
}
