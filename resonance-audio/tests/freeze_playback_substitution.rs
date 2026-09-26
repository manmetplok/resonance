//! Integration tests for frozen playback substitution in the shared mix
//! loop (`mixer/render_core.rs` → `render_block`, todo #573, doc #187).
//!
//! A track carrying an active `FrozenSource` must play its cached WAV
//! samples *instead of* the live timeline source + insert FX, while the
//! post-source mixer stage (volume, pan, mute/solo, routing/sends) keeps
//! applying live. Because the same `render_block` drives realtime
//! playback and the offline bounce/stem renderer, these tests exercise
//! the substitution through `to_wav` — the exact code path the realtime
//! callback uses — and assert the DoD:
//!
//! * a frozen project's bounce is **sample-identical** to the same
//!   project unfrozen (parity);
//! * a frozen track plays its **cache**, not the live source;
//! * **mute / solo / pan / volume** still affect a frozen track;
//! * a **sample-rate mismatch** between cache and engine is handled.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crossbeam_channel::unbounded;
use indexmap::IndexMap;
use parking_lot::RwLock;

use resonance_audio::__test_support::{AutomationSnapshot, PluginMap, SharedState, to_freeze_cache, to_wav};
use resonance_audio::types::*;
use resonance_common::{FreezeCacheRef, FreezeCacheStatus};

const SR: u32 = 48_000;

struct EngineState {
    shared: Arc<SharedState>,
    tracks: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: Arc<RwLock<MasterBus>>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<RwLock<Vec<MidiClip>>>,
    plugins: Arc<RwLock<PluginMap>>,
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

fn audio_clip(id: ClipId, track_id: TrackId, start_sample: u64, data: Vec<f32>) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample,
        source: ClipSource::Memory(data),
        name: "tone".into(),
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

/// Engine state with a single audio track (id 1) carrying a `frames`-long
/// tone clip from sample 0.
fn state_with_tone_track(frames: usize) -> EngineState {
    let state = empty_engine_state();
    state
        .tracks
        .write()
        .insert(1, Track::with_type(1, "track".into(), TrackType::Audio));
    state.clips.write().push(audio_clip(1, 1, 0, tone(frames)));
    state
}

/// Apply `f` to track 1 (volume / pan / mute setters use interior
/// mutability, so a read guard suffices).
fn with_track1(state: &EngineState, f: impl FnOnce(&Track)) {
    let tg = state.tracks.read();
    f(tg.get(&1).expect("track 1"));
}

fn read_wav(path: &std::path::Path) -> (hound::WavSpec, Vec<f32>) {
    let reader = hound::WavReader::open(path).expect("WAV must open");
    let spec = reader.spec();
    let samples = reader
        .into_samples::<f32>()
        .collect::<Result<Vec<_>, _>>()
        .expect("samples must decode");
    (spec, samples)
}

fn tmp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("resonance_freeze_sub_{name}.wav"))
}

