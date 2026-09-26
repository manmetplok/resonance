//! Offline bounce renderers. Two entry points share one chunked render
//! core (`render::render_chunk`):
//!
//! * [`run_export`] / [`export_spawn`] — render the whole project and
//!   feed the mix to the encoder sink selected by the export format (WAV
//!   16/24-bit/f32 or FLAC, with optional export resampling). Includes
//!   master FX + master volume + hard-clip so the file plays back
//!   identically outside the app.
//!
//! * [`to_audio_clip`] — render a single instrument track (and any of
//!   its sub-tracks) to an in-RAM stereo buffer, then push it as a fresh
//!   [`AudioClip`] on a target track. Excludes master FX / master volume
//!   / hard-clip because the audio will play through master on the next
//!   playback (which would otherwise apply master FX twice). Used by
//!   the "bounce in place" workflow for internal-synth instrument
//!   tracks.
//!
//! Both render loops mirror live playback: per-track plugin chain,
//! per-bus plugin chain and routing. They reset every plugin once at
//! the start so plugin internal state is deterministic.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crossbeam_channel::Sender;
use indexmap::IndexMap;
use parking_lot::RwLock;
use thiserror::Error;

use resonance_common::FreezeCacheRef;

use crate::clap_host::PluginMap;
use crate::types::*;

use super::SharedState;

mod clip;
mod encoder;
mod freeze;
mod limiter;
mod measure;
mod normalize;
mod render;
mod resample;
mod stem;
mod stem_export;
mod wav;

pub use clip::to_audio_clip;
pub use freeze::{read_freeze_cache, to_freeze_cache, FreezeError, FREEZE_CANCELLED_MSG};
pub use render::try_lock_with_backoff;
pub use render::{chunk_span, BOUNCE_CHUNK, MIN_CLAP_FRAMES};
pub use measure::{measure_mix, measure_rendered_buffer};
pub(crate) use measure::measure_mix_spawn;
pub use stem::{render_stem, stem_filter, stem_project_range, write_stem_wav, StemFilter};
pub use stem_export::export_stems;
pub(crate) use stem_export::export_stems_spawn;
pub use wav::{encode_buffer_for_test, normalize_buffer_for_test};
pub(crate) use wav::{run_export, ExportReporter};

/// A render's output file while it is being written (code review ENG-13).
///
/// Renderers write to a sibling `<target>.partial` and only
/// [`commit`](Self::commit) — rename it over the target — once the file
/// is complete. Until then the target is untouched, so a failed or
/// cancelled render never truncates or deletes the file it would have
/// replaced; dropping an uncommitted `PartialFile` removes the temp file.
/// The rename stays in one directory, so it is atomic on one filesystem.
pub(super) struct PartialFile {
    temp: std::path::PathBuf,
    target: std::path::PathBuf,
    committed: bool,
}

impl PartialFile {
    pub(super) fn new(target: impl Into<std::path::PathBuf>) -> Self {
        let target = target.into();
        let mut temp = target.clone().into_os_string();
        temp.push(".partial");
        Self {
            temp: temp.into(),
            target,
            committed: false,
        }
    }

    /// Where the renderer writes.
    pub(super) fn temp(&self) -> &std::path::Path {
        &self.temp
    }

    /// Move the finished file into place. On failure the temp file is
    /// removed (on drop) and the target left as it was.
    pub(super) fn commit(mut self) -> Result<(), PartialFileError> {
        std::fs::rename(&self.temp, &self.target).map_err(|e| PartialFileError {
            path: self.target.display().to_string(),
            source: e,
        })?;
        self.committed = true;
        Ok(())
    }
}

/// Failure moving a [`PartialFile`]'s temp file into place
/// ([`PartialFile::commit`]). Message text matches the historical
/// `format!()` string.
#[derive(Debug, Error)]
#[error("Could not move the finished file into place at {path}: {source}")]
pub struct PartialFileError {
    path: String,
    #[source]
    source: std::io::Error,
}

impl From<PartialFileError> for EngineError {
    fn from(e: PartialFileError) -> Self {
        EngineError::new(EngineErrorKind::Io, e.to_string())
    }
}

impl Drop for PartialFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.temp);
        }
    }
}

