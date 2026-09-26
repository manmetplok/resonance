//! Multi-target stem export (ba todo #325).
//!
//! Drives [`AudioCommand::ExportStems`]: render several mix slices (one
//! track, one bus, or the whole master) to separate WAV files, then emit
//! a queue of progress / completion events the app turns into a stem
//! export modal.
//!
//! Built on the stem render core (`super::stem`, ba todo #322):
//!
//! * Every target is rendered over ONE shared `[start, end)` so the stems
//!   share a zero origin and re-import sample-aligned. The range is the
//!   caller's explicit window or the full project range.
//! * Targets render **sequentially** on the worker thread — they share
//!   the live plugin instances, so they cannot render concurrently.
//! * Partial failure is first-class: a target that fails to render or
//!   write emits [`AudioEvent::StemExportTargetError`] but the already-
//!   written stems stay on disk and the queue continues, so the app can
//!   offer "retry remaining".
//! * Cancel is cooperative *between* targets: the worker polls this
//!   export's own cancel token (set by `AudioCommand::CancelStemExport`
//!   via `HandlerState::stem_cancel`) before each target and stops,
//!   leaving finished stems on disk and reporting them via
//!   [`AudioEvent::StemExportCancelled`].
//!
//! [`AudioCommand::ExportStems`]: crate::types::AudioCommand::ExportStems

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::Sender;
use indexmap::IndexMap;
use parking_lot::RwLock;

use crate::clap_host::PluginMap;
use crate::types::*;

use super::super::SharedState;
use super::stem::{render_stem, stem_project_range, write_stem_wav};

/// Render `targets` to WAV files and stream the export event queue.
///
/// Synchronous core, called on the worker thread spawned by
/// [`export_stems_spawn`]. Pulled out so integration tests can drive it
/// directly and assert the emitted event sequence.
///
/// `engine_rate` is the engine's native sample rate (what `render_stem`
/// produces); `out_rate` is the requested WAV sample rate, resampled on
/// write only when it differs. Returns nothing — every outcome is an
/// `AudioEvent` on `event_tx`.
#[allow(clippy::too_many_arguments)]
pub fn export_stems(
    targets: Vec<StemTarget>,
    range: Option<(SamplePos, SamplePos)>,
    out_rate: u32,
    bit_depth: StemBitDepth,
    include_fx_tail: bool,
    shared: &Arc<SharedState>,
    cancel: &AtomicBool,
    tracks: &Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: &Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: &Arc<RwLock<MasterBus>>,
    clips: &Arc<RwLock<Vec<AudioClip>>>,
    plugins: &Arc<RwLock<PluginMap>>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    engine_rate: u32,
    event_tx: &Sender<AudioEvent>,
) {
    // -- Pre-flight: a failed check writes no files. --
    if targets.is_empty() {
        let _ = event_tx.send(AudioEvent::StemExportError(EngineError::unsupported(
            "No stems selected to export",
        )));
        return;
    }
    // Same guard as the other offline renderers: rendering while the
    // transport rolls would interleave shared plugin process()/reset
    // calls with live playback and corrupt both outputs.
    if shared.playing.load(Ordering::Relaxed) {
        let _ = event_tx.send(AudioEvent::StemExportError(EngineError::busy(
            "Stop transport before exporting stems",
        )));
        return;
    }
    let Some((start, end)) = range.or_else(|| stem_project_range(clips, &shared.graph.load().midi_clips, tempo_map, engine_rate))
    else {
        let _ = event_tx.send(AudioEvent::StemExportError(EngineError::unsupported(
            "No audio to export",
        )));
        return;
    };
    if end <= start {
        let _ = event_tx.send(AudioEvent::StemExportError(EngineError::unsupported(
            "Empty render range",
        )));
        return;
    }

    // Extend the shared end by an FX tail when asked; every target uses
    // the SAME extended end so the stems stay sample-aligned. It is the
    // master export's tail too (code review ENG-07), so the stems and the
    // mix come out the same length.
    let tail = if include_fx_tail {
        super::super::bounce_common::offline_tail_frames(engine_rate)
    } else {
        0
    };
    let render_end = end + tail;

    let total = targets.len();
    let mut written: Vec<String> = Vec::with_capacity(total);

    for (index, target) in targets.iter().enumerate() {
        // Cooperative cancel between targets — `CancelStemExport` flips
        // this export's own token from the engine thread (never cleared
        // here: the token dies with this run). Stems already written stay.
        if cancel.load(Ordering::Relaxed) {
            let _ = event_tx.send(AudioEvent::StemExportCancelled { files: written });
            return;
        }

        let _ = event_tx.send(AudioEvent::StemExportProgress {
            target_index: index,
            total,
            fraction: index as f32 / total as f32,
        });

        let render = render_stem(
            target.source,
            start,
            render_end,
            shared,
            tracks,
            busses,
            master,
            clips,
            plugins,
            tempo_map,
            engine_rate,
        );
        let outcome = match render {
            Ok(samples) => write_stem_wav(&target.path, &samples, engine_rate, out_rate, bit_depth),
            Err(e) => Err(e),
        };

        match outcome {
            Ok(()) => {
                written.push(target.path.clone());
                let _ = event_tx.send(AudioEvent::StemExportTargetDone {
                    index,
                    path: target.path.clone(),
                });
            }
            // Keep the stems written so far and carry on with the queue.
            Err(e) => {
                let _ = event_tx.send(AudioEvent::StemExportTargetError {
                    index,
                    message: e.to_string(),
                });
            }
        }
    }

    let _ = event_tx.send(AudioEvent::StemExportComplete { files: written });
}

/// Spawn [`export_stems`] on a dedicated worker thread so the engine
/// dispatch loop stays responsive (rendering N stems can take seconds);
/// same rationale as [`super::to_wav_spawn`]. Returns this export's
/// freshly-created cancel token; the worker polls it between targets, so
/// `CancelStemExport` is delivered through the dispatch loop while the
/// render runs and can never abort a different render's run.
#[allow(clippy::too_many_arguments)]
pub(crate) fn export_stems_spawn(
    targets: Vec<StemTarget>,
    range: Option<(SamplePos, SamplePos)>,
    out_rate: u32,
    bit_depth: StemBitDepth,
    include_fx_tail: bool,
    shared: Arc<SharedState>,
    tracks: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: Arc<RwLock<MasterBus>>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    plugins: Arc<RwLock<PluginMap>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    engine_rate: u32,
    event_tx: Sender<AudioEvent>,
) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_render = Arc::clone(&cancel);
    // Gate raised on the engine thread; see `export_spawn`.
    let offline = super::OfflineRenderGuard::mark(&shared);
    std::thread::Builder::new()
        .name("export-stems".into())
        .spawn(move || {
            // Panic supervision: a panicking render must still emit the
            // path's terminal error event (see `crate::supervise`).
            let panic_tx = event_tx.clone();
            crate::supervise::run_supervised(
                "export-stems",
                || {
                    let _offline = offline;
                    export_stems(
                        targets,
                        range,
                        out_rate,
                        bit_depth,
                        include_fx_tail,
                        &shared,
                        &cancel_render,
                        &tracks,
                        &busses,
                        &master,
                        &clips,
                        &plugins,
                        &tempo_map,
                        engine_rate,
                        &event_tx,
                    );
                },
                |message| {
                    let _ = panic_tx.send(AudioEvent::StemExportError(EngineError::internal(
                        message,
                    )));
                },
            );
        })
        .expect("spawn export-stems thread");
    cancel
}
