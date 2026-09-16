//! Flat-weight-stream readers: everything that consumes the NAM weight
//! array to build the per-stack model pieces (rechannel, layers with their
//! FiLM sites, head rechannel) and the legacy head MLP.
//!
//! Consumption order is reference-exact (`set_weights_` in
//! NAM/wavenet/model.cpp); see the module docs on [`super`] for the layout.

use super::super::super::activations::Activation;
use super::super::super::parse::{checked_count, StackConfig, WaveNetConfig, WeightReader};
use super::super::conv_layer::{Conv1x1, Conv1x1Bias, LayerGating, WaveNetLayer};
use super::super::film::Film;
use super::super::head::{DenseLayer, HeadRechannel};
use super::super::params::{FilmParams, GatingMode};
use super::super::ring::RingBuffer;

/// Construction-time validation of a grouped convolution's channel counts
/// (reference `Conv1D::set_size_` / `Conv1x1` ctor): both the input and the
/// output channel count must divide evenly by the group count.
pub(super) fn check_groups(
    ctx: &str,
    what: &str,
    in_ch: usize,
    out_ch: usize,
    groups: usize,
) -> Result<(), String> {
    if groups == 0 {
        return Err(format!("{ctx}: {what} groups must be >= 1"));
    }
    if !in_ch.is_multiple_of(groups) {
        return Err(format!(
            "{ctx}: {what} in_channels ({in_ch}) must be divisible by groups ({groups})"
        ));
    }
    if !out_ch.is_multiple_of(groups) {
        return Err(format!(
            "{ctx}: {what} out_channels ({out_ch}) must be divisible by groups ({groups})"
        ));
    }
    Ok(())
}

/// Read one FiLM insertion point's weights, if active (reference
/// `FiLM::set_weights_` -> `Conv1x1::set_weights_`): the biased grouped 1x1
/// conv mapping the condition (`cond_ch` wide) to `(shift ? 2 : 1) * width`
/// scale/shift channels — compact grouped weights, then the bias.
fn read_film(
    reader: &mut WeightReader,
    ctx: &str,
    site: &str,
    cond_ch: usize,
    width: usize,
    params: &FilmParams,
) -> Result<Option<Film>, String> {
    if !params.active {
        return Ok(None);
    }
    let out_ch = if params.shift { 2 * width } else { width };
    check_groups(ctx, site, cond_ch, out_ch, params.groups)?;
    // groups divides out_ch (checked above), so the grouped compact count
    // out_ch * cond_ch / groups can be computed division-first, keeping
    // the checked product small.
    let weight = reader.read(checked_count(ctx, &[out_ch / params.groups, cond_ch])?)?;
    let bias = reader.read(out_ch)?;
    Ok(Some(Film {
        weight,
        bias,
        cond_ch,
        out_ch,
        width,
        groups: params.groups,
        shift: params.shift,
    }))
}

/// Everything one stack (reference LayerArray) contributes to the model.
pub(super) struct StackParts {
    pub(super) rechannel: Conv1x1,
    pub(super) layers: Vec<WaveNetLayer>,
    pub(super) rings: Vec<RingBuffer>,
    pub(super) head_rechannel: HeadRechannel,
    pub(super) head_ring: Option<RingBuffer>,
}

