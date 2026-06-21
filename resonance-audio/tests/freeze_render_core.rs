//! Integration tests for the freeze render core (`to_freeze_cache`,
//! todo #571, doc #187).
//!
//! Drives the offline freeze renderer directly with engine state built
//! around a plain audio clip (no CLAP plugin or audio device needed):
//! the clip's PCM plays through the track at unity gain, so the render
//! is deterministic. Covers the DoD: a known track renders non-silent
//! audio to a correct WAV, the source clips are left untouched, and
//! progress / cooperative cancel behave.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use resonance_audio::__test_support::{to_freeze_cache, SharedState, SyncClapInstance};
use resonance_audio::types::*;

const SR: u32 = 48_000;

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
    }
}

/// Engine state with a single audio track (id 1) carrying a 1-second
/// tone clip from sample 0.
fn state_with_tone_track() -> EngineState {
    let state = empty_engine_state();
    state
        .tracks
        .write()
        .insert(1, Track::with_type(1, "track".into(), TrackType::Audio));
    state
        .clips
        .write()
        .push(audio_clip(1, 1, 0, tone(SR as usize)));
    state
}

/// Read back a 32-bit float stereo WAV as interleaved samples.
fn read_wav(path: &std::path::Path) -> (hound::WavSpec, Vec<f32>) {
    let reader = hound::WavReader::open(path).expect("freeze cache WAV must open");
    let spec = reader.spec();
    let samples = reader
        .into_samples::<f32>()
        .collect::<Result<Vec<_>, _>>()
        .expect("samples must decode");
    (spec, samples)
}

fn tmp_path(name: &str) -> std::path::PathBuf {
    // Per-test unique name under the OS temp dir; cleaned up explicitly.
    std::env::temp_dir().join(format!("resonance_freeze_test_{name}.wav"))
}

