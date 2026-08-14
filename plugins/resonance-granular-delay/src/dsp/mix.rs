//! Stage 4 — the output mix: M/S width on the wet sum and the
//! equal-power dry/wet blend that leaves the block in the host's
//! buffers.

use crate::params::GranularSmoothers;

use super::grains::GrainBank;

/// M/S width on the wet sum only (ba todo #1077, doc #252 §5), then the
/// equal-power dry/wet mix (doc #252 §9); the dry path stays bit-exact
/// regardless of the stereo processing. Width sits *after* the feedback
/// tap, so the loop recirculates the un-widened wet and the width
/// control cannot destabilise it. Width 0 collapses the wet to its mid
/// signal (L == R).
pub(super) fn mix_output(
    grains: &GrainBank,
    left: &mut [f32],
    right: &mut [f32],
    frames: usize,
    smoothers: &mut GranularSmoothers,
) {
    for i in 0..frames {
        let width = smoothers.width.next().clamp(0.0, 1.5);
        let mid = 0.5 * (grains.wet_l[i] + grains.wet_r[i]);
        let side = 0.5 * (grains.wet_l[i] - grains.wet_r[i]) * width;
        let mix = smoothers.mix.next().clamp(0.0, 1.0);
        let dry_gain = (1.0 - mix).sqrt();
        let wet_gain = mix.sqrt();
        left[i] = left[i] * dry_gain + (mid + side) * wet_gain;
        right[i] = right[i] * dry_gain + (mid - side) * wet_gain;
    }
}
