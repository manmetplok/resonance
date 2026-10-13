//! `render.*` control handlers (ba doc #265, todo #1157): offline audio
//! export to EXPLICIT paths, as jobs returning the written file path.
//!
//! A plain whole-song `render.mixdown` reuses the app's maintained WAV
//! bounce path — the path-carrying `BouncePathSelected(Some(path))`
//! message, which drives `AudioCommand::BounceToWav` — never the rfd
//! bounce dialog. A ranged or normalized one sends
//! `AudioCommand::ExportAudio` (same 32-bit-float WAV) and is resolved by
//! `ExportComplete` through `ControlEndpointState::pending_exports`. The
//! job is keyed via [`JobToken::Export`], which carries the target path
//! so a completion resolves exactly the job that requested it.
//!
//! `render.stems` sends `AudioCommand::ExportStems`, the Export modal's
//! command, with the modal's defaults and file names
//! (`update::export::stem_targets_for`); its job ([`JobToken::StemExport`])
//! is resolved from the `StemExport*` events by the `stems_*` hooks
//! below, which `engine_events::export` calls ahead of the modal.
//!
//! Rendering never mutates project or undo state; it only reads the
//! current mix and writes files.
//!
//! Guards (doc #265):
//! - an existing file at `path` without `"overwrite": true` →
//!   `needs_confirmation`;
//! - a relative path, or one whose parent directory is missing →
//!   `invalid_params`;
//! - the transport recording, or another render already in flight →
//!   `busy`;
//! - an empty `range` (after clamping to the song) → `invalid_params`.

use crate::control_jobs::JobToken;
use crate::control_socket::ConnId;
use crate::message::{Message, ProjectIoMessage};
use crate::state::ExportSource;
use crate::Resonance;
use iced::Task;
use resonance_audio::types::{
    AudioCommand, EngineError, ExportSettings, NormalizeMode, NormalizeSpec, SamplePos,
};
use resonance_control::methods::render::{
    self as proto, MixdownParams, NormalizeReport, StemsParams,
};
use resonance_control::{ErrorKind, Request, Response, RpcError};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Handle a `render.*` request, or `None` when `method` belongs to
/// another namespace. Mutating dispatch: the returned [`Task`] must
/// reach the runtime.
pub(super) fn try_handle(
    app: &mut Resonance,
    conn: ConnId,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let handled = match request.method.as_str() {
        proto::MIXDOWN => mixdown(app, conn, request),
        proto::STEMS => stems(app, conn, request),
        _ => return None,
    };
    Some(handled)
}

// ---------------------------------------------------------------------------
// render.mixdown
// ---------------------------------------------------------------------------

fn mixdown(app: &mut Resonance, conn: ConnId, request: &Request) -> (Response, Task<Message>) {
    let params: MixdownParams = match request.params() {
        Ok(p) => p,
        Err(e) => return (super::failure(request, e), Task::none()),
    };

    if let Some(error) = busy_guard(app) {
        return (super::failure(request, error), Task::none());
    }
    let normalize = match normalize_request(&params) {
        Ok(n) => n,
        Err(e) => return (super::failure(request, e), Task::none()),
    };

    let path = Path::new(&params.path);
    if !path.is_absolute() {
        return (
            super::failure(
                request,
                RpcError::invalid_params(format!("path must be absolute, got {:?}", params.path)),
            ),
            Task::none(),
        );
    }
    // The renderer writes WAV (the bounce and the normalized export
    // both), so the target must be a `.wav`: with `overwrite` this path
    // is otherwise any file the user can write (code review STATE2-10).
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
    {
        return (
            super::failure(
                request,
                RpcError::invalid_params(format!(
                    "path must end in .wav (the mixdown is a WAV file), got {:?}",
                    params.path
                )),
            ),
            Task::none(),
        );
    }
    if !path.parent().is_some_and(Path::is_dir) {
        return (
            super::failure(
                request,
                RpcError::invalid_params(format!(
                    "parent directory of {:?} does not exist",
                    params.path
                )),
            ),
            Task::none(),
        );
    }
    if path.exists() && !params.overwrite {
        return (
            super::failure(
                request,
                RpcError::needs_confirmation(format!(
                    "{:?} already exists and would be overwritten; \
                     pass \"overwrite\": true to replace it",
                    params.path
                )),
            ),
            Task::none(),
        );
    }

    // `None` for the whole song (omitted or `{}`); a window is clamped to
    // the song like a `meter.*` range, and an empty one is refused.
    let range = match super::meter::resolve_range(app, params.range) {
        Ok(r) => r,
        Err(e) => return (super::failure(request, e), Task::none()),
    };

    // Correlate the eventual `BounceComplete { path }` back to this job:
    // the engine echoes the requested path verbatim.
    let started = app.start_control_job(
        proto::MIXDOWN,
        &format!("Mix down to {}", path.display()),
        JobToken::Export {
            path: path.to_path_buf(),
        },
        Some(conn),
    );

    let task = match (normalize, range) {
        // Plain whole-song mix: the path-carrying bounce message (never
        // the dialog).
        (None, None) => super::run_via_update(
            app,
            Message::ProjectIo(ProjectIoMessage::BouncePathSelected(Some(params.path))),
        ),
        // Normalized or ranged: the generalized export command (with a
        // normalize stage and/or a window), resolved by `ExportComplete`
        // (`engine_events::transport::export_complete`).
        (normalize, range) => {
            start_export(app, &params.path, normalize, range);
            Task::none()
        }
    };

    // `BouncePathSelected` sets `bouncing` and fires the engine command
    // synchronously; if it somehow didn't start, fail the job now rather
    // than leave the client waiting out the timeout.
    if !app.io.bouncing {
        app.control.jobs.fail(
            u64::from(started.job_id),
            "mixdown did not start (engine unavailable)",
        );
    }

    (super::success(request, &started), task)
}

