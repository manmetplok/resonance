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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use indexmap::IndexMap;
use parking_lot::RwLock;

use resonance_audio::test_support::{AutomationSnapshot, PluginMap, SharedState, to_audio_clip};
use resonance_audio::types::*;

struct EngineState {
    shared: Arc<SharedState>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    plugins: Arc<RwLock<PluginMap>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
}

fn empty_engine_state() -> EngineState {
    EngineState {
        shared: Arc::new(SharedState::default()),
        clips: Arc::new(RwLock::new(Vec::new())),
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
        &AtomicBool::new(false),
        &state.clips,
        &state.plugins,
        &state.tempo_map,
        &AutomationSnapshot::default(),
        48_000,
        &event_tx,
        &crossbeam_channel::unbounded().0,
    );
    event_rx.try_recv().expect("bounce must emit an event")
}

#[test]
fn bounce_in_place_refuses_while_transport_playing() {
    let state = empty_engine_state();
    state.shared.playing.store(true, Ordering::SeqCst);

    let ev = run_bounce(&state);
    match ev {
        AudioEvent::TrackBounceError(err) => {
            assert!(
                err.message.contains("Stop transport"),
                "guard must name the transport as the reason, got: {}",
                err.message
            );
            assert_eq!(err.kind, EngineErrorKind::Busy);
        }
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
        AudioEvent::TrackBounceError(err) => {
            assert!(
                err.message.contains("not found"),
                "stopped transport must fall through to track validation, got: {}",
                err.message
            );
            assert_eq!(err.kind, EngineErrorKind::NotFound);
        }
        other => panic!("expected TrackBounceError, got {other:?}"),
    }
}