/// Render `state` to a WAV via the shared offline bounce and return the
/// interleaved stereo samples.
fn bounce(state: &EngineState, name: &str) -> Vec<f32> {
    let path = tmp_path(name);
    let _ = std::fs::remove_file(&path);
    let (event_tx, _rx) = unbounded::<AudioEvent>();
    to_wav(
        path.to_string_lossy().into_owned(),
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
    let (_spec, samples) = read_wav(&path);
    let _ = std::fs::remove_file(&path);
    samples
}

/// Freeze track 1 of `state` to a cache WAV and load it back as a
/// `FrozenSource` (the same shape the engine attaches on project load).
fn freeze_track1(state: &EngineState, name: &str) -> FrozenSource {
    let path = tmp_path(&format!("cache_{name}"));
    let _ = std::fs::remove_file(&path);
    let cache_ref = to_freeze_cache(
        1,
        path.to_string_lossy().into_owned(),
        &state.shared,
        &AtomicBool::new(false),
        &state.tracks,
        &state.busses,
        &state.master,
        &state.clips,
        &state.midi_clips,
        &state.plugins,
        &state.tempo_map,
        &AutomationSnapshot::default(),
        SR,
        &mut |_| {},
    )
    .expect("freeze must succeed");
    let (spec, samples) = read_wav(&path);
    let _ = std::fs::remove_file(&path);
    let frame_count = (samples.len() / 2) as u64;
    FrozenSource::new(cache_ref, Arc::new(samples), spec.sample_rate, frame_count)
}

fn manual_source(dc: f32, frames: usize, sample_rate: u32) -> FrozenSource {
    let samples = Arc::new(vec![dc; frames * 2]);
    let cache_ref = FreezeCacheRef::new("manual.wav".into(), sample_rate, 32, 1, FreezeCacheStatus::Frozen);
    FrozenSource::new(cache_ref, samples, sample_rate, frames as u64)
}

fn attach(state: &EngineState, source: FrozenSource) {
    with_track1(state, |t| t.frozen_source.store(Some(Arc::new(source))));
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().cloned().fold(0.0f32, |m, s| m.max(s.abs()))
}

/// DoD parity: a bounce of a frozen project is sample-identical to the
/// same project unfrozen. Uses a non-unity volume and an off-centre pan
/// so the test fails if the cache had baked the fader/pan (it would then
/// be applied twice on playback) — proving the freeze cache is captured
/// pre-fader and the live mixer re-applies volume/pan transparently.
#[test]
fn frozen_bounce_matches_unfrozen_bounce() {
    let state = state_with_tone_track(SR as usize);
    with_track1(&state, |t| {
        t.set_volume(0.6);
        t.set_pan(0.35);
    });

    // Unfrozen reference render (live clip → fader/pan → master).
    let unfrozen = bounce(&state, "unfrozen");
    assert!(peak(&unfrozen) > 0.1, "reference bounce must be non-silent");

    // Freeze, attach the cache, and bounce again. The clip is left in
    // place (freeze is non-destructive) but the substitution plays the
    // cache instead.
    let source = freeze_track1(&state, "parity");
    attach(&state, source);
    let frozen = bounce(&state, "frozen");

    assert_eq!(
        unfrozen.len(),
        frozen.len(),
        "frozen and unfrozen bounce must have the same frame count"
    );
    assert_eq!(
        unfrozen, frozen,
        "frozen bounce must be sample-identical to the unfrozen bounce"
    );
}

/// A frozen track plays its cache, NOT the live source: with a cache that
/// is a distinct constant DC, the bounce must carry the DC (scaled by the
/// centre-pan gain), never the live tone clip's varying samples.
#[test]
fn frozen_track_plays_cache_not_live_source() {
    let frames = 512;
    let state = state_with_tone_track(frames);
    // Sanity: the live (unfrozen) render carries the varying tone.
    let live = bounce(&state, "cache_live");
    let live_var = live.windows(2).any(|w| (w[0] - w[1]).abs() > 1e-3);
    assert!(live_var, "live render of a tone must vary sample-to-sample");

    // Attach a constant-DC cache distinct from the tone.
    const DC: f32 = 0.25;
    attach(&state, manual_source(DC, frames, SR));
    let frozen = bounce(&state, "cache_frozen");

    // Centre pan is unity on a stereo-balance track (ba doc #276 BUG 3),
    // so a unity-volume track passes the DC cache through untouched.
    let expected = DC;
    // The export renders the shared FX tail past the project end (code
    // review ENG-07); the cache covers the project range only.
    assert!(
        frozen[..frames * 2].iter().all(|&s| (s - expected).abs() < 1e-4),
        "frozen bounce must carry the DC cache (~{expected}), got e.g. {}",
        frozen[0]
    );
}

/// Mute / volume / pan still affect a frozen track: the post-source mixer
/// stage runs live on the cached audio.
#[test]
fn mute_volume_pan_still_affect_frozen_track() {
    let frames = 256;
    let state = state_with_tone_track(frames);
    let source = freeze_track1(&state, "controls");
    attach(&state, source);

    // Baseline: unity volume, centre pan.
    with_track1(&state, |t| {
        t.set_volume(1.0);
        t.set_pan(0.0);
        t.set_muted(false);
    });
    let base = bounce(&state, "controls_base");
    assert!(peak(&base) > 0.1, "frozen track must be audible at unity");

    // Volume halved → every sample scales by 0.5.
    with_track1(&state, |t| t.set_volume(0.5));
    let halved = bounce(&state, "controls_half");
    assert_eq!(base.len(), halved.len());
    assert!(
        base.iter()
            .zip(&halved)
            .all(|(&b, &h)| (h - b * 0.5).abs() < 1e-5),
        "halving a frozen track's volume must halve its output"
    );

    // Hard-left pan → right channel silent, left channel present.
    with_track1(&state, |t| {
        t.set_volume(1.0);
        t.set_pan(-1.0);
    });
    let left = bounce(&state, "controls_left");
    let right_peak = left.iter().skip(1).step_by(2).cloned().fold(0.0f32, |m, s| m.max(s.abs()));
    let left_peak = left.iter().step_by(2).cloned().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(right_peak < 1e-5, "hard-left pan must silence the right channel (peak {right_peak})");
    assert!(left_peak > 0.1, "hard-left pan must keep the left channel (peak {left_peak})");

    // Muted → silent.
    with_track1(&state, |t| {
        t.set_pan(0.0);
        t.set_muted(true);
    });
    let muted = bounce(&state, "controls_muted");
    assert!(peak(&muted) < 1e-5, "muting a frozen track must silence it");
}

/// A sample-rate mismatch between the cache and the engine is handled
/// (linear resample): a half-rate DC cache still plays back as audio.
#[test]
fn sample_rate_mismatch_resamples() {
    let frames = 1024;
    let state = state_with_tone_track(frames);
    // Cache rendered at half the engine rate; frame_count generous enough
    // to cover the timeline window after the 0.5× resample stride.
    const DC: f32 = 0.3;
    attach(&state, manual_source(DC, frames, SR / 2));

    let frozen = bounce(&state, "resample");
    // DC interpolates to DC, so the resampled output is the same constant
    // (centre pan is unity) and, crucially, non-silent and finite.
    let expected = DC;
    assert!(peak(&frozen) > 0.1, "resampled cache must be non-silent");
    assert!(
        frozen.iter().all(|s| s.is_finite()),
        "resampled output must be finite"
    );
    assert!(
        frozen.iter().take(frames).all(|&s| (s - expected).abs() < 1e-3),
        "DC cache must resample to the same DC (~{expected})"
    );
}
