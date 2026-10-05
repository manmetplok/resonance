//! `master.assist` — the mastering assistant, offline and over the wire
//! (warmth-width-depth.md §7.4).
//!
//! The plugin's assistant panel captures 10 s of the live master and
//! compares it against a genre band or a reference track. This is the
//! same analysis and the same decision engine
//! (`resonance_mastering_assist::decide`), run on an OFFLINE render of
//! the master range instead of a live capture, and answered with the
//! suggestions — stage by stage, each with its rationale and the exact
//! mastering-plugin param writes — WITHOUT applying any of them. The agent
//! applies what it agrees with through `master.set_plugin_param`.
//!
//! # Shape
//!
//! A job, like `meter.measure`: the handler validates, registers a
//! [`JobToken::Measure`] under [`master::ASSIST`], and asks the engine
//! for the master render with the assistant's LTAS
//! ([`DetailSet::assist`]). In reference mode it first asks for the
//! pooled asset measured as decoded audio ([`AudioCommand::MeasureAudio`],
//! which reads the file the way a placed clip does), and sends the master
//! render only once that has succeeded — so the job never has more than
//! one engine command outstanding, and a failed decode cannot end the job
//! (releasing the offline-render guard) while a render still runs.
//! [`measured`] parks the reference in [`PendingAssist`] and builds the
//! result once the master is in. The reference measurement is told apart
//! by its [`MeasureSource::Decoded`] source.
//!
//! The analysis is exactly the panel's: `analyze::run` reads the same
//! `resonance-metering` functions (BS.1770 loudness, true peak, the
//! range's crest and correlation, `sixth_octave_ltas`) that the engine's
//! measurement reads here, so the panel and the wire suggest the same
//! thing for the same audio.

use std::path::PathBuf;

use crate::control_jobs::JobToken;
use crate::control_socket::ConnId;
use crate::message::Message;
use crate::state::{AssistTarget, PendingAssist};
use crate::Resonance;
use iced::Task;
use resonance_audio::types::{
    AudioCommand, AudioMeasureSource, DetailSet, MeasureSource, MixMeasurement, StemSource,
};
use resonance_control::methods::master::{
    self, AssistBandDeviation, AssistGenre, AssistMeasured, AssistMode, AssistParamValue,
    AssistParams, AssistResult, AssistSuggestion, AssistTargetInfo, MASTERING_PLUGIN_ID,
};
use resonance_control::{Request, Response, RpcError};
use resonance_mastering_assist::analyze::AnalysisResult;
use resonance_mastering_assist::decide::{self, Target};
use resonance_mastering_assist::{Genre, ReferenceTrack};

/// Handle `master.assist`, or `None` for any other method.
pub(super) fn try_handle(
    app: &mut Resonance,
    conn: ConnId,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    (request.method == master::ASSIST).then(|| (assist(app, conn, request), Task::none()))
}

fn assist(app: &mut Resonance, conn: ConnId, request: &Request) -> Response {
    let params: AssistParams = match request.params() {
        Ok(p) => p,
        Err(e) => return super::failure(request, e),
    };
    let (target, reference_file) = match resolve_target(app, &params) {
        Ok(t) => t,
        Err(e) => return super::failure(request, e),
    };
    if let Some(error) = super::meter::render_guard(app) {
        return super::failure(request, error);
    }
    let range = match super::meter::resolve_range(app, params.range) {
        Ok(range) => range,
        Err(e) => return super::failure(request, e),
    };

    let what = match &target {
        AssistTarget::Genre(g) => format!("genre {}", genre_of(*g).label()),
        AssistTarget::Reference { name, .. } => format!("reference {name}"),
    };
    let started = app.start_control_job(
        master::ASSIST,
        &format!("Master assistant against {what}"),
        JobToken::Measure {
            method: master::ASSIST,
            offline: true,
            compare: None,
        },
        Some(conn),
    );
    let job = u64::from(started.job_id);
    // Reference mode decodes the reference FIRST and renders the master
    // only once that succeeds ([`measured`]): with both commands in
    // flight, a failed decode would end the job — and release the
    // offline-render guard it holds — while the master render still ran,
    // so a bounce or freeze could start on top of it.
    let deferred = reference_file.is_some();
    app.control.pending_assists.insert(
        job,
        PendingAssist {
            target,
            mix: None,
            reference: None,
            mix_range: deferred.then_some(range),
        },
    );
    let sent = match reference_file {
        Some(path) => app
            .engine
            .send(AudioCommand::MeasureAudio {
                measure_id: job,
                source: AudioMeasureSource::File(path),
                detail: ASSIST_DETAIL,
            })
            .is_ok(),
        None => send_mix(app, job, range),
    };
    if !sent {
        app.control.pending_assists.remove(&job);
        app.control
            .jobs
            .fail(job, "the assistant did not start (engine unavailable)");
    }
    super::success(request, &started)
}

