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
use resonance_control::{MutationAck, Request, Response, RpcError};

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
        master::SET_FX_BYPASS => set_fx_bypass(app, request),
        _ => return None,
    };
    Some(out)
}

fn summary(app: &Resonance, request: &Request) -> Response {
    let result = MasterSummary {
        volume: crate::util::db_to_gain(app.master_volume),
        // `Resonance::master_volume` is already dB.
        volume_db: app.master_volume,
        fx_bypassed: app.master_fx_bypassed,
        plugins: app
            .master_plugins
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
                "plugin {:?} is an instrument; the master chain processes the summed mix and \
                 has no notes to play — put instruments on a track with track.add_instrument",
                params.plugin_id
            )),
        );
    }
    let task = super::run_via_update(
        app,
        Message::Master(MasterMessage::AddPluginToMaster(plugin)),
    );
    (ack(app, request), task)
}

/// `master.remove_effect` — take one plugin off the master chain,
/// addressed by slot or by (plugin_id, occurrence).
fn remove_effect(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: RemoveEffectParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };

    let instance_id = match (params.slot, &params.plugin_id) {
        (Some(_), Some(_)) => {
            return reject(
                request,
                RpcError::invalid_params(
                    "address the effect by slot OR by plugin_id (+ occurrence), not both",
                ),
            )
        }
        (None, None) => {
            return reject(
                request,
                RpcError::invalid_params(format!(
                    "name the effect to remove: slot, or plugin_id (+ occurrence). The master \
                     chain is [{}]",
                    chain_description(app)
                )),
            )
        }
        (Some(slot), None) => {
            let Some(p) = app.master_plugins.get(slot as usize) else {
                return reject(
                    request,
                    RpcError::not_found(format!(
                        "the master chain has no slot {slot}; it is [{}]",
                        chain_description(app)
                    )),
                );
            };
            p.instance_id
        }
        (None, Some(plugin_id)) => {
            let occurrence = params.occurrence.unwrap_or(0);
            let Some(p) = app
                .master_plugins
                .iter()
                .filter(|p| &p.clap_plugin_id == plugin_id)
                .nth(occurrence as usize)
            else {
                return reject(
                    request,
                    RpcError::not_found(format!(
                        "the master chain has no plugin {plugin_id:?} at occurrence \
                         {occurrence}; it is [{}]",
                        chain_description(app)
                    )),
                );
            };
            p.instance_id
        }
    };

    let task = super::run_via_update(
        app,
        Message::Master(MasterMessage::RemovePluginFromMaster(instance_id)),
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
    if app.master_fx_bypassed == params.bypassed {
        return (ack(app, request), Task::none());
    }
    let task = super::run_via_update(app, Message::Master(MasterMessage::ToggleMasterFxBypass));
    (ack(app, request), task)
}

/// The master chain as `slot:plugin_id` pairs, for error messages that
/// let the caller correct an id rather than guess.
fn chain_description(app: &Resonance) -> String {
    app.master_plugins
        .iter()
        .enumerate()
        .map(|(slot, p)| format!("{slot}:{}", p.clap_plugin_id))
        .collect::<Vec<_>>()
        .join(", ")
}

fn ack(app: &Resonance, request: &Request) -> Response {
    super::success(request, &MutationAck { revision: app.revision() })
}

fn reject(request: &Request, error: RpcError) -> (Response, Task<Message>) {
    (super::failure(request, error), Task::none())
}
