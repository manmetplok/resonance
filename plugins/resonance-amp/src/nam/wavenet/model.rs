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

use super::super::activations::Activation;
use super::super::parse::{WaveNetConfig, WeightReader};
use super::super::{
    grouped_matvec, grouped_matvec_add, matvec, matvec_add, validate_grouped_matvec_dims,
    validate_matvec_dims, NamInference,
};
use super::conv_layer::{Conv1x1, Conv1x1Bias, LayerGating, WaveNetLayer};
use super::film::Film;
use super::head::{DenseLayer, HeadRechannel};
use super::params::{FilmParams, GatingMode};
use super::ring::RingBuffer;

/// Construction-time validation of a grouped convolution's channel counts
/// (reference `Conv1D::set_size_` / `Conv1x1` ctor): both the input and the
/// output channel count must divide evenly by the group count.
fn check_groups(
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
    let weight = reader.read(out_ch * cond_ch / params.groups)?;
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

pub struct WaveNetModel {
    /// A2 condition_dsp sub-network: a nested WaveNet that transforms the
    /// raw input sample into the condition signal before it feeds the
    /// stacks (reference `WaveNet::_process_condition`). Its weights come
    /// from the nested model object's own weight array, never from the
    /// outer flat stream. `None` = the condition is the raw input under
    /// reference semantics (the reference passthrough), or the per-stack
    /// post-rechannel snapshot under legacy semantics (the engine's A1
    /// condition source, kept bit-identical).
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
    /// Forward-pass semantics (see
    /// [`super::super::parse::WaveNetConfig::reference_semantics`]):
    /// `false` = the historical engine wiring, bit-for-bit for A1 configs;
    /// `true` = reference NeuralAmpModelerCore wiring (raw-input/
    /// condition_dsp condition for every stack, chained head accumulators
    /// with the model output taken from the last stack only, no extra skip
    /// activation, identity residual for inactive layer1x1, exact
    /// activations).
    reference_semantics: bool,

    // Per-stack data
    rechannels: Vec<Option<Conv1x1>>,
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
    pub fn from_config_and_weights(
        config: WaveNetConfig,
        reader: &mut WeightReader,
    ) -> Result<Self, String> {
        // Degenerate configs would otherwise panic in process_sample (it
        // indexes stack[0] for the skip pre-activation).
        if config.stacks.is_empty() || config.stacks.iter().any(|s| s.dilations.is_empty()) {
            return Err("WaveNet config has no layers".into());
        }
        // Per-layer gating vectors must line up with the layer list.
        for (si, s) in config.stacks.iter().enumerate() {
            if s.gating_modes.len() != s.dilations.len()
                || s.secondary_activations.len() != s.dilations.len()
            {
                return Err(format!(
                    "WaveNet stack {si}: gating_modes ({}) and secondary_activations ({}) must match dilations ({})",
                    s.gating_modes.len(),
                    s.secondary_activations.len(),
                    s.dilations.len()
                ));
            }
            // Activations: one broadcast entry, or one per layer.
            if s.activations.len() != 1 && s.activations.len() != s.dilations.len() {
                return Err(format!(
                    "WaveNet stack {si}: activations ({}) must be a single broadcast entry or match dilations ({})",
                    s.activations.len(),
                    s.dilations.len()
                ));
            }
        }

        let reference = config.reference_semantics;
        // Legacy A1 files resolve `Tanh`/`Sigmoid` to the fast
        // approximations (the historical engine sound, also what the
        // official NAM plugin's `enable_fast_tanh()` does); reference-
        // semantics (A2) models use the exact functions, matching how the
        // reference fixture outputs were rendered (`tools/render` never
        // enables fast tanh).
        let fast = !reference;

        if reference {
            // The engine feeds one scalar sample per process call; a
            // multi-channel reference model cannot be driven correctly.
            if config.input_size != 1 {
                return Err(format!(
                    "WaveNet: only mono models are supported (input_size {})",
                    config.input_size
                ));
            }
            // Post-stack head MLPs only reach here through forced reference
            // semantics (container submodels); the A2 `head` object is
            // already rejected at parse.
            if !config.head.is_empty() {
                return Err(
                    "WaveNet config: a post-stack 'head' is not supported for A2 models"
                        .to_string(),
                );
            }
            for (si, s) in config.stacks.iter().enumerate() {
                // Without a condition_dsp the condition is the raw model
                // input (reference `_process_condition` passthrough), so
                // every stack's input mixin must expect exactly that width.
                if config.condition_dsp.is_none() && s.condition_size != config.input_size {
                    return Err(format!(
                        "WaveNet stack {si}: condition_size ({}) must match the model input channels ({}) without a condition_dsp",
                        s.condition_size, config.input_size
                    ));
                }
                // The declared input_size sizes the reference rechannel
                // (`_rechannel(input_size, channels)`), so it must match
                // what actually feeds the stack: the preceding stack's
                // channels (the model input for stack 0). The engine
                // consumes ch * prev_ch (actual) where the reference
                // consumes channels * input_size (declared); they agree
                // only for well-formed files, and a clear error beats
                // silent weight misconsumption.
                let fed_by = if si == 0 {
                    config.input_size
                } else {
                    config.stacks[si - 1].channels
                };
                if s.input_size != fed_by {
                    return Err(format!(
                        "WaveNet stack {si}: input_size ({}) doesn't match {} ({fed_by})",
                        s.input_size,
                        if si == 0 {
                            "the model input channels"
                        } else {
                            "the preceding stack's channels"
                        }
                    ));
                }
                if si > 0 {
                    let prev = &config.stacks[si - 1];
                    // Reference WaveNet ctor: the audio path chains through
                    // stacks whose channels must match the preceding
                    // stack's head_size.
                    if s.channels != prev.head_size {
                        return Err(format!(
                            "WaveNet: channels of stack {si} ({}) doesn't match head_size of preceding stack ({})",
                            s.channels, prev.head_size
                        ));
                    }
                    // The head path chains too: this stack's skip
                    // accumulator is seeded with the preceding stack's
                    // head-rechannel output, so the widths must agree
                    // (the reference memcpy assumes it).
                    let skip_ch = if s.head1x1.active {
                        s.head1x1.out_channels
                    } else {
                        s.bottleneck
                    };
                    if skip_ch != prev.head_size {
                        return Err(format!(
                            "WaveNet stack {si}: head accumulator width ({skip_ch}) doesn't match head_size of preceding stack ({})",
                            prev.head_size
                        ));
                    }
                }
            }
        }

        // --- condition_dsp sub-network (A2) ---
        // Built recursively from its own nested model JSON BEFORE the main
        // arrays touch the flat weight stream, and consuming none of it
        // (reference WaveNet ctor + set_weights_: "condition_dsp already
        // has its own weights from construction"). Validations mirror the
        // reference ctor: the nested net's input width must match what the
        // WaveNet feeds it (the raw model input), and its output width must
        // match every stack's condition_size.
        let condition_dsp = match &config.condition_dsp {
            Some(value) => {
                let nested = super::super::parse::build_condition_dsp(value)?;
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
                Some(Box::new(nested))
            }
            None => None,
        };

        let num_stacks = config.stacks.len();
        // The condition width can exceed the channel count (a condition_dsp
        // sub-model may feed wider condition signals than the audio path,
        // e.g. the wavenet_a2_max fixture); the condition snapshot buffer
        // and the input-mixin validation scratch must cover both.
        let max_ch = config
            .stacks
            .iter()
            .flat_map(|s| [s.channels, s.condition_size])
            .max()
            .unwrap_or(1)
            // The first rechannel reads `input_size` channels; wider-than-
            // channels inputs (possible in hand-written nested configs)
            // must fit the scratch buffers too.
            .max(config.input_size)
            .max(1);
        let max_bn = config
            .stacks
            .iter()
            .map(|s| s.bottleneck)
            .max()
            .unwrap_or(1);
        // Conv/mixin scratch width: doubled for layers with gated/blended
        // gating (primary + secondary halves).
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
        // Skip accumulator width per stack: head1x1.out_channels when the
        // stack's head1x1 is active, else bottleneck (reference
        // `_head_output_size`).
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
        let head_size = config.head_size;

        let mut rechannels = Vec::with_capacity(num_stacks);
        let mut stacks = Vec::with_capacity(num_stacks);
        let mut head_rechannels = Vec::with_capacity(num_stacks);
        let mut ring_buffers = Vec::with_capacity(num_stacks);
        let mut head_rings = Vec::with_capacity(num_stacks);

        let mut prev_ch = config.input_size;

        for (si, stack_cfg) in config.stacks.iter().enumerate() {
            let ch = stack_cfg.channels;
            let bottleneck = stack_cfg.bottleneck;
            let g_in = stack_cfg.groups_input;
            let g_mixin = stack_cfg.groups_input_mixin;
            let g_1x1 = stack_cfg.layer1x1_groups;
            // Whether this stack's layers carry layer1x1 weights: governed
            // by the config-wide format flag (old flat format has none)
            // AND the per-stack A2 `layer1x1.active` (reference
            // `Layer1x1Params`: inactive consumes no weights, identity
            // residual).
            let l1x1_active = config.has_layer1x1 && stack_cfg.layer1x1_active;
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

            // Per-layer activation config: a single entry broadcasts to
            // every layer (A1 / A2 single configs); A2 activation arrays
            // give each layer its own (e.g. the wavenet_a2_max nested
            // condition_dsp mixes PReLU and Softsign per layer).
            let layer_activation_cfg = |layer_idx: usize| {
                if stack_cfg.activations.len() == 1 {
                    &stack_cfg.activations[0]
                } else {
                    &stack_cfg.activations[layer_idx]
                }
            };

            // --- Rechannel (1x1, no bias) ---
            // Reference semantics: the reference LayerArray ctor constructs
            // `_rechannel(params.input_size, params.channels, false)`
            // UNCONDITIONALLY and `set_weights_` always consumes its
            // input_size*channels weights — a 1-to-1 rechannel is a learned
            // conv, not an identity. Legacy keeps the historical
            // skip-when-equal (bit-identical A1 path).
            if reference {
                let weight = reader.read(ch * prev_ch)?;
                rechannels.push(Some(Conv1x1 {
                    weight,
                    out_ch: ch,
                    in_ch: prev_ch,
                }));
            } else if prev_ch != ch {
                let weight = reader.read(ch * prev_ch)?;
                rechannels.push(Some(Conv1x1 {
                    weight,
                    out_ch: ch,
                    in_ch: prev_ch,
                }));
            } else {
                rechannels.push(None);
            }

            // --- Layers ---
            let mut layers = Vec::with_capacity(stack_cfg.dilations.len());
            let mut rings = Vec::with_capacity(stack_cfg.dilations.len());

            for (layer_idx, &dilation) in stack_cfg.dilations.iter().enumerate() {
                let ks = stack_cfg.kernel_sizes[layer_idx];

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
                // the fast sigmoid in legacy mode (bit-identical A1 gated
                // path) and the exact sigmoid under reference semantics.
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
                let per_tap = mid_ch * ch / g_in;
                let raw = reader.read(per_tap * ks)?;
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
                    Some(reader.read(mid_ch * stack_cfg.condition_size / g_mixin)?)
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
                    let w = reader.read(ch * bottleneck / g_1x1)?;
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
                    let w = reader.read(h_out * bottleneck / h_groups)?;
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
                let conv_pre_film =
                    read_film(reader, &ctx, "conv_pre_film", cond, ch, &films.conv_pre)?;
                let conv_post_film =
                    read_film(reader, &ctx, "conv_post_film", cond, mid_ch, &films.conv_post)?;
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
                rings.push(RingBuffer::new(ring_capacity, ch));

                layers.push(WaveNetLayer {
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
                    // Legacy mode maps the A1 "Tanh" config to the
                    // fast-tanh path, exactly as the previously hardcoded
                    // implementation (bit-identical); reference semantics
                    // use the exact functions.
                    activation: Activation::from_config(layer_activation_cfg(layer_idx), fast),
                    gating,
                    conv_pre_film,
                    conv_post_film,
                    input_mixin_pre_film,
                    input_mixin_post_film,
                    activation_pre_film,
                    activation_post_film,
                    layer1x1_post_film,
                    head1x1_post_film,
                });
            }

            stacks.push(layers);
            ring_buffers.push(rings);

            // --- Head rechannel (causal conv, kernel `head_kernel_size`,
            // dilation `head_dilation`, bias controlled by head_bias) ---
            // Its input is the accumulated skip signal:
            // head1x1.out_channels wide when the stack's head1x1 is active,
            // else bottleneck wide (reference `_head_rechannel(
            // head1x1.active ? head1x1.out_channels : bottleneck,
            // head_size, head_kernel_size, head_bias, head_dilation, 1)`).
            let skip_ch = if stack_cfg.head1x1.active {
                stack_cfg.head1x1.out_channels
            } else {
                bottleneck
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
            let per_tap = hr_out * skip_ch;
            let raw = reader.read(per_tap * hr_ks)?;
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
            head_rechannels.push(HeadRechannel {
                taps: hr_taps,
                bias: hr_bias,
                out_ch: hr_out,
                in_ch: skip_ch,
                dilation: stack_cfg.head_dilation,
            });
            // Windowed heads need a history of past skip-accumulator
            // frames; capacity mirrors the per-layer conv rings.
            head_rings.push(if hr_ks > 1 {
                Some(RingBuffer::new(
                    (hr_ks - 1) * stack_cfg.head_dilation + 2,
                    skip_ch,
                ))
            } else {
                None
            });

            prev_ch = ch;
        }

        // --- Head MLP layers ---
        let mut head_layers = Vec::new();
        let mut prev_size = head_size;
        for &hidden in &config.head {
            let weight = reader.read(hidden * prev_size)?;
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
            let weight = reader.read(head_size * prev_size)?;
            let bias = reader.read(head_size)?;
            head_layers.push(DenseLayer {
                weight,
                bias,
                in_features: prev_size,
                out_features: head_size,
                has_activation: false,
            });
        }

        // --- Head scale (last weight) ---
        let head_scale = if reader.remaining() >= 1 {
            reader.read(1)?[0]
        } else {
            1.0
        };

        // Head MLP hidden-layer activation (A1: "Tanh" -> fast tanh; taken
        // from the first stack's first layer activation, as before). The
        // head MLP is legacy-only (reference semantics reject it above).
        let head_activation = config
            .stacks
            .first()
            .and_then(|s| s.activations.first())
            .map(|a| Activation::from_config(a, fast))
            .unwrap_or(Activation::FastTanh);

        // Head buffers must cover every stack's head rechannel width, not
        // just the first stack's `head_size`: a nested condition_dsp net's
        // arrays may widen toward the output (e.g. head 2 -> 3 in the
        // wavenet_condition_dsp fixture). A1 models have their widest head
        // first, so these sizes are unchanged there.
        let max_stack_head = config
            .stacks
            .iter()
            .map(|s| s.head_size)
            .max()
            .unwrap_or(head_size);
        // Model output width (reference `wave_net_output_channels`): the
        // head MLP's output size when present, else the LAST stack's head
        // rechannel width.
        let out_channels = if config.head.is_empty() {
            config
                .stacks
                .last()
                .map(|s| s.head_size)
                .unwrap_or(head_size)
        } else {
            head_size
        };
        let max_head_buf = config
            .head
            .iter()
            .copied()
            .chain(std::iter::once(head_size))
            .chain(std::iter::once(max_ch))
            .chain(std::iter::once(max_stack_head))
            .max()
            .unwrap_or(1);

        // FiLM scale/shift scratch: widest active FiLM conv output across
        // all layers (0 films -> minimal 1-slot buffer, untouched).
        let max_film_ss = stacks
            .iter()
            .flatten()
            .flat_map(|l| l.films())
            .flatten()
            .map(|f| f.out_ch)
            .max()
            .unwrap_or(0)
            .max(1);

        // Validate matvec dimensions for all weight matrices at load time.
        let scratch_activation = vec![0.0f32; max_ch];
        let scratch_conv_out = vec![0.0f32; max_mid];
        let scratch_skip = vec![0.0f32; max_skip];
        let scratch_head_buf = vec![0.0f32; max_head_buf];
        let scratch_head_input = vec![0.0f32; head_size.max(max_stack_head)];
        let scratch_film_ss = vec![0.0f32; max_film_ss];

        for (si, rc) in rechannels.iter().enumerate() {
            if let Some(ref rc) = rc {
                if !validate_matvec_dims(
                    &rc.weight,
                    &scratch_activation[..rc.in_ch],
                    &scratch_activation[..rc.out_ch],
                    rc.out_ch,
                    rc.in_ch,
                ) {
                    return Err(format!("WaveNet stack {si}: rechannel dimension mismatch"));
                }
            }
        }
        for (si, stack) in stacks.iter().enumerate() {
            for (li, layer) in stack.iter().enumerate() {
                let ch = layer.channels;
                let mid_ch = layer.mid_ch;
                for (tap_idx, w) in layer.w_conv.iter().enumerate() {
                    if !validate_grouped_matvec_dims(
                        w,
                        &scratch_activation[..ch],
                        &scratch_conv_out[..mid_ch],
                        mid_ch,
                        ch,
                        layer.groups_input,
                    ) {
                        return Err(format!("WaveNet stack {si} layer {li} tap {tap_idx}: conv weight dimension mismatch"));
                    }
                }
                if let Some(ref w_mixin) = layer.w_input_mixin {
                    let cond_size = w_mixin.len() * layer.groups_input_mixin / mid_ch;
                    if !validate_grouped_matvec_dims(
                        w_mixin,
                        &scratch_activation[..cond_size],
                        &scratch_conv_out[..mid_ch],
                        mid_ch,
                        cond_size,
                        layer.groups_input_mixin,
                    ) {
                        return Err(format!(
                            "WaveNet stack {si} layer {li}: input_mixin dimension mismatch"
                        ));
                    }
                }
                if let Some(ref l1x1) = layer.layer1x1 {
                    // layer1x1 input is the bottleneck-wide activated z,
                    // which lives in the conv_out scratch during processing.
                    if !validate_grouped_matvec_dims(
                        &l1x1.weight,
                        &scratch_conv_out[..l1x1.in_ch],
                        &scratch_activation[..l1x1.out_ch],
                        l1x1.out_ch,
                        l1x1.in_ch,
                        l1x1.groups,
                    ) {
                        return Err(format!(
                            "WaveNet stack {si} layer {li}: layer1x1 dimension mismatch"
                        ));
                    }
                }
                if let Some(ref h1x1) = layer.head1x1 {
                    // head1x1 input is the bottleneck-wide activated z
                    // (conv_out scratch); its output feeds the skip
                    // accumulator scratch.
                    if !validate_grouped_matvec_dims(
                        &h1x1.weight,
                        &scratch_conv_out[..h1x1.in_ch],
                        &scratch_skip[..h1x1.out_ch],
                        h1x1.out_ch,
                        h1x1.in_ch,
                        h1x1.groups,
                    ) {
                        return Err(format!(
                            "WaveNet stack {si} layer {li}: head1x1 dimension mismatch"
                        ));
                    }
                }
                // FiLM convs read the condition snapshot and write the
                // scale/shift scratch.
                const FILM_SITES: [&str; 8] = [
                    "conv_pre_film",
                    "conv_post_film",
                    "input_mixin_pre_film",
                    "input_mixin_post_film",
                    "activation_pre_film",
                    "activation_post_film",
                    "layer1x1_post_film",
                    "head1x1_post_film",
                ];
                for (site, film) in FILM_SITES.iter().zip(layer.films()) {
                    let Some(f) = film else { continue };
                    if !validate_grouped_matvec_dims(
                        &f.weight,
                        &scratch_activation[..f.cond_ch],
                        &scratch_film_ss[..f.out_ch],
                        f.out_ch,
                        f.cond_ch,
                        f.groups,
                    ) {
                        return Err(format!(
                            "WaveNet stack {si} layer {li}: {site} dimension mismatch"
                        ));
                    }
                }
            }
            let hr = &head_rechannels[si];
            for (tap_idx, w) in hr.taps.iter().enumerate() {
                if !validate_matvec_dims(
                    w,
                    &scratch_skip[..hr.in_ch],
                    &scratch_head_buf[..hr.out_ch],
                    hr.out_ch,
                    hr.in_ch,
                ) {
                    return Err(format!(
                        "WaveNet stack {si} tap {tap_idx}: head_rechannel dimension mismatch"
                    ));
                }
            }
        }
        for (hi, hl) in head_layers.iter().enumerate() {
            if !validate_matvec_dims(
                &hl.weight,
                &scratch_head_buf[..hl.in_features],
                &scratch_head_buf[..hl.out_features],
                hl.out_features,
                hl.in_features,
            ) {
                return Err(format!("WaveNet head layer {hi}: dimension mismatch"));
            }
        }

        let condition_buf = condition_dsp
            .as_ref()
            .map_or(Vec::new(), |cd| vec![0.0f32; cd.out_channels()]);

        Ok(Self {
            condition_dsp,
            condition_buf,
            in_channels: config.input_size,
            head_size,
            out_channels,
            reference_semantics: reference,
            rechannels,
            stacks,
            head_rechannels,
            ring_buffers,
            head_rings,
            head_layers,
            head_scale,
            head_activation,
            activation: scratch_activation,
            conv_out: scratch_conv_out,
            mixin_buf: vec![0.0; max_mid],
            pre_act_buf: vec![0.0; max_bn],
            residual_buf: vec![0.0; max_ch],
            skip_accum: scratch_skip,
            head1x1_buf: vec![0.0; max_skip],
            film_ss_buf: scratch_film_ss,
            film_pre_buf: vec![0.0; max_ch],
            rechannel_buf: vec![0.0; max_ch],
            head_input: scratch_head_input,
            head_buf_a: scratch_head_buf,
            head_buf_b: vec![0.0; max_head_buf],
        })
    }
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

    /// One full forward pass; leaves the pre-`head_scale` output channels
    /// in `head_input[..out_channels]`.
    fn forward(&mut self, input: f32) {
        // condition_dsp (A2): the nested net transforms the raw input into
        // the condition once per sample, before the stacks run (reference
        // `WaveNet::process`: `_process_condition` ahead of the layer
        // arrays). The stacks still receive the raw input on the audio
        // path; only the condition source changes.
        if let Some(cd) = &mut self.condition_dsp {
            cd.process_sample_into(input, &mut self.condition_buf);
        }

        // Seed activation with the raw input (will be rechanneled by first stack's rechannel)
        self.activation[0] = input;

        // Zero head_input accumulator
        self.head_input.fill(0.0);

        for (stack_idx, stack) in self.stacks.iter().enumerate() {
            // Construction guarantees every stack has at least one layer.
            let ch = stack[0].channels;
            let bottleneck = stack[0].bottleneck;

            // Rechannel if needed
            if let Some(ref rc) = self.rechannels[stack_idx] {
                matvec(
                    &rc.weight,
                    &self.activation[..rc.in_ch],
                    rc.out_ch,
                    rc.in_ch,
                    &mut self.rechannel_buf,
                );
                self.activation[..rc.out_ch].copy_from_slice(&self.rechannel_buf[..rc.out_ch]);
            }

            // Save the condition snapshot the layers' input_mixin and FiLM
            // sites read (all of them index rechannel_buf). With a
            // condition_dsp its per-sample output IS the condition for
            // every stack (reference passes `_condition_output` to each
            // LayerArray). Without one:
            //  - reference semantics: the condition is the RAW model input
            //    for every stack (reference `_process_condition` copies
            //    `_condition_input` through; construction validated
            //    condition_size == input_size == 1);
            //  - legacy: the post-rechannel activation snapshot (the
            //    #1113-recorded engine-ism; layers modify activation
            //    in-place, so it must be saved here) — unchanged,
            //    bit-identical A1 behavior.
            match &self.condition_dsp {
                Some(_) => {
                    let n = self.condition_buf.len();
                    self.rechannel_buf[..n].copy_from_slice(&self.condition_buf);
                }
                None if self.reference_semantics => self.rechannel_buf[0] = input,
                None => self.rechannel_buf[..ch].copy_from_slice(&self.activation[..ch]),
            }

            // Skip accumulator for this stack. Its width follows the
            // head path: head1x1.out_channels when the stack's head1x1 is
            // active, else bottleneck (reference `_head_output_size`). All
            // layers of a stack share one head1x1 config (per-layer-array in
            // the reference), so the width is uniform within the stack.
            //
            // Reference semantics CHAIN the stacks' head paths
            // (`LayerArray::Process` with head inputs): stack 0 starts at
            // zero, and every later stack starts from the preceding
            // stack's head-rechannel output (`head_input` here, widths
            // validated equal at construction). Legacy always starts at
            // zero and sums the per-stack head-rechannel outputs instead.
            let skip_ch = stack[0]
                .head1x1
                .as_ref()
                .map_or(bottleneck, |h| h.out_ch);
            if self.reference_semantics && stack_idx > 0 {
                let (seed, _) = self.head_input.split_at(skip_ch);
                self.skip_accum[..skip_ch].copy_from_slice(seed);
            } else {
                self.skip_accum[..skip_ch].fill(0.0);
            }

            for (layer_idx, layer) in stack.iter().enumerate() {
                let ring = &mut self.ring_buffers[stack_idx][layer_idx];
                let ks = layer.kernel_size;
                let mid_ch = layer.mid_ch;

                // Write current activation into ring buffer. With an active
                // conv_pre FiLM the conv (and its tap history) sees the
                // FiLM-modulated input instead (reference
                // `_conv.Process(_conv_pre_film->GetOutput())`); the
                // residual path keeps the raw input, which stays in
                // `self.activation` until the residual update below.
                match &layer.conv_pre_film {
                    Some(f) => {
                        f.modulate(
                            &self.rechannel_buf,
                            &self.activation,
                            &mut self.film_pre_buf,
                            &mut self.film_ss_buf,
                        );
                        ring.write(&self.film_pre_buf[..ch]);
                    }
                    None => ring.write(&self.activation[..ch]),
                }

                // Dilated convolution (combined filter+gate). Grouped
                // (groups_input > 1) runs blocked per-group matvecs; the
                // g == 1 case is the historical dense matvec unchanged.
                let x0 = ring.read_delayed((ks - 1) * layer.dilation);
                grouped_matvec(
                    &layer.w_conv[0],
                    x0,
                    mid_ch,
                    ch,
                    layer.groups_input,
                    &mut self.conv_out,
                );
                for tap in 1..ks {
                    let delay = (ks - 1 - tap) * layer.dilation;
                    let xt = ring.read_delayed(delay);
                    grouped_matvec_add(
                        &layer.w_conv[tap],
                        xt,
                        mid_ch,
                        ch,
                        layer.groups_input,
                        &mut self.conv_out,
                    );
                }
                for c in 0..mid_ch {
                    self.conv_out[c] += layer.b_conv[c];
                }

                // conv_post FiLM: modulate the conv output in place before
                // the mixin sum (reference `Process_` on `_conv.GetOutput()`).
                if let Some(f) = &layer.conv_post_film {
                    f.modulate_in_place(
                        &self.rechannel_buf,
                        &mut self.conv_out[..mid_ch],
                        &mut self.film_ss_buf,
                    );
                }

                // Input mixin: add condition signal projected to mid_ch.
                // An active input_mixin_pre FiLM modulates the condition fed
                // to the mixin ONLY (self-conditioned, reference
                // `Process(condition, condition)`); later sites still see
                // the raw condition snapshot.
                if let Some(ref w_mixin) = layer.w_input_mixin {
                    let cond_size = w_mixin.len() * layer.groups_input_mixin / mid_ch;
                    let mixin_in: &[f32] = match &layer.input_mixin_pre_film {
                        Some(f) => {
                            f.modulate(
                                &self.rechannel_buf,
                                &self.rechannel_buf,
                                &mut self.film_pre_buf,
                                &mut self.film_ss_buf,
                            );
                            &self.film_pre_buf[..cond_size]
                        }
                        None => &self.rechannel_buf[..cond_size],
                    };
                    grouped_matvec(
                        w_mixin,
                        mixin_in,
                        mid_ch,
                        cond_size,
                        layer.groups_input_mixin,
                        &mut self.mixin_buf,
                    );
                    // input_mixin_post FiLM: modulate the mixin output in
                    // place before summing into z.
                    if let Some(f) = &layer.input_mixin_post_film {
                        f.modulate_in_place(
                            &self.rechannel_buf,
                            &mut self.mixin_buf[..mid_ch],
                            &mut self.film_ss_buf,
                        );
                    }
                    for c in 0..mid_ch {
                        self.conv_out[c] += self.mixin_buf[c];
                    }
                }

                // activation_pre FiLM: modulate z = conv + mixin (full
                // conv width, incl. the secondary half when gated/blended)
                // before the activation (reference `Process_(_z)`).
                if let Some(f) = &layer.activation_pre_film {
                    f.modulate_in_place(
                        &self.rechannel_buf,
                        &mut self.conv_out[..mid_ch],
                        &mut self.film_ss_buf,
                    );
                }

                // Activation / gating (dispatch resolved at construction).
                // The activated z is bottleneck-wide and lives in
                // conv_out[..bottleneck]; for gated/blended layers the
                // secondary half sits in conv_out[bottleneck..2*bottleneck]
                // and must not be used past this point (reference
                // NAM/gating_activations.h). A1 (bottleneck == channels)
                // semantics preserved bit-identically: the gated path
                // computes fast_tanh(z) * fast_sigmoid(g) per channel.
                match &layer.gating {
                    LayerGating::None => {
                        layer.activation.apply(&mut self.conv_out[..bottleneck]);
                    }
                    LayerGating::Gated(secondary) => {
                        let (z, g) = self.conv_out.split_at_mut(bottleneck);
                        layer.activation.apply(&mut z[..bottleneck]);
                        secondary.apply(&mut g[..bottleneck]);
                        for c in 0..bottleneck {
                            z[c] *= g[c];
                        }
                    }
                    LayerGating::Blended(secondary) => {
                        // Reference BlendingActivation: alpha = blend(g);
                        // out = alpha * primary(z) + (1 - alpha) * z_pre,
                        // with z_pre the pre-activation primary half.
                        let (z, g) = self.conv_out.split_at_mut(bottleneck);
                        self.pre_act_buf[..bottleneck].copy_from_slice(&z[..bottleneck]);
                        layer.activation.apply(&mut z[..bottleneck]);
                        secondary.apply(&mut g[..bottleneck]);
                        for c in 0..bottleneck {
                            let alpha = g[c];
                            z[c] = alpha * z[c] + (1.0 - alpha) * self.pre_act_buf[c];
                        }
                    }
                }

                // activation_post FiLM: modulate the activated z (the
                // primary bottleneck-wide half in all gating modes;
                // reference applies it to `_z` when ungated and to
                // `_z.topRows(bottleneck)` in the gated/blended branches).
                if let Some(f) = &layer.activation_post_film {
                    f.modulate_in_place(
                        &self.rechannel_buf,
                        &mut self.conv_out[..bottleneck],
                        &mut self.film_ss_buf,
                    );
                }

                // Skip contribution: head1x1(activated z) when the stack's
                // head1x1 is active (reference `_head1x1->process_(z)` on
                // the activated top-bottleneck rows), else the activated z
                // itself (A1 direct skip, bottleneck-wide).
                match &layer.head1x1 {
                    Some(h1x1) => {
                        grouped_matvec(
                            &h1x1.weight,
                            &self.conv_out[..h1x1.in_ch],
                            h1x1.out_ch,
                            h1x1.in_ch,
                            h1x1.groups,
                            &mut self.head1x1_buf,
                        );
                        for c in 0..h1x1.out_ch {
                            self.head1x1_buf[c] += h1x1.bias[c];
                        }
                        // head1x1_post FiLM: modulate the (biased) head1x1
                        // output before it joins the skip accumulator
                        // (reference `Process_` on `_head1x1->GetOutput()`).
                        if let Some(f) = &layer.head1x1_post_film {
                            f.modulate_in_place(
                                &self.rechannel_buf,
                                &mut self.head1x1_buf[..h1x1.out_ch],
                                &mut self.film_ss_buf,
                            );
                        }
                        for c in 0..h1x1.out_ch {
                            self.skip_accum[c] += self.head1x1_buf[c];
                        }
                    }
                    None => {
                        for c in 0..bottleneck {
                            self.skip_accum[c] += self.conv_out[c];
                        }
                    }
                }

                // Residual connection: layer1x1 maps z (bottleneck) back to
                // channels; without a layer1x1, bottleneck == channels
                // (enforced at construction) and z IS the residual. The raw
                // layer input still sits in `self.activation` (the conv_pre
                // FiLM, when active, modulated only the ring copy), so the
                // residual reads it from there — bit-identical to the
                // historical ring `read_current()`, whose newest frame was
                // that same value.
                match &layer.layer1x1 {
                    Some(l1x1) => {
                        grouped_matvec(
                            &l1x1.weight,
                            &self.conv_out[..l1x1.in_ch],
                            l1x1.out_ch,
                            l1x1.in_ch,
                            l1x1.groups,
                            &mut self.residual_buf,
                        );
                        for c in 0..l1x1.out_ch {
                            self.residual_buf[c] += l1x1.bias[c];
                        }
                        // layer1x1_post FiLM: reference-exact, the
                        // modulation is applied ONLY in the BLENDED gating
                        // branch of `Layer::Process` (NAM/wavenet/model.cpp
                        // applies it after `_layer1x1` there alone; the
                        // NONE and GATED branches run the layer1x1 without
                        // it, even though its weights were consumed).
                        if matches!(layer.gating, LayerGating::Blended(_)) {
                            if let Some(f) = &layer.layer1x1_post_film {
                                f.modulate_in_place(
                                    &self.rechannel_buf,
                                    &mut self.residual_buf[..l1x1.out_ch],
                                    &mut self.film_ss_buf,
                                );
                            }
                        }
                        for c in 0..ch {
                            self.activation[c] += self.residual_buf[c];
                        }
                    }
                    None => {
                        // No layer1x1. Reference semantics: the residual is
                        // the identity — the next layer's input is the raw
                        // layer input alone (`_output_next_layer = input`
                        // in Layer::Process), which already sits in
                        // `self.activation`. Legacy (old-format) semantics
                        // add the activated z (bottleneck == channels,
                        // enforced at construction).
                        if !self.reference_semantics {
                            for c in 0..ch {
                                self.activation[c] += self.conv_out[c];
                            }
                        }
                    }
                }
            }

            // Head rechannel: project skip_accum (skip_ch wide) to
            // head_size and accumulate.
            let hr = &self.head_rechannels[stack_idx];
            match &mut self.head_rings[stack_idx] {
                None => {
                    // Kernel-1 path (A1 / head_kernel_size == 1, memoryless).
                    // Legacy semantics apply a pre-activation on skip_accum
                    // before the head_rechannel, using the stack's
                    // configured activation (A1: fast tanh) — the
                    // historical engine head path, kept bit-identical. The
                    // reference has NO such activation
                    // (`_head_rechannel.Process(_head_inputs)` directly in
                    // LayerArray::ProcessInner): the per-layer skip
                    // contributions are already activated z.
                    if !self.reference_semantics {
                        stack[0].activation.apply(&mut self.skip_accum[..skip_ch]);
                    }
                    matvec(
                        &hr.taps[0],
                        &self.skip_accum[..hr.in_ch],
                        hr.out_ch,
                        hr.in_ch,
                        &mut self.head_buf_a,
                    );
                }
                Some(ring) => {
                    // Windowed path (head_kernel_size > 1): reference-exact
                    // causal dilated convolution over the raw accumulated
                    // skip frames. The reference applies NO activation here
                    // (`_head_rechannel.Process(_head_inputs)` directly in
                    // LayerArray::ProcessInner, NAM/wavenet/model.cpp); the
                    // per-layer skip contributions are already activated z.
                    // Tap k reads the frame (ks - 1 - k) * dilation samples
                    // back (reference `Conv1D::Process` offsets), so
                    // taps[ks - 1] multiplies the current frame.
                    ring.write(&self.skip_accum[..skip_ch]);
                    let ks = hr.kernel_size();
                    matvec(
                        &hr.taps[0],
                        ring.read_delayed((ks - 1) * hr.dilation),
                        hr.out_ch,
                        hr.in_ch,
                        &mut self.head_buf_a,
                    );
                    for tap in 1..ks {
                        let delay = (ks - 1 - tap) * hr.dilation;
                        matvec_add(
                            &hr.taps[tap],
                            ring.read_delayed(delay),
                            hr.out_ch,
                            hr.in_ch,
                            &mut self.head_buf_a,
                        );
                    }
                }
            }
            // Reference semantics: `head_input` holds the current stack's
            // head-rechannel output alone — it seeds the next stack's skip
            // accumulator, and after the last stack it IS the model output
            // (reference `WaveNet::process` reads only
            // `_layer_arrays.back().GetHeadOutputs()`). Legacy sums every
            // stack's head-rechannel output instead.
            if self.reference_semantics {
                for c in 0..hr.out_ch {
                    self.head_input[c] = self.head_buf_a[c] + hr.bias[c];
                }
            } else {
                for c in 0..hr.out_ch {
                    self.head_input[c] += self.head_buf_a[c] + hr.bias[c];
                }
            }
        }

        // Head MLP
        if self.head_layers.is_empty() {
            // No head MLP — the pre-head_scale output channels are the
            // accumulated head_input as-is.
            return;
        }
        let head_size = self.head_size;

        self.head_buf_a[..head_size].copy_from_slice(&self.head_input[..head_size]);
        let mut current_size = head_size;
        let mut use_a = true;

        for head_layer in &self.head_layers {
            let (src, dst) = if use_a {
                (&self.head_buf_a as &[f32], &mut self.head_buf_b)
            } else {
                (&self.head_buf_b as &[f32], &mut self.head_buf_a)
            };
            matvec(
                &head_layer.weight,
                &src[..current_size],
                head_layer.out_features,
                head_layer.in_features,
                dst,
            );
            for (j, v) in dst.iter_mut().enumerate().take(head_layer.out_features) {
                *v += head_layer.bias[j];
            }
            if head_layer.has_activation {
                self.head_activation.apply(&mut dst[..head_layer.out_features]);
            }
            current_size = head_layer.out_features;
            use_a = !use_a;
        }

        // Expose the MLP result in head_input so every path reads the
        // pre-head_scale output channels from the same place
        // (out_channels == head_size on the MLP path).
        let src = if use_a {
            &self.head_buf_a
        } else {
            &self.head_buf_b
        };
        self.head_input[..head_size].copy_from_slice(&src[..head_size]);
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
