//! Regression test: an offline bounce must refuse to run while the
//! transport is playing. The offline renderers share plugin instances
//! with the live mixer, so interleaved `process()` calls (plus the
//! reset at bounce start) would corrupt both the live output and the
//! bounce. `to_audio_clip` / `to_wav` now bail with an error instead,
//! mirroring the realtime bounce path's existing guard.
//!
//! Drives `to_audio_clip` directly with empty engine state — the guard
//! fires before any track/plugin work, so no CLAP plugin or audio
//! device is needed.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use resonance_audio::__test_support::{to_audio_clip, SharedState, SyncClapInstance};
use resonance_audio::types::*;

struct EngineState {
    shared: Arc<SharedState>,
    tracks: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: Arc<RwLock<MasterBus>>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<RwLock<Vec<MidiClip>>>,
    plugins: Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
}

fn empty_engine_state() -> EngineState {
    EngineState {
        shared: Arc::new(SharedState::default()),
        tracks: Arc::new(RwLock::new(IndexMap::new())),
        busses: Arc::new(RwLock::new(IndexMap::new())),
        master: Arc::new(RwLock::new(MasterBus::new())),
        clips: Arc::new(RwLock::new(Vec::new())),
        midi_clips: Arc::new(RwLock::new(Vec::new())),
        plugins: Arc::new(RwLock::new(IndexMap::new())),
        tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
    }
}

fn run_bounce(state: &EngineState) -> AudioEvent {
    let (event_tx, event_rx) = crossbeam_channel::unbounded::<AudioEvent>();
    to_audio_clip(
        /* source_track_id */ 1,
        /* target_track_id */ 2,
        /* target_clip_id */ 1,
        "bounced".into(),
        &state.shared,
        &state.tracks,
        &state.busses,
        &state.master,
        &state.clips,
        &state.midi_clips,
        &state.plugins,
        &state.tempo_map,
        48_000,
        &event_tx,
    );
    event_rx.try_recv().expect("bounce must emit an event")
}

#[test]
fn bounce_in_place_refuses_while_transport_playing() {
    let state = empty_engine_state();
    state.shared.playing.store(true, Ordering::SeqCst);

    let ev = run_bounce(&state);
    match ev {
        AudioEvent::TrackBounceError(msg) => assert!(
            msg.contains("Stop transport"),
            "guard must name the transport as the reason, got: {msg}"
        ),
        other => panic!("expected TrackBounceError, got {other:?}"),
    }
    // The renderer must not have produced a clip.
    assert!(state.clips.read().is_empty());
}

#[test]
fn bounce_in_place_passes_guard_when_transport_stopped() {
    // Identical empty state with the transport stopped reaches track
    // validation instead — proving the guard above keys on `playing`,
    // not on the empty project.
    let state = empty_engine_state();

    let ev = run_bounce(&state);
    match ev {
        AudioEvent::TrackBounceError(msg) => assert!(
            msg.contains("not found"),
            "stopped transport must fall through to track validation, got: {msg}"
        ),
        other => panic!("expected TrackBounceError, got {other:?}"),
    }
}
