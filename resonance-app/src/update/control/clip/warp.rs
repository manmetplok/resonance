//! `clip.set_warp` / `clip.set_warp_markers` / `clip.detect_tempo` — the
//! wire half of the clip inspector's warp section.
//!
//! Each mutation runs the same `ClipWarpMessage` the GUI sends, through
//! `update`, so it is one undo entry and one revision, and the mirror is
//! updated before the reply (read-your-own-writes). `clip.detect_tempo` is
//! a job ([`JobToken::DetectTempo`]): the engine replies with
//! `ClipTempoDetected` after the request has been answered, and
//! `engine_events::clips::tempo_detected` resolves it.

use iced::Task;
use resonance_audio::types::{WarpAlgorithm as EngineAlgorithm, WarpMarker};
use resonance_control::methods::clip::{
    self as proto, DetectTempoParams, DetectTempoResult, SetWarpMarkersParams, SetWarpParams,
    WarpAlgorithm, WarpMarkerSpec, WarpResult,
};
use resonance_control::{Request, Response, RpcError};

use super::super::reply::reject;
use crate::control_jobs::JobToken;
use crate::control_socket::ConnId;
use crate::message::{ClipMessage, ClipWarpMessage, Message};
use crate::Resonance;

fn engine_algorithm(algorithm: WarpAlgorithm) -> Option<EngineAlgorithm> {
    match algorithm {
        WarpAlgorithm::Transient => Some(EngineAlgorithm::Transient),
        WarpAlgorithm::Tonal => Some(EngineAlgorithm::Tonal),
        WarpAlgorithm::Unknown => None,
    }
}

fn wire_algorithm(algorithm: EngineAlgorithm) -> WarpAlgorithm {
    match algorithm {
        EngineAlgorithm::Transient => WarpAlgorithm::Transient,
        EngineAlgorithm::Tonal => WarpAlgorithm::Tonal,
    }
}

/// The clip's warp state as the mirror holds it now.
fn warp_result(app: &Resonance, clip_id: u64) -> WarpResult {
    let warp = app
        .clips
        .iter()
        .find(|c| c.id == clip_id)
        .map(|c| c.warp.clone())
        .unwrap_or_default();
    WarpResult {
        clip_id: resonance_control::ids::ClipId(clip_id),
        enabled: warp.enabled,
        original_bpm: warp.original_bpm,
        transpose_semitones: warp.transpose_semitones,
        algorithm: wire_algorithm(warp.algorithm),
        markers: warp
            .markers
            .iter()
            .map(|m| WarpMarkerSpec {
                source_frame: m.source_frame,
                beat: m.timeline_beat,
            })
            .collect(),
        revision: app.revision(),
    }
}

pub(in crate::update::control) fn set_warp(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: SetWarpParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let clip_id = params.clip_id.0;
    let Some(clip) = app.clips.iter().find(|c| c.id == clip_id) else {
        return super::clip_not_found(app, request, clip_id);
    };
    if params.enabled.is_none()
        && params.original_bpm.is_none()
        && !params.clear_original_bpm
        && params.transpose_semitones.is_none()
        && params.algorithm.is_none()
    {
        return reject(request, RpcError::invalid_params("clip.set_warp changed nothing"));
    }
    if params.clear_original_bpm && params.original_bpm.is_some() {
        return reject(
            request,
            RpcError::invalid_params("give original_bpm or clear_original_bpm, not both"),
        );
    }
    let mut warp = clip.warp.clone();
    if let Some(bpm) = params.original_bpm {
        if !bpm.is_finite() || !(proto::MIN_WARP_BPM..=proto::MAX_WARP_BPM).contains(&bpm) {
            return reject(
                request,
                RpcError::invalid_params(format!(
                    "original_bpm must be between {} and {} BPM",
                    proto::MIN_WARP_BPM,
                    proto::MAX_WARP_BPM
                )),
            );
        }
        warp.original_bpm = Some(bpm);
    }
    if params.clear_original_bpm {
        warp.original_bpm = None;
    }
    if let Some(semitones) = params.transpose_semitones {
        if !semitones.is_finite() || semitones.abs() > proto::MAX_TRANSPOSE_SEMITONES {
            return reject(
                request,
                RpcError::invalid_params(format!(
                    "transpose_semitones must be within ±{}",
                    proto::MAX_TRANSPOSE_SEMITONES
                )),
            );
        }
        warp.transpose_semitones = semitones;
    }
    if let Some(algorithm) = params.algorithm {
        match engine_algorithm(algorithm) {
            Some(a) => warp.algorithm = a,
            None => {
                return reject(
                    request,
                    RpcError::invalid_params("unknown warp algorithm; use transient or tonal"),
                )
            }
        }
    }
    if let Some(enabled) = params.enabled {
        warp.enabled = enabled;
    }
    let task = super::super::run_via_update(
        app,
        Message::Clip(ClipMessage::Warp(ClipWarpMessage::SetWarp {
            clip_id,
            enabled: warp.enabled,
            original_bpm: warp.original_bpm,
            transpose_semitones: warp.transpose_semitones,
            algorithm: warp.algorithm,
        })),
    );
    let result = warp_result(app, clip_id);
    (super::super::success(request, &result), task)
}

