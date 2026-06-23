//! Offline bounce renderers. Two entry points share one chunked render
//! core (`render::render_chunk`):
//!
//! * [`to_wav`] / [`to_wav_spawn`] — render the whole project to a
//!   32-bit float stereo WAV file. Includes master FX + master volume +
//!   hard-clip so the file plays back identically outside the app.
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

use crate::clap_host::SyncClapInstance;
use crate::types::*;

use super::SharedState;

mod clip;
mod render;
mod wav;

pub use clip::to_audio_clip;
pub use render::try_lock_with_backoff;
pub use wav::to_wav;

/// Spawn the bounce on a dedicated worker thread so the engine
/// dispatch loop is not blocked. A 5-minute project takes hundreds of
/// ms to render and previously froze every other command until the WAV
/// was written; now `Play`/`Pause`/MIDI input drain stay responsive.
#[allow(clippy::too_many_arguments)]
pub(crate) fn to_wav_spawn(
    path: String,
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
        .name("bounce-to-wav".into())
        .spawn(move || {
            to_wav(
                path,
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
        .expect("spawn bounce-to-wav thread");
}

/// Spawn the bounce-in-place render on a dedicated worker thread, same
/// rationale as [`to_wav_spawn`]: a long render previously blocked the
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
