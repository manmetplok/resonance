//! WaveNet inference engine for NAM models.
//!
//! Follows the NAM (Neural Amp Modeler) weight serialization order
//! (reference `set_weights_` in NAM/wavenet/model.cpp):
//! Per LayerArray (stack): rechannel, layers (conv+bias, input_mixin, layer1x1), head_rechannel
//! Then: head MLP layers, head_scale.
//!
//! Tensor shapes follow the A2 bottleneck convention: the dilated conv and
//! input mixin output `bottleneck` channels (doubled for layers whose gating
//! mode is `gated` or `blended` — primary + secondary halves), the
//! activation and skip path run at bottleneck width, and the layer1x1 maps
//! bottleneck back to `channels`. A1 models have `bottleneck == channels`,
//! which reproduces the historical layout bit-identically.

use super::super::activations::Activation;
use super::super::parse::{WaveNetConfig, WeightReader};
use super::super::{matvec, matvec_add, validate_matvec_dims, NamInference};
use super::conv_layer::{Conv1x1, Conv1x1Bias, LayerGating, WaveNetLayer};
use super::head::DenseLayer;
use super::params::GatingMode;
use super::ring::RingBuffer;

pub struct WaveNetModel {
    // Per-stack data
    rechannels: Vec<Option<Conv1x1>>,
    stacks: Vec<Vec<WaveNetLayer>>,
    head_rechannels: Vec<Conv1x1Bias>,
    ring_buffers: Vec<Vec<RingBuffer>>,

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
    skip_accum: Vec<f32>, // bottleneck sized
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
        }

        let num_stacks = config.stacks.len();
        let max_ch = config.stacks.iter().map(|s| s.channels).max().unwrap_or(1);
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
        let head_size = config.head_size;

        let mut rechannels = Vec::with_capacity(num_stacks);
        let mut stacks = Vec::with_capacity(num_stacks);
        let mut head_rechannels = Vec::with_capacity(num_stacks);
        let mut ring_buffers = Vec::with_capacity(num_stacks);

        let mut prev_ch = config.input_size;

        for stack_cfg in &config.stacks {
            let ch = stack_cfg.channels;
            let bottleneck = stack_cfg.bottleneck;
            // Reference validation: without a layer1x1 there is nothing to
            // map the bottleneck-wide activation back to `channels`.
            if !config.has_layer1x1 && bottleneck != ch {
                return Err(format!(
                    "WaveNet config: bottleneck ({bottleneck}) must equal channels ({ch}) when layer1x1 is inactive"
                ));
            }

            // Resolve activation dispatch once, at construction. fast_tanh
            // mode maps the A1 "Tanh" config to the fast-tanh path, exactly
            // as the previously hardcoded implementation (bit-identical).
            let stack_activation = Activation::from_config(&stack_cfg.activation, true);

            // --- Rechannel (1x1, no bias) ---
            if prev_ch != ch {
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
                // the fast sigmoid (bit-identical A1 gated path).
                let gating = match mode {
                    GatingMode::None => LayerGating::None,
                    GatingMode::Gated | GatingMode::Blended => {
                        let secondary = match &stack_cfg.secondary_activations[layer_idx] {
                            Some(cfg) => Activation::secondary_from_config(cfg, true),
                            None => Activation::FastSigmoid,
                        };
                        if mode == GatingMode::Gated {
                            LayerGating::Gated(secondary)
                        } else {
                            LayerGating::Blended(secondary)
                        }
                    }
                };

                // _conv.weight [mid_ch, ch, kernel_size] row-major
                let raw = reader.read(mid_ch * ch * ks)?;
                let mut w_conv = Vec::with_capacity(ks);
                for tap in 0..ks {
                    let mut w = vec![0.0f32; mid_ch * ch];
                    for out_c in 0..mid_ch {
                        for in_c in 0..ch {
                            w[out_c * ch + in_c] = raw[(out_c * ch + in_c) * ks + tap];
                        }
                    }
                    w_conv.push(w);
                }

                // _conv.bias [mid_ch]
                let b_conv = reader.read(mid_ch)?;

                // _input_mixin.weight [mid_ch, condition_size] (no bias)
                let w_input_mixin = if stack_cfg.condition_size > 0 {
                    Some(reader.read(mid_ch * stack_cfg.condition_size)?)
                } else {
                    None
                };

                // _layer1x1: learned 1x1 residual conv mapping the
                // bottleneck-wide activation back to `channels` (active by
                // default in new-format NAM). Weight [ch, bottleneck], bias
                // [ch] — reference Conv1x1(bottleneck, channels, bias).
                let layer1x1 = if config.has_layer1x1 {
                    let w = reader.read(ch * bottleneck)?;
                    let b = reader.read(ch)?;
                    Some(Conv1x1Bias {
                        weight: w,
                        bias: b,
                        out_ch: ch,
                        in_ch: bottleneck,
                    })
                } else {
                    None
                };

                let ring_capacity = (ks - 1) * dilation + 2;
                rings.push(RingBuffer::new(ring_capacity, ch));

                layers.push(WaveNetLayer {
                    w_conv,
                    b_conv,
                    w_input_mixin,
                    layer1x1,
                    kernel_size: ks,
                    dilation,
                    channels: ch,
                    bottleneck,
                    mid_ch,
                    activation: stack_activation.clone(),
                    gating,
                });
            }

            stacks.push(layers);
            ring_buffers.push(rings);

            // --- Head rechannel (1x1, bias controlled by head_bias) ---
            // Its input is the accumulated skip signal, which is
            // bottleneck-wide (reference `_head_rechannel(bottleneck,
            // head_size, ...)` when head1x1 is inactive).
            let hr_out = stack_cfg.head_size;
            let hr_weight = reader.read(hr_out * bottleneck)?;
            let hr_bias = if config.head_bias {
                reader.read(hr_out)?
            } else {
                vec![0.0; hr_out]
            };
            head_rechannels.push(Conv1x1Bias {
                weight: hr_weight,
                bias: hr_bias,
                out_ch: hr_out,
                in_ch: bottleneck,
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

        // Head MLP hidden-layer activation (A1: "Tanh" -> fast tanh).
        let head_activation = config
            .stacks
            .first()
            .map(|s| Activation::from_config(&s.activation, true))
            .unwrap_or(Activation::FastTanh);

        let max_head_buf = config
            .head
            .iter()
            .copied()
            .chain(std::iter::once(head_size))
            .chain(std::iter::once(max_ch))
            .max()
            .unwrap_or(1);

        // Validate matvec dimensions for all weight matrices at load time.
        let scratch_activation = vec![0.0f32; max_ch];
        let scratch_conv_out = vec![0.0f32; max_mid];
        let scratch_skip = vec![0.0f32; max_bn];
        let scratch_head_buf = vec![0.0f32; max_head_buf];
        let scratch_head_input = vec![0.0f32; head_size];

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
                    if !validate_matvec_dims(
                        w,
                        &scratch_activation[..ch],
                        &scratch_conv_out[..mid_ch],
                        mid_ch,
                        ch,
                    ) {
                        return Err(format!("WaveNet stack {si} layer {li} tap {tap_idx}: conv weight dimension mismatch"));
                    }
                }
                if let Some(ref w_mixin) = layer.w_input_mixin {
                    let cond_size = w_mixin.len() / mid_ch;
                    if !validate_matvec_dims(
                        w_mixin,
                        &scratch_activation[..cond_size],
                        &scratch_conv_out[..mid_ch],
                        mid_ch,
                        cond_size,
                    ) {
                        return Err(format!(
                            "WaveNet stack {si} layer {li}: input_mixin dimension mismatch"
                        ));
                    }
                }
                if let Some(ref l1x1) = layer.layer1x1 {
                    // layer1x1 input is the bottleneck-wide activated z,
                    // which lives in the conv_out scratch during processing.
                    if !validate_matvec_dims(
                        &l1x1.weight,
                        &scratch_conv_out[..l1x1.in_ch],
                        &scratch_activation[..l1x1.out_ch],
                        l1x1.out_ch,
                        l1x1.in_ch,
                    ) {
                        return Err(format!(
                            "WaveNet stack {si} layer {li}: layer1x1 dimension mismatch"
                        ));
                    }
                }
            }
            let hr = &head_rechannels[si];
            if !validate_matvec_dims(
                &hr.weight,
                &scratch_skip[..hr.in_ch],
                &scratch_head_buf[..hr.out_ch],
                hr.out_ch,
                hr.in_ch,
            ) {
                return Err(format!(
                    "WaveNet stack {si}: head_rechannel dimension mismatch"
                ));
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

        Ok(Self {
            rechannels,
            stacks,
            head_rechannels,
            ring_buffers,
            head_layers,
            head_scale,
            head_activation,
            activation: scratch_activation,
            conv_out: scratch_conv_out,
            mixin_buf: vec![0.0; max_mid],
            pre_act_buf: vec![0.0; max_bn],
            residual_buf: vec![0.0; max_ch],
            skip_accum: scratch_skip,
            rechannel_buf: vec![0.0; max_ch],
            head_input: scratch_head_input,
            head_buf_a: scratch_head_buf,
            head_buf_b: vec![0.0; max_head_buf],
        })
    }
}