const ASSIST_DETAIL: DetailSet = DetailSet {
    spectrum: false,
    stereo: false,
    dynamics: false,
    depth: false,
    decay: false,
    assist: true,
};

/// Ask the engine for the master render with the assistant's LTAS.
fn send_mix(app: &Resonance, job: u64, range: Option<(u64, u64)>) -> bool {
    app.engine
        .send(AudioCommand::MeasureMix {
            measure_id: job,
            targets: vec![StemSource::Master],
            range,
            source: MeasureSource::Render,
            detail: ASSIST_DETAIL,
        })
        .is_ok()
}

/// Check the mode's fields and resolve the target (and, for a
/// reference, the pooled file to measure).
fn resolve_target(
    app: &Resonance,
    params: &AssistParams,
) -> Result<(AssistTarget, Option<PathBuf>), RpcError> {
    match params.mode {
        AssistMode::Genre => {
            if params.pool_asset_id.is_some() {
                return Err(RpcError::invalid_params(
                    "mode \"genre\" takes `genre`, not `pool_asset_id`",
                ));
            }
            let genre = params.genre.ok_or_else(|| {
                RpcError::invalid_params(
                    "mode \"genre\" needs `genre`: rock, indie, acoustic, jazz or pop",
                )
            })?;
            Ok((AssistTarget::Genre(genre), None))
        }
        AssistMode::Reference => {
            if params.genre.is_some() {
                return Err(RpcError::invalid_params(
                    "mode \"reference\" takes `pool_asset_id`, not `genre`",
                ));
            }
            let id = params.pool_asset_id.ok_or_else(|| {
                RpcError::invalid_params(
                    "mode \"reference\" needs `pool_asset_id`; import the reference with \
                     pool.import and list it with pool.list",
                )
            })?;
            let (path, name) = pooled_asset_file(app, id.0)?;
            Ok((
                AssistTarget::Reference {
                    asset_id: id.0,
                    name,
                },
                Some(path),
            ))
        }
    }
}

/// The pooled (engine-format, project-rate) WAV of asset `id` and the
/// name its clips get — the file a clip placed from it plays.
pub(super) fn pooled_asset_file(app: &Resonance, id: u64) -> Result<(PathBuf, String), RpcError> {
    let Some(asset) = app.media.pool.assets.iter().find(|a| a.id == id) else {
        return Err(RpcError::not_found(format!(
            "no pool asset with id {id}; list them with pool.list"
        )));
    };
    if asset.missing {
        return Err(RpcError::not_found(format!(
            "pool asset {id}'s file is missing from the project's audio folder; relink it \
             in the GUI first"
        )));
    }
    let Some(project_dir) = app.io.project_path.clone() else {
        return Err(RpcError::busy(
            "the project has no folder yet, so its pool has no files; save it first",
        ));
    };
    let name = super::clip::asset_view(app, asset).name;
    Ok((project_dir.join(&asset.project_relative_path), name))
}

/// Fold one engine measurement into its pending assist, and complete the
/// job once every measurement it needs has arrived.
pub(crate) fn measured(app: &mut Resonance, job: u64, results: Vec<MixMeasurement>) {
    let Some(m) = results.into_iter().next() else {
        app.control.pending_assists.remove(&job);
        app.control
            .jobs
            .fail(job, "the engine returned no usable measurement");
        return;
    };
    let Some(pending) = app.control.pending_assists.get_mut(&job) else {
        app.control.jobs.fail(job, "the assistant's request was lost");
        return;
    };
    if m.source == MeasureSource::Decoded {
        pending.reference = Some(m);
        // The reference is in: now render the master (see `assist`).
        if let Some(range) = pending.mix_range.take() {
            if !send_mix(app, job, range) {
                app.control.pending_assists.remove(&job);
                app.control
                    .jobs
                    .fail(job, "the master render did not start (engine unavailable)");
            }
            return;
        }
    } else {
        pending.mix = Some(m);
    }
    let ready = pending.mix.is_some()
        && (matches!(pending.target, AssistTarget::Genre(_)) || pending.reference.is_some());
    if !ready {
        return;
    }
    let pending = app
        .control
        .pending_assists
        .remove(&job)
        .expect("checked above");
    match result(app, &pending) {
        Ok(result) => match serde_json::to_value(result) {
            Ok(payload) => app.control.jobs.complete(job, payload),
            Err(e) => app.control.jobs.fail(job, e.to_string()),
        },
        Err(message) => app.control.jobs.fail(job, message),
    }
}

