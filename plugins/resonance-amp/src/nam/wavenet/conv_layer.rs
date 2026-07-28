//! A single WaveNet dilated convolution layer plus the small 1x1 conv
//! primitives it composes from.

use super::super::activations::Activation;

/// 1x1 convolution (no bias).
pub(super) struct Conv1x1 {
    pub(super) weight: Vec<f32>, // [out_ch * in_ch]
    pub(super) out_ch: usize,
    pub(super) in_ch: usize,
}

/// 1x1 convolution with optional bias and optional grouping.
///
/// With `groups == 1` the weight is the dense row-major `[out_ch x in_ch]`
/// matrix. With `groups == G` it is the compact grouped layout of the
/// reference `Conv1x1` (NAM/dsp.cpp `set_weights_`): `G` concatenated
/// row-major `[out_ch/G x in_ch/G]` per-group blocks, `out_ch * in_ch / G`
/// values total.
pub(super) struct Conv1x1Bias {
    pub(super) weight: Vec<f32>,
    pub(super) bias: Vec<f32>, // empty if no bias
    pub(super) out_ch: usize,
    pub(super) in_ch: usize,
    pub(super) groups: usize,
}

/// Per-layer gating behavior, resolved from config at construction
/// (reference `GatingMode` + `NAM/gating_activations.h`).
///
/// For `Gated` and `Blended` the conv/mixin output is split into a primary
/// (top, bottleneck-wide) and a secondary (bottom) half; the payload is the
/// resolved secondary activation applied to the bottom half.
pub(super) enum LayerGating {
    /// No gating: primary activation only.
    None,
    /// `out[c] = primary(z[c]) * secondary(z[bottleneck + c])`
    /// (reference `GatingActivation`). A1 gated models resolve the default
    /// `Sigmoid` secondary to the fast sigmoid — bit-identical to the
    /// historical hardcoded path.
    Gated(Activation),
    /// `alpha = secondary(z[bottleneck + c])`;
    /// `out[c] = alpha * primary(z[c]) + (1 - alpha) * z[c]` where the last
    /// term is the pre-activation value (reference `BlendingActivation`).
    Blended(Activation),
}

/// A single WaveNet dilated convolution layer.
///
/// NAM weight order per layer (reference `Layer::set_weights_` in
/// NAM/wavenet/model.cpp):
///   _conv.weight  [mid_ch, ch/groups_input, kernel_size]
///                 (primary+secondary combined if gated/blended; flat
///                  consumption order [group][out][in][tap], reference
///                  `Conv1D::set_weights_` in NAM/conv1d.cpp)
///   _conv.bias    [mid_ch]
///   _input_mixin.weight [mid_ch, condition_size/groups_input_mixin]
///                 (no bias; flat order [group][out][in], reference
///                  `Conv1x1::set_weights_` in NAM/dsp.cpp)
///   _layer1x1.weight [ch, bottleneck/layer1x1.groups]  (if active)
///   _layer1x1.bias [ch]                                (if active)
///   _head1x1.weight [head1x1.out_channels, bottleneck/head1x1.groups]
///                                                      (if active)
///   _head1x1.bias  [head1x1.out_channels]              (if active)
///
/// where `mid_ch = 2*bottleneck` when the layer's gating mode is gated or
/// blended, else `bottleneck`. In A1 models `bottleneck == channels` and all
/// groups are 1, so this degenerates to the historical dense layout.
pub(super) struct WaveNetLayer {
    /// Combined filter+gate conv weights per kernel tap. With
    /// `groups_input == 1`, w_conv[tap] is the dense row-major
    /// `[mid_ch x ch]` matrix (size mid_ch * ch); with G groups it is the
    /// compact grouped layout (G concatenated `[mid_ch/G x ch/G]` row-major
    /// blocks, size mid_ch * ch / G).
    pub(super) w_conv: Vec<Vec<f32>>,
    /// Combined filter+gate conv bias [mid_ch].
    pub(super) b_conv: Vec<f32>,

    /// Input mixin weights (condition mixing). None if condition_size == 0.
    pub(super) w_input_mixin: Option<Vec<f32>>,

    /// Layer 1x1 residual conv (bottleneck -> channels). None when the
    /// config has no layer1x1, which requires bottleneck == channels.
    pub(super) layer1x1: Option<Conv1x1Bias>,

    /// Head 1x1 skip conv (bottleneck -> head1x1.out_channels, always
    /// biased, reference `Conv1x1(bottleneck, out_channels, true, groups)`).
    /// When present, the layer's skip contribution is `head1x1(activated z)`
    /// instead of the activated z itself; None (A1) keeps the direct skip.
    pub(super) head1x1: Option<Conv1x1Bias>,

    pub(super) kernel_size: usize,
    pub(super) dilation: usize,
    pub(super) channels: usize,
    /// Internal (bottleneck) channel count: the width of the activated
    /// signal feeding the skip accumulator and layer1x1. A1: == channels.
    pub(super) bottleneck: usize,
    /// Conv/mixin output channels = 2*bottleneck if gated/blended, else
    /// bottleneck.
    pub(super) mid_ch: usize,
    /// Groups of the dilated input conv (in: channels, out: mid_ch). A1: 1.
    pub(super) groups_input: usize,
    /// Groups of the condition input-mixin conv (in: condition_size,
    /// out: mid_ch). A1: 1.
    pub(super) groups_input_mixin: usize,

    /// Primary layer activation, resolved from config at construction
    /// (A1: fast tanh).
    pub(super) activation: Activation,
    /// Gating behavior of this layer (mode + resolved secondary activation).
    pub(super) gating: LayerGating,
}