#[test]
fn known_track_renders_non_silent_wav() {
    let state = state_with_tone_track();
    let path = tmp_path("non_silent");
    let _ = std::fs::remove_file(&path);

    let mut fractions = Vec::new();
    let cache = to_freeze_cache(
        1,
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
        &mut |f| fractions.push(f),
    )
    .expect("freeze of a populated track must succeed");

    // Returned ref describes the file we just wrote.
    assert_eq!(cache.sample_rate, SR);
    assert_eq!(cache.bit_depth, 32);
    assert_eq!(cache.cache_filename, "resonance_freeze_test_non_silent.wav");
    assert!(cache.is_valid(), "fresh freeze cache must be Frozen/valid");
    assert_ne!(cache.render_fingerprint, 0, "fingerprint must be populated");

    // The WAV is a correct 32-bit float stereo file with audible audio.
    let (spec, samples) = read_wav(&path);
    assert_eq!(spec.channels, 2);
    assert_eq!(spec.sample_rate, SR);
    assert_eq!(spec.bits_per_sample, 32);
    let peak = samples.iter().cloned().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.1, "rendered audio must be non-silent (peak {peak})");

    // Progress is emitted, starts at 0.0, ends at 1.0, and is monotonic.
    assert_eq!(fractions.first().copied(), Some(0.0));
    assert_eq!(fractions.last().copied(), Some(1.0));
    assert!(
        fractions.windows(2).all(|w| w[1] >= w[0]),
        "progress must be monotonically non-decreasing: {fractions:?}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn freeze_does_not_mutate_source_clips() {
    let state = state_with_tone_track();
    let path = tmp_path("no_mutate");
    let _ = std::fs::remove_file(&path);

    // Snapshot the source clip before freezing.
    let before: Vec<f32> = match &state.clips.read()[0].source {
        ClipSource::Memory(v) => v.clone(),
        _ => unreachable!(),
    };
    let clip_count_before = state.clips.read().len();
    let track_count_before = state.tracks.read().len();

    to_freeze_cache(
        1,
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
        &mut |_| {},
    )
    .expect("freeze must succeed");

    // No clip / track was added, removed, or mutated.
    assert_eq!(state.clips.read().len(), clip_count_before);
    assert_eq!(state.tracks.read().len(), track_count_before);
    let after: Vec<f32> = match &state.clips.read()[0].source {
        ClipSource::Memory(v) => v.clone(),
        _ => unreachable!(),
    };
    assert_eq!(before, after, "source PCM must be byte-identical after freeze");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn fingerprint_changes_when_notes_change() {
    // Two MIDI tracks rendered identically except for one note's pitch
    // must produce different fingerprints (staleness detection relies on
    // this). Use MIDI clips so the note data feeds the fingerprint.
    fn fingerprint_with_note(pitch: u8) -> u64 {
        let state = empty_engine_state();
        state
            .tracks
            .write()
            .insert(1, Track::with_type(1, "t".into(), TrackType::Audio));
        // A tone clip so the render range is non-empty, plus a MIDI clip
        // whose note drives the fingerprint.
        state
            .clips
            .write()
            .push(audio_clip(1, 1, 0, tone(SR as usize)));
        state.midi_clips.write().push(MidiClip {
            id: 1,
            track_id: 1,
            start_sample: 0,
            duration_ticks: 480,
            notes: vec![MidiNote {
                note: pitch,
                velocity: 0.8,
                start_tick: 0,
                duration_ticks: 240,
            }],
            name: "m".into(),
            trim_start_ticks: 0,
            trim_end_ticks: 0,
        });
        let path = tmp_path(&format!("fp_{pitch}"));
        let _ = std::fs::remove_file(&path);
        let cache = to_freeze_cache(
            1,
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
            &mut |_| {},
        )
        .expect("freeze must succeed");
        let _ = std::fs::remove_file(&path);
        cache.render_fingerprint
    }

    assert_ne!(
        fingerprint_with_note(60),
        fingerprint_with_note(67),
        "changing a note's pitch must change the freeze fingerprint"
    );
}

#[test]
fn cancel_aborts_and_removes_partial_file() {
    let state = state_with_tone_track();
    let path = tmp_path("cancel");
    let _ = std::fs::remove_file(&path);

    // The renderer clears any stale cancel flag at start, so cancellation
    // must arrive mid-render. The progress callback fires inside the
    // render loop — flip the flag from there and the next chunk's
    // cooperative check aborts (mirrors the engine thread flipping it).
    let shared = Arc::clone(&state.shared);
    let result = to_freeze_cache(
        1,
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
        &mut |_| shared.bounce_cancel.store(true, Ordering::SeqCst),
    );

    match result {
        Err(msg) => assert!(
            msg.contains("cancel"),
            "cancel must name the reason, got: {msg}"
        ),
        Ok(_) => panic!("pre-armed cancel must abort the freeze"),
    }
    assert!(!path.exists(), "cancelled freeze must remove the partial WAV");
    // The cancel flag is reset so the next freeze starts fresh.
    assert!(!state.shared.bounce_cancel.load(Ordering::SeqCst));
}

#[test]
fn freeze_refuses_while_transport_playing() {
    let state = state_with_tone_track();
    state.shared.playing.store(true, Ordering::SeqCst);
    let path = tmp_path("playing");

    let result = to_freeze_cache(
        1,
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
        &mut |_| {},
    );

    match result {
        Err(msg) => assert!(msg.contains("Stop transport"), "got: {msg}"),
        Ok(_) => panic!("freeze must refuse while the transport plays"),
    }
    assert!(!path.exists(), "guarded freeze must not create a file");
}

#[test]
fn missing_source_track_errors() {
    let state = empty_engine_state();
    let path = tmp_path("missing");

    let result = to_freeze_cache(
        42,
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
        &mut |_| {},
    );

    match result {
        Err(msg) => assert!(msg.contains("not found"), "got: {msg}"),
        Ok(_) => panic!("freeze of a missing track must error"),
    }
}
