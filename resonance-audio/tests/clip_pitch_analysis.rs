//! Drives the engine-internal vocal pitch-analysis path
//! (`engine/vocal_analysis.rs`) at the command boundary without spinning
//! up the engine thread:
//!
//! * `analyze_clip_pitch_in_place` reads a clip's mono mix, runs f0
//!   detection + note segmentation, stores the result in the clip's
//!   `VocalTuning` cache, and emits exactly one `ClipPitchDetected` whose
//!   payload mirrors the cache.
//! * The missing-clip lookup is a silent no-op (no cache write, no event).
//! * `analyze_pitch` (the pure DSP mapping) turns a mono sine into a
//!   contour and a note blob at the right MIDI pitch.
//!
//! A steady A3 (220 Hz) sine is the stimulus: the detector pins it to
//! MIDI 57 with sub-semitone accuracy, and the whole clip segments into
//! one sustained note.

use std::f32::consts::TAU;
use std::sync::Arc;

use crossbeam_channel::unbounded;
use parking_lot::RwLock;

use resonance_audio::types::{AudioClip, AudioEvent, ClipSource, FadeCurve};
use resonance_audio::{analyze_clip_pitch_in_place, analyze_pitch};

const SR: u32 = 48_000;
const A3_HZ: f32 = 220.0;
/// MIDI note number of A3 (220 Hz): `69 + 12·log2(220/440) = 57`.
const A3_MIDI: u8 = 57;

/// Stereo-interleaved `[l, r, l, r, …]` sine of `freq` Hz for `dur_secs`
/// (both channels identical), so the mono mix is the same sine.
fn stereo_sine(freq: f32, dur_secs: f32, amp: f32) -> Vec<f32> {
    let frames = (SR as f32 * dur_secs) as usize;
    let mut out = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let s = amp * (TAU * freq * i as f32 / SR as f32).sin();
        out.push(s);
        out.push(s);
    }
    out
}

fn sine_clip(id: u64, freq: f32, dur_secs: f32) -> AudioClip {
    AudioClip {
        id,
        track_id: 1,
        start_sample: 0,
        source: ClipSource::Memory(stereo_sine(freq, dur_secs, 0.5)),
        name: format!("clip_{id}"),
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
    }
}

#[test]
fn analyze_fills_cache_and_emits_matching_event() {
    let dur = 0.6;
    let total_frames = (SR as f32 * dur) as u64;
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sine_clip(7, A3_HZ, dur)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    analyze_clip_pitch_in_place(&clips, &event_tx, 7, SR);

    // Exactly one event, and it is the analysis result for clip 7.
    let (notes, contour) = match event_rx.try_recv() {
        Ok(AudioEvent::ClipPitchDetected {
            clip_id,
            notes,
            contour,
        }) => {
            assert_eq!(clip_id, 7);
            (notes, contour)
        }
        other => panic!("expected ClipPitchDetected, got {other:?}"),
    };
    assert!(
        event_rx.try_recv().is_err(),
        "exactly one event per analysis"
    );

    // The contour has voiced frames anchored inside the clip's audio.
    assert!(!contour.is_empty(), "a 0.6 s sine yields analysis frames");
    assert!(
        contour.iter().any(|f| f.voiced),
        "a steady tone must produce voiced frames"
    );
    assert!(
        contour.iter().all(|f| f.frame < total_frames),
        "frame anchors stay within the clip's PCM"
    );
    assert!(
        contour.windows(2).all(|w| w[1].frame >= w[0].frame),
        "contour frames are in ascending order"
    );

    // A sustained tone segments into at least one note pinned to A3.
    assert!(!notes.is_empty(), "a sustained tone yields a note blob");
    let note = &notes[0];
    assert_eq!(
        note.mean_pitch_midi.round() as u8, A3_MIDI,
        "220 Hz must read as MIDI {A3_MIDI} (got {})",
        note.mean_pitch_midi
    );
    assert!(
        note.start_frame < note.end_frame,
        "note spans a positive duration"
    );
    assert!(
        note.end_frame <= total_frames,
        "note offset stays within the clip"
    );
    assert!(
        note.edit == Default::default(),
        "a freshly detected note carries an identity edit"
    );

    // The emitted payload mirrors exactly what was cached on the clip.
    let guard = clips.read();
    let tuning = guard[0]
        .vocal_tuning
        .as_ref()
        .expect("analysis attaches a VocalTuning cache");
    assert_eq!(tuning.contour, contour, "cached contour mirrors the event");
    assert_eq!(tuning.notes, notes, "cached notes mirror the event");
}

#[test]
fn missing_clip_is_a_silent_no_op() {
    let clips: Arc<RwLock<Vec<AudioClip>>> = Arc::new(RwLock::new(vec![sine_clip(1, A3_HZ, 0.4)]));
    let (event_tx, event_rx) = unbounded::<AudioEvent>();

    // Clip id 99 does not exist.
    analyze_clip_pitch_in_place(&clips, &event_tx, 99, SR);

    assert!(
        event_rx.try_recv().is_err(),
        "an unknown clip emits no event"
    );
    assert!(
        clips.read()[0].vocal_tuning.is_none(),
        "an unknown clip leaves every existing clip's cache untouched"
    );
}

#[test]
fn analyze_pitch_maps_sine_to_a3_note() {
    let mono: Vec<f32> = {
        let stereo = stereo_sine(A3_HZ, 0.5, 0.5);
        stereo.chunks_exact(2).map(|lr| (lr[0] + lr[1]) * 0.5).collect()
    };

    let (contour, notes) = analyze_pitch(&mono, SR);

    assert!(contour.iter().any(|f| f.voiced));
    // Voiced frames sit close to 220 Hz.
    let voiced: Vec<f32> = contour.iter().filter(|f| f.voiced).map(|f| f.f0_hz).collect();
    let mean_hz = voiced.iter().sum::<f32>() / voiced.len() as f32;
    let cents = 1200.0 * (mean_hz / A3_HZ).log2();
    assert!(cents.abs() < 20.0, "mean f0 {mean_hz} Hz is {cents:.1} cents off A3");

    assert!(!notes.is_empty());
    assert_eq!(notes[0].mean_pitch_midi.round() as u8, A3_MIDI);
    // cents_contour is re-based to the blob mean, so it averages ~0.
    let cc = &notes[0].cents_contour;
    if !cc.is_empty() {
        let mean_cents = cc.iter().sum::<f32>() / cc.len() as f32;
        assert!(
            mean_cents.abs() < 5.0,
            "per-frame cents should average near zero around the mean (got {mean_cents})"
        );
    }
}
