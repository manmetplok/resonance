//! Construction: `from_config_and_weights` orchestration, condition_dsp
//! sub-network building, and scratch-buffer sizing.

use super::super::super::activations::Activation;
use super::super::super::parse::{WaveNetConfig, WeightReader};
use super::super::conv_layer::{Conv1x1, WaveNetLayer};
use super::super::params::GatingMode;
use super::super::super::gemm::transpose_grouped;
use super::super::head::HeadRechannel;
use super::{validate, weights, WaveNetModel, MAX_BLOCK};

/// Scratch-buffer widths derived from the config (each the max over all
/// stacks of the relevant per-stack width).
struct ScratchDims {
    /// Widest channel-shaped buffer: covers channels AND the condition
    /// width (a condition_dsp sub-model may feed wider condition signals
    /// than the audio path, e.g. the wavenet_a2_max fixture), plus the
    /// model input width (the first rechannel reads `input_size` channels;
    /// wider-than-channels inputs are possible in hand-written nested
    /// configs).
    max_ch: usize,
    /// Widest bottleneck.
    max_bn: usize,
    /// Conv/mixin scratch width: doubled for layers with gated/blended
    /// gating (primary + secondary halves).
    max_mid: usize,
    /// Skip accumulator width per stack: head1x1.out_channels when the
    /// stack's head1x1 is active, else bottleneck (reference
    /// `_head_output_size`).
    max_skip: usize,
    /// Widest per-stack head rechannel output. Head buffers must cover
    /// every stack's head rechannel width, not just the first stack's
    /// `head_size`: a nested condition_dsp net's arrays may widen toward
    /// the output (e.g. head 2 -> 3 in the wavenet_condition_dsp fixture).
    /// A1 models have their widest head first, so sizes are unchanged
    /// there.
    max_stack_head: usize,
    /// Head MLP ping-pong buffer width.
    max_head_buf: usize,
}

impl ScratchDims {
    fn from_config(config: &WaveNetConfig) -> Self {
        let max_ch = config
            .stacks
            .iter()
            .flat_map(|s| [s.channels, s.condition_size])
            .max()
            .unwrap_or(1)
            .max(config.input_size)
            .max(1);
        let max_bn = config
            .stacks
            .iter()
            .map(|s| s.bottleneck)
            .max()
            .unwrap_or(1);
        let max_mid = config
            .stacks
            .iter()
            .map(|s| {
                if s.gating_modes.iter().any(|m| *m != GatingMode::None) {
                    s.bottleneck * 2
                } else {
                    s.bottleneck
                }
            })
            .max()
            .unwrap_or(1);
        let max_skip = config
            .stacks
            .iter()
            .map(|s| {
                if s.head1x1.active {
                    s.head1x1.out_channels
                } else {
                    s.bottleneck
                }
            })
            .max()
            .unwrap_or(1)
            .max(1);
        let max_stack_head = config
            .stacks
            .iter()
            .map(|s| s.head_size)
            .max()
            .unwrap_or(config.head_size);
        let max_head_buf = config
            .head
            .iter()
            .copied()
            .chain(std::iter::once(config.head_size))
            .chain(std::iter::once(max_ch))
            .chain(std::iter::once(max_stack_head))
            .max()
            .unwrap_or(1);
        Self {
            max_ch,
            max_bn,
            max_mid,
            max_skip,
            max_stack_head,
            max_head_buf,
        }
    }
}