impl NamInference for WaveNetModel {
    fn process_sample(&mut self, input: f32) -> f32 {
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

            // Save condition signal (activation after rechannel, before layers modify it)
            // We'll read it from activation snapshot. Since layers modify activation in-place,
            // we need to save condition for input_mixin. We reuse rechannel_buf for this.
            self.rechannel_buf[..ch].copy_from_slice(&self.activation[..ch]);

            // Zero skip accumulator for this stack (bottleneck-wide)
            self.skip_accum[..bottleneck].fill(0.0);

            for (layer_idx, layer) in stack.iter().enumerate() {
                let ring = &mut self.ring_buffers[stack_idx][layer_idx];
                let ks = layer.kernel_size;
                let mid_ch = layer.mid_ch;

                // Write current activation into ring buffer
                ring.write(&self.activation[..ch]);

                // Dilated convolution (combined filter+gate)
                let x0 = ring.read_delayed((ks - 1) * layer.dilation);
                matvec(&layer.w_conv[0], x0, mid_ch, ch, &mut self.conv_out);
                for tap in 1..ks {
                    let delay = (ks - 1 - tap) * layer.dilation;
                    let xt = ring.read_delayed(delay);
                    matvec_add(&layer.w_conv[tap], xt, mid_ch, ch, &mut self.conv_out);
                }
                for c in 0..mid_ch {
                    self.conv_out[c] += layer.b_conv[c];
                }

                // Input mixin: add condition signal projected to mid_ch
                if let Some(ref w_mixin) = layer.w_input_mixin {
                    let cond_size = w_mixin.len() / mid_ch;
                    matvec(
                        w_mixin,
                        &self.rechannel_buf[..cond_size],
                        mid_ch,
                        cond_size,
                        &mut self.mixin_buf,
                    );
                    for c in 0..mid_ch {
                        self.conv_out[c] += self.mixin_buf[c];
                    }
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

                // Add z to the skip accumulator (bottleneck-wide)
                for c in 0..bottleneck {
                    self.skip_accum[c] += self.conv_out[c];
                }

                // Residual connection: layer1x1 maps z (bottleneck) back to
                // channels; without a layer1x1, bottleneck == channels
                // (enforced at construction) and z IS the residual.
                match &layer.layer1x1 {
                    Some(l1x1) => {
                        matvec(
                            &l1x1.weight,
                            &self.conv_out[..l1x1.in_ch],
                            l1x1.out_ch,
                            l1x1.in_ch,
                            &mut self.residual_buf,
                        );
                        for c in 0..l1x1.out_ch {
                            self.residual_buf[c] += l1x1.bias[c];
                        }
                        let x_curr = ring.read_current();
                        for (c, a) in self.activation.iter_mut().enumerate().take(ch) {
                            *a = x_curr[c] + self.residual_buf[c];
                        }
                    }
                    None => {
                        let x_curr = ring.read_current();
                        for (c, a) in self.activation.iter_mut().enumerate().take(ch) {
                            *a = self.conv_out[c] + x_curr[c];
                        }
                    }
                }
            }

            // Head rechannel: project skip_accum (bottleneck) to head_size
            // and accumulate
            let hr = &self.head_rechannels[stack_idx];
            // Pre-activation on skip_accum before head_rechannel, using the
            // stack's configured activation (A1: fast tanh, as before).
            stack[0].activation.apply(&mut self.skip_accum[..bottleneck]);
            matvec(
                &hr.weight,
                &self.skip_accum[..hr.in_ch],
                hr.out_ch,
                hr.in_ch,
                &mut self.head_buf_a,
            );
            for c in 0..hr.out_ch {
                self.head_input[c] += self.head_buf_a[c] + hr.bias[c];
            }
        }

        // Head MLP
        let head_size = self.head_input.len();
        if self.head_layers.is_empty() {
            // No head MLP — output is head_input[0] * head_scale
            return self.head_input[0] * self.head_scale;
        }

        self.head_buf_a[..head_size].copy_from_slice(&self.head_input);
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

        let result = if use_a {
            self.head_buf_a[0]
        } else {
            self.head_buf_b[0]
        };
        result * self.head_scale
    }

    fn reset(&mut self) {
        for stack_rings in &mut self.ring_buffers {
            for ring in stack_rings {
                ring.reset();
            }
        }
        self.activation.fill(0.0);
        self.skip_accum.fill(0.0);
        self.head_input.fill(0.0);
    }
}
