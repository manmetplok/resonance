//! Regression: recorded takes on an external-instrument track must be
//! audible on playback.
//!
//! An external-instrument track is created as `TrackType::Instrument`
//! (the app's `AddExternalInstrumentTrack` sends `AddInstrumentTrack`),
//! but its "instrument" is outboard hardware — there is no plugin to
//! render. Its audio comes from the return input: live while monitoring,
//! or from the recorded take once one exists. Both live on the audio
//! side of the mixer's per-track branch, so an external track has to
//! take that branch even though its type says Instrument.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use parking_lot::RwLock;

use resonance_audio::test_support::{AutomationSnapshot, SharedState, to_freeze_cache};
use resonance_audio::types::*;

const SR: u32 = 48_000;

struct EngineState {
    shared: Arc<SharedState>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
}

fn empty_engine_state() -> EngineState {
    EngineState {
        shared: Arc::new(SharedState::default()),
        clips: Arc::new(RwLock::new(Vec::new())),
        tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
    }
}

/// A stereo interleaved 220 Hz sine, `frames` long at amplitude 0.5.
fn tone(frames: usize) -> Vec<f32> {
    let mut data = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let s = (i as f32 * 220.0 * std::f32::consts::TAU / SR as f32).sin() * 0.5;
        data.push(s);
        data.push(s);
    }
    data
}

fn audio_clip(id: ClipId, track_id: TrackId, data: Vec<f32>) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::Memory(data),
        name: "take".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: Default::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

fn peak_of_render(track: Track, name: &str) -> f32 {
    let state = empty_engine_state();
    state.shared.edit_tracks(|m| { m.insert(1, std::sync::Arc::new(track)); });
    state
        .clips
        .write()
        .push(audio_clip(1, 1, tone(SR as usize)));

    let path = std::env::temp_dir().join(format!("resonance_ext_playback_{name}.wav"));
    let _ = std::fs::remove_file(&path);
    to_freeze_cache(
        1,
        path.to_string_lossy().into_owned(),
        &state.shared,
        &AtomicBool::new(false),
        &state.clips,
        &state.tempo_map,
        &AutomationSnapshot::default(),
        SR,
        &mut |_| {},
    )
    .expect("render must succeed");

    let reader = hound::WavReader::open(&path).expect("render WAV must open");
    let peak = reader
        .into_samples::<f32>()
        .map(|s| s.expect("samples must decode").abs())
        .fold(0.0f32, f32::max);
    let _ = std::fs::remove_file(&path);
    peak
}

/// Baseline: the same take on a plain audio track is audible.
#[test]
fn audio_track_plays_its_take() {
    let track = Track::with_type(1, "audio".into(), TrackType::Audio);
    let peak = peak_of_render(track, "audio");
    assert!(peak > 0.1, "audio track take must be audible (peak {peak})");
}

/// The regression: an external-instrument track carrying a recorded take
/// with playback source `Recorded` used to render silence, because its
/// `Instrument` type sent it down the plugin path where the clip mix
/// never runs.
#[test]
fn external_instrument_track_plays_its_take() {
    let track = Track::with_type(1, "ext".into(), TrackType::Instrument);
    track.set_external(true);
    track.set_playback_source(resonance_common::PlaybackSource::Recorded);
    let peak = peak_of_render(track, "ext");
    assert!(
        peak > 0.1,
        "external-instrument track take must be audible (peak {peak})"
    );
}
