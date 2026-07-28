/// LSTM inference engine for NAM models.
///
/// Mirrors the reference `NAM/lstm.cpp` (NeuralAmpModelerCore) exactly,
/// including its weight layout, which is NOT PyTorch's: the trainer export
/// concatenates each cell's input-to-hidden and hidden-to-hidden matrices
/// into one `[4*hidden, input+hidden]` row-major matrix, sums the two bias
/// vectors into one `[4*hidden]` bias, and then stores LEARNED initial
/// hidden and cell states (`h0`, `c0`) per cell. The head is a plain
/// `[1, hidden]` dense with bias and NO trailing head-scale weight (the
/// reference constructor asserts the weight vector is fully consumed).
///
/// Gate math (reference `LSTMCell::process_`, gate order i, f, g, o):
///
/// ```text
/// ifgo = W * [x; h] + b
/// c    = sigmoid(f) * c + sigmoid(i) * tanh(g)
/// h    = sigmoid(o) * tanh(c)
/// ```
///
/// Activation flavor: the official NAM plugin runs
/// `nam::activations::Activation::enable_fast_tanh()`, which switches the
/// reference cell to `fast_sigmoid` / `fast_tanh` — that is what "correct"
/// means for LSTM files in practice, so this engine uses the fast flavor
/// (the same reference rational formulas, see `activations.rs`).
/// Parity pin: `tests/nam_lstm_reference_parity.rs`.
use super::activations::Activation;
use super::parse::{LstmConfig, WeightReader};
use super::{matvec, NamInference};

struct LstmCell {
    /// Combined weights `[4*hidden_size, input_size + hidden_size]`,
    /// row-major (input columns first, then hidden columns).
    w: Vec<f32>,
    /// Combined bias `[4*hidden_size]` (the export sums b_ih + b_hh).
    b: Vec<f32>,
    /// Learned initial hidden state `[hidden_size]`.
    h0: Vec<f32>,
    /// Learned initial cell state `[hidden_size]`.
    c0: Vec<f32>,
    /// Current hidden state `[hidden_size]`.
    h: Vec<f32>,
    /// Current cell state `[hidden_size]`.
    c: Vec<f32>,
    input_size: usize,
}

pub struct LstmModel {
    hidden_size: usize,
    layers: Vec<LstmCell>,

    /// Output dense layer: weight [1, hidden_size], bias [1].
    output_weight: Vec<f32>,
    output_bias: f32,

    /// Gate activation (i/f/o gates). LSTM gate activations are fixed by the
    /// architecture, not model config; the NAM plugin runs the fast sigmoid.
    gate_activation: Activation,
    /// Cell activation (g gate and cell-state output): NAM fast tanh.
    cell_activation: Activation,

    // Pre-allocated scratch
    /// Concatenated `[x; h]` input to the current cell
    /// `[max input_size + hidden_size]`.
    xh: Vec<f32>,
    /// Gate pre-activations for the current cell `[4 * hidden_size]`.
    gates: Vec<f32>,
}

impl LstmModel {
    pub fn from_config_and_weights(
        config: LstmConfig,
        reader: &mut WeightReader,
    ) -> Result<Self, String> {
        let hs = config.hidden_size;
        if hs == 0 || config.num_layers == 0 {
            return Err("LSTM config: hidden_size and num_layers must be nonzero".into());
        }
        let mut layers = Vec::with_capacity(config.num_layers);

        for i in 0..config.num_layers {
            let layer_input = if i == 0 { config.input_size } else { hs };

            // Reference layout per cell: combined W [4hs, in+hs] row-major,
            // combined bias [4hs], learned initial h [hs], learned initial
            // c [hs] (NAM/lstm.cpp LSTMCell constructor order).
            let w = reader.read(4 * hs * (layer_input + hs))?;
            let b = reader.read(4 * hs)?;
            let h0 = reader.read(hs)?;
            let c0 = reader.read(hs)?;

            layers.push(LstmCell {
                w,
                b,
                h: h0.clone(),
                c: c0.clone(),
                h0,
                c0,
                input_size: layer_input,
            });
        }

        // Output dense layer: weight [1, hs] + bias [1]. The reference
        // consumes exactly these and asserts nothing is left — there is no
        // trailing head-scale weight in the LSTM export.
        let output_weight = reader.read(hs)?;
        let output_bias = reader.read(1)?[0];

        let max_input = layers
            .iter()
            .map(|l| l.input_size)
            .max()
            .unwrap_or(config.input_size);

        Ok(Self {
            hidden_size: hs,
            layers,
            output_weight,
            output_bias,
            gate_activation: Activation::FastSigmoid,
            cell_activation: Activation::FastTanh,
            xh: vec![0.0; max_input + hs],
            gates: vec![0.0; 4 * hs],
        })
    }
}

impl NamInference for LstmModel {
    fn process_sample(&mut self, input: f32) -> f32 {
        let hs = self.hidden_size;

        // First cell input is the scalar sample. Any extra input slots
        // (input_size > 1) stay zero, like the reference's `_input` vector,
        // which only ever receives the mono channel.
        self.xh[0] = input;
        let first_input = self.layers[0].input_size;
        self.xh[1..first_input].fill(0.0);

        for layer_idx in 0..self.layers.len() {
            let layer = &self.layers[layer_idx];
            let xh_len = layer.input_size + hs;

            // xh = [x; h] — x was written by the previous iteration (or the
            // scalar input above); append this cell's hidden state.
            self.xh[layer.input_size..xh_len].copy_from_slice(&layer.h);

            // ifgo = W * xh + b
            matvec(&layer.w, &self.xh[..xh_len], 4 * hs, xh_len, &mut self.gates);
            for j in 0..4 * hs {
                self.gates[j] += layer.b[j];
            }

            // Elementwise state update, gate order i, f, g, o:
            //   c = sigmoid(f) * c + sigmoid(i) * tanh(g)
            //   h = sigmoid(o) * tanh(c)
            let layer = &mut self.layers[layer_idx];
            for j in 0..hs {
                let i_gate = self.gate_activation.scalar(self.gates[j]);
                let f_gate = self.gate_activation.scalar(self.gates[hs + j]);
                let g_gate = self.cell_activation.scalar(self.gates[2 * hs + j]);
                let o_gate = self.gate_activation.scalar(self.gates[3 * hs + j]);

                layer.c[j] = f_gate * layer.c[j] + i_gate * g_gate;
                layer.h[j] = o_gate * self.cell_activation.scalar(layer.c[j]);
            }

            // This cell's hidden state is the next cell's input.
            self.xh[..hs].copy_from_slice(&self.layers[layer_idx].h);
        }

        // Output dense layer: dot(weight, h_last) + bias.
        let h_last = &self.layers[self.layers.len() - 1].h;
        let mut out = self.output_bias;
        for (w, h) in self.output_weight.iter().zip(h_last.iter()) {
            out += w * h;
        }
        out
    }

    fn reset(&mut self) {
        // Back to the learned initial states — the state a freshly
        // constructed reference model starts its prewarm from.
        for layer in &mut self.layers {
            layer.h.copy_from_slice(&layer.h0);
            layer.c.copy_from_slice(&layer.c0);
        }
    }
}
