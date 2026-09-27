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
//! - [`forward`] — the block forward pass, split into per-stage methods.

mod build;
mod forward;
mod validate;
mod weights;

use super::super::activations::Activation;
use super::super::NamInference;
use super::conv_layer::{Conv1x1, WaveNetLayer};
use super::head::{DenseLayer, HeadRechannel};
use super::history::History;

/// Frames per internal block. Host blocks are split into chunks of at most
/// this many frames; 64 keeps every frame-major scratch block of the
/// standard architecture within a few KB, so a whole layer's working set
/// stays in L1.
pub(super) const MAX_BLOCK: usize = 64;

pub struct WaveNetModel {
    /// A2 condition_dsp sub-network: a nested WaveNet that transforms the
    /// raw input sample into the condition signal before it feeds the
    /// stacks (reference `WaveNet::_process_condition`). Its weights come
    /// from the nested model object's own weight array, never from the
    /// outer flat stream. `None` = the condition is the raw input (the
    /// reference `_process_condition` passthrough).
    condition_dsp: Option<Box<WaveNetModel>>,
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
    /// Per-layer dilated-conv input history.
    histories: Vec<Vec<History>>,
    /// Per-stack history of past skip-accumulator frames for windowed head
    /// rechannels (`Some` iff `head_kernel_size > 1`; the kernel-1 path is
    /// memoryless and keeps none).
    head_histories: Vec<Option<History>>,

    // Head MLP (may be empty)
    head_layers: Vec<DenseLayer>,
    head_scale: f32,
    /// Activation for head MLP hidden layers, resolved from config at
    /// construction (A1: fast tanh).
    head_activation: Activation,

    // Block scratch, frame-major: frame `t` of an `n`-frame block occupies
    // `[t * width .. t * width + width]`, with `width` the tensor's width
    // at that point of the pass (so every block is a dense matrix the
    // GEMM reads directly). Each is `MAX_BLOCK` frames of the widest
    // width it ever holds.
    /// Condition per frame, `cond_w` wide: the condition_dsp output, or the
    /// raw input (width 1) without one.
    cond: Vec<f32>,
    cond_w: usize,
    /// Layer input / residual stream (stack channels wide).
    act: Vec<f32>,
    /// Rechannel output, swapped with `act` (max_ch wide).
    act_next: Vec<f32>,
    /// Conv + mixin output z (mid_ch wide); holds the activated z in
    /// the first `bottleneck` columns of each frame after activation.
    z: Vec<f32>,
    /// Input-mixin output when a post-FiLM has to see it on its own
    /// (mid_ch wide).
    mixin: Vec<f32>,
    /// layer1x1 / head1x1 output when a post-FiLM has to see it on its
    /// own (max(channels, skip) wide).
    conv_tmp: Vec<f32>,
    /// FiLM-modulated copies for the const-input sites (conv_pre input,
    /// input_mixin_pre condition); max_ch wide.
    film_in: Vec<f32>,
    /// Skip accumulator (skip_ch wide).
    skip: Vec<f32>,
    /// Head rechannel outputs, `head_w` wide for every stack: after the
    /// last stack (and the head MLP) its first `out_channels` columns are
    /// the pre-`head_scale` model output. Zeroed per block, so columns a
    /// narrower stack leaves unwritten read as zero.
    head: Vec<f32>,
    head_w: usize,

    // Per-frame scratch.
    /// FiLM scale/shift scratch, sized for the widest active FiLM conv
    /// output (`max out_ch` = up to twice the widest modulated tensor).
    film_ss_buf: Vec<f32>,
    /// Pre-activation copy of the primary half, for blended gating
    /// (bottleneck sized).
    pre_act_buf: Vec<f32>,
    /// Head MLP ping-pong buffers.
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

    /// Multi-channel inference of one sample: writes the `out_channels()`
    /// head-scaled output channels into `out` (at least that wide).
    pub fn process_sample_into(&mut self, input: f32, out: &mut [f32]) {
        self.process_block_into(&[input], out);
    }

    /// Multi-channel block inference used for nested condition_dsp models:
    /// runs `input.len()` samples and writes, frame-major, the
    /// `out_channels()` head-scaled output channels of each into `out`
    /// (at least `input.len() * out_channels()` long). Allocation-free.
    pub fn process_block_into(&mut self, input: &[f32], out: &mut [f32]) {
        let oc = self.out_channels;
        for (inp, o) in input.chunks(MAX_BLOCK).zip(out.chunks_mut(MAX_BLOCK * oc)) {
            self.forward_block(inp);
            for t in 0..inp.len() {
                let row = &self.head[t * self.head_w..][..oc];
                for (d, v) in o[t * oc..][..oc].iter_mut().zip(row) {
                    *d = v * self.head_scale;
                }
            }
        }
    }
}

impl NamInference for WaveNetModel {
    fn process_sample(&mut self, input: f32) -> f32 {
        self.forward_block(&[input]);
        self.head[0] * self.head_scale
    }

    fn process_block(&mut self, input: &[f32], output: &mut [f32]) {
        for (inp, out) in input.chunks(MAX_BLOCK).zip(output.chunks_mut(MAX_BLOCK)) {
            self.forward_block(inp);
            for (t, o) in out.iter_mut().enumerate() {
                *o = self.head[t * self.head_w] * self.head_scale;
            }
        }
    }

    fn reset(&mut self) {
        // Nested condition_dsp state (its histories, recursively) must
        // clear too, or the first samples after reset would see a stale
        // condition.
        if let Some(cd) = &mut self.condition_dsp {
            cd.reset();
        }
        for h in self.histories.iter_mut().flatten() {
            h.reset();
        }
        for h in self.head_histories.iter_mut().flatten() {
            h.reset();
        }
    }
}
