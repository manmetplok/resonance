/// WaveNet inference engine for NAM models.
///
/// Implements dilated causal convolutions with gated activations,
/// skip connections, and a head MLP.

use super::parse::{WaveNetConfig, WeightReader};
use super::{matvec, matvec_add, sigmoid, NamInference};

/// Ring buffer for storing channel vectors needed by dilated convolutions.
struct RingBuffer {
    data: Vec<f32>,
    capacity: usize,
    channels: usize,
    write_pos: usize,
}

impl RingBuffer {
    fn new(capacity: usize, channels: usize) -> Self {
        Self {
            data: vec![0.0; capacity * channels],
            capacity,
            channels,
            write_pos: 0,
        }
    }

    fn write(&mut self, values: &[f32]) {
        let base = self.write_pos * self.channels;
        self.data[base..base + self.channels].copy_from_slice(&values[..self.channels]);
        self.write_pos = (self.write_pos + 1) % self.capacity;
    }

    /// Read the most recently written values (delay=0).
    fn read_current(&self) -> &[f32] {
        self.read_delayed(0)
    }

    /// Read values written `delay` steps ago.
    fn read_delayed(&self, delay: usize) -> &[f32] {
        let pos = (self.write_pos + self.capacity - 1 - delay) % self.capacity;
        let base = pos * self.channels;
        &self.data[base..base + self.channels]
    }

    fn reset(&mut self) {
        self.data.fill(0.0);
        self.write_pos = 0;
    }
}

/// A single WaveNet dilated convolution layer.
struct WaveNetLayer {
    /// Filter convolution weights for the delayed sample [channels * channels].
    w_filter_prev: Vec<f32>,
    /// Filter convolution weights for the current sample [channels * channels].
    w_filter_curr: Vec<f32>,
    b_filter: Vec<f32>,

    /// Gate convolution weights (only used if gated).
    w_gate_prev: Vec<f32>,
    w_gate_curr: Vec<f32>,
    b_gate: Vec<f32>,

    /// 1x1 convolution for residual path [channels * channels].
    w_1x1: Vec<f32>,
    b_1x1: Vec<f32>,

    dilation: usize,
}

/// Dense (fully connected) layer for the head network.
struct DenseLayer {
    weight: Vec<f32>,
    bias: Vec<f32>,
    in_features: usize,
    out_features: usize,
    has_activation: bool,
}

pub struct WaveNetModel {
    channels: usize,
    gated: bool,

    // Input conditioning: x = input * weight + bias
    input_weight: Vec<f32>,
    input_bias: Vec<f32>,

    // Dilated conv layers [stack][layer]
    stacks: Vec<Vec<WaveNetLayer>>,

    // Head MLP
    head_layers: Vec<DenseLayer>,

    // State: ring buffer per layer
    ring_buffers: Vec<Vec<RingBuffer>>,

    // Pre-allocated scratch buffers
    activation: Vec<f32>,
    filter_buf: Vec<f32>,
    gate_buf: Vec<f32>,
    residual_buf: Vec<f32>,
    skip_accum: Vec<f32>,
    head_buf_a: Vec<f32>,
    head_buf_b: Vec<f32>,
}

impl WaveNetModel {
    pub fn from_config_and_weights(
        config: WaveNetConfig,
        reader: &mut WeightReader,
    ) -> Result<Self, String> {
        let ch = config.channels;

        // Input conditioning
        let input_weight = reader.read(ch * config.input_size)?;
        let input_bias = reader.read(ch)?;

        // Layers per stack
        let mut stacks = Vec::with_capacity(config.layers.len());
        let mut ring_buffers = Vec::with_capacity(config.layers.len());

        for &num_layers in &config.layers {
            let mut layers = Vec::with_capacity(num_layers);
            let mut rings = Vec::with_capacity(num_layers);

            for layer_idx in 0..num_layers {
                let dilation = 1 << layer_idx;

                // Dilated convolution weights (kernel_size = 2)
                // Stored as [out_channels, in_channels, kernel_size]
                // We split into prev (kernel[0]) and curr (kernel[1])
                let conv_weights = reader.read(ch * ch * 2)?;
                let mut w_filter_prev = vec![0.0f32; ch * ch];
                let mut w_filter_curr = vec![0.0f32; ch * ch];
                for out_c in 0..ch {
                    for in_c in 0..ch {
                        let base = (out_c * ch + in_c) * 2;
                        w_filter_prev[out_c * ch + in_c] = conv_weights[base];
                        w_filter_curr[out_c * ch + in_c] = conv_weights[base + 1];
                    }
                }

                let (w_gate_prev, w_gate_curr, b_gate) = if config.gated {
                    let gate_weights = reader.read(ch * ch * 2)?;
                    let mut gp = vec![0.0f32; ch * ch];
                    let mut gc = vec![0.0f32; ch * ch];
                    for out_c in 0..ch {
                        for in_c in 0..ch {
                            let base = (out_c * ch + in_c) * 2;
                            gp[out_c * ch + in_c] = gate_weights[base];
                            gc[out_c * ch + in_c] = gate_weights[base + 1];
                        }
                    }
                    let b = reader.read(ch)?;
                    // Read filter bias after gate weights but before gate bias
                    // Actually, let me reconsider the weight ordering...
                    (gp, gc, b)
                } else {
                    (Vec::new(), Vec::new(), Vec::new())
                };

                let b_filter = reader.read(ch)?;

                // 1x1 convolution (residual)
                let w_1x1 = reader.read(ch * ch)?;
                let b_1x1 = reader.read(ch)?;

                let ring_capacity = dilation + 2;
                rings.push(RingBuffer::new(ring_capacity, ch));

                layers.push(WaveNetLayer {
                    w_filter_prev,
                    w_filter_curr,
                    b_filter,
                    w_gate_prev,
                    w_gate_curr,
                    b_gate,
                    w_1x1,
                    b_1x1,
                    dilation,
                });
            }

            stacks.push(layers);
            ring_buffers.push(rings);
        }

        // Head layers
        let mut head_layers = Vec::new();
        let mut prev_size = ch;

        for &head_size in &config.head {
            let weight = reader.read(head_size * prev_size)?;
            let bias = if config.head_bias {
                reader.read(head_size)?
            } else {
                vec![0.0; head_size]
            };
            head_layers.push(DenseLayer {
                weight,
                bias,
                in_features: prev_size,
                out_features: head_size,
                has_activation: true,
            });
            prev_size = head_size;
        }

        // Final output layer
        let weight = reader.read(config.head_size * prev_size)?;
        let bias = if config.head_bias {
            reader.read(config.head_size)?
        } else {
            vec![0.0; config.head_size]
        };
        head_layers.push(DenseLayer {
            weight,
            bias,
            in_features: prev_size,
            out_features: config.head_size,
            has_activation: false,
        });

        // Compute max head buffer size
        let max_head = config
            .head
            .iter()
            .copied()
            .chain(std::iter::once(config.head_size))
            .chain(std::iter::once(ch))
            .max()
            .unwrap_or(ch);

        Ok(Self {
            channels: ch,
            gated: config.gated,
            input_weight,
            input_bias,
            stacks,
            head_layers,
            ring_buffers,
            activation: vec![0.0; ch],
            filter_buf: vec![0.0; ch],
            gate_buf: vec![0.0; ch],
            residual_buf: vec![0.0; ch],
            skip_accum: vec![0.0; ch],
            head_buf_a: vec![0.0; max_head],
            head_buf_b: vec![0.0; max_head],
        })
    }
}

