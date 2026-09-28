//! `master.*` control methods (ba doc #273, todo #1226).
//!
//! The master bus is the app's final summing stage — every track and
//! every bus lands here — and it already had a volume, an insert chain
//! and an FX bypass long before the control API could see any of it.
//! These handlers only expose what `resonance-app/src/update/master.rs`
//! and `TrackMessage::SetMasterVolume` already do.
//!
//! `master.summary` is read-only but needs an open project, so it is
//! deliberately NOT listed in
//! [`is_read_only_method`](super::is_read_only_method): the mutation
//! gate then answers a stable `busy` instead of reporting a default
//! master for a project that isn't there. For the same reason its result
//! carries no `revision`.

use crate::message::{MasterMessage, Message, TrackMessage};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::master::{
    self, AddEffectParams, MasterPluginEntry, MasterSummary, RemoveEffectParams, SetFxBypassParams,
    SetMasterVolumeParams,
};
use resonance_control::methods::mixer::{VOLUME_DB_MAX, VOLUME_DB_MIN};
use resonance_control::methods::track;
use resonance_control::{Request, Response, RpcError};

use super::chain_presets::{self, Chain};
use super::effect_addressing::{self, ChainWording};
use super::plugin_target::{self, ChainOwner};
use super::reply::{ack, reject};
use super::sidechain;
use super::view_model::master_plugin_entries;

/// Handle a `master.*` request, or `None` when `method` belongs to
/// another namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let out = match request.method.as_str() {
        master::SUMMARY => (summary(app, request), Task::none()),
        master::SET_VOLUME => set_volume(app, request),
        master::ADD_EFFECT => add_effect(app, request),
        master::REMOVE_EFFECT => remove_effect(app, request),
        master::MOVE_EFFECT => move_effect(app, request),
        master::REPLACE_EFFECT => replace_effect(app, request),
        master::SET_FX_BYPASS => set_fx_bypass(app, request),
        master::PLUGIN_PARAMS => plugin_params(app, request),
        master::SET_PLUGIN_PARAM => set_plugin_param(app, request),
        master::SET_PLUGIN_BYPASS => set_plugin_bypass(app, request),
        master::PLUGIN_PRESETS => plugin_presets(app, request),
        master::LOAD_PLUGIN_PRESET => load_plugin_preset(app, request),
        master::SAVE_PLUGIN_PRESET => save_plugin_preset(app, request),
        master::SET_SIDECHAIN => set_sidechain(app, request),
        master::CLEAR_SIDECHAIN => clear_sidechain(app, request),
        _ => return None,
    };
    Some(out)
}

/// `master.set_sidechain` — key a plugin on the master chain from a
/// track or bus (ba doc #275 P4, todo #1311).
///
/// The narrow but real mastering case: a bus compressor on the master
/// keyed from the kick, so the mix breathes with the rhythm rather than
/// with whichever transient happens to be loudest. No `master_id` — there
/// is exactly one master.
fn set_sidechain(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: master::SetSidechainParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let chain = app.master.plugins.clone();
    const HOST: &str = "the master chain";

    let source = match sidechain::resolve_key_source(
        app,
        master::SET_SIDECHAIN,
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
        HOST,
    ) {
        Ok(id) => id,
        Err(e) => return reject(request, e),
    };
    if let Err(e) = sidechain::require_key_port(&chain, instance_id, HOST) {
        return reject(request, e);
    }

    let task = super::run_via_update(
        app,
        sidechain::route_message(instance_id, Some(source), params.enabled),
    );
    (ack(app, request), task)
}

