//! Stem render core (ba todo #322).
//!
//! Exercises the building blocks for "export stems":
//!
//! * `stem_filter` resolves the right contributing-track set for a
//!   track (incl. sub-tracks), a bus (all top-level tracks routed to
//!   it, incl. their sub-tracks) and master (everything).
//! * `render_stem` renders an isolated source over a SHARED render
//!   range so two track stems come out equal-length and sample-aligned,
//!   each carrying only its own audio.
//! * `write_stem_wav` emits valid WAV headers for every bit depth and
//!   honours the target sample rate (resample vs passthrough).
//!
//! Plugin-free: tracks carry plain DC audio clips, so no CLAP host or
//! audio device is needed and the rendered samples are exactly known.

use std::sync::Arc;

use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use resonance_audio::__test_support::{
    render_stem, stem_filter, stem_project_range, write_stem_wav, SharedState, StemBitDepth,
    StemSource, SyncClapInstance,
};
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

impl EngineState {
    fn new() -> Self {
        Self {
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

    fn add_track(&self, id: TrackId, output: TrackOutput) {
        let t = Track::new(id, format!("track {id}"));
        t.set_output(output);
        self.tracks.write().insert(id, t);
    }

    /// Push a constant-`value` DC clip on `track` over `[start, start+frames)`.
    fn add_dc_clip(&self, id: ClipId, track: TrackId, start: u64, frames: usize, value: f32) {
        self.clips.write().push(AudioClip {
            id,
            track_id: track,
            start_sample: start,
            source: ClipSource::Memory(vec![value; frames * 2]),
            name: "dc".into(),
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
            warp_algorithm: WarpAlgorithm::default(),
            warp_markers: Vec::new(),
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn render(&self, source: StemSource, start: u64, end: u64) -> Result<Vec<f32>, String> {
        render_stem(
            source,
            start,
            end,
            &self.shared,
            &self.tracks,
            &self.busses,
            &self.master,
            &self.clips,
            &self.midi_clips,
            &self.plugins,
            &self.tempo_map,
            SR,
        )
    }
}

// ---- stem_filter routing rules -------------------------------------------

#[test]
fn track_filter_includes_track_and_its_sub_tracks() {
    let state = EngineState::new();
    state.add_track(1, TrackOutput::Master);
    state.add_track(2, TrackOutput::Master);
    // Sub-track of track 1 (multi-output instrument sibling).
    {
        let sub = Track::new_sub_track(10, "sub".into(), 1, 1);
        state.tracks.write().insert(10, sub);
    }

    let f = stem_filter(StemSource::Track(1), &state.tracks.read());
    assert!(f.contains(1), "the track itself contributes");
    assert!(f.contains(10), "its sub-track contributes");
    assert!(!f.contains(2), "an unrelated track does not");
    assert!(!f.include_master_fx, "track stems exclude master FX");
}

#[test]
fn bus_filter_includes_tracks_routed_to_bus_with_sub_tracks() {
    let state = EngineState::new();
    state.busses.write().insert(7, Bus::new(7, "reverb".into()));
    state.add_track(1, TrackOutput::Bus(7));
    state.add_track(2, TrackOutput::Master);
    state.add_track(3, TrackOutput::Bus(7));
    {
        let sub = Track::new_sub_track(11, "sub".into(), 1, 1);
        state.tracks.write().insert(11, sub);
    }

    let f = stem_filter(StemSource::Bus(7), &state.tracks.read());
    assert!(f.contains(1), "track routed to the bus contributes");
    assert!(f.contains(3), "second track routed to the bus contributes");
    assert!(f.contains(11), "sub-track of a routed track contributes");
    assert!(!f.contains(2), "a master-routed track is excluded");
    assert!(!f.include_master_fx, "bus stems exclude master FX");
}

#[test]
fn master_filter_includes_everything_with_master_fx() {
    let state = EngineState::new();
    state.add_track(1, TrackOutput::Master);
    state.add_track(2, TrackOutput::Bus(7));

    let f = stem_filter(StemSource::Master, &state.tracks.read());
    assert!(f.all, "master contributes every track");
    assert!(f.contains(1) && f.contains(2) && f.contains(999));
    assert!(f.include_master_fx, "master stem applies master FX/volume");
}

// ---- render_stem: shared range, alignment, isolation ---------------------

#[test]
fn two_track_stems_over_shared_range_are_equal_length_and_aligned() {
    let state = EngineState::new();
    state.add_track(1, TrackOutput::Master);
    state.add_track(2, TrackOutput::Master);
    // Two clips at different positions; the shared range spans both.
    state.add_dc_clip(1, 1, 0, 100, 0.5); // track 1: frames [0,100)
    state.add_dc_clip(2, 2, 200, 100, 0.25); // track 2: frames [200,300)

    let (start, end) = stem_project_range(
        &state.clips,
        &state.midi_clips,
        &state.tempo_map,
        SR,
    )
    .expect("project has clips");
    assert_eq!(start, 0);
    assert_eq!(end, 300);

    let stem1 = state.render(StemSource::Track(1), start, end).unwrap();
    let stem2 = state.render(StemSource::Track(2), start, end).unwrap();

    // Equal length: both stems span the common range exactly.
    let expected_len = (end - start) as usize * 2;
    assert_eq!(stem1.len(), expected_len);
    assert_eq!(stem2.len(), expected_len);

    // Sample alignment: stem 1's clip sits at frames [0,100) and is
    // silent afterwards; stem 2's clip sits at frames [200,300) and is
    // silent before. Both share frame 0 as the zero origin. (Absolute
    // levels are scaled by the track pan law, so assert presence vs
    // silence rather than exact amplitudes.)
    assert!(stem1[0].abs() > 0.1, "stem1 frame 0 carries its clip");
    assert!(stem1[99 * 2].abs() > 0.1, "stem1 clip runs to frame 99");
    assert!(stem1[200 * 2].abs() < 1e-6, "stem1 is silent where track 2 plays");
    assert!(stem2[0].abs() < 1e-6, "stem2 is silent where track 1 plays");
    assert!(stem2[199 * 2].abs() < 1e-6, "stem2 still silent at frame 199");
    assert!(
        stem2[200 * 2].abs() > 0.05,
        "stem2 frame 200 carries its clip, aligned to the shared origin"
    );
}

#[test]
fn stem_isolates_its_source_from_other_tracks() {
    let state = EngineState::new();
    state.add_track(1, TrackOutput::Master);
    state.add_track(2, TrackOutput::Master);
    // Two tracks with DIFFERENT clip levels over the same frames. If a
    // stem leaked the other track, both stems would carry the identical
    // summed level; isolated, each stem reflects only its own clip, so
    // their levels keep the source clips' 5:3 ratio.
    state.add_dc_clip(1, 1, 0, 64, 0.5);
    state.add_dc_clip(2, 2, 0, 64, 0.3);

    let stem1 = state.render(StemSource::Track(1), 0, 64).unwrap();
    let stem2 = state.render(StemSource::Track(2), 0, 64).unwrap();

    let l1 = stem1[0];
    let l2 = stem2[0];
    assert!(l1 > 0.0 && l2 > 0.0, "both stems carry their own audio");
    // Each stem is a constant DC level across the clip.
    for frame in 0..64 {
        assert!((stem1[frame * 2] - l1).abs() < 1e-6, "stem1 is constant DC");
        assert!((stem2[frame * 2] - l2).abs() < 1e-6, "stem2 is constant DC");
    }
    // Same pan law on both tracks ⇒ the level ratio is the clip ratio.
    // A leak would make l1 == l2 (both = the summed level).
    assert!(
        (l1 / l2 - 0.5 / 0.3).abs() < 1e-4,
        "stems keep the source 5:3 level ratio (l1={l1}, l2={l2}) — no cross-track leak"
    );
}

#[test]
fn render_stem_rejects_rolling_transport() {
    let state = EngineState::new();
    state.add_track(1, TrackOutput::Master);
    state.add_dc_clip(1, 1, 0, 64, 0.5);
    state
        .shared
        .playing
        .store(true, std::sync::atomic::Ordering::SeqCst);

    let err = state.render(StemSource::Track(1), 0, 64).unwrap_err();
    assert!(err.contains("transport"), "guard names the transport: {err}");
}

#[test]
fn render_stem_rejects_empty_range() {
    let state = EngineState::new();
    state.add_track(1, TrackOutput::Master);
    assert!(state.render(StemSource::Track(1), 100, 100).is_err());
    assert!(state.render(StemSource::Track(1), 200, 100).is_err());
}

// ---- write_stem_wav: bit depth + sample rate -----------------------------

fn tmp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("resonance_stem_test_{}_{name}.wav", std::process::id()));
    p
}

#[test]
fn wav_headers_are_valid_for_every_bit_depth() {
    // 8 stereo frames of a mid-level DC tone.
    let samples = vec![0.5f32; 8 * 2];

    for (name, depth, want_bits, want_float) in [
        ("i16", StemBitDepth::Int16, 16u16, false),
        ("i24", StemBitDepth::Int24, 24, false),
        ("f32", StemBitDepth::Float32, 32, true),
    ] {
        let path = tmp_path(name);
        write_stem_wav(path.to_str().unwrap(), &samples, SR, SR, depth).unwrap();

        let reader = hound::WavReader::open(&path).expect("written WAV must be readable");
        let spec = reader.spec();
        assert_eq!(spec.channels, 2, "{name}: stereo");
        assert_eq!(spec.sample_rate, SR, "{name}: passthrough sample rate");
        assert_eq!(spec.bits_per_sample, want_bits, "{name}: bit depth");
        let is_float = spec.sample_format == hound::SampleFormat::Float;
        assert_eq!(is_float, want_float, "{name}: sample format");
        // 8 frames * 2 channels = 16 samples regardless of encoding.
        assert_eq!(reader.len(), 16, "{name}: sample count");

        let _ = std::fs::remove_file(&path);
    }
}

#[test]
fn wav_resamples_only_when_target_rate_differs() {
    let frames = 100usize;
    let samples = vec![0.5f32; frames * 2];

    // Passthrough: same rate → identical sample count.
    let p_pass = tmp_path("pass");
    write_stem_wav(p_pass.to_str().unwrap(), &samples, SR, SR, StemBitDepth::Float32).unwrap();
    let pass = hound::WavReader::open(&p_pass).unwrap();
    assert_eq!(pass.spec().sample_rate, SR);
    assert_eq!(pass.len(), (frames * 2) as u32, "passthrough keeps frame count");
    let _ = std::fs::remove_file(&p_pass);

    // Downsample to half-rate: header reports the target rate and the
    // frame count drops roughly by half.
    let target = SR / 2;
    let p_rs = tmp_path("resample");
    write_stem_wav(p_rs.to_str().unwrap(), &samples, SR, target, StemBitDepth::Float32).unwrap();
    let rs = hound::WavReader::open(&p_rs).unwrap();
    assert_eq!(rs.spec().sample_rate, target, "header reports target rate");
    let out_frames = rs.len() / 2;
    assert!(
        out_frames < frames as u32 && out_frames >= (frames as u32 / 2).saturating_sub(2),
        "half-rate output should be ~50 frames, got {out_frames}"
    );
    let _ = std::fs::remove_file(&p_rs);
}