/// The normalization a mixdown asks for, as the report it will carry,
/// or `None` for a plain mix.
fn normalize_request(params: &MixdownParams) -> Result<Option<NormalizeReport>, RpcError> {
    let (platform, (target_lufs, ceiling_dbtp)) = match (params.platform, params.normalize) {
        (Some(_), Some(_)) => {
            return Err(RpcError::invalid_params(
                "pass `platform` or `normalize`, not both: a platform is shorthand for its \
                 own targets",
            ))
        }
        (Some(p), None) => (Some(p), p.targets()),
        (None, Some(n)) => (None, (n.target_lufs, n.ceiling_dbtp)),
        (None, None) => return Ok(None),
    };
    if !(target_lufs.is_finite() && (-40.0..=-5.0).contains(&target_lufs)) {
        return Err(RpcError::invalid_params(format!(
            "target_lufs {target_lufs} must be between -40 and -5"
        )));
    }
    if !(ceiling_dbtp.is_finite() && (-12.0..=0.0).contains(&ceiling_dbtp)) {
        return Err(RpcError::invalid_params(format!(
            "ceiling_dbtp {ceiling_dbtp} must be between -12 and 0"
        )));
    }
    Ok(Some(NormalizeReport {
        platform,
        target_lufs,
        ceiling_dbtp,
        achieved_lufs: None,
        achieved_dbtp: 0.0,
    }))
}

/// Start a WAV export through `AudioCommand::ExportAudio`: the same
/// 32-bit-float WAV at the engine rate the plain mixdown writes, over
/// `range` when given (sample 0 = the range start), and with the
/// engine's two-pass normalize stage (measure, gain to the target,
/// true-peak limit at the ceiling, re-measure) in front of the encoder
/// when `normalize` is given. Sets the same bounce state the plain path
/// does, so the busy guards and the progress modal hold.
fn start_export(
    app: &mut Resonance,
    path: &str,
    normalize: Option<NormalizeReport>,
    range: Option<(SamplePos, SamplePos)>,
) {
    app.io.bouncing = true;
    app.io.bounce_fraction = 0.0;
    app.io.bounce_cancel_requested = false;
    app.io.bounce_target = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned());
    app.control
        .pending_exports
        .insert(Path::new(path).to_path_buf(), normalize);
    let settings = match normalize {
        Some(report) => ExportSettings {
            normalize: NormalizeSpec {
                enabled: true,
                mode: NormalizeMode::IntegratedLufs,
                target_db: report.target_lufs as f32,
                ceiling_dbtp: report.ceiling_dbtp as f32,
            },
            ..ExportSettings::default_wav()
        },
        None => ExportSettings::default_wav(),
    };
    if app
        .engine
        .send(AudioCommand::ExportAudio {
            path: path.to_owned(),
            settings,
            range,
        })
        .is_err()
    {
        app.io.bouncing = false;
        app.control.pending_exports.remove(Path::new(path));
    }
}

