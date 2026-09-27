//! Sample-accurate clip fades, clip gain, the automatic same-track
//! crossfade, and the automatic edge declick, applied in the shared mix
//! loop (`mix_track_clips`, called by both the live mixer and the offline
//! bounce via `render_block`).
//!
//! These exercise the render math on a known DC clip so the envelope and
//! gain are exactly predictable on the output samples. Because live and
//! bounce share `mix_track_clips`, asserting it here also pins the
//! "offline render matches live render" guarantee.
//!
//! Every clip edge carries a `CLIP_DECLICK_FRAMES` anti-click ramp, so the
//! unity assertions below deliberately look at the clip's interior; the
//! edges themselves are pinned by the declick tests at the bottom.

use resonance_audio::test_support::{mix_track_clips, CLIP_DECLICK_FRAMES};
use resonance_audio::types::*;

const TRACK: TrackId = 1;
/// Declick length as a `usize`, for slicing.
const DECLICK: usize = CLIP_DECLICK_FRAMES as usize;
/// Clip length used by the envelope tests: long enough that the declick
/// only touches the outermost frames, so an explicit fade is what shapes
/// the edge under test.
const LONG: usize = 4096;
/// Explicit fade length used by the envelope tests, comfortably longer
/// than the declick so it wins the `max`.
const FADE: usize = 1000;

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
        source: ClipSource::memory(vec![1.0; frames * 2]),
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
        tuning_render_cache: None,
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
    let clip = dc_clip(1, 0, LONG, FADE as u64, FadeCurve::Linear, 0, FadeCurve::default(), 0.0);
    let out = render_left(&[clip], LONG);

    // The explicit fade ramps 0 -> 1 linearly (coefficient(t) == t).
    for (i, &v) in out.iter().take(FADE).enumerate() {
        let expected = i as f32 / FADE as f32;
        assert!(
            (v - expected).abs() < 1e-6,
            "frame {i}: got {v}, want {expected}"
        );
    }
    // After the fade the clip plays at unity, up to the tail declick.
    for &v in &out[FADE..LONG - DECLICK] {
        assert!((v - 1.0).abs() < 1e-6, "post-fade frame should be unity, got {v}");
    }
}