/// Read one stack's weights in reference order: input rechannel, then each
/// layer, then the head rechannel.
pub(super) fn read_stack(
    reader: &mut WeightReader,
    si: usize,
    stack_cfg: &StackConfig,
    prev_ch: usize,
    has_layer1x1: bool,
    fast: bool,
) -> Result<StackParts, String> {
    let ch = stack_cfg.channels;
    let bottleneck = stack_cfg.bottleneck;
    // Whether this stack's layers carry layer1x1 weights: governed
    // by the config-wide format flag (old flat format has none)
    // AND the per-stack A2 `layer1x1.active` (reference
    // `Layer1x1Params`: inactive consumes no weights, identity
    // residual).
    let l1x1_active = has_layer1x1 && stack_cfg.layer1x1_active;
    // Reference validation: without a layer1x1 there is nothing to
    // map the bottleneck-wide activation back to `channels`.
    if !l1x1_active && bottleneck != ch {
        return Err(format!(
            "WaveNet config: bottleneck ({bottleneck}) must equal channels ({ch}) when layer1x1 is inactive"
        ));
    }
    // Reference validations (Layer ctor, NAM/wavenet/detail.h): a
    // post-FiLM on an inactive conv would be redundant weights.
    if stack_cfg.films.layer1x1_post.active && !l1x1_active {
        return Err(format!(
            "WaveNet stack {si}: layer1x1_post_film cannot be active when layer1x1 is not active"
        ));
    }
    if stack_cfg.films.head1x1_post.active && !stack_cfg.head1x1.active {
        return Err(format!(
            "WaveNet stack {si}: head1x1_post_film cannot be active when head1x1 is not active"
        ));
    }

    // --- Rechannel (1x1, no bias) ---
    // The reference LayerArray ctor constructs
    // `_rechannel(params.input_size, params.channels, false)`
    // UNCONDITIONALLY and `set_weights_` always consumes its
    // input_size*channels weights — a 1-to-1 rechannel is a learned
    // conv, not an identity (trainers export its weights even for
    // equal widths).
    let weight = reader.read(checked_count(
        &format!("WaveNet stack {si} rechannel"),
        &[ch, prev_ch],
    )?)?;
    let rechannel = Conv1x1 {
        weight,
        out_ch: ch,
        in_ch: prev_ch,
    };

    // --- Layers ---
    let mut layers = Vec::with_capacity(stack_cfg.dilations.len());
    let mut rings = Vec::with_capacity(stack_cfg.dilations.len());
    for (layer_idx, &dilation) in stack_cfg.dilations.iter().enumerate() {
        let (layer, ring) = read_layer(reader, si, layer_idx, stack_cfg, dilation, l1x1_active, fast)?;
        layers.push(layer);
        rings.push(ring);
    }

    let (head_rechannel, head_ring) = read_head_rechannel(reader, si, stack_cfg)?;

    Ok(StackParts {
        rechannel,
        layers,
        rings,
        head_rechannel,
        head_ring,
    })
}

