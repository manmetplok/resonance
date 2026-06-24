//! Vocal-tuning applied on the render + bounce path (ba todo #358, doc #160).
//!
//! Verifies that a clip carrying [`VocalTuning`] edits is rendered with
//! formant-preserving pitch correction in the shared mix loop (exercised
//! here through `render_stem`, the same `mix_track_clips` path the live
//! mixer and every bounce/export use), while:
//!
//! * an untuned clip renders bit-for-bit unchanged (zero-overhead path);
//! * the original PCM in the [`ClipSource`] is never mutated
//!   (non-destructive);
//! * a retuned clip's detected pitch shifts toward its target.
//!
//! Plugin-free: tracks carry a synthesised harmonic tone, so no CLAP host
//! or audio device is needed and the detector sees a clean monophonic pitch.

use std::sync::Arc;

use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use resonance_audio::__test_support::{
    ensure_tuning_caches, pitch_ratio_curve, render_stem, SharedState, StemSource,
    SyncClapInstance,
};
use resonance_audio::analyze_pitch;
use resonance_audio::types::*;

const SR: u32 = 48_000;

/// MIDI note number of `hz` (A4 = 69 = 440 Hz).
fn hz_to_midi(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// Synthesise a mono harmonic tone (fundamental + 7 overtones at 1/k
/// amplitude) at `f0` Hz — a vocal-like spectrum with a broad envelope, so
/// the formant-preserving shifter actually moves the harmonic comb (a pure
/// sine's lone partial would sit on its own formant peak and barely move).
fn harmonic_mono(f0: f32, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames];
    for (n, s) in out.iter_mut().enumerate() {
        let t = n as f32 / SR as f32;
        let mut v = 0.0f32;
        for k in 1..=8 {
            v += (1.0 / k as f32) * (std::f32::consts::TAU * f0 * k as f32 * t).sin();
        }
        *s = v * 0.25;
    }
    out
}

/// Interleave a mono buffer into stereo (both channels identical).
fn to_stereo(mono: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0f32; mono.len() * 2];
    for (i, &m) in mono.iter().enumerate() {
        out[i * 2] = m;
        out[i * 2 + 1] = m;
    }
    out
}

/// Collapse interleaved stereo to its mono mix for the pitch detector.
fn mono_mix(stereo: &[f32]) -> Vec<f32> {
    stereo.chunks_exact(2).map(|lr| (lr[0] + lr[1]) * 0.5).collect()
}

/// Detected mean pitch (MIDI) of the longest note in a mono buffer, or
/// `None` if the detector found nothing voiced.
fn detected_pitch(mono: &[f32]) -> Option<f32> {
    let (_contour, notes) = analyze_pitch(mono, SR);
    notes
        .iter()
        .max_by_key(|n| n.duration_frames())
        .map(|n| n.mean_pitch_midi)
}

struct Engine {
    shared: Arc<SharedState>,
    tracks: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: Arc<RwLock<MasterBus>>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<RwLock<Vec<MidiClip>>>,
    plugins: Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
}

impl Engine {
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

    fn add_master_track(&self, id: TrackId) {
        let t = Track::new(id, format!("track {id}"));
        t.set_output(TrackOutput::Master);
        self.tracks.write().insert(id, t);
    }

    fn push_clip(&self, clip: AudioClip) {
        self.clips.write().push(clip);
    }

    fn render(&self, source: StemSource, start: u64, end: u64) -> Vec<f32> {
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
        .expect("render_stem")
    }
}