/// Build the `render.mixdown` job result for a completed bounce. Reads
/// the written WAV's header for its true sample rate and frame count
/// (the file is authoritative — it may have been resampled on write);
/// falls back to the engine rate and a zero duration if the header can't
/// be parsed (the file still exists, so the render itself succeeded).
///
/// Public to the control module so the engine-event completion hook
/// (`engine_events::transport::bounce_complete`) shares one definition
/// of the result shape.
pub(crate) fn mixdown_result(
    app: &crate::Resonance,
    path: &str,
    engine_sample_rate: u32,
    normalize: Option<NormalizeReport>,
) -> serde_json::Value {
    let (duration_s, sample_rate) = match read_wav_geometry(Path::new(path)) {
        Some(geo) => (geo.duration_s(), geo.sample_rate),
        None => (0.0, engine_sample_rate),
    };
    serde_json::to_value(proto::MixdownResult {
        path: path.to_owned(),
        duration_s,
        sample_rate,
        // A bounce taken with a track soloed contains only that track.
        // The file is what was asked for, but nothing else in the result
        // says so (ba doc #275 P1.6).
        soloed_track_ids: super::meter::soloed_track_ids(app),
        normalize,
    })
    .unwrap_or(serde_json::Value::Null)
}

/// The few RIFF/WAVE header fields needed to report the render's
/// duration: bytes in the `data` chunk plus the frame size.
struct WavGeometry {
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
    data_bytes: u64,
}

impl WavGeometry {
    fn duration_s(&self) -> f64 {
        let bytes_per_frame = self.channels as u64 * (self.bits_per_sample as u64 / 8);
        if bytes_per_frame == 0 || self.sample_rate == 0 {
            return 0.0;
        }
        let frames = self.data_bytes / bytes_per_frame;
        frames as f64 / self.sample_rate as f64
    }
}

/// Parse just enough of a canonical RIFF/WAVE file to recover the
/// sample rate, channel count, bit depth, and `data` chunk length —
/// walking the chunk list so a non-44-byte header (e.g. an `fmt ` with
/// extension, or a `fact`/`LIST` chunk before `data`) still parses.
/// `None` on any malformed / truncated header.
///
/// I/O is bounded: this runs on the update thread right after a bounce
/// (an 80 MB+ file for a few minutes of 24-bit stereo), so it reads only
/// the 8-byte chunk headers plus the 16 `fmt ` fields and *seeks* past
/// every chunk body — never the whole file.
fn read_wav_geometry(path: &Path) -> Option<WavGeometry> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path).ok()?;
    let file_len = file.metadata().ok()?.len();

    let mut riff = [0u8; 12];
    file.read_exact(&mut riff).ok()?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return None;
    }

    let mut sample_rate = None;
    let mut channels = None;
    let mut bits = None;
    let mut data_bytes = None;

    // A rendered WAV is `fmt ` then `data` within a handful of chunks;
    // the cap only stops a crafted file of millions of empty chunks from
    // pinning the update thread with seeks.
    const MAX_CHUNKS: u32 = 1024;
    let mut walked = 0;

    let mut pos: u64 = 12; // past "RIFF<size>WAVE"
    while pos + 8 <= file_len && walked < MAX_CHUNKS {
        walked += 1;
        file.seek(SeekFrom::Start(pos)).ok()?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header).ok()?;
        let size = u64::from(u32::from_le_bytes([header[4], header[5], header[6], header[7]]));
        let body = pos + 8;
        match &header[0..4] {
            b"fmt " if body + 16 <= file_len => {
                let mut fmt = [0u8; 16];
                file.read_exact(&mut fmt).ok()?;
                channels = Some(u16::from_le_bytes([fmt[2], fmt[3]]));
                sample_rate = Some(u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]));
                bits = Some(u16::from_le_bytes([fmt[14], fmt[15]]));
            }
            b"data" => {
                // The declared size may overrun a still-flushing file;
                // clamp to what's actually on disk.
                data_bytes = Some(size.min(file_len.saturating_sub(body)));
            }
            _ => {}
        }
        // Everything wanted is in hand once `data` follows `fmt ` (the
        // canonical order); don't walk a trailing chunk list for nothing.
        if sample_rate.is_some() && data_bytes.is_some() {
            break;
        }
        // Chunks are word-aligned: an odd size is followed by a pad byte.
        pos = body + size + (size & 1);
    }

    Some(WavGeometry {
        sample_rate: sample_rate?,
        channels: channels?,
        bits_per_sample: bits?,
        data_bytes: data_bytes?,
    })
}

// ---------------------------------------------------------------------------
// render.stems
// ---------------------------------------------------------------------------