/// Read one layer's weights (reference `Layer::set_weights_` order: conv,
/// input_mixin, layer1x1, head1x1, then the FiLM sites) and build its
/// dilated-conv state ring.
fn read_layer(
    reader: &mut WeightReader,
    si: usize,
    layer_idx: usize,
    stack_cfg: &StackConfig,
    dilation: usize,
    l1x1_active: bool,
    fast: bool,
) -> Result<(WaveNetLayer, RingBuffer), String> {
    let ch = stack_cfg.channels;
    let bottleneck = stack_cfg.bottleneck;
    let g_in = stack_cfg.groups_input;
    let g_mixin = stack_cfg.groups_input_mixin;
    let g_1x1 = stack_cfg.layer1x1_groups;
    let ks = stack_cfg.kernel_sizes[layer_idx];

    // Per-layer activation config: a single entry broadcasts to
    // every layer (A1 / A2 single configs); A2 activation arrays
    // give each layer its own (e.g. the wavenet_a2_max nested
    // condition_dsp mixes PReLU and Softsign per layer).
    let activation_cfg = if stack_cfg.activations.len() == 1 {
        &stack_cfg.activations[0]
    } else {
        &stack_cfg.activations[layer_idx]
    };

    // Conv/mixin output width: doubled for gated/blended layers
    // (primary + secondary halves), reference Layer ctor:
    // `gating_mode != NONE ? 2*bottleneck : bottleneck`.
    let mode = stack_cfg.gating_modes[layer_idx];
    let mid_ch = if mode == GatingMode::None {
        bottleneck
    } else {
        bottleneck * 2
    };
    // Secondary (gate/blend) activation: config-driven, with the
    // reference backward-compat `Sigmoid` default resolving to
    // the fast sigmoid for A1-flavor files and the exact
    // sigmoid for A2-marked ones.
    let gating = match mode {
        GatingMode::None => LayerGating::None,
        GatingMode::Gated | GatingMode::Blended => {
            let secondary = match &stack_cfg.secondary_activations[layer_idx] {
                Some(cfg) => Activation::secondary_from_config(cfg, fast),
                None if fast => Activation::FastSigmoid,
                None => Activation::Sigmoid,
            };
            if mode == GatingMode::Gated {
                LayerGating::Gated(secondary)
            } else {
                LayerGating::Blended(secondary)
            }
        }
    };

    let ctx = format!("WaveNet stack {si} layer {layer_idx}");

    // _conv.weight [mid_ch, ch/g, kernel_size]: the grouped conv
    // shrinks the weight tensor to per-group [mid_ch/g x ch/g]
    // blocks (reference Conv1D::set_size_/set_weights_ in
    // NAM/conv1d.cpp). Flat consumption order is
    // [group][out][in][tap]; splitting off the innermost tap
    // index (`raw[m * ks + tap]`) leaves exactly the compact
    // per-tap grouped layout `grouped_matvec` consumes —
    // concatenated per-group row-major blocks — and for g == 1
    // this is the historical dense [mid_ch x ch] matrix.
    check_groups(&ctx, "conv", ch, mid_ch, g_in)?;
    let per_tap = checked_count(&ctx, &[mid_ch / g_in, ch])?;
    let raw = reader.read(checked_count(&ctx, &[per_tap, ks])?)?;
    let mut w_conv = Vec::with_capacity(ks);
    for tap in 0..ks {
        let mut w = vec![0.0f32; per_tap];
        for (m, wv) in w.iter_mut().enumerate() {
            *wv = raw[m * ks + tap];
        }
        w_conv.push(w);
    }

    // _conv.bias [mid_ch]
    let b_conv = reader.read(mid_ch)?;

    // _input_mixin.weight [mid_ch, condition_size/g] (no bias).
    // The reference Conv1x1 flat order [group][out][in] IS the
    // compact concatenated per-group layout — read as-is.
    let w_input_mixin = if stack_cfg.condition_size > 0 {
        check_groups(&ctx, "input_mixin", stack_cfg.condition_size, mid_ch, g_mixin)?;
        Some(reader.read(checked_count(
            &ctx,
            &[mid_ch / g_mixin, stack_cfg.condition_size],
        )?)?)
    } else {
        None
    };

    // _layer1x1: learned 1x1 residual conv mapping the
    // bottleneck-wide activation back to `channels` (active by
    // default in new-format NAM). Weight [ch, bottleneck/g],
    // bias [ch] — reference Conv1x1(bottleneck, channels, bias,
    // layer1x1.groups); the flat grouped order is the compact
    // per-group layout, read as-is.
    let layer1x1 = if l1x1_active {
        check_groups(&ctx, "layer1x1", bottleneck, ch, g_1x1)?;
        let w = reader.read(checked_count(&ctx, &[ch / g_1x1, bottleneck])?)?;
        let b = reader.read(ch)?;
        Some(Conv1x1Bias {
            weight: w,
            bias: b,
            out_ch: ch,
            in_ch: bottleneck,
            groups: g_1x1,
        })
    } else {
        None
    };

    // _head1x1: optional 1x1 skip conv mapping the activated z
    // (bottleneck) to head1x1.out_channels. Always biased and
    // consumed AFTER the layer1x1 (reference Layer::set_weights_:
    // conv, input_mixin, layer1x1, head1x1) — reference
    // `Conv1x1(bottleneck, out_channels, true, groups)`; the
    // flat grouped order is the compact per-group layout, read
    // as-is.
    let head1x1 = if stack_cfg.head1x1.active {
        let h_out = stack_cfg.head1x1.out_channels;
        let h_groups = stack_cfg.head1x1.groups;
        check_groups(&ctx, "head1x1", bottleneck, h_out, h_groups)?;
        let w = reader.read(checked_count(&ctx, &[h_out / h_groups, bottleneck])?)?;
        let b = reader.read(h_out)?;
        Some(Conv1x1Bias {
            weight: w,
            bias: b,
            out_ch: h_out,
            in_ch: bottleneck,
            groups: h_groups,
        })
    } else {
        None
    };

    // FiLM insertion points, consumed AFTER the layer's other
    // tensors and in reference site order (Layer::set_weights_
    // in NAM/wavenet/model.cpp: conv, input_mixin, layer1x1,
    // head1x1, then conv_pre/conv_post/input_mixin_pre/
    // input_mixin_post/activation_pre/activation_post/
    // layer1x1_post/head1x1_post films). Widths per site follow
    // the reference Layer ctor: conv_pre modulates the layer
    // input (`channels`), conv_post / input_mixin_post /
    // activation_pre the conv-width z (`2*bottleneck` when
    // gated/blended, else `bottleneck` — mid_ch),
    // input_mixin_pre the condition itself (`condition_size`),
    // activation_post the activated z (`bottleneck`),
    // layer1x1_post the residual conv output (`channels`), and
    // head1x1_post the head1x1 output (`head1x1.out_channels`).
    let cond = stack_cfg.condition_size;
    let films = &stack_cfg.films;
    let conv_pre_film = read_film(reader, &ctx, "conv_pre_film", cond, ch, &films.conv_pre)?;
    let conv_post_film = read_film(reader, &ctx, "conv_post_film", cond, mid_ch, &films.conv_post)?;
    let input_mixin_pre_film = read_film(
        reader,
        &ctx,
        "input_mixin_pre_film",
        cond,
        cond,
        &films.input_mixin_pre,
    )?;
    let input_mixin_post_film = read_film(
        reader,
        &ctx,
        "input_mixin_post_film",
        cond,
        mid_ch,
        &films.input_mixin_post,
    )?;
    let activation_pre_film = read_film(
        reader,
        &ctx,
        "activation_pre_film",
        cond,
        mid_ch,
        &films.activation_pre,
    )?;
    let activation_post_film = read_film(
        reader,
        &ctx,
        "activation_post_film",
        cond,
        bottleneck,
        &films.activation_post,
    )?;
    let layer1x1_post_film =
        read_film(reader, &ctx, "layer1x1_post_film", cond, ch, &films.layer1x1_post)?;
    let head1x1_post_film = read_film(
        reader,
        &ctx,
        "head1x1_post_film",
        cond,
        stack_cfg.head1x1.out_channels,
        &films.head1x1_post,
    )?;

    let ring_capacity = (ks - 1) * dilation + 2;
    let ring = RingBuffer::new(ring_capacity, ch);

    let layer = WaveNetLayer {
        w_conv,
        b_conv,
        w_input_mixin,
        layer1x1,
        head1x1,
        kernel_size: ks,
        dilation,
        channels: ch,
        bottleneck,
        mid_ch,
        groups_input: g_in,
        groups_input_mixin: g_mixin,
        // Resolve activation dispatch once, at construction.
        // A1-flavor files map the "Tanh" config to the
        // fast-tanh path (matching the official plugin's
        // enable_fast_tanh); A2-marked files use the exact
        // functions.
        activation: Activation::from_config(activation_cfg, fast),
        gating,
        conv_pre_film,
        conv_post_film,
        input_mixin_pre_film,
        input_mixin_post_film,
        activation_pre_film,
        activation_post_film,
        layer1x1_post_film,
        head1x1_post_film,
    };
    Ok((layer, ring))
}

