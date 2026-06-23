//! Sample-accurate clip fades, clip gain, and the automatic same-track
//! crossfade, applied in the shared mix loop (`mix_track_clips`, called
//! by both the live mixer and the offline bounce via `render_block`).
//!
//! These exercise the render math on a known DC clip so the envelope and
//! gain are exactly predictable on the output samples. Because live and
//! bounce share `mix_track_clips`, asserting it here also pins the
//! "offline render matches live render" guarantee.

use resonance_audio::__test_support::mix_track_clips;
use resonance_audio::types::*;

const TRACK: TrackId = 1;

/// A clip whose PCM is constant `1.0` on both channels, so the output
/// sample at each frame equals the applied fade/gain coefficient.
#[allow(clippy::too_many_arguments)]
fn dc_clip(
    id: ClipId,
    start: u64,
    frames: usize,
    fade_in_frames: u64,
    fade_in_curve: FadeCurve,
    fade_out_frames: u64,
    fade_out_curve: FadeCurve,
    gain_db: f32,
) -> AudioClip {
    AudioClip {
        id,
        track_id: TRACK,
        start_sample: start,
        source: ClipSource::Memory(vec![1.0; frames * 2]),
        name: "dc".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames,
        fade_in_curve,
        fade_out_frames,
        fade_out_curve,
        gain_db,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: Default::default(),
        warp_markers: Vec::new(),
    }
}

/// Mix `clips` over `[0, frames)` and return the left-channel output.
fn render_left(clips: &[AudioClip], frames: usize) -> Vec<f32> {
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let has_audio = mix_track_clips(clips, TRACK, 0, frames, &mut l, &mut r);
    assert!(has_audio, "expected the clip to contribute audio");
    // Both channels carry identical DC, so the envelope is the same.
    assert_eq!(l, r, "left/right envelopes must match for DC input");
    l
}

#[test]
fn fade_in_linear_ramps_then_holds_unity() {
    let clip = dc_clip(1, 0, 100, 10, FadeCurve::Linear, 0, FadeCurve::default(), 0.0);
    let out = render_left(&[clip], 100);

    // First 10 frames ramp 0 -> 1 linearly (coefficient(t) == t).
    for (i, &v) in out.iter().take(10).enumerate() {
        let expected = i as f32 / 10.0;
        assert!(
            (v - expected).abs() < 1e-6,
            "frame {i}: got {v}, want {expected}"
        );
    }
    // After the fade the clip plays at unity.
    for &v in &out[10..] {
        assert!((v - 1.0).abs() < 1e-6, "post-fade frame should be unity, got {v}");
    }
}

#[test]
fn fade_out_equal_power_reaches_zero_at_last_frame() {
    let clip = dc_clip(1, 0, 100, 0, FadeCurve::default(), 10, FadeCurve::EqualPower, 0.0);
    let out = render_left(&[clip], 100);

    // Unity until the fade-out begins at frame 90.
    for &v in &out[..90] {
        assert!((v - 1.0).abs() < 1e-6, "pre-fade-out frame should be unity, got {v}");
    }
    // The last frame is fully silenced.
    assert!(out[99].abs() < 1e-6, "last frame should be ~0, got {}", out[99]);
    // Equal-power complement at the fade-out midpoint: 5 frames before the
    // end -> coefficient(0.5) == sin(pi/4) ~= 0.7071.
    let mid = out[94];
    assert!(
        (mid - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4,
        "equal-power fade-out midpoint should be ~0.7071, got {mid}"
    );
    // Monotonic non-increasing across the fade-out.
    for w in out[90..100].windows(2) {
        assert!(w[1] <= w[0] + 1e-6, "fade-out must not rise: {} -> {}", w[0], w[1]);
    }
}

#[test]
fn gain_scales_every_frame() {
    // +6.0206 dB ~= x2.0 linear.
    let gain_db = 20.0 * 2.0f32.log10();
    let clip = dc_clip(1, 0, 16, 0, FadeCurve::default(), 0, FadeCurve::default(), gain_db);
    let out = render_left(&[clip], 16);
    for &v in &out {
        assert!((v - 2.0).abs() < 1e-4, "gain should scale to ~2.0, got {v}");
    }
}

#[test]
fn unity_clip_is_unchanged() {
    // Default clip (no fade, 0 dB) must mix bit-identically to the raw PCM.
    let clip = dc_clip(1, 0, 32, 0, FadeCurve::default(), 0, FadeCurve::default(), 0.0);
    let out = render_left(&[clip], 32);
    assert!(out.iter().all(|&v| v == 1.0), "unity clip must pass through unchanged");
}

#[test]
fn overlapping_clips_crossfade_removes_seam() {
    // Two unity DC clips on the same track overlap by 20 frames with no
    // explicit fades: A = [0, 100), B = [80, 180). The overlap is an
    // automatic equal-power crossfade.
    let a = dc_clip(1, 0, 100, 0, FadeCurve::default(), 0, FadeCurve::default(), 0.0);
    let b = dc_clip(2, 80, 100, 0, FadeCurve::default(), 0, FadeCurve::default(), 0.0);
    let out = render_left(&[a, b], 180);

    // Outside the overlap exactly one clip plays at unity.
    assert!((out[79] - 1.0).abs() < 1e-6, "pre-overlap frame should be unity");
    assert!((out[100] - 1.0).abs() < 1e-6, "post-overlap frame should be unity");

    // Seam is gone: at the overlap edges the level matches the surrounding
    // unity level rather than stepping to 2.0 (which a naive sum would do).
    assert!(
        (out[80] - 1.0).abs() < 0.02,
        "overlap start should stay ~unity, got {} (naive sum would be ~2.0)",
        out[80]
    );
    assert!(
        (out[99] - 1.0).abs() < 0.02,
        "overlap end should stay ~unity, got {}",
        out[99]
    );

    // No seam anywhere across the block: the equal-power bump is smooth,
    // so adjacent frames stay close. A naive sum would step ~1.0 at the
    // overlap edges — well above this bound.
    for (i, w) in out.windows(2).enumerate() {
        assert!(
            (w[1] - w[0]).abs() < 0.1,
            "discontinuity at frame {i}: {} -> {}",
            w[0],
            w[1]
        );
    }

    // Correlated DC sums above unity through the equal-power overlap, but
    // never silences (no power dip at the seam).
    let mid = out[90];
    assert!(mid > 1.0, "equal-power overlap of correlated DC should sum >1, got {mid}");
    assert!(out[80..100].iter().all(|&v| v > 0.9), "overlap must never dip toward silence");
}
