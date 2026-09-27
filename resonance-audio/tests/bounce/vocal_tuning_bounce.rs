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

use resonance_audio::test_support::{
    attach_tuning_caches, build_tuning_caches, ensure_tuning_caches, pitch_ratio_curve,
    render_stem, snapshot_tuning_jobs, SharedState, StemSource,
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
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
}

impl Engine {
    fn new() -> Self {
        Self {
            shared: Arc::new(SharedState::default()),
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
        }
    }

    fn add_master_track(&self, id: TrackId) {
        let mut t = Track::new(id, format!("track {id}"));
        t.set_output(TrackOutput::Master);
        self.shared.edit_tracks(|m| { m.insert(id, std::sync::Arc::new(t)); });
    }

    fn push_clip(&self, clip: AudioClip) {
        self.shared.edit_clips(|c| c.push(Arc::new(clip)));
    }

    fn render(&self, source: StemSource, start: u64, end: u64) -> Vec<f32> {
        render_stem(
            source,
            start,
            end,
            &self.shared,
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
        source: ClipSource::memory(to_stereo(&harmonic_mono(f0, frames))),
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
    let original = ClipSource::memory(to_stereo(&harmonic_mono(f0, frames))).as_frames().to_vec();

    let eng = Engine::new();
    eng.add_master_track(1);
    eng.push_clip(tone_clip(1, 1, f0, frames, Some(full_clip_tuning(hz_to_midi(f0), frames, 5.0))));

    // Render (builds the corrected cache), then confirm the clip's *source*
    // PCM is byte-identical to the freshly-synthesised reference.
    let _ = eng.render(StemSource::Track(1), 0, frames as u64);
    let clips = eng.shared.clips();
    assert_eq!(
        clips[0].source.as_frames(),
        original.as_slice(),
        "the original ClipSource PCM must be untouched by tuning"
    );
    // The render reads the retune through an overlay of derived caches
    // (and posts them for the engine thread to attach — ARCH-02 B-5); the
    // retuned clip it renders holds a derived cache beside its source.
    let overlay = ensure_tuning_caches(&eng.shared, SR);
    let rendered = overlay.apply(&clips);
    assert!(
        rendered[0].tuning_render_cache.is_some(),
        "the retuned clip should hold a derived render cache"
    );
    assert_eq!(rendered[0].source.as_frames(), original.as_slice());
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
        eng.shared.clips()[0].tuning_render_cache.is_none(),
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

    let overlay = ensure_tuning_caches(&eng.shared, SR);
    assert_eq!(overlay.rebuilt(), 0, "identity tuning must build no cache");
    assert!(overlay.is_empty(), "nothing to patch or post");
    assert!(eng.shared.clips()[0].tuning_render_cache.is_none());
}

// ---- the off-lock cache pass (code review ARCH-02 A2-3) ------------------

/// Two tuned clips, one untuned clip with a stale cache, one plain clip.
fn cache_pass_fixture() -> Engine {
    let frames = SR as usize / 8;
    let eng = Engine::new();
    eng.add_master_track(1);
    eng.push_clip(tone_clip(1, 1, 220.0, frames, Some(full_clip_tuning(hz_to_midi(220.0), frames, 2.0))));
    eng.push_clip(tone_clip(2, 1, 330.0, frames, Some(full_clip_tuning(hz_to_midi(330.0), frames, -1.0))));
    let mut stale = tone_clip(3, 1, 440.0, frames, None);
    stale.tuning_render_cache = Some(vec![0.5; frames * 2].into());
    eng.push_clip(stale);
    eng.push_clip(tone_clip(4, 1, 110.0, frames, None));
    eng
}

fn caches(clips: &[Arc<AudioClip>]) -> Vec<(ClipId, Option<Vec<f32>>)> {
    clips
        .iter()
        .map(|c| (c.id, c.tuning_render_cache.as_deref().map(<[f32]>::to_vec)))
        .collect()
}

#[test]
fn three_phase_cache_pass_matches_the_single_call_bitwise() {
    let single = cache_pass_fixture();
    let phased = cache_pass_fixture();

    let overlay = ensure_tuning_caches(&single.shared, SR);
    let built_single = overlay.rebuilt();

    let mut list = phased.shared.clips().to_vec();
    let jobs = snapshot_tuning_jobs(&list);
    // Two retunes plus one stale clear; the plain clip is not a job.
    assert_eq!(jobs.len(), 3);
    assert_eq!(jobs.iter().filter(|j| j.retune.is_some()).count(), 2);
    assert_eq!(jobs.iter().find(|j| j.clip_id == 3).map(|j| j.retune.is_none()), Some(true));
    let built = build_tuning_caches(jobs, SR);
    let built_phased = attach_tuning_caches(&mut list, &built);

    assert_eq!(built_single, 2);
    assert_eq!(built_phased, 2);
    let a = caches(&overlay.apply(&single.shared.clips()));
    let b = caches(&list);
    assert_eq!(a, b, "the phased pass must produce the identical caches");
    assert!(a[0].1.is_some() && a[1].1.is_some());
    assert!(a[2].1.is_none(), "stale cache cleared");
    assert!(a[3].1.is_none(), "untuned clip stays cache-free");
}

#[test]
fn resynthesis_runs_off_the_published_graph_and_attaches_copy_on_write() {
    // The freeze worker used to take the clip list's write lock for the
    // whole FFT pass (A2-3 shrank it to the attach). Since ARCH-02 B-5 the
    // worker reads the published graph — an `Arc` it holds, no lock — and
    // never attaches at all: the engine thread does, copy-on-write, so the
    // graph a reader pinned is never touched.
    let eng = cache_pass_fixture();
    let pinned = eng.shared.clips();
    let clips = eng.shared.clips();
    let built = std::thread::spawn(move || {
        let jobs = snapshot_tuning_jobs(&clips);
        build_tuning_caches(jobs, SR)
    })
    .join()
    .expect("worker");
    assert_eq!(built.len(), 3);

    let mut list = pinned.to_vec();
    assert_eq!(attach_tuning_caches(&mut list, &built), 2);
    assert!(list[0].tuning_render_cache.is_some());
    assert!(
        pinned[0].tuning_render_cache.is_none(),
        "the pinned graph's clip is not mutated by the attach"
    );
    assert!(
        Arc::ptr_eq(&list[3], &pinned[3]),
        "a clip the pass leaves alone is shared, not copied"
    );
}

#[test]
fn attach_skips_a_clip_removed_since_the_snapshot() {
    let eng = cache_pass_fixture();
    let mut list = eng.shared.clips().to_vec();
    let jobs = snapshot_tuning_jobs(&list);
    let built = build_tuning_caches(jobs, SR);
    // Clip 2 is deleted between snapshot and attach (an engine-thread
    // edit landing mid-pass).
    list.retain(|c| c.id != 2);
    assert_eq!(attach_tuning_caches(&mut list, &built), 1);
    let after = caches(&list);
    assert_eq!(after.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![1, 3, 4]);
    assert!(after[0].1.is_some());
    assert!(after[1].1.is_none());
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