/// Read the stack's head rechannel (causal conv, kernel
/// `head_kernel_size`, dilation `head_dilation`, bias controlled by
/// head_bias) and, for windowed heads, build its skip-frame history ring.
///
/// Its input is the accumulated skip signal: head1x1.out_channels wide
/// when the stack's head1x1 is active, else bottleneck wide (reference
/// `_head_rechannel(head1x1.active ? head1x1.out_channels : bottleneck,
/// head_size, head_kernel_size, head_bias, head_dilation, 1)`).
fn read_head_rechannel(
    reader: &mut WeightReader,
    si: usize,
    stack_cfg: &StackConfig,
) -> Result<(HeadRechannel, Option<RingBuffer>), String> {
    let skip_ch = if stack_cfg.head1x1.active {
        stack_cfg.head1x1.out_channels
    } else {
        stack_cfg.bottleneck
    };
    let hr_out = stack_cfg.head_size;
    let hr_ks = stack_cfg.head_kernel_size;
    if hr_ks == 0 {
        return Err(format!(
            "WaveNet stack {si}: head_kernel_size must be >= 1"
        ));
    }
    // Weight order matches the reference `Conv1D::set_weights_`
    // (groups = 1): flat [out][in][tap]. Splitting off the innermost
    // tap index (`raw[m * hr_ks + tap]`) yields one compact
    // [out x in] matrix per tap; for hr_ks == 1 this is the
    // historical dense 1x1 head rechannel matrix bit-for-bit.
    let hr_ctx = format!("WaveNet stack {si} head rechannel");
    let per_tap = checked_count(&hr_ctx, &[hr_out, skip_ch])?;
    let raw = reader.read(checked_count(&hr_ctx, &[per_tap, hr_ks])?)?;
    let mut hr_taps = Vec::with_capacity(hr_ks);
    for tap in 0..hr_ks {
        let mut w = vec![0.0f32; per_tap];
        for (m, wv) in w.iter_mut().enumerate() {
            *wv = raw[m * hr_ks + tap];
        }
        hr_taps.push(w);
    }
    let hr_bias = if stack_cfg.head_bias {
        reader.read(hr_out)?
    } else {
        vec![0.0; hr_out]
    };
    let head_rechannel = HeadRechannel {
        taps: hr_taps,
        bias: hr_bias,
        out_ch: hr_out,
        in_ch: skip_ch,
        dilation: stack_cfg.head_dilation,
    };
    // Windowed heads need a history of past skip-accumulator
    // frames; capacity mirrors the per-layer conv rings.
    let head_ring = if hr_ks > 1 {
        Some(RingBuffer::new(
            (hr_ks - 1) * stack_cfg.head_dilation + 2,
            skip_ch,
        ))
    } else {
        None
    };
    Ok((head_rechannel, head_ring))
}

/// Read the legacy head MLP layers (hidden layers + final output layer),
/// if the config carries any.
pub(super) fn read_head_mlp(
    reader: &mut WeightReader,
    config: &WaveNetConfig,
) -> Result<Vec<DenseLayer>, String> {
    let head_size = config.head_size;
    let mut head_layers = Vec::new();
    let mut prev_size = head_size;
    for &hidden in &config.head {
        let weight = reader.read(checked_count("WaveNet head MLP", &[hidden, prev_size])?)?;
        let bias = reader.read(hidden)?;
        head_layers.push(DenseLayer {
            weight,
            bias,
            in_features: prev_size,
            out_features: hidden,
            has_activation: true,
        });
        prev_size = hidden;
    }
    // Final output layer (if head has hidden layers)
    if !config.head.is_empty() {
        let weight = reader.read(checked_count("WaveNet head MLP", &[head_size, prev_size])?)?;
        let bias = reader.read(head_size)?;
        head_layers.push(DenseLayer {
            weight,
            bias,
            in_features: prev_size,
            out_features: head_size,
            has_activation: false,
        });
    }
    Ok(head_layers)
}