/// `master.clear_sidechain` — drop a master plugin's key route so its
/// detector goes back to the mix itself.
fn clear_sidechain(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: master::ClearSidechainParams = match super::optional_params(request) {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let chain = app.master.plugins.clone();
    let instance_id = match sidechain::resolve_chain_target(
        &chain,
        params.plugin_id.as_deref(),
        params.occurrence,
        "the master chain",
    ) {
        Ok(id) => id,
        Err(e) => return reject(request, e),
    };
    let task = super::run_via_update(app, sidechain::route_message(instance_id, None, false));
    (ack(app, request), task)
}

fn summary(app: &Resonance, request: &Request) -> Response {
    let result = MasterSummary {
        volume: crate::util::db_to_gain(app.master.volume),
        // `Resonance::master.volume` is already dB.
        volume_db: app.master.volume,
        fx_bypassed: app.master.fx_bypassed,
        plugins: app
            .master
            .plugins
            .iter()
            .enumerate()
            .map(|(slot, p)| MasterPluginEntry {
                slot: slot as u32,
                plugin_id: p.clap_plugin_id.clone(),
                name: p.plugin_name.clone(),
            })
            .collect(),
    };
    super::success(request, &result)
}

/// `master.set_volume` — the final fader, in either unit.
///
/// Routes `TrackMessage::SetMasterVolume` through the full update path,
/// so a remote change is undoable and reaches the GUI master strip like
/// a manual fader move.
fn set_volume(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetMasterVolumeParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };

    let db = match (params.volume, params.volume_db) {
        (Some(_), Some(_)) => {
            return reject(
                request,
                RpcError::invalid_params(
                    "give exactly one of volume (linear gain) or volume_db (decibels), not both",
                ),
            )
        }
        (None, None) => {
            return reject(
                request,
                RpcError::invalid_params(
                    "give exactly one of volume (linear gain) or volume_db (decibels)",
                ),
            )
        }
        (None, Some(db)) => {
            if !db.is_finite() {
                return reject(
                    request,
                    RpcError::invalid_params(format!(
                        "volume_db must be finite (got {db}); {VOLUME_DB_MIN} dB is the \
                         app's silence floor, not -inf"
                    )),
                );
            }
            db
        }
        (Some(linear), None) => {
            if !linear.is_finite() || linear < 0.0 {
                return reject(
                    request,
                    RpcError::invalid_params(format!(
                        "volume must be a non-negative linear gain (got {linear})"
                    )),
                );
            }
            // Silence maps to the app's floor rather than -inf, so the
            // fader state stays finite.
            if linear <= 0.0 {
                VOLUME_DB_MIN
            } else {
                20.0 * linear.log10()
            }
        }
    };

    if !(VOLUME_DB_MIN..=VOLUME_DB_MAX).contains(&db) {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "master level must be within {VOLUME_DB_MIN}..={VOLUME_DB_MAX} dB — the range \
                 the master fader spans (linear {:.4}..={:.4}) — got {db} dB",
                crate::util::db_to_gain(VOLUME_DB_MIN),
                crate::util::db_to_gain(VOLUME_DB_MAX),
            )),
        );
    }

    let task = super::run_via_update(app, Message::Track(TrackMessage::SetMasterVolume(db)));
    (ack(app, request), task)
}

// ---------------------------------------------------------------------------
// The master insert chain (ba doc #273, todo #1227)
// ---------------------------------------------------------------------------

/// Master's [`ChainWording`]: no id (there is exactly one master) and
/// no instrument slot — every entry [`master_plugin_entries`] builds is
/// `PluginKind::Effect` — so [`ChainWording::instrument_refusal`] never
/// actually fires; it's implemented anyway to satisfy the shared trait.
struct MasterWording;

impl ChainWording for MasterWording {
    fn no_address(&self, verb: &str, listing: &str) -> String {
        format!(
            "name the effect to {verb}: slot, or plugin_id (+ occurrence). The master chain is \
             [{listing}]"
        )
    }

    fn slot_not_found(&self, slot: u32, listing: &str) -> String {
        format!("the master chain has no slot {slot}; it carries [{listing}]")
    }

    fn id_not_found(&self, plugin_id: &str, occurrence: u32, listing: &str) -> RpcError {
        RpcError::not_found(format!(
            "the master chain has no plugin {plugin_id:?} at occurrence {occurrence}; it \
             carries [{listing}]"
        ))
    }

