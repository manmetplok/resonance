//! `render.*` control handlers (ba doc #265, todo #1157): offline audio
//! export to EXPLICIT paths, as jobs returning the written file path.
//!
//! `render.mixdown` reuses the app's maintained WAV bounce path — the
//! path-carrying `BouncePathSelected(Some(path))` message, which drives
//! `AudioCommand::BounceToWav` — never the rfd bounce dialog. The job is
//! keyed off the engine's bounce completion (`BounceComplete` /
//! `BounceError`, mirrored in `engine_events::transport`) via
//! [`JobToken::Export`], which carries the target path so a completion
//! resolves exactly the job that requested it.
//!
//! Rendering never mutates project or undo state; it only reads the
//! current mix and writes a file.
//!
//! Guards (doc #265):
//! - an existing file at `path` without `"overwrite": true` →
//!   `needs_confirmation`;
//! - a relative path, or one whose parent directory is missing →
//!   `invalid_params`;
//! - the transport recording, or another render already in flight →
//!   `busy`.
//!
//! `render.stems` is `unsupported` in this todo: the stem-export
//! orchestration is not wired into the app on this branch, and doc #265
//! says not to build new stem plumbing here.

use crate::control_jobs::JobToken;
use crate::control_socket::ConnId;
use crate::message::{Message, ProjectIoMessage};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::render::{self as proto, MixdownParams, RangeSpec, StemsParams};
use resonance_control::{Request, Response, RpcError};
use std::path::Path;

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
        proto::STEMS => stems(request),
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

    // The maintained WAV bounce renders the whole project; a partial
    // range would need the stem/export range plumbing that isn't wired
    // here. Accept an omitted or explicitly-whole range, reject the rest.
    if let Some(range) = params.range {
        if !is_whole(&range) {
            return (
                super::failure(
                    request,
                    RpcError::unsupported(
                        "render.mixdown renders the whole project; a partial \
                         range is not supported on this build",
                    ),
                ),
                Task::none(),
            );
        }
    }

    if let Some(error) = busy_guard(app) {
        return (super::failure(request, error), Task::none());
    }

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

    // Route through the path-carrying bounce message (never the dialog).
    let task = super::run_via_update(
        app,
        Message::ProjectIo(ProjectIoMessage::BouncePathSelected(Some(params.path))),
    );

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

/// True when a range covers the whole song — no start and no end
/// coordinates given.
fn is_whole(range: &RangeSpec) -> bool {
    range.start.is_none_or(|p| p.is_empty()) && range.end.is_none_or(|p| p.is_empty())
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
// render.stems (unsupported here)
// ---------------------------------------------------------------------------

fn stems(request: &Request) -> (Response, Task<Message>) {
    // Validate params so a malformed call still gets `invalid_params`
    // rather than a misleading `unsupported`.
    if let Err(e) = request.params::<StemsParams>() {
        return (super::failure(request, e), Task::none());
    }
    (
        super::failure(
            request,
            RpcError::unsupported(
                "render.stems is not available on this build; stem export lands \
                 with the export epic (doc #196)",
            ),
        ),
        Task::none(),
    )
}

// ---------------------------------------------------------------------------
// Guards
// ---------------------------------------------------------------------------

/// `busy` while the transport is recording (a bounce requires a stopped
/// transport) or another render is already in flight.
fn busy_guard(app: &Resonance) -> Option<RpcError> {
    if app.transport.recording {
        return Some(RpcError::busy(
            "the transport is recording; stop it before rendering",
        ));
    }
    if app.io.bouncing {
        return Some(RpcError::busy("a render is already in progress"));
    }
    None
}
