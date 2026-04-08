/// LSTM inference engine for NAM models.

use super::parse::{LstmConfig, WeightReader};
use super::{fast_tanh, matvec, matvec_add, sigmoid, NamInference};

struct LstmLayer {
    /// Input-to-hidden weights [4*hidden_size, input_size_for_layer] row-major.
    w_ih: Vec<f32>,
    /// Hidden-to-hidden weights [4*hidden_size, hidden_size] row-major.
    w_hh: Vec<f32>,
    /// Input-to-hidden bias [4*hidden_size].
    b_ih: Vec<f32>,
    /// Hidden-to-hidden bias [4*hidden_size].
    b_hh: Vec<f32>,
    input_size: usize,
}

pub struct LstmModel {
    hidden_size: usize,
    layers: Vec<LstmLayer>,

    /// Output dense layer: weight [1, hidden_size], bias [1].
    output_weight: Vec<f32>,
    output_bias: f32,
    /// Learned output scaling factor (last weight in the blob).
    head_scale: f32,

    // Pre-allocated state
    /// Hidden state per layer [num_layers][hidden_size].
    h: Vec<Vec<f32>>,
    /// Cell state per layer [num_layers][hidden_size].
    c: Vec<Vec<f32>>,
    /// Scratch buffer for gate computation [4 * hidden_size].
    gates: Vec<f32>,
    /// Scratch buffer holding the input to the current layer.
    input_buf: Vec<f32>,
}

impl LstmModel {
    pub fn from_config_and_weights(
        config: LstmConfig,
        reader: &mut WeightReader,
    ) -> Result<Self, String> {
        let hs = config.hidden_size;
        let mut layers = Vec::with_capacity(config.num_layers);

        for i in 0..config.num_layers {
            let layer_input = if i == 0 { config.input_size } else { hs };

            // PyTorch LSTM weight ordering: w_ih, w_hh, b_ih, b_hh per layer
            let w_ih = reader.read(4 * hs * layer_input)?;
            let w_hh = reader.read(4 * hs * hs)?;
            let b_ih = reader.read(4 * hs)?;
            let b_hh = reader.read(4 * hs)?;

            layers.push(LstmLayer {
                w_ih,
                w_hh,
                b_ih,
                b_hh,
                input_size: layer_input,
            });
        }

        // Output dense layer
        let output_weight = reader.read(hs)?;
        let output_bias_vec = reader.read(1)?;

        // Head scale (last weight in the blob, like WaveNet)
        let head_scale = if reader.remaining() >= 1 {
            reader.read(1)?[0]
        } else {
            1.0
        };

        Ok(Self {
            hidden_size: hs,
            layers,
            output_weight,
            output_bias: output_bias_vec[0],
            head_scale,
            h: vec![vec![0.0; hs]; config.num_layers],
            c: vec![vec![0.0; hs]; config.num_layers],
            gates: vec![0.0; 4 * hs],
            input_buf: vec![0.0; hs.max(config.input_size)],
        })
    }
}

impl NamInference for LstmModel {
    fn process_sample(&mut self, input: f32) -> f32 {
        let hs = self.hidden_size;

        // First layer input is the scalar input
        self.input_buf[0] = input;
        let mut input_len = self.layers[0].input_size;

        for layer_idx in 0..self.layers.len() {
            let layer = &self.layers[layer_idx];

            // gates = W_ih * x + b_ih + W_hh * h + b_hh
            matvec(
                &layer.w_ih,
                &self.input_buf[..input_len],
                4 * hs,
                layer.input_size,
                &mut self.gates,
            );
            for j in 0..4 * hs {
                self.gates[j] += layer.b_ih[j];
            }
            matvec_add(
                &layer.w_hh,
                &self.h[layer_idx],
                4 * hs,
                hs,
                &mut self.gates,
            );
            for j in 0..4 * hs {
                self.gates[j] += layer.b_hh[j];
            }

            // Apply gate activations (PyTorch ordering: i, f, g, o)
            let h = &mut self.h[layer_idx];
            let c = &mut self.c[layer_idx];
            for j in 0..hs {
                let i_gate = sigmoid(self.gates[j]);
                let f_gate = sigmoid(self.gates[hs + j]);
                let g_gate = fast_tanh(self.gates[2 * hs + j]);
                let o_gate = sigmoid(self.gates[3 * hs + j]);

                c[j] = f_gate * c[j] + i_gate * g_gate;
                h[j] = o_gate * fast_tanh(c[j]);
            }

            // Output of this layer becomes input to the next
            self.input_buf[..hs].copy_from_slice(h);
            input_len = hs;
        }

        // Output dense layer: dot(weight, h_last) + bias
        let h_last = &self.h[self.layers.len() - 1];
        let mut out = self.output_bias;
        for j in 0..hs {
            out += self.output_weight[j] * h_last[j];
        }
        out * self.head_scale
    }

    fn reset(&mut self) {
        for h in &mut self.h {
            h.fill(0.0);
        }
        for c in &mut self.c {
            c.fill(0.0);
        }
    }
}