/// RAII marker for "an offline render is running on a worker thread"
/// (ba todo #1218).
///
/// Every offline renderer drives the same live CLAP plugin instances, so
/// two concurrent renders corrupt each other — and so does the live audio
/// callback running alongside one. Holding one of these for the duration
/// of a render publishes that fact in
/// [`SharedState::offline_render_count`], which is the one gate (code
/// review MIX-02 / ENG-05) that
///
/// * lets the read-only mix-measurement command refuse to start rather
///   than interfere with a render that is producing a file,
/// * makes the audio callback output silence and hold the transport
///   instead of touching a plugin (`mixer::callback::mix_audio`), and
/// * makes Play / Record / realtime bounce / MIDI-clock start refuse
///   (`transport::refuse_while_offline_render`).
///
/// The file-writing spawn paths take it on the **engine thread**, before
/// the worker exists, and move it into the worker: the transport
/// handlers and the renderers' own "stop the transport first" check are
/// then ordered by the engine thread, so a Play can neither slip in
/// between the spawn and the worker's check nor be accepted once the
/// render is under way.
///
/// [`mark`](Self::mark) always succeeds and just counts (the file-writing
/// renderers keep their existing behaviour);
/// [`try_acquire_exclusive`](Self::try_acquire_exclusive) succeeds only
/// when nothing else is rendering. Because `mark` is unconditional, the
/// reverse exclusion — no bounce / freeze / export starting while a
/// measurement holds the renderer — is enforced app-side at every render
/// START path (`Resonance::offline_measure_in_progress`).
pub struct OfflineRenderGuard {
    shared: Arc<SharedState>,
}

/// Message reported when a measurement is refused because an offline
/// render already holds the renderer. Public so the app (and tests) can
/// recognise the busy case without string-matching a literal.
pub const MEASURE_BUSY_MSG: &str = "Another offline render is in progress";

/// Reason the transport (Play / Record / realtime bounce / MIDI-clock
/// start) is refused while an offline render holds the plugin instances.
/// Public so the app and tests can recognise the refusal.
pub const OFFLINE_RENDER_BUSY_MSG: &str =
    "An offline render (export, bounce, freeze or stem render) is in progress";

impl OfflineRenderGuard {
    /// Join the set of running offline renders unconditionally. `pub`
    /// (doc-hidden re-export) so the gate tests can hold the real guard.
    pub fn mark(shared: &Arc<SharedState>) -> Self {
        shared
            .offline_render_count
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Self {
            shared: Arc::clone(shared),
        }
    }

