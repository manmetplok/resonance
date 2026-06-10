//! The mixer's gain-application helpers ramp from the previous block's
//! effective gain to the current block's per sample, so a fader, pan,
//! or mute step between two blocks fades smoothly across the block
//! instead of producing zipper noise / clicks.

use resonance_audio::__test_support::{ramped_gain, sum_to_output, sum_to_stereo};

const FRAMES: usize = 64;

/// Extract the left channel from an interleaved stereo buffer.
fn left(data: &[f32], channels: usize) -> Vec<f32> {
    data.chunks(channels).map(|fr| fr[0]).collect()
}

#[test]
fn constant_gain_is_exact() {
    // from == to degenerates to the plain constant-gain multiply.
    let src = vec![1.0f32; FRAMES];
    let mut data = vec![0.0f32; FRAMES * 2];
    sum_to_output(&mut data, 2, FRAMES, &src, &src, (0.5, 0.5), (0.5, 0.5));
    for &s in &data {
        assert_eq!(s, 0.5);
    }
}

#[test]
fn gain_step_between_blocks_ramps_monotonically() {
    let src = vec![1.0f32; FRAMES];

    // Block 1 at steady gain 1.0.
    let mut block1 = vec![0.0f32; FRAMES * 2];
    sum_to_output(&mut block1, 2, FRAMES, &src, &src, (1.0, 1.0), (1.0, 1.0));

    // Block 2 after the fader stepped to 0.25.
    let mut block2 = vec![0.0f32; FRAMES * 2];
    sum_to_output(&mut block2, 2, FRAMES, &src, &src, (1.0, 0.25), (1.0, 0.25));

    let l1 = left(&block1, 2);
    let l2 = left(&block2, 2);

    // No discontinuity at the block seam or inside the block: every
    // sample-to-sample step is at most one ramp increment.
    let max_step = (1.0 - 0.25) / FRAMES as f32 + 1e-6;
    let mut prev = l1[FRAMES - 1];
    for &s in &l2 {
        assert!(s < prev, "ramp must decrease monotonically: {s} >= {prev}");
        assert!(prev - s <= max_step, "step too large: {prev} -> {s}");
        prev = s;
    }
    // Last sample lands exactly on the new gain.
    assert_eq!(l2[FRAMES - 1], 0.25);
}

#[test]
fn mute_ramps_to_exact_zero() {
    let src = vec![1.0f32; FRAMES];
    let mut data = vec![0.0f32; FRAMES * 2];
    // Mute: target gain 0.0 from a previous block at 0.8.
    sum_to_output(&mut data, 2, FRAMES, &src, &src, (0.8, 0.0), (0.8, 0.0));
    let l = left(&data, 2);
    let mut prev = 0.8f32;
    for &s in &l {
        assert!(s < prev, "mute fade must decrease monotonically");
        prev = s;
    }
    assert_eq!(l[FRAMES - 1], 0.0);

    // Unmute ramps back up symmetrically.
    let mut data = vec![0.0f32; FRAMES * 2];
    sum_to_output(&mut data, 2, FRAMES, &src, &src, (0.0, 0.8), (0.0, 0.8));
    let l = left(&data, 2);
    let mut prev = 0.0f32;
    for &s in &l {
        assert!(s > prev, "unmute fade must increase monotonically");
        prev = s;
    }
    assert_eq!(l[FRAMES - 1], 0.8);
}

#[test]
fn stereo_sum_ramps_and_accumulates() {
    let src_l = vec![1.0f32; FRAMES];
    let src_r = vec![-1.0f32; FRAMES];
    let mut dst_l = vec![0.25f32; FRAMES];
    let mut dst_r = vec![0.25f32; FRAMES];
    sum_to_stereo(
        &mut dst_l,
        &mut dst_r,
        FRAMES,
        &src_l,
        &src_r,
        (0.0, 1.0),
        (1.0, 0.0),
    );
    let inv = 1.0 / FRAMES as f32;
    for f in 0..FRAMES {
        let t = (f + 1) as f32 * inv;
        assert_eq!(dst_l[f], 0.25 + t);
        assert_eq!(dst_r[f], 0.25 - (1.0 - t));
    }
    // Both channels track their own ramp independently.
    assert_eq!(dst_l[FRAMES - 1], 1.25);
    assert_eq!(dst_r[FRAMES - 1], 0.25);
}

#[test]
fn mono_output_sums_both_channels_with_ramp() {
    let src_l = vec![1.0f32; FRAMES];
    let src_r = vec![1.0f32; FRAMES];
    let mut data = vec![0.0f32; FRAMES];
    sum_to_output(&mut data, 1, FRAMES, &src_l, &src_r, (0.0, 0.5), (0.0, 0.5));
    let inv = 1.0 / FRAMES as f32;
    for (f, &s) in data.iter().enumerate() {
        let g = ramped_gain((0.0, 0.5), inv, f);
        assert_eq!(s, 2.0 * g);
    }
}

#[test]
fn zero_frames_is_a_no_op() {
    let mut data: Vec<f32> = Vec::new();
    sum_to_output(&mut data, 2, 0, &[], &[], (1.0, 0.0), (1.0, 0.0));
    let (mut dl, mut dr) = (Vec::new(), Vec::new());
    sum_to_stereo(&mut dl, &mut dr, 0, &[], &[], (1.0, 0.0), (1.0, 0.0));
}
