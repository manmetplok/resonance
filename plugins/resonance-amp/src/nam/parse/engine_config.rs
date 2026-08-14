//! The WaveNet configuration the inference engine consumes.
//!
//! This is the engine-side shape — flattened per stack, with every A2 field
//! already resolved to a concrete per-layer value. [`super::config`] is the
//! only place that produces it.

use super::super::activations::ActivationConfig;
use super::super::wavenet::params::{GatingMode, Head1x1Params, LayerFilms};

/// Internal WaveNet config used by the inference engine.
pub struct WaveNetConfig {
    pub input_size: usize,
    /// Per-stack config.
    pub stacks: Vec<StackConfig>,
    /// Head hidden layer sizes (e.g. [8]). Empty if no head MLP.
    pub head: Vec<usize>,
    /// Final output size from head (typically 1).
    pub head_size: usize,
    /// Whether layers have a learned 1x1 residual conv (_layer1x1).
    /// True for new-format NAM models (default), false for old format.
    pub has_layer1x1: bool,
    /// Raw A2 `condition_dsp` sub-model JSON: a complete nested .nam-style
    /// model object (`architecture`/`config`/`weights`/`sample_rate`) that
    /// preprocesses the raw model input into the condition signal fed to
    /// every layer array's input mixin and FiLM points. Constructed
    /// recursively at model-build time (see
    /// [`super::build_condition_dsp`]); `None` (A1 and condition_dsp-less
    /// A2) means the condition is the raw model input.
    pub condition_dsp: Option<serde_json::Value>,
    /// Activation FLAVOR the model resolves `Tanh`/`Sigmoid` with (ba todo
    /// #1116). Every model runs the reference (NeuralAmpModelerCore
    /// `WaveNet::process`) STRUCTURAL semantics — raw-input/condition_dsp
    /// condition for every stack, unconditionally consumed learned input
    /// rechannels, chained head accumulators with the model output taken
    /// from the last stack only, no extra skip activation, identity
    /// residual for layers without a layer1x1 — the historical legacy
    /// wiring was proven wrong against every official implementation
    /// (doc #258) and its removal is a user-approved sound change.
    ///
    /// `true` — files expressible in the pre-A2 surface resolve
    /// `Tanh`/`Sigmoid` to the fast approximations: the official NAM
    /// plugin runs with `enable_fast_tanh()`, so this IS correct-vs-plugin
    /// for A1 files.
    ///
    /// `false` — configs carrying any A2 marker (see
    /// [`super::config_has_a2_markers`]) and all nested/container submodels
    /// use the exact functions (`Tanh` = `std::tanh`-equivalent), matching
    /// the A2 fixture reference renders (`tools/render` never enables fast
    /// tanh).
    pub fast_activations: bool,
}

pub struct StackConfig {
    pub input_size: usize,
    pub condition_size: usize,
    pub head_size: usize,
    /// Kernel size of this stack's head rechannel convolution (reference
    /// `LayerArrayParams::head_kernel_size`, from the A2 nested `head`
    /// object's `kernel_size`). A1 / legacy flat configs: 1, which is the
    /// historical 1x1 head rechannel exactly.
    pub head_kernel_size: usize,
    /// Dilation of the head rechannel convolution (`head.head_dilation`).
    /// A1: 1. Irrelevant when `head_kernel_size == 1`.
    pub head_dilation: usize,
    /// Whether this stack's head rechannel has a bias (per-array in the
    /// reference: nested `head.bias`, or legacy flat `head_bias`).
    pub head_bias: bool,
    pub channels: usize,
    /// Internal (bottleneck) channel count of each layer in this stack
    /// (A2). The dilated conv and input mixin output `bottleneck` channels
    /// (doubled when gated), activation runs at bottleneck width, and the
    /// layer1x1 maps bottleneck back to `channels`. A1 models — and A2
    /// models that omit the field — use `bottleneck == channels`, which
    /// degenerates to the historical layout exactly.
    pub bottleneck: usize,
    /// Whether this stack's layers have a learned 1x1 residual conv whose
    /// weights sit in the stream (reference `Layer1x1Params::active`).
    /// Old-format models: false. New-format models: true unless the A2
    /// `layer1x1` object says `"active": false` — in which case the
    /// reference consumes NO layer1x1 weights and the residual is the
    /// identity.
    pub layer1x1_active: bool,
    pub dilations: Vec<usize>,
    pub kernel_sizes: Vec<usize>,
    /// Layer activation(s) for this stack: a single entry is broadcast to
    /// every layer (A1 string configs and A2 single-object configs), or one
    /// entry per layer (A2 per-layer activation arrays). A1 models use
    /// `"Tanh"`, which resolves to the fast-tanh path at model
    /// construction.
    pub activations: Vec<ActivationConfig>,
    /// Gating mode per layer (same length as `dilations`; A2 allows mixed
    /// per-layer modes). Layers with a non-`None` mode have their conv and
    /// input-mixin output width doubled (primary + secondary halves).
    pub gating_modes: Vec<GatingMode>,
    /// Secondary (gate/blend) activation config per layer. `None` for
    /// ungated layers; a gated/blended layer without an explicit config
    /// defaults to `Sigmoid` (reference backward-compat), which the engine
    /// resolves to the fast sigmoid so A1 gated models stay bit-identical.
    pub secondary_activations: Vec<Option<ActivationConfig>>,
    /// Groups of the dilated input convolution (reference
    /// `LayerArrayParams::groups_input`). Splits the conv's `channels`
    /// inputs and `bottleneck`-wide (doubled when gated) outputs into
    /// contiguous per-group blocks. A1: 1.
    pub groups_input: usize,
    /// Groups of the condition input-mixin 1x1 convolution (reference
    /// `groups_input_mixin`). A1: 1.
    pub groups_input_mixin: usize,
    /// Groups of the layer1x1 residual convolution (reference
    /// `Layer1x1Params::groups`); only meaningful when the config has a
    /// layer1x1. A1: 1.
    pub layer1x1_groups: usize,
    /// Optional head1x1 skip-path convolution (reference `Head1x1Params`).
    /// When active, each layer's skip contribution is
    /// `head1x1(activated z)` (bottleneck -> `out_channels`, grouped) and
    /// the stack's skip accumulator / head_rechannel input are
    /// `out_channels` wide instead of `bottleneck`. A1: inactive.
    pub head1x1: Head1x1Params,
    /// The 8 per-layer FiLM insertion points (reference `_FiLMParams`
    /// members of `LayerArrayParams`). A1: all inactive, which leaves the
    /// signal untouched at every site.
    pub films: LayerFilms,
}