    /// Take the renderer only if no other offline render is running.
    /// Returns `None` when one is, so the caller can report "busy".
    pub(crate) fn try_acquire_exclusive(shared: &Arc<SharedState>) -> Option<Self> {
        shared
            .offline_render_count
            .compare_exchange(
                0,
                1,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .ok()
            .map(|_| Self {
                shared: Arc::clone(shared),
            })
    }
}

impl Drop for OfflineRenderGuard {
    fn drop(&mut self) {
        self.shared
            .offline_render_count
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// Classify a freeze render's result into its terminal `AudioEvent`.
///
/// Pulled out of [`to_freeze_cache_spawn`]'s worker closure so the
/// complete / cancel / error mapping is unit-testable without spawning a
/// render: a successful render maps to `FreezeCompleted`, the cooperative
/// cancel variant ([`freeze::FreezeError::Cancelled`]) to `FreezeCancelled`,
/// and any other error to `AudioEvent::FreezeError`.
pub fn freeze_terminal_event(
    track_id: TrackId,
    result: Result<FreezeCacheRef, freeze::FreezeError>,
) -> AudioEvent {
    match result {
        Ok(cache_ref) => AudioEvent::FreezeCompleted { track_id, cache_ref },
        Err(freeze::FreezeError::Cancelled) => AudioEvent::FreezeCancelled { track_id },
        Err(e) => AudioEvent::FreezeError {
            track_id,
            message: e.to_string(),
        },
    }
}

/// Test-only compatibility shim for the legacy `to_wav` bounce entry point,
/// kept after the export pipeline (epic #46) renamed the renderer to
/// [`run_export`]. Renders with the default 32-bit-float WAV settings and the
/// `Bounce` reporter, so reference-A/B export-exclusion tests that predate the
/// rename keep exercising the real render path.
#[allow(clippy::too_many_arguments)]
pub fn to_wav(
    path: String,
    shared: &Arc<SharedState>,
    tracks: &Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: &Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: &Arc<RwLock<MasterBus>>,
    clips: &Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: &Arc<RwLock<Vec<MidiClip>>>,
    plugins: &Arc<RwLock<PluginMap>>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    sample_rate: u32,
    event_tx: &Sender<AudioEvent>,
) {
    // Test-only shim with no automation snapshot to thread; render with an
    // empty one (the automation-aware paths go through `export_spawn`).
    let automation = super::AutomationSnapshot::default();
    // Direct synchronous call — nothing can cancel it, so hand the
    // renderer a token nobody else holds.
    let cancel = AtomicBool::new(false);
    run_export(
        path,
        &crate::types::ExportSettings::default_wav(),
        ExportReporter::Bounce,
        shared,
        &cancel,
        tracks,
        busses,
        master,
        clips,
        midi_clips,
        plugins,
        tempo_map,
        &automation,
        sample_rate,
        event_tx,
    );
}

/// Test surface: run the real export renderer ([`run_export`]) with
/// explicit settings, automation snapshot and cancel token, and return
/// every event it emitted (`Export*` family). Unlike [`to_wav`] this
/// reaches the normalization passes and the cancel / temp-file paths.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn export_for_test(
    path: String,
    settings: &ExportSettings,
    cancel: &AtomicBool,
    shared: &Arc<SharedState>,
    tracks: &Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: &Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: &Arc<RwLock<MasterBus>>,
    clips: &Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: &Arc<RwLock<Vec<MidiClip>>>,
    plugins: &Arc<RwLock<PluginMap>>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    automation: &super::AutomationSnapshot,
    sample_rate: u32,
) -> Vec<AudioEvent> {
    let (tx, rx) = crossbeam_channel::unbounded();
    run_export(
        path,
        settings,
        ExportReporter::Export,
        shared,
        cancel,
        tracks,
        busses,
        master,
        clips,
        midi_clips,
        plugins,
        tempo_map,
        automation,
        sample_rate,
        &tx,
    );
    drop(tx);
    rx.try_iter().collect()
}

/// Spawn an offline export on a dedicated worker thread so the engine
/// dispatch loop is not blocked. A 5-minute project takes hundreds of
/// ms to render and previously froze every other command until the file
/// was written; now `Play`/`Pause`/MIDI input drain stay responsive.
///
/// `reporter` selects which event family the run reports through: the
/// legacy `BounceToWav` shim uses [`ExportReporter::Bounce`] (`Bounce*`
/// events, byte-for-byte the old behaviour); `ExportAudio` uses
/// [`ExportReporter::Export`] (`Export*` events with the encoded byte
/// size).
///
/// Returns this render's freshly-created cancel token; flipping it to
/// `true` aborts the render between chunks. The token belongs to this
/// render alone, so cancelling it can never abort a different render
/// and a pending cancel can never be cleared by a later render starting.
#[allow(clippy::too_many_arguments)]
pub(crate) fn export_spawn(
    path: String,
    settings: ExportSettings,
    reporter: ExportReporter,
    shared: Arc<SharedState>,
    tracks: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: Arc<RwLock<MasterBus>>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<RwLock<Vec<MidiClip>>>,
    plugins: Arc<RwLock<PluginMap>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    automation: Arc<super::AutomationSnapshot>,
    sample_rate: u32,
    event_tx: Sender<AudioEvent>,
) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_render = Arc::clone(&cancel);
    // Raise the offline-render gate here, on the engine thread, so a
    // Play dispatched after this command is refused before the worker
    // even checks the transport.
    let offline = OfflineRenderGuard::mark(&shared);
    std::thread::Builder::new()
        .name("export".into())
        .spawn(move || {
            // A panic anywhere in the offline mixer (third-party CLAP
            // `process()` included) must still resolve the export with a
            // terminal event, or the app's modal and a control client's
            // `job_wait` hang forever. Report it through the same
            // reporter expected failures use; `Io` is the closest
            // existing kind for "the render died mid-run".
            let panic_tx = event_tx.clone();
            crate::supervise::run_supervised(
                "export",
                || {
                    let _offline = offline;
                    run_export(
                        path,
                        &settings,
                        reporter,
                        &shared,
                        &cancel_render,
                        &tracks,
                        &busses,
                        &master,
                        &clips,
                        &midi_clips,
                        &plugins,
                        &tempo_map,
                        &automation,
                        sample_rate,
                        &event_tx,
                    );
                },
                |message| reporter.error(&panic_tx, ExportErrorKind::Io, message),
            );
        })
        .expect("spawn export thread");
    cancel
}

/// Spawn the bounce-in-place render on a dedicated worker thread, same
/// rationale as [`export_spawn`]: a long render previously blocked the
/// engine dispatch loop, making `CancelBounce` (and every other
/// command) undeliverable until the clip finished. The worker observes
/// the returned per-render cancel token between chunks and reports back
/// through `event_tx`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn to_audio_clip_spawn(
    source_track_id: TrackId,
    target_track_id: TrackId,
    target_clip_id: ClipId,
    name: String,
    shared: Arc<SharedState>,
    tracks: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: Arc<RwLock<MasterBus>>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<RwLock<Vec<MidiClip>>>,
    plugins: Arc<RwLock<PluginMap>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    automation: Arc<super::AutomationSnapshot>,
    sample_rate: u32,
    event_tx: Sender<AudioEvent>,
) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_render = Arc::clone(&cancel);
    // Gate raised on the engine thread; see `export_spawn`.
    let offline = OfflineRenderGuard::mark(&shared);
    std::thread::Builder::new()
        .name("bounce-in-place".into())
        .spawn(move || {
            // Panic supervision: a panicking render must still emit the
            // path's terminal error event (see `crate::supervise`).
            let panic_tx = event_tx.clone();
            crate::supervise::run_supervised(
                "bounce-in-place",
                || {
                    let _offline = offline;
                    to_audio_clip(
                        source_track_id,
                        target_track_id,
                        target_clip_id,
                        name,
                        &shared,
                        &cancel_render,
                        &tracks,
                        &busses,
                        &master,
                        &clips,
                        &midi_clips,
                        &plugins,
                        &tempo_map,
                        &automation,
                        sample_rate,
                        &event_tx,
                    );
                },
                |message| {
                    let _ = panic_tx.send(AudioEvent::TrackBounceError(EngineError::internal(
                        message,
                    )));
                },
            );
        })
        .expect("spawn bounce-in-place thread");
    cancel
}

