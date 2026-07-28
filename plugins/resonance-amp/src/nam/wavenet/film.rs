//! FiLM (feature-wise linear modulation) block, reference `nam::FiLM` in
//! NAM/film.h (NeuralAmpModelerCore, MIT).
//!
//! A FiLM maps the condition signal through ONE biased grouped 1x1
//! convolution producing `(shift ? 2 : 1) * width` channels — the top
//! `width` rows are the per-channel scale, the bottom `width` rows (when
//! `shift`) the per-channel shift — and applies
//! `y[c] = x[c] * scale[c] (+ shift[c])` elementwise (reference
//! `FiLM::Process`: `_cond_to_scale_shift(condition_dim,
//! (shift ? 2 : 1) * input_dim, /*bias=*/true, groups)`, then
//! `scale = topRows(input_dim)`, `shift = bottomRows(input_dim)`).
//!
//! Weight consumption (reference `FiLM::set_weights_` ->
//! `Conv1x1::set_weights_` in NAM/dsp.cpp): the conv weights in the compact
//! grouped `[group][out][in]` layout (`out_ch * cond_ch / groups` values),
//! then the bias (`out_ch` values, always present).

use super::super::grouped_matvec;

/// One FiLM insertion point, resolved at construction. All scratch is
/// caller-provided (`process_sample` stays allocation-free).
pub(super) struct Film {
    /// Condition -> scale/shift conv weights, compact grouped layout
    /// (`groups` concatenated row-major `[out_ch/groups x cond_ch/groups]`
    /// blocks).
    pub(super) weight: Vec<f32>,
    /// Conv bias `[out_ch]` (the reference FiLM conv is always biased).
    pub(super) bias: Vec<f32>,
    /// Condition width (conv input channels).
    pub(super) cond_ch: usize,
    /// Conv output channels: `2 * width` when `shift`, else `width`.
    pub(super) out_ch: usize,
    /// Width of the modulated tensor (reference `input_dim`).
    pub(super) width: usize,
    pub(super) groups: usize,
    /// Scale + shift (`true`) or scale only (`false`).
    pub(super) shift: bool,
}

impl Film {
    /// Modulate `x[..width]` in place from `cond` (reference
    /// `FiLM::Process_`). `ss` is the scale/shift scratch, at least
    /// `out_ch` long.
    #[inline]
    pub(super) fn modulate_in_place(&self, cond: &[f32], x: &mut [f32], ss: &mut [f32]) {
        grouped_matvec(
            &self.weight,
            &cond[..self.cond_ch],
            self.out_ch,
            self.cond_ch,
            self.groups,
            ss,
        );
        for (s, b) in ss[..self.out_ch].iter_mut().zip(&self.bias) {
            *s += b;
        }
        if self.shift {
            for c in 0..self.width {
                x[c] = x[c] * ss[c] + ss[self.width + c];
            }
        } else {
            for c in 0..self.width {
                x[c] *= ss[c];
            }
        }
    }

    /// Modulate `x[..width]` into `out[..width]`, leaving `x` untouched
    /// (reference `FiLM::Process` on a const input — the conv_pre /
    /// input_mixin_pre sites). `cond` and `x` may alias (the
    /// input_mixin_pre FiLM conditions the condition on itself); `out` must
    /// not alias either.
    #[inline]
    pub(super) fn modulate(&self, cond: &[f32], x: &[f32], out: &mut [f32], ss: &mut [f32]) {
        out[..self.width].copy_from_slice(&x[..self.width]);
        self.modulate_in_place(cond, out, ss);
    }
}
