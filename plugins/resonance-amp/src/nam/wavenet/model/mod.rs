//! WaveNet inference engine for NAM models.
//!
//! Follows the NAM (Neural Amp Modeler) weight serialization order
//! (reference `set_weights_` in NAM/wavenet/model.cpp):
//! Per LayerArray (stack): rechannel, layers (conv+bias, input_mixin,
//! layer1x1, head1x1), head_rechannel (kernel taps in Conv1D
//! [out][in][tap] order, then bias when head_bias)
//! Then: head MLP layers, head_scale.
//!
//! Tensor shapes follow the A2 bottleneck convention: the dilated conv and
//! input mixin output `bottleneck` channels (doubled for layers whose gating
//! mode is `gated` or `blended` — primary + secondary halves), the
//! activation and skip path run at bottleneck width, and the layer1x1 maps
//! bottleneck back to `channels`. With an active `head1x1`, each layer's
//! skip contribution goes through a dedicated grouped 1x1 conv (bottleneck
//! -> `head1x1.out_channels`), so the stack's skip accumulator and the
//! head_rechannel input are `out_channels` wide (reference `_head_rechannel`
//! / `_head_output_size` in NAM/wavenet/model.cpp). A1 models have
//! `bottleneck == channels` and no head1x1, which reproduces the historical
//! layout bit-identically.
//!
//! A2 models may carry a `condition_dsp` sub-network: a complete nested
//! model (with its OWN weight array — the outer flat stream is unaffected)
//! that runs on the raw input once per sample and whose multi-channel
//! output becomes the condition for every stack's input mixin and FiLM
//! sites (reference `WaveNet::_process_condition`).
//!
//! Submodules (one concern per unit):
//! - [`build`] — `from_config_and_weights` orchestration + scratch sizing.
//! - [`weights`] — flat-weight-stream readers (stack / layer / head).
//! - [`validate`] — config-shape and load-time dimension validation.
//! - [`forward`] — the per-sample forward pass, split into per-stage
//!   methods.

mod build;
mod forward;
mod validate;
mod weights;

use super::super::activations::Activation;
use super::super::NamInference;
use super::conv_layer::{Conv1x1, WaveNetLayer};
use super::head::{DenseLayer, HeadRechannel};
use super::ring::RingBuffer;

pub struct WaveNetModel {
    /// A2 condition_dsp sub-network: a nested WaveNet that transforms the
    /// raw input sample into the condition signal before it feeds the
    /// stacks (reference `WaveNet::_process_condition`). Its weights come
    /// from the nested model object's own weight array, never from the
    /// outer flat stream. `None` = the condition is the raw input (the
    /// reference `_process_condition` passthrough).
    condition_dsp: Option<Box<WaveNetModel>>,
    /// Nested condition output, `condition_dsp.out_channels()` wide
    /// (empty when there is no condition_dsp). Preallocated; refreshed
    /// once per sample before the stack loop.
    condition_buf: Vec<f32>,
    /// Number of input channels (`config.input_size`; the engine processes
    /// mono, so this is 1 for every loadable file — recorded for the
    /// reference's condition_dsp input-width validation).
    in_channels: usize,
    /// Head MLP input width (`config.head_size`). `head_input` may be
    /// allocated wider when a later stack's head rechannel outputs more
    /// channels than the first (multi-channel nested nets).
    head_size: usize,
    /// Model output width: `head_size` with a head MLP, else the LAST
    /// stack's head rechannel width (reference `wave_net_output_channels`).
    /// 1 for the main (audio) model; nested condition_dsp models are
    /// multi-channel.
    out_channels: usize,