/// The job failed in the engine: drop what it was waiting for.
pub(crate) fn failed(app: &mut Resonance, job: u64) {
    app.control.pending_assists.remove(&job);
}

fn genre_of(g: AssistGenre) -> Genre {
    match g {
        AssistGenre::Rock => Genre::Rock,
        AssistGenre::Indie => Genre::Indie,
        AssistGenre::Acoustic => Genre::Acoustic,
        AssistGenre::Jazz => Genre::Jazz,
        AssistGenre::Pop => Genre::Pop,
    }
}

/// The assistant's view of one measurement: the same figures its
/// `analyze::run` computes on a captured buffer.
pub(crate) fn analysis_of(m: &MixMeasurement, sample_rate: u32) -> Result<AnalysisResult, String> {
    let spectrum_db = m
        .detail
        .assist_ltas
        .clone()
        .ok_or("the engine measured no assistant spectrum")?;
    Ok(AnalysisResult {
        sample_rate: sample_rate as f32,
        duration_s: m.frames as f32 / sample_rate.max(1) as f32,
        integrated_lufs: m.lufs_integrated,
        short_term_lufs: m.lufs_short_term_max,
        true_peak_dbtp: m.true_peak_dbtp,
        crest_db: m.crest_db,
        correlation: m.correlation,
        spectrum_db,
    })
}

fn round(v: f32, per_unit: f64) -> f64 {
    (f64::from(v) * per_unit).round() / per_unit
}

fn result(app: &Resonance, pending: &PendingAssist) -> Result<AssistResult, String> {
    let rate = app.sample_rate;
    let mix = pending.mix.as_ref().ok_or("the master render never arrived")?;
    if !mix.lufs_integrated.is_finite() {
        return Err(
            "the master is silent over this range (no gated loudness), so there is \
             nothing to analyse; check the range, and that nothing is muted or soloed away"
                .into(),
        );
    }
    let analysis = analysis_of(mix, rate)?;

    let (target, info) = match &pending.target {
        AssistTarget::Genre(g) => {
            let genre = genre_of(*g);
            let info = AssistTargetInfo {
                mode: AssistMode::Genre,
                genre: Some(*g),
                pool_asset_id: None,
                label: genre.label().to_owned(),
                target_lufs: f64::from(genre.target_lufs()),
            };
            (Target::Genre(genre), info)
        }
        AssistTarget::Reference { asset_id, name } => {
            let reference = pending
                .reference
                .as_ref()
                .ok_or("the reference measurement never arrived")?;
            if !reference.lufs_integrated.is_finite() {
                return Err(format!(
                    "the reference (pool asset {asset_id}) is silent, so there is nothing to \
                     compare against"
                ));
            }
            let track = ReferenceTrack {
                display_name: name.clone(),
                sample_rate: rate as f32,
                analysis: analysis_of(reference, rate)?,
            };
            let info = AssistTargetInfo {
                mode: AssistMode::Reference,
                genre: None,
                pool_asset_id: Some(resonance_control::ids::AssetId(*asset_id)),
                label: name.clone(),
                target_lufs: round(reference.lufs_integrated, 100.0),
            };
            (Target::Reference(track), info)
        }
    };

    let suggestions = decide::build(&analysis, &target);
    Ok(AssistResult {
        target: info,
        plugin_id: MASTERING_PLUGIN_ID.to_owned(),
        master_slot: app
            .master
            .plugins
            .iter()
            .position(|p| p.clap_plugin_id == MASTERING_PLUGIN_ID)
            .map(|i| i as u32),
        measured: AssistMeasured {
            lufs_integrated: Some(round(mix.lufs_integrated, 100.0)),
            true_peak_db: round(mix.true_peak_dbtp, 100.0),
            crest_db: round(mix.crest_db, 100.0),
            correlation: round(mix.correlation, 1_000.0),
            measured_seconds: (mix.frames as f64 / f64::from(rate.max(1)) * 100.0).round() / 100.0,
        },
        suggestions: suggestions
            .stages()
            .into_iter()
            .map(|stage| AssistSuggestion {
                stage: stage.stage.to_owned(),
                rationale: stage.rationale,
                params: stage
                    .params
                    .into_iter()
                    .map(|c| AssistParamValue {
                        key: c.key.to_owned(),
                        value: round(c.value, 1_000.0),
                    })
                    .collect(),
            })
            .collect(),
        deviations: suggestions
            .deviations
            .iter()
            .map(|d| AssistBandDeviation {
                hz: round(d.center_hz, 10.0),
                lo_db: round(d.lo_db, 10.0),
                hi_db: round(d.hi_db, 10.0),
                measured_db: round(d.measured_db, 10.0),
                deviation_db: round(d.deviation_db, 10.0),
            })
            .collect(),
    })
}