    fn instrument_refusal(&self, _entry: &track::PluginParamsEntry, _verb: &str) -> Option<String> {
        None
    }

    fn vanished(&self, plugin_id: &str, verb: &str) -> String {
        format!("plugin {plugin_id:?} vanished from the master chain between lookup and {verb}")
    }
}

/// `master.add_effect` — append an effect to the master chain.
///
/// This is what makes a limiter loadable at all: the mastering plugin
/// sat in the catalog with no valid home, so the only lever on overall
/// level was scaling every fader, which is capped by the loudest
/// transient.
fn add_effect(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: AddEffectParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let (slot, occurrence) = (
        app.master.plugins.len() as u32,
        app.master.plugins
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
                "plugin {:?} is an instrument; the master chain processes the summed mix and \
                 has no notes to play — put instruments on a track with track.add_instrument",
                params.plugin_id
            )),
        );
    }
    // Same synchronous commit as `track.add_effect` / `bus.add_effect`:
    // the id is allocated app-side and the slot mirrored at dispatch, so
    // `master.plugin_params` and `master.set_plugin_param` can address
    // the plugin in the same cycle as this reply instead of racing the
    // engine's `MasterPluginAdded` echo.
    let instance_id = app.allocate_plugin_id();
    let task = super::run_via_update(
        app,
        Message::Master(MasterMessage::AddPluginToMasterWithId {
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

/// `master.remove_effect` — take one plugin off the master chain,
/// addressed by slot or by (plugin_id, occurrence).
fn remove_effect(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: RemoveEffectParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };

    let entries = master_plugin_entries(app);
    let instance_id = match effect_addressing::resolve_effect(
        &entries,
        params.slot,
        params.plugin_id.as_deref(),
        params.occurrence,
        "remove",
        &MasterWording,
        |id, occurrence| effect_addressing::instance_at(&app.master.plugins, id, occurrence),
    ) {
        Ok((_, id)) => id,
        Err(error) => return reject(request, error),
    };

    let task = super::run_via_update(
        app,
        Message::Master(MasterMessage::RemovePluginFromMaster(instance_id)),
    );
    (ack(app, request), task)
}

/// `master.replace_effect` — put a different plugin in one of the
/// master chain's slots, keeping its position (ba doc #275 P5, todo
/// #1309).
///
/// Position matters more here than anywhere: the master chain's order is
/// the mastering chain, and a limiter that has to sit last must still
/// sit last after the EQ before it is swapped out.
fn replace_effect(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: master::ReplaceEffectParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let entries = master_plugin_entries(app);
    let (entry, instance_id) = match effect_addressing::resolve_effect(
        &entries,
        params.slot,
        params.plugin_id.as_deref(),
        params.occurrence,
        "replace",
        &MasterWording,
        |id, occurrence| effect_addressing::instance_at(&app.master.plugins, id, occurrence),
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

/// `master.move_effect` — reorder the master chain.
///
/// Order decides whether the chain does its job: a limiter holding a
/// ceiling has to be last, because anything after it can push the sum
/// back over that ceiling.
fn move_effect(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: master::MoveEffectParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let entries = master_plugin_entries(app);
    let (entry, instance_id) = match effect_addressing::resolve_effect(
        &entries,
        params.slot,
        params.plugin_id.as_deref(),
        params.occurrence,
        "move",
        &MasterWording,
        |id, occurrence| effect_addressing::instance_at(&app.master.plugins, id, occurrence),
    ) {
        Ok(found) => found,
        Err(error) => return reject(request, error),
    };
    // Clamp rather than error: naming a slot past the end means "the
    // end", and the engine clamps identically so the echo agrees.
    let to_slot = params
        .to_slot
        .min(app.master.plugins.len().saturating_sub(1) as u32);
    if to_slot == entry.slot {
        return (ack(app, request), Task::none());
    }
    let task = super::run_via_update(
        app,
        Message::Master(MasterMessage::MovePluginInMaster {
            instance_id,
            to_index: to_slot as usize,
        }),
    );
    (ack(app, request), task)
}

/// `master.plugin_params` — read the master chain, in the same shape
/// `track.plugin_params` reports a track's.
///
/// Read-only but project-requiring, so like `master.summary` it stays
/// OUT of `is_read_only_method`: with no project open the honest answer
/// is `busy`, not an empty chain.
fn plugin_params(app: &Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: master::PluginParamsParams = match super::optional_params(request) {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let entries = master_plugin_entries(app);
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
                        "the master chain has no plugin {wanted:?} at occurrence {occurrence}; \
                         it carries [{}]",
                        effect_addressing::chain_description(&entries)
                    )),
                );
            }
            matched
        }
    };
    let result = master::PluginParamsView {
        plugins,
        revision: app.revision(),
    };
    (super::success(request, &result), Task::none())
}