impl NamInference for WaveNetModel {
    fn process_sample(&mut self, input: f32) -> f32 {
        let ch = self.channels;

        // 1. Input conditioning: activation[c] = input * weight[c] + bias[c]
        for c in 0..ch {
            self.activation[c] = input * self.input_weight[c] + self.input_bias[c];
        }

        // 2. Zero skip accumulator
        self.skip_accum.fill(0.0);

        // 3. Process each stack
        for (stack_idx, stack) in self.stacks.iter().enumerate() {
            for (layer_idx, layer) in stack.iter().enumerate() {
                let ring = &mut self.ring_buffers[stack_idx][layer_idx];

                // Write current activation into ring buffer
                ring.write(&self.activation);

                // Read current and delayed values
                let x_curr = ring.read_current();
                let x_prev = ring.read_delayed(layer.dilation);

                // Dilated convolution: filter path
                matvec(
                    &layer.w_filter_curr,
                    x_curr,
                    ch,
                    ch,
                    &mut self.filter_buf,
                );
                matvec_add(
                    &layer.w_filter_prev,
                    x_prev,
                    ch,
                    ch,
                    &mut self.filter_buf,
                );
                for c in 0..ch {
                    self.filter_buf[c] += layer.b_filter[c];
                }

                // Apply activation
                if self.gated {
                    // Gate path
                    matvec(
                        &layer.w_gate_curr,
                        x_curr,
                        ch,
                        ch,
                        &mut self.gate_buf,
                    );
                    matvec_add(
                        &layer.w_gate_prev,
                        x_prev,
                        ch,
                        ch,
                        &mut self.gate_buf,
                    );
                    for c in 0..ch {
                        self.gate_buf[c] += layer.b_gate[c];
                    }

                    // Gated activation: z = tanh(filter) * sigmoid(gate)
                    for c in 0..ch {
                        self.activation[c] =
                            self.filter_buf[c].tanh() * sigmoid(self.gate_buf[c]);
                    }
                } else {
                    for c in 0..ch {
                        self.activation[c] = self.filter_buf[c].tanh();
                    }
                }

                // Add to skip accumulator
                for c in 0..ch {
                    self.skip_accum[c] += self.activation[c];
                }

                // 1x1 convolution (residual)
                matvec(
                    &layer.w_1x1,
                    &self.activation,
                    ch,
                    ch,
                    &mut self.residual_buf,
                );
                for c in 0..ch {
                    self.residual_buf[c] += layer.b_1x1[c];
                }

                // Residual connection: activation = x_curr + residual
                for c in 0..ch {
                    self.activation[c] = x_curr[c] + self.residual_buf[c];
                }
            }
        }

        // 4. Head network
        // Apply tanh to skip accumulator
        for c in 0..ch {
            self.head_buf_a[c] = self.skip_accum[c].tanh();
        }

        let mut current_size = ch;
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
            for j in 0..head_layer.out_features {
                dst[j] += head_layer.bias[j];
            }
            if head_layer.has_activation {
                for j in 0..head_layer.out_features {
                    dst[j] = dst[j].tanh();
                }
            }

            current_size = head_layer.out_features;
            use_a = !use_a;
        }

        // Result is in whichever buffer was last written to
        if use_a {
            self.head_buf_a[0]
        } else {
            self.head_buf_b[0]
        }
    }

    fn reset(&mut self) {
        for stack_rings in &mut self.ring_buffers {
            for ring in stack_rings {
                ring.reset();
            }
        }
        self.activation.fill(0.0);
        self.skip_accum.fill(0.0);
    }
}