/// `render.stems`: one WAV per top-level track (or per `track_ids`
/// entry) into `dir`, through the Export modal's `ExportStems` command
/// (sent by `ExportMessage::RenderStems`) with the modal's defaults
/// (`ExportFormat::default()`: 48 kHz, 24-bit, FX tail) and its file
/// names (`NN-<track name>.wav`, numbered in track id order).
fn stems(app: &mut Resonance, conn: ConnId, request: &Request) -> (Response, Task<Message>) {
    let params: StemsParams = match request.params() {
        Ok(p) => p,
        Err(e) => return (super::failure(request, e), Task::none()),
    };
    if let Some(error) = busy_guard(app) {
        return (super::failure(request, error), Task::none());
    }
    // The mutation gate already refuses while a stem export runs; say so
    // in this method's own words when it is reached directly.
    if app.stem_export_in_progress() {
        return (
            super::failure(request, RpcError::busy("a stem export is already in progress")),
            Task::none(),
        );
    }

    let dir = PathBuf::from(&params.dir);
    if !dir.is_absolute() {
        return (
            super::failure(
                request,
                RpcError::invalid_params(format!("dir must be absolute, got {:?}", params.dir)),
            ),
            Task::none(),
        );
    }
    if dir.exists() && !dir.is_dir() {
        return (
            super::failure(
                request,
                RpcError::invalid_params(format!("{:?} exists and is not a directory", params.dir)),
            ),
            Task::none(),
        );
    }
    let sources = match stem_sources(app, params.track_ids.as_deref()) {
        Ok(s) => s,
        Err(e) => return (super::failure(request, e), Task::none()),
    };
    let range = match super::meter::resolve_range(app, params.range) {
        Ok(r) => r,
        Err(e) => return (super::failure(request, e), Task::none()),
    };

    // The modal's names, never suffixed: a name already on disk is the
    // file this call would replace, so it needs `overwrite`.
    let targets = crate::update::export::stem_targets_for(
        app,
        &sources,
        &dir,
        crate::state::ExportOverwrite::Overwrite,
    );
    if !params.overwrite {
        let existing: Vec<&str> = targets
            .iter()
            .filter(|t| Path::new(&t.path).exists())
            .map(|t| t.path.as_str())
            .collect();
        if !existing.is_empty() {
            return (
                super::failure(
                    request,
                    RpcError::needs_confirmation(format!(
                        "{} stem file(s) already exist and would be overwritten: {}; pass \
                         \"overwrite\": true to replace them",
                        existing.len(),
                        existing.join(", ")
                    )),
                ),
                Task::none(),
            );
        }
    }
    // Created like the modal creates its destination.
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return (
            super::failure(
                request,
                RpcError::invalid_params(format!("can't create {}: {e}", dir.display())),
            ),
            Task::none(),
        );
    }

    let total = targets.len();
    let started = app.start_control_job(
        proto::STEMS,
        &format!("Export {total} stem(s) to {}", dir.display()),
        JobToken::StemExport { dir },
        Some(conn),
    );
    let job_id = u64::from(started.job_id);
    let task = super::run_via_update(
        app,
        Message::Export(crate::message::ExportMessage::RenderStems {
            job_id,
            targets,
            range,
        }),
    );
    // `RenderStems` sends synchronously; if it did not start, fail the
    // job now rather than leave the client waiting out the timeout.
    if app.control.pending_stems.is_none() {
        app.control
            .jobs
            .fail(job_id, "stem export did not start (engine unavailable)");
    }
    (super::success(request, &started), task)
}

/// The sources a `render.stems` call exports: every top-level track (the
/// Export modal's track list — a track's stem carries its sub-tracks), or
/// exactly `track_ids`, each of which must name one.
fn stem_sources(
    app: &Resonance,
    track_ids: Option<&[resonance_control::ids::TrackId]>,
) -> Result<BTreeSet<ExportSource>, RpcError> {
    let Some(ids) = track_ids else {
        let all: BTreeSet<ExportSource> = app
            .registry
            .tracks
            .iter()
            .filter(|t| t.sub_track.is_none())
            .map(|t| ExportSource::Track(t.id))
            .collect();
        if all.is_empty() {
            return Err(RpcError::invalid_params("the project has no tracks to export"));
        }
        return Ok(all);
    };
    if ids.is_empty() {
        return Err(RpcError::invalid_params(
            "track_ids is empty; omit it to export every track",
        ));
    }
    let mut sources = BTreeSet::new();
    for id in ids {
        let Some(track) = app.registry.tracks.iter().find(|t| t.id == id.0) else {
            return Err(RpcError::invalid_params(format!(
                "track_ids names no track with id {id}"
            )));
        };
        if let Some(link) = &track.sub_track {
            return Err(RpcError::invalid_params(format!(
                "track {id} is a sub-track; its parent's stem carries it (export track {})",
                link.parent_track_id
            )));
        }
        sources.insert(ExportSource::Track(track.id));
    }
    Ok(sources)
}

