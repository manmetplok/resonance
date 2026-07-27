//! Tests for note-blob segmentation of an f0 contour.

use resonance_dsp::{detect_f0, segment_notes, F0Config, F0Frame, SegmentConfig};
use std::f32::consts::TAU;

const HOP_SECS: f32 = 256.0 / 48_000.0;

fn midi_to_hz(midi: f32) -> f32 {
    440.0 * 2.0_f32.powf((midi - 69.0) / 12.0)
}

/// Build a contour frame at hop index `i`. A `None` MIDI value is an unvoiced
/// (gap) frame.
fn frame(i: usize, midi: Option<f32>) -> F0Frame {
    match midi {
        Some(m) => F0Frame {
            time_secs: i as f32 * HOP_SECS,
            f0_hz: midi_to_hz(m),
            confidence: 0.9,
            voiced: true,
        },
        None => F0Frame {
            time_secs: i as f32 * HOP_SECS,
            f0_hz: 0.0,
            confidence: 0.0,
            voiced: false,
        },
    }
}

/// Build a contour from `(midi, count)` segments; a `None` MIDI is a gap.
fn contour(segments: &[(Option<f32>, usize)]) -> Vec<F0Frame> {
    let mut out = Vec::new();
    for &(midi, count) in segments {
        for _ in 0..count {
            let i = out.len();
            out.push(frame(i, midi));
        }
    }
    out
}

#[test]
fn empty_contour_yields_no_blobs() {
    assert!(segment_notes(&[], SegmentConfig::default()).is_empty());
}

#[test]
fn all_unvoiced_yields_no_blobs() {
    let c = contour(&[(None, 50)]);
    assert!(segment_notes(&c, SegmentConfig::default()).is_empty());
}

#[test]
fn detached_phrase_splits_on_silence() {
    // C4, E4, G4 separated by 10-frame silences (> max_gap_frames).
    let c = contour(&[
        (Some(60.0), 30),
        (None, 10),
        (Some(64.0), 30),
        (None, 10),
        (Some(67.0), 30),
    ]);
    let blobs = segment_notes(&c, SegmentConfig::default());
    let notes: Vec<u8> = blobs.iter().map(|b| b.note).collect();
    assert_eq!(notes, vec![60, 64, 67], "expected three detached notes");

    // Boundaries land on the voiced runs (silence excluded).
    assert_eq!((blobs[0].onset_frame, blobs[0].offset_frame), (0, 29));
    assert_eq!((blobs[1].onset_frame, blobs[1].offset_frame), (40, 69));
    assert_eq!((blobs[2].onset_frame, blobs[2].offset_frame), (80, 109));
}

#[test]
fn legato_phrase_splits_on_pitch_steps() {
    // Contiguous C4 -> D4 -> E4 with no silence between.
    let c = contour(&[(Some(60.0), 30), (Some(62.0), 30), (Some(64.0), 30)]);
    let blobs = segment_notes(&c, SegmentConfig::default());
    let notes: Vec<u8> = blobs.iter().map(|b| b.note).collect();
    assert_eq!(notes, vec![60, 62, 64], "expected three legato notes");

    // Each step opens a new blob exactly at the new note's first frame.
    assert_eq!(blobs[0].onset_frame, 0);
    assert_eq!(blobs[1].onset_frame, 30);
    assert_eq!(blobs[2].onset_frame, 60);
}

#[test]
fn vibrato_stays_in_one_blob() {
    // C4 with ±0.45-semitone vibrato: oscillates but never settles elsewhere.
    let n = 72;
    let c: Vec<F0Frame> = (0..n)
        .map(|i| frame(i, Some(60.0 + 0.45 * (TAU * i as f32 / 12.0).sin())))
        .collect();
    let blobs = segment_notes(&c, SegmentConfig::default());
    assert_eq!(blobs.len(), 1, "vibrato must not fragment the note");
    assert_eq!(blobs[0].note, 60);

    // The contour carries the vibrato, centred near zero.
    let peak = blobs[0]
        .cents_contour
        .iter()
        .fold(0.0_f32, |a, &c| a.max(c.abs()));
    assert!(
        peak > 20.0,
        "vibrato should be visible in the contour: {peak}"
    );
    assert!(blobs[0].cents_offset.abs() < 8.0, "mean should be ~0");
}