/// Build the condition_dsp sub-network (A2), if the config carries one.
///
/// Built recursively from its own nested model JSON BEFORE the main
/// arrays touch the flat weight stream, and consuming none of it
/// (reference WaveNet ctor + set_weights_: "condition_dsp already
/// has its own weights from construction"). Validations mirror the
/// reference ctor: the nested net's input width must match what the
/// WaveNet feeds it (the raw model input), and its output width must
/// match every stack's condition_size.
fn build_condition_net(config: &WaveNetConfig) -> Result<Option<Box<WaveNetModel>>, String> {
    let Some(value) = &config.condition_dsp else {
        return Ok(None);
    };
    let nested = super::super::super::parse::build_condition_dsp(value)?;
    if nested.in_channels() != config.input_size {
        return Err(format!(
            "input channels of WaveNet ({}) don't match input channels of condition DSP ({})",
            config.input_size,
            nested.in_channels()
        ));
    }
    for (si, s) in config.stacks.iter().enumerate() {
        if s.condition_size != nested.out_channels() {
            return Err(format!(
                "condition_size of stack {si} ({}) doesn't match output channels of condition DSP ({})",
                s.condition_size,
                nested.out_channels()
            ));
        }
    }
    Ok(Some(Box::new(nested)))
}

/// Model output width (reference `wave_net_output_channels`): the head
/// MLP's output size when present, else the LAST stack's head rechannel
/// width.
fn output_channels(config: &WaveNetConfig) -> usize {
    if config.head.is_empty() {
        config
            .stacks
            .last()
            .map(|s| s.head_size)
            .unwrap_or(config.head_size)
    } else {
        config.head_size
    }
}

/// FiLM scale/shift scratch width: widest active FiLM conv output across
/// all layers (0 films -> minimal 1-slot buffer, untouched).
fn max_film_out(stacks: &[Vec<WaveNetLayer>]) -> usize {
    stacks
        .iter()
        .flatten()
        .flat_map(|l| l.films())
        .flatten()
        .map(|f| f.out_ch)
        .max()
        .unwrap_or(0)
        .max(1)
}

/// Transpose every block-GEMM weight (rechannels, dilated-conv taps, input
/// mixins, layer1x1, head1x1, head-rechannel taps) from the reference
/// `[out][in]` per-group layout to lane-padded `[in][out]`, the layout
/// [`grouped_gemm_acc`](super::super::super::gemm::grouped_gemm_acc)
/// consumes. Runs once, after the dimension validation. FiLM and head-MLP
/// weights stay `[out][in]`: they run per frame through the matvec.
fn transpose_for_block_gemm(
    rechannels: &mut [Conv1x1],
    stacks: &mut [Vec<WaveNetLayer>],
    head_rechannels: &mut [HeadRechannel],
) {
    for rc in rechannels {
        rc.weight = transpose_grouped(&rc.weight, rc.out_ch, rc.in_ch, 1);
    }
    for layer in stacks.iter_mut().flatten() {
        let mid = layer.mid_ch;
        for w in &mut layer.w_conv {
            *w = transpose_grouped(w, mid, layer.channels, layer.groups_input);
        }
        if let Some(w) = &mut layer.w_input_mixin {
            *w = transpose_grouped(w, mid, layer.condition_size, layer.groups_input_mixin);
        }
        for c in [&mut layer.layer1x1, &mut layer.head1x1].into_iter().flatten() {
            c.weight = transpose_grouped(&c.weight, c.out_ch, c.in_ch, c.groups);
        }
    }
    for hr in head_rechannels {
        for w in &mut hr.taps {
            *w = transpose_grouped(w, hr.out_ch, hr.in_ch, 1);
        }
    }
}

