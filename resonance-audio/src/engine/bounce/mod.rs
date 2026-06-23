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

use std::sync::Arc;

use crossbeam_channel::Sender;
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use resonance_common::FreezeCacheRef;

use crate::clap_host::SyncClapInstance;
use crate::types::*;

use super::SharedState;

mod clip;
mod encoder;
mod freeze;
mod limiter;
mod normalize;
mod render;
mod resample;
mod stem;
mod stem_export;
mod wav;

pub use clip::to_audio_clip;
pub use freeze::{to_freeze_cache, FREEZE_CANCELLED_MSG};
pub use render::try_lock_with_backoff;
pub use stem::{render_stem, stem_filter, stem_project_range, write_stem_wav, StemFilter};
pub use stem_export::export_stems;
pub(crate) use stem_export::export_stems_spawn;
pub use wav::{encode_buffer_for_test, normalize_buffer_for_test};
pub(crate) use wav::{run_export, ExportReporter};

/// Classify a freeze render's result into its terminal `AudioEvent`.
///
/// Pulled out of [`to_freeze_cache_spawn`]'s worker closure so the
/// complete / cancel / error mapping is unit-testable without spawning a
/// render: a successful render maps to `FreezeCompleted`, the cooperative
/// cancel sentinel ([`FREEZE_CANCELLED_MSG`]) to `FreezeCancelled`, and any
/// other error to `FreezeError`.
pub fn freeze_terminal_event(
    track_id: TrackId,
    result: Result<FreezeCacheRef, String>,
) -> AudioEvent {
    match result {
        Ok(cache_ref) => AudioEvent::FreezeCompleted { track_id, cache_ref },
        Err(msg) if msg == FREEZE_CANCELLED_MSG => AudioEvent::FreezeCancelled { track_id },
        Err(message) => AudioEvent::FreezeError { track_id, message },
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
    plugins: &Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    sample_rate: u32,
    event_tx: &Sender<AudioEvent>,
) {
    run_export(
        path,
        &crate::types::ExportSettings::default_wav(),
        ExportReporter::Bounce,
        shared,
        tracks,
        busses,
        master,
        clips,
        midi_clips,
        plugins,
        tempo_map,
        sample_rate,
        event_tx,
    );
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
    plugins: Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    sample_rate: u32,
    event_tx: Sender<AudioEvent>,
) {
    std::thread::Builder::new()
        .name("export".into())
        .spawn(move || {
            run_export(
                path,
                &settings,
                reporter,
                &shared,
                &tracks,
                &busses,
                &master,
                &clips,
                &midi_clips,
                &plugins,
                &tempo_map,
                sample_rate,
                &event_tx,
            );
        })
        .expect("spawn export thread");
}

/// Spawn the bounce-in-place render on a dedicated worker thread, same
/// rationale as [`export_spawn`]: a long render previously blocked the
/// engine dispatch loop, making `CancelBounce` (and every other
/// command) undeliverable until the clip finished. The worker observes
/// `shared.bounce_cancel` between chunks and reports back through
/// `event_tx`.
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
    plugins: Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    sample_rate: u32,
    event_tx: Sender<AudioEvent>,
) {
    std::thread::Builder::new()
        .name("bounce-in-place".into())
        .spawn(move || {
            to_audio_clip(
                source_track_id,
                target_track_id,
                target_clip_id,
                name,
                &shared,
                &tracks,
                &busses,
                &master,
                &clips,
                &midi_clips,
                &plugins,
                &tempo_map,
                sample_rate,
                &event_tx,
            );
        })
        .expect("spawn bounce-in-place thread");
}

/// Spawn the freeze render on a dedicated worker thread, same rationale as
/// [`to_wav_spawn`]: the offline render blocks for hundreds of ms and would
/// otherwise make `AudioCommand::CancelFreeze` (and every other command)
/// undeliverable until the cache WAV finished. The worker observes
/// `shared.bounce_cancel` between chunks (flipped by `CancelFreeze`) and
/// reports back through `event_tx` with the `Freeze*` event family:
/// `FreezeProgress` while rendering, then exactly one of `FreezeCompleted`,
/// `FreezeCancelled`, or `FreezeError`.
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
    plugins: Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    sample_rate: u32,
    event_tx: Sender<AudioEvent>,
) {
    std::thread::Builder::new()
        .name("freeze-render".into())
        .spawn(move || {
            let mut progress = |fraction: f32| {
                let _ = event_tx.send(AudioEvent::FreezeProgress { track_id, fraction });
            };
            let result = to_freeze_cache(
                track_id,
                cache_path,
                &shared,
                &tracks,
                &busses,
                &master,
                &clips,
                &midi_clips,
                &plugins,
                &tempo_map,
                sample_rate,
                &mut progress,
            );
            let _ = event_tx.send(freeze_terminal_event(track_id, result));
        })
        .expect("spawn freeze-render thread");
}
