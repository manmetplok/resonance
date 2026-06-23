//! Tests for the clip-warp data model (todo #417): the `AudioClip` warp
//! fields default to "no stretch" so existing clips are untouched, the
//! `WarpAlgorithm` / `WarpMarker` types serialize for project persistence,
//! and `AudioClip::warp_source_frame` maps a timeline read position to a
//! source read position both in the uniform (marker-free) and the
//! piecewise-linear (markered) cases.

use resonance_audio::types::{AudioClip, ClipSource, FadeCurve, WarpAlgorithm, WarpMarker};

/// Build an in-RAM audio clip with `frames` stereo frames of silence and
/// default (no) warp settings.
fn make_clip(frames: usize) -> AudioClip {
    AudioClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        source: ClipSource::Memory(vec![0.0f32; frames * 2]),
        name: "clip".into(),
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
    }
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "expected {b}, got {a}");
}

#[test]
fn warp_defaults_are_no_stretch() {
    let clip = make_clip(1000);
    assert!(!clip.warp_enabled);
    assert_eq!(clip.original_bpm, None);
    assert_eq!(clip.transpose_semitones, 0.0);
    assert_eq!(clip.warp_algorithm, WarpAlgorithm::Transient);
    assert!(clip.warp_markers.is_empty());
}

#[test]
fn warp_algorithm_default_is_transient() {
    assert_eq!(WarpAlgorithm::default(), WarpAlgorithm::Transient);
}

#[test]
fn unknown_original_bpm_maps_identity() {
    // No markers and no original BPM => 1:1 source read, unchanged behaviour.
    let clip = make_clip(1000);
    approx(clip.warp_source_frame(0.0, 120.0, 48_000), 0.0);
    approx(clip.warp_source_frame(500.0, 120.0, 48_000), 500.0);
    approx(clip.warp_source_frame(999.0, 240.0, 44_100), 999.0);
}

#[test]
fn uniform_ratio_speeds_up_for_faster_project() {
    // Recorded at 120 BPM, played in a 240 BPM project => read source twice
    // as fast (ratio = project / original = 2.0).
    let mut clip = make_clip(1000);
    clip.original_bpm = Some(120.0);
    approx(clip.warp_source_frame(0.0, 240.0, 48_000), 0.0);
    approx(clip.warp_source_frame(100.0, 240.0, 48_000), 200.0);
    // And slows down for a slower project (ratio 0.5).
    approx(clip.warp_source_frame(100.0, 60.0, 48_000), 50.0);
}

#[test]
fn markers_interpolate_piecewise_linear() {
    // frames_per_beat at 120 BPM, 48 kHz = 48000 * 60 / 120 = 24000.
    // Marker A: beat 1.0 (tl frame 24000) -> source 30000
    // Marker B: beat 2.0 (tl frame 48000) -> source 54000
    // Slope across the segment = (54000-30000)/(48000-24000) = 1.0.
    let mut clip = make_clip(100_000);
    clip.warp_enabled = true;
    clip.warp_markers = vec![
        WarpMarker { source_frame: 30_000, timeline_beat: 1.0 },
        WarpMarker { source_frame: 54_000, timeline_beat: 2.0 },
    ];

    // On a marker: exact.
    approx(clip.warp_source_frame(24_000.0, 120.0, 48_000), 30_000.0);
    approx(clip.warp_source_frame(48_000.0, 120.0, 48_000), 54_000.0);
    // Midway between markers (beat 1.5, tl frame 36000): halfway in source.
    approx(clip.warp_source_frame(36_000.0, 120.0, 48_000), 42_000.0);
}

#[test]
fn markers_extrapolate_beyond_span_with_segment_slope() {
    let mut clip = make_clip(100_000);
    clip.warp_enabled = true;
    clip.warp_markers = vec![
        WarpMarker { source_frame: 30_000, timeline_beat: 1.0 },
        WarpMarker { source_frame: 54_000, timeline_beat: 2.0 },
    ];

    // Before the first marker (beat 0.5, tl frame 12000): extend slope 1.0
    // backwards => 30000 + (12000 - 24000) * 1.0 = 18000.
    approx(clip.warp_source_frame(12_000.0, 120.0, 48_000), 18_000.0);
    // After the last marker (beat 3.0, tl frame 72000): extend forwards =>
    // 54000 + (72000 - 48000) * 1.0 = 78000.
    approx(clip.warp_source_frame(72_000.0, 120.0, 48_000), 78_000.0);
}

#[test]
fn non_uniform_segment_stretches_between_markers() {
    // Two beats of timeline mapped onto 24000 source frames: the segment
    // reads the source at half speed relative to the 120 BPM grid.
    let mut clip = make_clip(100_000);
    clip.warp_enabled = true;
    clip.warp_markers = vec![
        WarpMarker { source_frame: 0, timeline_beat: 0.0 },
        WarpMarker { source_frame: 24_000, timeline_beat: 2.0 },
    ];

    // tl frame for beat 2 at 120 BPM / 48 kHz = 48000.
    approx(clip.warp_source_frame(0.0, 120.0, 48_000), 0.0);
    approx(clip.warp_source_frame(48_000.0, 120.0, 48_000), 24_000.0);
    // Beat 1 (tl frame 24000) sits halfway through the segment in beat space.
    approx(clip.warp_source_frame(24_000.0, 120.0, 48_000), 12_000.0);
}

#[test]
fn single_marker_extrapolates_with_uniform_ratio() {
    // One marker plus a known original BPM: the uniform ratio governs the
    // read rate on both sides of the anchor.
    let mut clip = make_clip(100_000);
    clip.warp_enabled = true;
    clip.original_bpm = Some(60.0); // ratio = project/original = 120/60 = 2.0
    clip.warp_markers = vec![WarpMarker { source_frame: 10_000, timeline_beat: 1.0 }];

    // frames_per_beat at 120 BPM = 24000, so the marker sits at tl 24000.
    approx(clip.warp_source_frame(24_000.0, 120.0, 48_000), 10_000.0);
    // After: 10000 + (48000 - 24000) * 2.0 = 58000.
    approx(clip.warp_source_frame(48_000.0, 120.0, 48_000), 58_000.0);
}

#[test]
fn warp_types_serde_round_trip() {
    let marker = WarpMarker { source_frame: 4_242, timeline_beat: 3.5 };
    let json = serde_json::to_string(&marker).unwrap();
    let back: WarpMarker = serde_json::from_str(&json).unwrap();
    assert_eq!(marker, back);

    for algo in [WarpAlgorithm::Tonal, WarpAlgorithm::Transient] {
        let json = serde_json::to_string(&algo).unwrap();
        let back: WarpAlgorithm = serde_json::from_str(&json).unwrap();
        assert_eq!(algo, back);
    }
}
