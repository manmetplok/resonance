//! `mixer.*` — the per-track fader, pan, mute and solo.

use super::{ack, find_track, not_found_track, reject};
use crate::message::{Message, TrackMessage};
use crate::update::control::run_via_update;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::mixer::{
    self, SetMuteParams, SetPanParams, SetSoloParams, SetVolumeDbParams, SetVolumeParams,
};
use resonance_control::{Request, Response, RpcError};

pub(super) fn set_volume(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetVolumeParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_track(app, params.track_id.0).is_none() {
        return not_found_track(request, params.track_id.0);
    }
    if !params.volume.is_finite() || params.volume < 0.0 {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "volume must be a non-negative linear gain (got {})",
                params.volume
            )),
        );
    }
    // Wire is linear gain; the app stores/sends dB. Silence maps to the
    // app's floor rather than -inf so the fader state stays finite.
    let db = if params.volume <= 0.0 {
        mixer::VOLUME_DB_MIN
    } else {
        20.0 * params.volume.log10()
    };
    // `powf`/`log10` are not exact inverses in f32: the linear image of
    // +6 dB converts back as 6.000001 dB, and 0.001 as -60.000004 dB.
    // Snap that conversion noise onto the bound so the documented
    // endpoints are reachable from the linear form too. 1e-3 dB is a
    // gain factor of ~1.0001 — conversion noise, not a level.
    const DB_EPS: f32 = 1e-3;
    let db = if (db - mixer::VOLUME_DB_MAX).abs() <= DB_EPS {
        mixer::VOLUME_DB_MAX
    } else if (db - mixer::VOLUME_DB_MIN).abs() <= DB_EPS {
        mixer::VOLUME_DB_MIN
    } else {
        db
    };
    // The same fader as `set_volume_db`, so the same effective range:
    // out-of-range is rejected rather than clamped (see below).
    if !(mixer::VOLUME_DB_MIN..=mixer::VOLUME_DB_MAX).contains(&db) {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "volume must be within {}..={} dB — the range the mixer fader spans \
                 (linear {:.4}..={:.4}; 0 is the {} dB silence floor) — got {} ({} dB)",
                mixer::VOLUME_DB_MIN,
                mixer::VOLUME_DB_MAX,
                crate::util::db_to_gain(mixer::VOLUME_DB_MIN),
                crate::util::db_to_gain(mixer::VOLUME_DB_MAX),
                mixer::VOLUME_DB_MIN,
                params.volume,
                db
            )),
        );
    }
    let task = run_via_update(
        app,
        Message::Track(TrackMessage::SetTrackVolume(params.track_id.0, db)),
    );
    (ack(app, request), task)
}

/// `mixer.set_volume_db` — the same fader as [`set_volume`], in dB (ba
/// doc #273).
///
/// `TrackState.volume` is already stored in dB, so this dispatches the
/// caller's value verbatim: no `log10` on the way in, no `powf` on the
/// way out. Out-of-range values are rejected rather than clamped —
/// silently moving a level the caller asked for is how a mix ends up
/// subtly wrong with nothing to point at.
pub(super) fn set_volume_db(
    app: &mut Resonance,
    request: &Request,
) -> (Response, Task<Message>) {
    let params: SetVolumeDbParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_track(app, params.track_id.0).is_none() {
        return not_found_track(request, params.track_id.0);
    }
    if !params.volume_db.is_finite() {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "volume_db must be finite (got {}); the app's silence floor is {} dB, \
                 not -inf",
                params.volume_db,
                mixer::VOLUME_DB_MIN
            )),
        );
    }
    if !(mixer::VOLUME_DB_MIN..=mixer::VOLUME_DB_MAX).contains(&params.volume_db) {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "volume_db must be within {}..={} dB — the range the mixer fader spans \
                 ({} dB is silence, 0 dB is unity) — got {}",
                mixer::VOLUME_DB_MIN,
                mixer::VOLUME_DB_MAX,
                mixer::VOLUME_DB_MIN,
                params.volume_db
            )),
        );
    }
    let task = run_via_update(
        app,
        Message::Track(TrackMessage::SetTrackVolume(
            params.track_id.0,
            params.volume_db,
        )),
    );
    (ack(app, request), task)
}

pub(super) fn set_pan(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetPanParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if find_track(app, params.track_id.0).is_none() {
        return not_found_track(request, params.track_id.0);
    }
    if !params.pan.is_finite() || !(-1.0..=1.0).contains(&params.pan) {
        return reject(
            request,
            RpcError::invalid_params(format!("pan must be in -1.0..=1.0 (got {})", params.pan)),
        );
    }
    let task = run_via_update(
        app,
        Message::Track(TrackMessage::SetTrackPan(params.track_id.0, params.pan)),
    );
    (ack(app, request), task)
}

pub(super) fn set_mute(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetMuteParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(track) = find_track(app, params.track_id.0) else {
        return not_found_track(request, params.track_id.0);
    };
    // The GUI has only a toggle; only dispatch when the state actually
    // needs to flip, so an idempotent set never records a no-op undo
    // entry or double-toggles.
    if track.muted == params.muted {
        return (ack(app, request), Task::none());
    }
    let task = run_via_update(
        app,
        Message::Track(TrackMessage::ToggleMute(params.track_id.0)),
    );
    (ack(app, request), task)
}

pub(super) fn set_solo(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetSoloParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(track) = find_track(app, params.track_id.0) else {
        return not_found_track(request, params.track_id.0);
    };
    if track.soloed == params.soloed {
        return (ack(app, request), Task::none());
    }
    let task = run_via_update(
        app,
        Message::Track(TrackMessage::ToggleSolo(params.track_id.0)),
    );
    (ack(app, request), task)
}