/// `master.set_plugin_param` — configure a plugin on the finished mix.
///
/// Without this the master chain is write-only: `master.add_effect`
/// loads a limiter and it then sits at its defaults, which on a mix
/// left at normal mixing headroom means it never engages at all.
fn set_plugin_param(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: master::SetPluginParamParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    // The addressing every chain shares (plugin_target.rs): no id names
    // the first plugin on the chain — unambiguous on a one-effect chain,
    // the common case — and a plugin whose parameter list has not arrived
    // yet answers `busy` (todo #1234).
    let (target, param) = match plugin_target::resolve_plugin_param(
        app,
        ChainOwner::Master,
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
    let value = match super::track::resolve_param_value(&param, &params.value) {
        Ok(value) => value,
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

/// `master.set_fx_bypass` — SET, not toggle, so a client that lost track
/// of the current value cannot flip it the wrong way. Setting the state
/// it is already in is a no-op with no undo entry.
fn set_fx_bypass(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetFxBypassParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if app.master.fx_bypassed == params.bypassed {
        return (ack(app, request), Task::none());
    }
    let task = super::run_via_update(app, Message::Master(MasterMessage::ToggleMasterFxBypass));
    (ack(app, request), task)
}

// ---------------------------------------------------------------------------
// Plugin presets on the master chain (ba todo #1333)
// ---------------------------------------------------------------------------
//
// Thin parameter-shaped wrappers: the master chain addresses a plugin
// exactly as a bus does, so the handlers themselves live in
// [`super::chain_presets`], and the preset machinery under those is
// [`super::plugin_presets`], shared with the track surface — so a preset
// saved off the master limiter is the same file the plugin's own window
// lists, and the same one a track could recall.

fn plugin_presets(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    // Every field is optional — a bare call means "the first plugin on
    // the chain" — so an absent params object is legal, as it is for
    // `master.plugin_params`.
    let params: master::PluginPresetsParams = match super::optional_params(request) {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    chain_presets::view(
        app,
        request,
        Chain::Master,
        &params.plugin_id,
        params.occurrence,
    )
}

fn load_plugin_preset(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: master::LoadPluginPresetParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    chain_presets::load(
        app,
        request,
        Chain::Master,
        &params.plugin_id,
        params.occurrence,
        &params.preset,
        params.source,
    )
}

fn save_plugin_preset(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: master::SavePluginPresetParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    chain_presets::save(
        app,
        request,
        Chain::Master,
        &params.plugin_id,
        params.occurrence,
        &params.name,
        params.overwrite,
    )
}

/// `master.set_plugin_bypass` — one slot on the master chain.
fn set_plugin_bypass(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: master::SetPluginBypassParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let chain = app.master.plugins.clone();
    super::bypass::run(
        app,
        request,
        &chain,
        params.plugin_id.as_deref(),
        params.occurrence,
        params.bypassed,
        super::bypass::first_slot,
        "the master chain",
    )
}