/// Build a tone clip on `track`. `tuning` is the optional non-destructive
/// vocal-tuning model attached to it.
fn tone_clip(
    id: ClipId,
    track: TrackId,
    f0: f32,
    frames: usize,
    tuning: Option<VocalTuning>,
) -> AudioClip {
    AudioClip {
        id,
        track_id: track,
        start_sample: 0,
        source: ClipSource::Memory(to_stereo(&harmonic_mono(f0, frames))),
        name: "tone".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        vocal_tuning: tuning,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: WarpAlgorithm::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

/// A full-clip note blob at detected `mean_midi`, pulled `semitones` up at
/// full per-note + global strength (chromatic, so the target is exactly the
/// dragged pitch with no scale snap).
fn full_clip_tuning(mean_midi: f32, frames: usize, semitones: f32) -> VocalTuning {
    let mut vt = VocalTuning {
        notes: vec![NoteBlob {
            start_frame: 0,
            end_frame: frames as u64,
            mean_pitch_midi: mean_midi,
            cents_contour: Vec::new(),
            edit: NoteEdit {
                semitone_offset: semitones,
                correction_strength: 1.0,
                drift: 1.0,
                timing_nudge_frames: 0,
            },
        }],
        ..Default::default()
    };
    vt.global.correction_amount = 1.0;
    vt
}

// ---- the headline test: a tuned clip bounces with shifted pitch ----------

#[test]
fn retuned_clip_bounces_with_shifted_pitch() {
    // ~0.5 s of A3 (220 Hz, MIDI ≈ 57).
    let frames = SR as usize / 2;
    let f0 = 220.0;
    let detected_in = hz_to_midi(f0); // ≈ 57.0

    let eng = Engine::new();
    eng.add_master_track(1);
    // Pull the note up +5 semitones (A3 → D4).
    let tuning = full_clip_tuning(detected_in, frames, 5.0);
    eng.push_clip(tone_clip(1, 1, f0, frames, Some(tuning)));

    let rendered = eng.render(StemSource::Track(1), 0, frames as u64);
    let pitch_after = detected_pitch(&mono_mix(&rendered)).expect("detector finds a voiced note");

    // The detected pitch should land near the +5-semitone target. Allow a
    // semitone of slack for the phase-vocoder + detector error.
    let target = detected_in + 5.0;
    assert!(
        (pitch_after - target).abs() < 1.0,
        "retuned pitch {pitch_after:.2} should land near target {target:.2} \
         (detected_in {detected_in:.2})"
    );
    assert!(
        pitch_after - detected_in > 3.5,
        "retuned pitch {pitch_after:.2} must be clearly above the input {detected_in:.2}"
    );
}

// ---- non-destructive: the original PCM is never mutated ------------------

#[test]
fn tuning_does_not_mutate_source_pcm() {
    let frames = SR as usize / 4;
    let f0 = 220.0;
    let original = ClipSource::Memory(to_stereo(&harmonic_mono(f0, frames))).as_frames().to_vec();

    let eng = Engine::new();
    eng.add_master_track(1);
    eng.push_clip(tone_clip(1, 1, f0, frames, Some(full_clip_tuning(hz_to_midi(f0), frames, 5.0))));

    // Render (builds the corrected cache), then confirm the clip's *source*
    // PCM is byte-identical to the freshly-synthesised reference.
    let _ = eng.render(StemSource::Track(1), 0, frames as u64);
    let clips = eng.clips.read();
    let clip = &clips[0];
    assert_eq!(
        clip.source.as_frames(),
        original.as_slice(),
        "the original ClipSource PCM must be untouched by tuning"
    );
    assert!(
        clip.tuning_render_cache.is_some(),
        "the retuned clip should hold a derived render cache"
    );
}

// ---- zero-overhead: an untuned clip renders unchanged --------------------

#[test]
fn untuned_clip_renders_without_cache_and_keeps_pitch() {
    let frames = SR as usize / 2;
    let f0 = 220.0;

    let eng = Engine::new();
    eng.add_master_track(1);
    eng.push_clip(tone_clip(1, 1, f0, frames, None));

    let rendered = eng.render(StemSource::Track(1), 0, frames as u64);
    let pitch = detected_pitch(&mono_mix(&rendered)).expect("voiced note");
    assert!(
        (pitch - hz_to_midi(f0)).abs() < 1.0,
        "untuned clip keeps its source pitch (got {pitch:.2}, want {:.2})",
        hz_to_midi(f0)
    );

    assert!(
        eng.clips.read()[0].tuning_render_cache.is_none(),
        "an untuned clip must not allocate a render cache"
    );
}

// ---- an analysed-but-unedited (identity) clip is left on the source path -

#[test]
fn identity_tuning_keeps_zero_overhead_path() {
    let frames = SR as usize / 4;
    let f0 = 220.0;

    let eng = Engine::new();
    eng.add_master_track(1);
    // Analysed (a note blob present) but no edits: identity model.
    let mut vt = VocalTuning::default();
    vt.notes.push(NoteBlob {
        start_frame: 0,
        end_frame: frames as u64,
        mean_pitch_midi: hz_to_midi(f0),
        cents_contour: Vec::new(),
        edit: NoteEdit::default(),
    });
    assert!(!vt.has_edits(), "default edits are identity");
    eng.push_clip(tone_clip(1, 1, f0, frames, Some(vt)));

    let built = ensure_tuning_caches(&eng.clips, SR);
    assert_eq!(built, 0, "identity tuning must build no cache");
    assert!(eng.clips.read()[0].tuning_render_cache.is_none());
}

// ---- the pure ratio model (no FFT) ---------------------------------------

#[test]
fn ratio_curve_is_unity_for_identity_and_shifts_under_correction() {
    let frames = 1_000usize;

    // Identity tuning → unity ratio everywhere.
    let identity = VocalTuning::default();
    let curve = pitch_ratio_curve(&identity, frames);
    assert!(curve.iter().all(|&r| (r - 1.0).abs() < 1e-6), "identity → unity ratio");

    // A note pulled +12 semitones at full strength → ratio 2.0 across it.
    let vt = full_clip_tuning(57.0, frames, 12.0);
    let curve = pitch_ratio_curve(&vt, frames);
    assert!(
        curve.iter().all(|&r| (r - 2.0).abs() < 1e-3),
        "full +12 correction → octave-up ratio (2.0)"
    );

    // Half the global correction amount → half the semitone delta (ratio
    // 2^(6/12) ≈ 1.4142).
    let mut half = full_clip_tuning(57.0, frames, 12.0);
    half.global.correction_amount = 0.5;
    let curve = pitch_ratio_curve(&half, frames);
    let expected = 2.0f32.powf(6.0 / 12.0);
    assert!(
        curve.iter().all(|&r| (r - expected).abs() < 1e-3),
        "half global amount halves the semitone delta"
    );
}

// ---- scale snap pulls a drag to the nearest in-key degree ----------------

#[test]
fn scale_snap_targets_in_key_degree() {
    let frames = 500usize;
    // Detected C4 (MIDI 60); drag +1.4 semitones → ~61.4. In C major the
    // nearest in-key degree is D (62), not C# (61). Full correction lands
    // the note on the snapped grid, so the ratio reflects 60 → 62.
    let mut vt = full_clip_tuning(60.0, frames, 1.4);
    vt.global.key = 0; // C
    vt.global.scale = TuningScale::Major;
    let curve = pitch_ratio_curve(&vt, frames);
    let expected = 2.0f32.powf(2.0 / 12.0); // +2 semitones (C→D)
    assert!(
        curve.iter().all(|&r| (r - expected).abs() < 1e-3),
        "C major snaps the +1.4 drag to D (a +2 semitone shift), got {}",
        curve[0]
    );
}