    // Per-stack data. Every model runs the reference NeuralAmpModelerCore
    // forward pass (ba todo #1116): raw-input/condition_dsp condition for
    // every stack, unconditionally consumed learned input rechannels,
    // chained head accumulators with the model output taken from the last
    // stack only, no extra skip activation, and an identity residual for
    // layers without a layer1x1. The only per-file variation left is the
    // ACTIVATION FLAVOR (see
    // [`super::super::parse::WaveNetConfig::fast_activations`]).
    rechannels: Vec<Conv1x1>,
    stacks: Vec<Vec<WaveNetLayer>>,
    head_rechannels: Vec<HeadRechannel>,
    ring_buffers: Vec<Vec<RingBuffer>>,
    /// Per-stack history of past skip-accumulator frames for windowed head
    /// rechannels (`Some` iff `head_kernel_size > 1`; the kernel-1 path is
    /// memoryless and skips the ring entirely).
    head_rings: Vec<Option<RingBuffer>>,

    // Head MLP (may be empty)
    head_layers: Vec<DenseLayer>,
    head_scale: f32,
    /// Activation for head MLP hidden layers, resolved from config at
    /// construction (A1: fast tanh).
    head_activation: Activation,

    // Pre-allocated scratch buffers (sized for max needed)
    activation: Vec<f32>,
    conv_out: Vec<f32>,  // mid_ch sized; holds the activated z (bottleneck)
    mixin_buf: Vec<f32>, // mid_ch sized
    /// Pre-activation copy of the primary half, for blended gating
    /// (bottleneck sized).
    pre_act_buf: Vec<f32>,
    residual_buf: Vec<f32>,
    /// Skip accumulator: `head1x1.out_channels` wide for stacks with an
    /// active head1x1, else bottleneck wide.
    skip_accum: Vec<f32>,
    /// Per-layer head1x1 output (head1x1.out_channels sized).
    head1x1_buf: Vec<f32>,
    /// FiLM scale/shift scratch, sized for the widest active FiLM conv
    /// output (`max out_ch` = up to twice the widest modulated tensor).
    film_ss_buf: Vec<f32>,
    /// FiLM output scratch for the const-input sites (conv_pre input copy,
    /// input_mixin_pre modulated condition); max(channels, condition_size)
    /// sized.
    film_pre_buf: Vec<f32>,
    rechannel_buf: Vec<f32>,
    head_input: Vec<f32>, // head_size sized, accumulated across stacks
    head_buf_a: Vec<f32>,
    head_buf_b: Vec<f32>,
}

impl WaveNetModel {
    /// Number of input channels this model expects (`config.input_size`).
    pub fn in_channels(&self) -> usize {
        self.in_channels
    }

    /// Number of output channels this model produces (reference
    /// `wave_net_output_channels`): the head MLP width when present, else
    /// the last stack's head rechannel width. 1 for main audio models;
    /// nested condition_dsp models are multi-channel.
    pub fn out_channels(&self) -> usize {
        self.out_channels
    }

    /// Multi-channel inference used for nested condition_dsp models: runs
    /// one input sample and writes the `out_channels()` head-scaled output
    /// channels into `out` (which must be at least that wide).
    /// Allocation-free; shares the forward pass with `process_sample`.
    pub fn process_sample_into(&mut self, input: f32, out: &mut [f32]) {
        self.forward(input);
        let n = self.out_channels;
        for (o, v) in out[..n].iter_mut().zip(&self.head_input[..n]) {
            *o = v * self.head_scale;
        }
    }
}

impl NamInference for WaveNetModel {
    fn process_sample(&mut self, input: f32) -> f32 {
        self.forward(input);
        self.head_input[0] * self.head_scale
    }

    fn reset(&mut self) {
        // Nested condition_dsp state (its rings, recursively) must clear
        // too, or the first samples after reset would see a stale
        // condition.
        if let Some(cd) = &mut self.condition_dsp {
            cd.reset();
        }
        self.condition_buf.fill(0.0);
        for stack_rings in &mut self.ring_buffers {
            for ring in stack_rings {
                ring.reset();
            }
        }
        for ring in self.head_rings.iter_mut().flatten() {
            ring.reset();
        }
        self.activation.fill(0.0);
        self.skip_accum.fill(0.0);
        self.head_input.fill(0.0);
    }
}