/// The `render.stems` job result: the written paths, and the length of
/// the first (every stem renders over the same range, so all match),
/// read from its header as for a mixdown.
fn stems_result(files: &[String]) -> serde_json::Value {
    let duration_s = files
        .first()
        .and_then(|f| read_wav_geometry(Path::new(f)))
        .map_or(0.0, |geo| geo.duration_s());
    serde_json::to_value(proto::StemsResult {
        paths: files.to_vec(),
        duration_s,
    })
    .unwrap_or(serde_json::Value::Null)
}

// The `StemExport*` hooks, called by `engine_events::export` ahead of the
// modal. Each returns `true` when a control job owns the running export
// (the event is then fully handled here) and `false` when the event is
// the modal's.

/// `StemExportProgress`: target `target_index` is starting.
pub(crate) fn stems_progress(
    app: &mut Resonance,
    target_index: usize,
    total: usize,
    fraction: f32,
) -> bool {
    let Some(stems) = app.control.pending_stems.as_mut() else {
        return false;
    };
    stems.done = target_index.min(total);
    stems.fraction = fraction.clamp(0.0, 1.0);
    app.control.jobs.set_progress(stems.job_id, stems.fraction);
    true
}

/// `StemExportTargetDone`: one stem is on disk.
pub(crate) fn stems_target_done(app: &mut Resonance, index: usize) -> bool {
    let Some(stems) = app.control.pending_stems.as_mut() else {
        return false;
    };
    stems.done = (index + 1).max(stems.done).min(stems.total);
    stems.fraction = stems.done as f32 / stems.total.max(1) as f32;
    app.control.jobs.set_progress(stems.job_id, stems.fraction);
    true
}

/// `StemExportTargetError`: one stem failed; the queue goes on, and the
/// job fails once it finishes.
pub(crate) fn stems_target_error(app: &mut Resonance, index: usize, message: &str) -> bool {
    let Some(stems) = app.control.pending_stems.as_mut() else {
        return false;
    };
    stems.errors.push(format!("stem {}: {message}", index + 1));
    stems_target_done(app, index)
}

/// `StemExportComplete`: done with every path when every target wrote.
/// Any target error fails the job instead: a result listing only the
/// stems that wrote would read as "done" to a client that does not count
/// them, so the message carries both the errors and what was written.
pub(crate) fn stems_complete(app: &mut Resonance, files: &[String]) -> bool {
    let Some(stems) = app.control.pending_stems.take() else {
        return false;
    };
    if stems.errors.is_empty() {
        app.control.jobs.complete(stems.job_id, stems_result(files));
    } else {
        app.control.jobs.fail_with_kind(
            stems.job_id,
            format!("{}; written: {}", stems.errors.join("; "), written_list(files)),
            Some(ErrorKind::Internal),
        );
    }
    true
}

/// `StemExportError`: the export could not start; nothing was written.
pub(crate) fn stems_error(app: &mut Resonance, e: &EngineError) -> bool {
    let Some(stems) = app.control.pending_stems.take() else {
        return false;
    };
    app.control.jobs.fail_with_kind(
        stems.job_id,
        e.message.clone(),
        Some(super::job::engine_kind_to_rpc(e.kind)),
    );
    true
}

/// `StemExportCancelled`: the user cancelled from the progress overlay;
/// the stems written before it stay on disk.
pub(crate) fn stems_cancelled(app: &mut Resonance, files: &[String]) -> bool {
    let Some(stems) = app.control.pending_stems.take() else {
        return false;
    };
    app.control.jobs.fail(
        stems.job_id,
        format!(
            "stem export cancelled in the app after {} of {} stem(s); written: {}",
            files.len(),
            stems.total,
            written_list(files)
        ),
    );
    true
}

fn written_list(files: &[String]) -> String {
    if files.is_empty() {
        "none".to_owned()
    } else {
        files.join(", ")
    }
}

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

/// `busy` while the transport is recording (a bounce requires a stopped
/// transport), another render is already in flight, or an offline
/// measurement holds the offline renderer.
fn busy_guard(app: &Resonance) -> Option<RpcError> {
    if app.transport.recording {
        return Some(RpcError::busy(
            "the transport is recording; stop it before rendering",
        ));
    }
    if app.io.bouncing {
        return Some(RpcError::busy("a render is already in progress"));
    }
    // The mirror image of `meter.measure`'s guard: an offline
    // measurement took the offline renderer exclusively, and a mixdown
    // started on top of it would drive the same live plugin instances
    // from two renderers at once.
    if app.offline_measure_in_progress() {
        return Some(RpcError::busy(
            "a measurement is in progress; render again when it finishes",
        ));
    }
    None
}
