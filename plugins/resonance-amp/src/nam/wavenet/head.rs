//! Head modules: the per-stack head rechannel conv and the dense (fully
//! connected) layers of the legacy MLP output head.

/// Per-stack head rechannel: a causal dilated convolution projecting the
/// accumulated skip signal to `head_size` (reference `_head_rechannel` in
/// NAM/wavenet/model.cpp — `Conv1D(skip_ch, head_size, head_kernel_size,
/// head_bias, head_dilation, 1)`).
///
/// A1 models (and A2 layer arrays with `head.kernel_size == 1`) have a
/// single tap, which degenerates to the historical 1x1 matvec exactly —
/// same weight layout, same math.
pub(super) struct HeadRechannel {
    /// Per-tap compact `[out_ch x in_ch]` row-major weight matrices.
    /// `taps[kernel_size - 1]` multiplies the current frame; `taps[k]`
    /// multiplies the frame `(kernel_size - 1 - k) * dilation` samples ago
    /// (reference `Conv1D::Process` tap offsets).
    pub(super) taps: Vec<Vec<f32>>,
    /// Bias per output channel; zeros when the config's `head_bias` is
    /// false (no weights consumed).
    pub(super) bias: Vec<f32>,
    pub(super) out_ch: usize,
    pub(super) in_ch: usize,
    /// Spacing between kernel taps in samples (`head.head_dilation`, A1: 1).
    pub(super) dilation: usize,
}

impl HeadRechannel {
    #[inline(always)]
    pub(super) fn kernel_size(&self) -> usize {
        self.taps.len()
    }
}

/// Dense (fully connected) layer for the head network.
pub(super) struct DenseLayer {
    pub(super) weight: Vec<f32>,
    pub(super) bias: Vec<f32>,
    pub(super) in_features: usize,
    pub(super) out_features: usize,
    pub(super) has_activation: bool,
}
