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
//! * Cancel is cooperative *between* targets: the worker polls
//!   `shared.bounce_cancel` (set by `AudioCommand::CancelStemExport`)
//!   before each target and stops, leaving finished stems on disk and
//!   reporting them via [`AudioEvent::StemExportCancelled`].
//!
//! [`AudioCommand::ExportStems`]: crate::types::AudioCommand::ExportStems

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crossbeam_channel::Sender;
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use crate::clap_host::SyncClapInstance;
use crate::types::*;

use super::super::SharedState;
use super::stem::{render_stem, stem_project_range, write_stem_wav};

/// Seconds of extra render past the range end so reverb / delay tails
/// decay into the stem instead of being cut off (when `include_fx_tail`).
const FX_TAIL_SECONDS: u64 = 2;

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
    tracks: &Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: &Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: &Arc<RwLock<MasterBus>>,
    clips: &Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: &Arc<RwLock<Vec<MidiClip>>>,
    plugins: &Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    engine_rate: u32,
    event_tx: &Sender<AudioEvent>,
) {
    // -- Pre-flight: a failed check writes no files. --
    if targets.is_empty() {
        let _ = event_tx.send(AudioEvent::StemExportError("No stems selected to export".into()));
        return;
    }
    // Same guard as the other offline renderers: rendering while the
    // transport rolls would interleave shared plugin process()/reset
    // calls with live playback and corrupt both outputs.
    if shared.playing.load(Ordering::Relaxed) {
        let _ = event_tx.send(AudioEvent::StemExportError(
            "Stop transport before exporting stems".into(),
        ));
        return;
    }
    let Some((start, end)) = range.or_else(|| stem_project_range(clips, midi_clips, tempo_map, engine_rate))
    else {
        let _ = event_tx.send(AudioEvent::StemExportError("No audio to export".into()));
        return;
    };
    if end <= start {
        let _ = event_tx.send(AudioEvent::StemExportError("Empty render range".into()));
        return;
    }

    // Extend the shared end by an FX tail when asked; every target uses
    // the SAME extended end so the stems stay sample-aligned.
    let tail = if include_fx_tail {
        FX_TAIL_SECONDS * engine_rate as u64
    } else {
        0
    };
    let render_end = end + tail;

    let total = targets.len();
    let mut written: Vec<String> = Vec::with_capacity(total);

    for (index, target) in targets.iter().enumerate() {
        // Cooperative cancel between targets — `CancelStemExport` flips
        // this flag from the engine thread. Stems already written stay.
        if shared.bounce_cancel.load(Ordering::Relaxed) {
            shared.bounce_cancel.store(false, Ordering::Relaxed);
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
            midi_clips,
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
            Err(message) => {
                let _ = event_tx.send(AudioEvent::StemExportTargetError { index, message });
            }
        }
    }

    let _ = event_tx.send(AudioEvent::StemExportComplete { files: written });
}

/// Spawn [`export_stems`] on a dedicated worker thread so the engine
/// dispatch loop stays responsive (rendering N stems can take seconds);
/// same rationale as [`super::to_wav_spawn`]. The worker polls
/// `shared.bounce_cancel` between targets, so `CancelStemExport` is
/// delivered through the dispatch loop while the render runs.
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
    midi_clips: Arc<RwLock<Vec<MidiClip>>>,
    plugins: Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    engine_rate: u32,
    event_tx: Sender<AudioEvent>,
) {
    std::thread::Builder::new()
        .name("export-stems".into())
        .spawn(move || {
            export_stems(
                targets,
                range,
                out_rate,
                bit_depth,
                include_fx_tail,
                &shared,
                &tracks,
                &busses,
                &master,
                &clips,
                &midi_clips,
                &plugins,
                &tempo_map,
                engine_rate,
                &event_tx,
            );
        })
        .expect("spawn export-stems thread");
}
