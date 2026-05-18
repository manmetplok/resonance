//! A single WaveNet dilated convolution layer plus the small 1x1 conv
//! primitives it composes from.

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
/// NAM weight order per layer:
///   _conv.weight  [mid_ch, ch, kernel_size]  (filter+gate combined if gated)
///   _conv.bias    [mid_ch]
///   _input_mixin.weight [mid_ch, condition_size]  (no bias)
///   _layer1x1.weight [ch, bottleneck]  (if active)
///   _layer1x1.bias [ch]                (if active)
pub(super) struct WaveNetLayer {
    /// Combined filter+gate conv weights per kernel tap.
    /// w_conv[tap] has size [mid_ch * ch].
    pub(super) w_conv: Vec<Vec<f32>>,
    /// Combined filter+gate conv bias [mid_ch].
    pub(super) b_conv: Vec<f32>,

    /// Input mixin weights (condition mixing). None if condition_size == 0.
    pub(super) w_input_mixin: Option<Vec<f32>>,

    /// Layer 1x1 residual conv. None if bottleneck == channels.
    pub(super) layer1x1: Option<Conv1x1Bias>,

    pub(super) kernel_size: usize,
    pub(super) dilation: usize,
    pub(super) channels: usize,
    /// mid_channels = 2*channels if gated, else channels.
    pub(super) mid_ch: usize,
}
