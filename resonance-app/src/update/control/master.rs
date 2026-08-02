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

use crate::message::{Message, TrackMessage};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::master::{
    self, MasterPluginEntry, MasterSummary, SetMasterVolumeParams,
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
    (
        super::success(request, &MutationAck { revision: app.revision() }),
        task,
    )
}

fn reject(request: &Request, error: RpcError) -> (Response, Task<Message>) {
    (super::failure(request, error), Task::none())
}