pub(in crate::update::control) fn set_warp_markers(
    app: &mut Resonance,
    request: &Request,
) -> (Response, Task<Message>) {
    let params: SetWarpMarkersParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let clip_id = params.clip_id.0;
    let Some(clip) = app.clips.iter().find(|c| c.id == clip_id) else {
        return super::clip_not_found(app, request, clip_id);
    };
    if params.markers.len() > proto::MAX_WARP_MARKERS {
        return reject(
            request,
            RpcError::invalid_params(format!(
                "at most {} warp markers per clip",
                proto::MAX_WARP_MARKERS
            )),
        );
    }
    let total_frames = clip.total_frames;
    let mut markers: Vec<WarpMarker> = Vec::with_capacity(params.markers.len());
    for (i, m) in params.markers.iter().enumerate() {
        if !m.beat.is_finite() || m.beat < 0.0 {
            return reject(
                request,
                RpcError::invalid_params(format!("markers[{i}].beat must be a number >= 0")),
            );
        }
        if total_frames > 0 && m.source_frame > total_frames {
            return reject(
                request,
                RpcError::invalid_params(format!(
                    "markers[{i}].source_frame {} is past the end of the clip's source \
                     ({total_frames} frames)",
                    m.source_frame
                )),
            );
        }
        markers.push(WarpMarker {
            source_frame: m.source_frame,
            timeline_beat: m.beat,
        });
    }
    crate::state::sort_warp_markers(&mut markers);
    for pair in markers.windows(2) {
        if pair[1].timeline_beat - pair[0].timeline_beat < crate::state::MIN_WARP_MARKER_GAP_BEATS {
            return reject(
                request,
                RpcError::invalid_params(format!(
                    "two markers at beats {} and {} are closer than 1/16 beat",
                    pair[0].timeline_beat, pair[1].timeline_beat
                )),
            );
        }
        if pair[1].source_frame < pair[0].source_frame {
            return reject(
                request,
                RpcError::invalid_params(format!(
                    "markers run the source backwards (beat {} plays frame {}, beat {} plays \
                     frame {}); source_frame must not decrease as beat increases",
                    pair[0].timeline_beat,
                    pair[0].source_frame,
                    pair[1].timeline_beat,
                    pair[1].source_frame
                )),
            );
        }
    }
    let task = super::super::run_via_update(
        app,
        Message::Clip(ClipMessage::Warp(ClipWarpMessage::SetWarpMarkers {
            clip_id,
            markers,
        })),
    );
    let result = warp_result(app, clip_id);
    (super::super::success(request, &result), task)
}

pub(in crate::update::control) fn detect_tempo(
    app: &mut Resonance,
    conn: ConnId,
    request: &Request,
) -> (Response, Task<Message>) {
    let params: DetectTempoParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let clip_id = params.clip_id.0;
    let Some(clip) = app.clips.iter().find(|c| c.id == clip_id) else {
        return super::clip_not_found(app, request, clip_id);
    };
    let name = clip.name.clone();
    let started = app.start_control_job(
        proto::DETECT_TEMPO,
        &format!("Detect the tempo of clip {clip_id} ({name:?})"),
        JobToken::DetectTempo { clip_id },
        Some(conn),
    );
    // The same path as the inspector's Detect button, so the GUI shows
    // the result too.
    crate::update::clip_warp::detect_tempo(app, clip_id);
    if !app.ui.interaction.tempo_detect.contains_key(&clip_id) {
        app.control
            .jobs
            .fail(u64::from(started.job_id), "tempo detection did not start (engine unavailable)");
    }
    (super::super::success(request, &started), Task::none())
}

/// Resolve the `clip.detect_tempo` job waiting on `clip_id`, if any, from
/// the engine's `ClipTempoDetected` (called by `engine_events::clips`).
/// `bpm == 0` is the detector's "no tempo found", which fails the job.
pub(crate) fn tempo_detected(app: &Resonance, clip_id: u64, bpm: f32, confidence: f32) {
    let token = JobToken::DetectTempo { clip_id };
    if bpm.is_finite() && bpm > 0.0 {
        let result = DetectTempoResult {
            clip_id: resonance_control::ids::ClipId(clip_id),
            bpm,
            confidence: confidence.clamp(0.0, 1.0),
        };
        let value = serde_json::to_value(result).unwrap_or_default();
        app.control.jobs.complete_token(&token, value);
    } else {
        app.control.jobs.fail_token(
            &token,
            format!("no tempo found in clip {clip_id} (too short, or no steady pulse)"),
        );
    }
}