/// Spawn the freeze render on a dedicated worker thread, same rationale as
/// [`to_wav_spawn`]: the offline render blocks for hundreds of ms and would
/// otherwise make `AudioCommand::CancelFreeze` (and every other command)
/// undeliverable until the cache WAV finished. The worker observes the
/// returned per-render cancel token between chunks (flipped by
/// `CancelFreeze`) and reports back through `event_tx` with the `Freeze*`
/// event family: `FreezeProgress` while rendering, then exactly one of
/// `FreezeCompleted`, `FreezeCancelled`, or `FreezeError`.
#[allow(clippy::too_many_arguments)]
pub fn to_freeze_cache_spawn(
    track_id: TrackId,
    cache_path: String,
    shared: Arc<SharedState>,
    tracks: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: Arc<RwLock<MasterBus>>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<RwLock<Vec<MidiClip>>>,
    plugins: Arc<RwLock<PluginMap>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    automation: Arc<super::AutomationSnapshot>,
    sample_rate: u32,
    event_tx: Sender<AudioEvent>,
) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_render = Arc::clone(&cancel);
    // Gate raised on the engine thread; see `export_spawn`.
    let offline = OfflineRenderGuard::mark(&shared);
    std::thread::Builder::new()
        .name("freeze-render".into())
        .spawn(move || {
            // Panic supervision: a panicking render must still emit the
            // path's terminal error event (see `crate::supervise`).
            let panic_tx = event_tx.clone();
            crate::supervise::run_supervised(
                "freeze-render",
                || {
                    let _offline = offline;
                    let mut progress = |fraction: f32| {
                        let _ = event_tx.send(AudioEvent::FreezeProgress { track_id, fraction });
                    };
                    let result = to_freeze_cache(
                        track_id,
                        cache_path,
                        &shared,
                        &cancel_render,
                        &tracks,
                        &busses,
                        &master,
                        &clips,
                        &midi_clips,
                        &plugins,
                        &tempo_map,
                        &automation,
                        sample_rate,
                        &mut progress,
                    );
                    let _ = event_tx.send(freeze_terminal_event(track_id, result));
                },
                |message| {
                    let _ = panic_tx.send(AudioEvent::FreezeError { track_id, message });
                },
            );
        })
        .expect("spawn freeze-render thread");
    cancel
}