impl WaveNetModel {
    pub fn from_config_and_weights(
        config: WaveNetConfig,
        reader: &mut WeightReader,
    ) -> Result<Self, String> {
        validate::validate_config(&config)?;
        validate::validate_bounds(&config, reader.remaining())?;

        // Activation flavor (the ONLY per-file semantic left, ba todo
        // #1116): files expressible in the pre-A2 surface resolve
        // `Tanh`/`Sigmoid` to the fast approximations — the official NAM
        // plugin runs with `enable_fast_tanh()`, so correct-vs-plugin for
        // A1 files means reference structure + fast activations. A2-marked
        // files (and forced-exact nested/container submodels) use the
        // exact functions, matching how the A2 fixture outputs were
        // rendered (`tools/render` never enables fast tanh).
        let fast = config.fast_activations;

        let condition_dsp = build_condition_net(&config)?;

        let dims = ScratchDims::from_config(&config);
        let head_size = config.head_size;
        let out_channels = output_channels(&config);

        let num_stacks = config.stacks.len();
        let mut rechannels: Vec<Conv1x1> = Vec::with_capacity(num_stacks);
        let mut stacks = Vec::with_capacity(num_stacks);
        let mut head_rechannels = Vec::with_capacity(num_stacks);
        let mut histories = Vec::with_capacity(num_stacks);
        let mut head_histories = Vec::with_capacity(num_stacks);

        let mut prev_ch = config.input_size;
        for (si, stack_cfg) in config.stacks.iter().enumerate() {
            let parts =
                weights::read_stack(reader, si, stack_cfg, prev_ch, config.has_layer1x1, fast)?;
            rechannels.push(parts.rechannel);
            stacks.push(parts.layers);
            histories.push(parts.rings);
            head_rechannels.push(parts.head_rechannel);
            head_histories.push(parts.head_ring);
            prev_ch = stack_cfg.channels;
        }

        // --- Head MLP layers ---
        let head_layers = weights::read_head_mlp(reader, &config)?;

        // --- Head scale (last weight) ---
        let head_scale = if reader.remaining() >= 1 {
            reader.read(1)?[0]
        } else {
            1.0
        };

        // Head MLP hidden-layer activation (A1: "Tanh" -> fast tanh; taken
        // from the first stack's first layer activation, as before). Only
        // configs carrying a legacy MLP-shaped head reach this path (the
        // A2-style post-stack head is rejected at parse).
        let head_activation = config
            .stacks
            .first()
            .and_then(|s| s.activations.first())
            .map(|a| Activation::from_config(a, fast))
            .unwrap_or(Activation::FastTanh);

        let max_film_ss = max_film_out(&stacks);

        // Per-frame representative slices for the load-time dimension
        // validation below.
        let scratch_activation = vec![0.0f32; dims.max_ch];
        let scratch_conv_out = vec![0.0f32; dims.max_mid];
        let scratch_skip = vec![0.0f32; dims.max_skip];
        let scratch_head_buf = vec![0.0f32; dims.max_head_buf];
        let scratch_film_ss = vec![0.0f32; max_film_ss];

        validate::validate_dims(
            &rechannels,
            &stacks,
            &head_rechannels,
            &head_layers,
            &validate::ScratchSlices {
                activation: &scratch_activation,
                conv_out: &scratch_conv_out,
                skip: &scratch_skip,
                head_buf: &scratch_head_buf,
                film_ss: &scratch_film_ss,
            },
        )?;

        transpose_for_block_gemm(&mut rechannels, &mut stacks, &mut head_rechannels);

        // Without a condition_dsp the condition is the raw (mono) input.
        let cond_w = condition_dsp.as_ref().map_or(1, |cd| cd.out_channels());
        let head_w = head_size.max(dims.max_stack_head);
        let block = |width: usize| vec![0.0f32; MAX_BLOCK * width];

        Ok(Self {
            condition_dsp,
            in_channels: config.input_size,
            head_size,
            out_channels,
            rechannels,
            stacks,
            head_rechannels,
            histories,
            head_histories,
            head_layers,
            head_scale,
            head_activation,
            cond: block(cond_w),
            cond_w,
            act: block(dims.max_ch),
            act_next: block(dims.max_ch),
            z: block(dims.max_mid),
            mixin: block(dims.max_mid),
            conv_tmp: block(dims.max_ch.max(dims.max_skip)),
            film_in: block(dims.max_ch),
            skip: block(dims.max_skip),
            head: block(head_w),
            head_w,
            film_ss_buf: scratch_film_ss,
            pre_act_buf: vec![0.0; dims.max_bn],
            head_buf_a: scratch_head_buf,
            head_buf_b: vec![0.0; dims.max_head_buf],
        })
    }
}