#[test]
fn single_frame_octave_glitch_is_rejected() {
    // Steady C4 with one frame jumping an octave (classic YIN octave error).
    let mut c = contour(&[(Some(60.0), 40)]);
    c[20] = frame(20, Some(72.0));
    let blobs = segment_notes(&c, SegmentConfig::default());
    assert_eq!(blobs.len(), 1, "a lone glitch must not spawn a note");
    assert_eq!(blobs[0].note, 60);

    // Median smoothing removes the spike from the pitch statistics.
    let peak = blobs[0]
        .cents_contour
        .iter()
        .fold(0.0_f32, |a, &c| a.max(c.abs()));
    assert!(peak < 20.0, "glitch leaked into the contour: {peak}");
}

#[test]
fn mean_pitch_reports_cents_deviation() {
    // A note tuned 30 cents sharp of C4.
    let c = contour(&[(Some(60.30), 40)]);
    let blobs = segment_notes(&c, SegmentConfig::default());
    assert_eq!(blobs.len(), 1);
    assert_eq!(blobs[0].note, 60);
    assert!(
        (blobs[0].cents_offset - 30.0).abs() < 2.0,
        "expected ~+30 cents, got {}",
        blobs[0].cents_offset
    );
    assert!(
        (blobs[0].midi - 60.30).abs() < 0.05,
        "mean MIDI off: {}",
        blobs[0].midi
    );
}

#[test]
fn short_blip_is_filtered_out() {
    // Two voiced frames in a sea of silence: below min_blob_frames.
    let c = contour(&[(None, 20), (Some(60.0), 2), (None, 20)]);
    assert!(segment_notes(&c, SegmentConfig::default()).is_empty());
}

#[test]
fn brief_unvoiced_dip_is_bridged() {
    // A 3-frame unvoiced dip (e.g. a consonant) inside one sustained note must
    // not split it, given default max_gap_frames = 4.
    let c = contour(&[(Some(62.0), 20), (None, 3), (Some(62.0), 20)]);
    let blobs = segment_notes(&c, SegmentConfig::default());
    assert_eq!(blobs.len(), 1, "short dip should be bridged");
    assert_eq!(blobs[0].onset_frame, 0);
    assert_eq!(blobs[0].offset_frame, 42);
}

#[test]
fn blob_bounds_are_ordered_and_timed() {
    let c = contour(&[(Some(60.0), 30), (None, 10), (Some(67.0), 30)]);
    let blobs = segment_notes(&c, SegmentConfig::default());
    for b in &blobs {
        assert!(b.onset_frame <= b.offset_frame);
        assert!(b.offset_secs >= b.onset_secs);
        assert_eq!(b.cents_contour.len(), b.offset_frame - b.onset_frame + 1);
        // onset_secs matches the frame's own timestamp.
        assert!((b.onset_secs - c[b.onset_frame].time_secs).abs() < 1e-6);
    }
}

#[test]
fn segments_real_detected_contour() {
    // End-to-end: two sung tones (A3, E4) separated by a short silence.
    const SR: f32 = 48_000.0;
    let sine = |freq: f32, dur: f32| -> Vec<f32> {
        let n = (SR * dur) as usize;
        (0..n)
            .map(|i| 0.5 * (TAU * freq * i as f32 / SR).sin())
            .collect()
    };
    let mut audio = sine(220.0, 0.35); // A3 -> MIDI 57
    audio.extend(std::iter::repeat_n(0.0, (SR * 0.12) as usize));
    audio.extend(sine(329.63, 0.35)); // E4 -> MIDI 64

    let frames = detect_f0(&audio, F0Config::new(SR));
    let blobs = segment_notes(&frames, SegmentConfig::default());
    let notes: Vec<u8> = blobs.iter().map(|b| b.note).collect();
    assert_eq!(notes, vec![57, 64], "expected A3 then E4, got {notes:?}");
    // First note clearly precedes the second in time.
    assert!(blobs[0].offset_secs < blobs[1].onset_secs);
}
