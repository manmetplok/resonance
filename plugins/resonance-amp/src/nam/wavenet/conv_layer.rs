//! A single WaveNet dilated convolution layer plus the small 1x1 conv
//! primitives it composes from.

use super::super::activations::Activation;

/// 1x1 convolution (no bias).
pub(super) struct Conv1x1 {
    pub(super) weight: Vec<f32>, // [out_ch * in_ch]
    pub(super) out_ch: usize,
    pub(super) in_ch: usize,
}

/// 1x1 convolution with optional bias.
pub(super) struct Conv1x1Bias {
    pub(super) weight: Vec<f32>,
    pub(super) bias: Vec<f32>, // empty if no bias
    pub(super) out_ch: usize,
    pub(super) in_ch: usize,
}

/// A single WaveNet dilated convolution layer.
///
/// NAM weight order per layer (reference `Layer::set_weights_` in
/// NAM/wavenet/model.cpp):
///   _conv.weight  [mid_ch, ch, kernel_size]  (filter+gate combined if gated)
///   _conv.bias    [mid_ch]
///   _input_mixin.weight [mid_ch, condition_size]  (no bias)
///   _layer1x1.weight [ch, bottleneck]  (if active)
///   _layer1x1.bias [ch]                (if active)
///
/// where `mid_ch = 2*bottleneck` when gated, else `bottleneck`. In A1 models
/// `bottleneck == channels`, so this degenerates to the historical layout.
pub(super) struct WaveNetLayer {
    /// Combined filter+gate conv weights per kernel tap.
    /// w_conv[tap] has size [mid_ch * ch].
    pub(super) w_conv: Vec<Vec<f32>>,
    /// Combined filter+gate conv bias [mid_ch].
    pub(super) b_conv: Vec<f32>,

    /// Input mixin weights (condition mixing). None if condition_size == 0.
    pub(super) w_input_mixin: Option<Vec<f32>>,

    /// Layer 1x1 residual conv (bottleneck -> channels). None when the
    /// config has no layer1x1, which requires bottleneck == channels.
    pub(super) layer1x1: Option<Conv1x1Bias>,

    pub(super) kernel_size: usize,
    pub(super) dilation: usize,
    pub(super) channels: usize,
    /// Internal (bottleneck) channel count: the width of the activated
    /// signal feeding the skip accumulator and layer1x1. A1: == channels.
    pub(super) bottleneck: usize,
    /// Conv/mixin output channels = 2*bottleneck if gated, else bottleneck.
    pub(super) mid_ch: usize,

    /// Layer activation, resolved from config at construction (A1: fast tanh).
    pub(super) activation: Activation,
    /// Gate activation applied to the second half of the conv output when
    /// gated. A1 gated models use the fast sigmoid; `None` when not gated.
    pub(super) gate_activation: Option<Activation>,
}