#[test]
fn fade_out_equal_power_reaches_zero_at_last_frame() {
    let clip = dc_clip(1, 0, LONG, 0, FadeCurve::default(), FADE as u64, FadeCurve::EqualPower, 0.0);
    let out = render_left(&[clip], LONG);

    let fade_start = LONG - FADE;
    // Unity between the head declick and the start of the fade-out.
    for &v in &out[DECLICK..fade_start] {
        assert!((v - 1.0).abs() < 1e-6, "pre-fade-out frame should be unity, got {v}");
    }
    // The last frame is fully silenced.
    let last = out[LONG - 1];
    assert!(last.abs() < 1e-6, "last frame should be ~0, got {last}");
    // Equal-power complement at the fade-out midpoint: half the fade before
    // the end -> coefficient(0.5) == sin(pi/4) ~= 0.7071.
    let mid = out[LONG - 1 - FADE / 2];
    assert!(
        (mid - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4,
        "equal-power fade-out midpoint should be ~0.7071, got {mid}"
    );
    // Monotonic non-increasing across the fade-out.
    for w in out[fade_start..].windows(2) {
        assert!(w[1] <= w[0] + 1e-6, "fade-out must not rise: {} -> {}", w[0], w[1]);
    }
}

#[test]
fn gain_scales_every_frame() {
    // +6.0206 dB ~= x2.0 linear.
    let gain_db = 20.0 * 2.0f32.log10();
    let clip = dc_clip(1, 0, LONG, 0, FadeCurve::default(), 0, FadeCurve::default(), gain_db);
    let out = render_left(&[clip], LONG);
    for &v in &out[DECLICK..LONG - DECLICK] {
        assert!((v - 2.0).abs() < 1e-4, "gain should scale to ~2.0, got {v}");
    }
    // The declick ramps toward the same scaled level, never past it.
    assert!(out.iter().all(|&v| v <= 2.0 + 1e-4), "declick must not overshoot the gain");
}

#[test]
fn unity_clip_is_unchanged_away_from_its_edges() {
    // Default clip (no fade, 0 dB) mixes bit-identically to the raw PCM
    // everywhere except the two declick ramps on its audible edges.
    let clip = dc_clip(1, 0, LONG, 0, FadeCurve::default(), 0, FadeCurve::default(), 0.0);
    let out = render_left(&[clip], LONG);
    assert!(
        out[DECLICK..LONG - DECLICK].iter().all(|&v| v == 1.0),
        "clip interior must pass through unchanged"
    );
}

#[test]
fn overlapping_clips_crossfade_removes_seam() {
    // Two unity DC clips on the same track overlap by `FADE` frames with no
    // explicit fades: A = [0, LONG), B = [LONG - FADE, 2*LONG - FADE). The
    // overlap is an automatic equal-power crossfade.
    let a = dc_clip(1, 0, LONG, 0, FadeCurve::default(), 0, FadeCurve::default(), 0.0);
    let b_start = (LONG - FADE) as u64;
    let b = dc_clip(2, b_start, LONG, 0, FadeCurve::default(), 0, FadeCurve::default(), 0.0);
    let total = 2 * LONG - FADE;
    let out = render_left(&[a, b], total);

    let ov_start = LONG - FADE;
    // Outside the overlap exactly one clip plays at unity.
    assert!((out[ov_start - 1] - 1.0).abs() < 1e-6, "pre-overlap frame should be unity");
    assert!((out[LONG] - 1.0).abs() < 1e-6, "post-overlap frame should be unity");

    // Seam is gone: at the overlap edges the level matches the surrounding
    // unity level rather than stepping to 2.0 (which a naive sum would do).
    assert!(
        (out[ov_start] - 1.0).abs() < 0.02,
        "overlap start should stay ~unity, got {} (naive sum would be ~2.0)",
        out[ov_start]
    );
    assert!(
        (out[LONG - 1] - 1.0).abs() < 0.02,
        "overlap end should stay ~unity, got {}",
        out[LONG - 1]
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
    let mid = out[ov_start + FADE / 2];
    assert!(mid > 1.0, "equal-power overlap of correlated DC should sum >1, got {mid}");
    assert!(
        out[ov_start..LONG].iter().all(|&v| v > 0.9),
        "overlap must never dip toward silence"
    );
}

// ---------------------------------------------------------------------------
// Automatic edge declick
// ---------------------------------------------------------------------------

#[test]
fn clip_edges_ramp_instead_of_stepping() {
    // A clip with no fades still ramps in and out over CLIP_DECLICK_FRAMES:
    // its audible edges are splices (a trim, a split, a punch-in take) and
    // raw DC there is a click.
    let clip = dc_clip(1, 0, LONG, 0, FadeCurve::default(), 0, FadeCurve::default(), 0.0);
    let out = render_left(&[clip], LONG);

    assert_eq!(out[0], 0.0, "first frame of a clip must start from silence");
    assert!(out[LONG - 1].abs() < 1e-6, "last frame of a clip must reach silence");
    assert!((out[DECLICK] - 1.0).abs() < 1e-6, "declick must be over after its length");

    // Both ramps are monotonic and bounded by unity.
    for (i, w) in out[..=DECLICK].windows(2).enumerate() {
        assert!(w[1] >= w[0], "declick-in must not dip at frame {i}: {} -> {}", w[0], w[1]);
    }
    for (i, w) in out[LONG - DECLICK - 1..].windows(2).enumerate() {
        assert!(w[1] <= w[0], "declick-out must not rise at frame {i}: {} -> {}", w[0], w[1]);
    }
}

#[test]
fn declick_removes_the_step_at_a_trim_point() {
    // A clip trimmed mid-waveform: the source is constant 0.5, so the first
    // audible frame would step 0 -> 0.5 without a declick.
    let mut clip = dc_clip(1, 0, LONG, 0, FadeCurve::default(), 0, FadeCurve::default(), 0.0);
    clip.source = ClipSource::memory(vec![0.5; LONG * 2]);
    clip.trim_start_frames = 1000;
    clip.trim_end_frames = 1000;
    let audible = LONG - 2000;
    let out = render_left(&[clip], LONG);

    // Interior plays the trimmed source untouched.
    assert!(
        out[DECLICK..audible - DECLICK].iter().all(|&v| (v - 0.5).abs() < 1e-6),
        "trimmed interior must play the source at unity"
    );
    // Neither edge steps: the biggest single-frame jump is the declick's own
    // slope, orders of magnitude below the raw 0.5 step.
    let max_step = out
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(
        max_step < 0.5 / DECLICK as f32 * 2.0,
        "trim edges must not step: largest jump was {max_step}"
    );
    assert!(out[audible..].iter().all(|&v| v == 0.0), "nothing plays past the trim");
}

#[test]
fn explicit_fade_longer_than_the_declick_wins() {
    let clip = dc_clip(1, 0, LONG, FADE as u64, FadeCurve::Linear, 0, FadeCurve::default(), 0.0);
    let out = render_left(&[clip], LONG);
    // The declick would have reached unity by DECLICK; the longer explicit
    // fade is still climbing there.
    let at_declick = out[DECLICK];
    assert!(
        (at_declick - DECLICK as f32 / FADE as f32).abs() < 1e-6,
        "the longer explicit fade must shape the edge, got {at_declick}"
    );
}

#[test]
fn declick_on_a_very_short_clip_is_capped_at_half() {
    // A clip shorter than two declicks (a sliced grain) still reaches its
    // peak in the middle instead of being attenuated twice over.
    let frames = DECLICK; // half a declick per edge
    let clip = dc_clip(1, 0, frames, 0, FadeCurve::default(), 0, FadeCurve::default(), 0.0);
    let out = render_left(&[clip], frames);

    assert_eq!(out[0], 0.0, "short clip still starts from silence");
    assert!(out[frames - 1].abs() < 1e-6, "short clip still ends at silence");
    // The head ramp finishes exactly where the tail ramp starts, so the
    // midpoint sits at unity bar the tail curve's first step — not at the
    // ~0.5 an uncapped double attenuation would give.
    let peak = out.iter().fold(0.0f32, |m, &v| m.max(v));
    assert!(
        peak > 0.99,
        "a short clip must still reach ~unity at its midpoint, got {peak}"
    );
}
