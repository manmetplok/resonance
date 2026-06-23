//! Export-exclusion guarantee for the reference A/B feature (todo #693).
//!
//! The reference monitor tap lives solely in the live audio callback
//! (`mixer::mix_audio`); every offline render drives `render_chunk`,
//! which never reads `shared.reference`. This test pins that guarantee
//! end-to-end: with the A/B monitor switched to a loaded reference, an
//! offline `to_wav` bounce must still render the *mix*, not the
//! reference PCM.

use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::unbounded;
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use resonance_audio::__test_support::{to_wav, SharedState, SyncClapInstance};
use resonance_audio::types::*;
use resonance_audio::{
    handle_reference_analyzed, handle_set_ab_source, handle_set_active_reference,
    register_reference, ReferencePlayer,
};

const SR: u32 = 48_000;
const FRAMES: usize = 64;
const MIX_DC: f32 = 0.5;
const REF_DC: f32 = -0.9;

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

/// One audio track carrying a single in-memory DC clip at `MIX_DC`.
fn engine_with_dc_clip() -> EngineState {
    let mut tracks = IndexMap::new();
    tracks.insert(1, Track::new(1, "audio".into()));

    let clip = AudioClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        source: ClipSource::Memory(vec![MIX_DC; FRAMES * 2]),
        name: "dc".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::Linear,
        gain_db: 0.0,
        vocal_tuning: None,
    };

    EngineState {
        shared: Arc::new(SharedState::default()),
        tracks: Arc::new(RwLock::new(tracks)),
        busses: Arc::new(RwLock::new(IndexMap::new())),
        master: Arc::new(RwLock::new(MasterBus::new())),
        clips: Arc::new(RwLock::new(vec![clip])),
        midi_clips: Arc::new(RwLock::new(Vec::new())),
        plugins: Arc::new(RwLock::new(IndexMap::new())),
        tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
    }
}

/// Engage `shared.reference` with an active reference whose PCM is a
/// distinct DC value, exactly as the live engine would after the user
/// switches A/B to a loaded reference.
fn engage_reference_monitor(shared: &SharedState) {
    let mut player = ReferencePlayer::new();
    let (tx, _rx) = unbounded::<AudioEvent>();
    let id = register_reference(&mut player, Some(ReferenceId(1)), PathBuf::from("/ref.wav"));
    handle_set_active_reference(&mut player, &tx, id);
    handle_reference_analyzed(&mut player, id, Arc::new(vec![REF_DC; FRAMES * 2]), -10.0);
    handle_set_ab_source(&mut player, &tx, ABSource::Reference);
    player.publish(&shared.reference, true);
    // Sanity: the live monitor really is engaged on the reference.
    assert!(shared.reference.is_reference_for_test());
}

#[test]
fn bounce_excludes_the_reference_and_renders_the_mix() {
    let state = engine_with_dc_clip();
    engage_reference_monitor(&state.shared);

    let dir = std::env::temp_dir().join(format!("resonance_ref_export_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("bounce.wav");
    let path_str = path.to_string_lossy().into_owned();

    let (event_tx, event_rx) = unbounded::<AudioEvent>();
    to_wav(
        path_str.clone(),
        &state.shared,
        &state.tracks,
        &state.busses,
        &state.master,
        &state.clips,
        &state.midi_clips,
        &state.plugins,
        &state.tempo_map,
        SR,
        &event_tx,
    );

    match event_rx.try_recv() {
        Ok(AudioEvent::BounceComplete { .. }) => {}
        other => panic!("expected BounceComplete, got {other:?}"),
    }

    let mut reader = hound::WavReader::open(&path).expect("open bounced wav");
    let samples: Vec<f32> = reader.samples::<f32>().map(|s| s.expect("sample")).collect();
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);

    assert_eq!(samples.len(), FRAMES * 2, "stereo frame count");
    // The clip pans to centre, so each channel carries the DC scaled by
    // the constant-power centre gain (-3 dB). The exact value doesn't
    // matter — what matters is that it's the *mix*, never the reference.
    let expected_mix = MIX_DC * std::f32::consts::FRAC_1_SQRT_2;
    assert!(
        samples.iter().all(|&s| (s - expected_mix).abs() < 1e-4),
        "bounce must contain the mix (~{expected_mix}), got e.g. {}",
        samples[0]
    );
    // Decisive exclusion check: the reference DC is negative; the mix is
    // positive. Not a single sample may resemble the reference PCM.
    assert!(
        samples.iter().all(|&s| s > 0.0 && (s - REF_DC).abs() > 0.1),
        "bounce must never contain the reference PCM ({REF_DC})"
    );
}
