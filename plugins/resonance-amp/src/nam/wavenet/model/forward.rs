//! The per-sample forward pass, split into per-stage methods: condition,
//! per-stack rechannel/seed, the five per-layer stages (conv, mixin,
//! activation/gating, skip, residual), the per-stack head rechannel, and
//! the legacy head MLP.
//!
//! All stages are allocation-free and reference-exact; the split is pure
//! code motion from the historical single-body `forward` (ba todo #1131).

use super::super::super::{grouped_matvec, grouped_matvec_add, matvec, matvec_add};
use super::super::conv_layer::LayerGating;
use super::WaveNetModel;

impl WaveNetModel {
    /// One full forward pass; leaves the pre-`head_scale` output channels
    /// in `head_input[..out_channels]`.
    pub(super) fn forward(&mut self, input: f32) {
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

        for stack_idx in 0..self.stacks.len() {
            self.process_stack(stack_idx, input);
        }

        self.run_head_mlp();
    }

    /// One stack (reference LayerArray): rechannel, condition snapshot,
    /// skip-accumulator seed, the layer chain, and the head rechannel.
    fn process_stack(&mut self, stack_idx: usize, input: f32) {
        let stack = &self.stacks[stack_idx];
        // Construction guarantees every stack has at least one layer.
        let bottleneck = stack[0].bottleneck;

        // Rechannel (always present; the reference constructs and
        // applies it unconditionally, a 1-to-1 rechannel included).
        let rc = &self.rechannels[stack_idx];
        matvec(
            &rc.weight,
            &self.activation[..rc.in_ch],
            rc.out_ch,
            rc.in_ch,
            &mut self.rechannel_buf,
        );
        self.activation[..rc.out_ch].copy_from_slice(&self.rechannel_buf[..rc.out_ch]);

        // Save the condition snapshot the layers' input_mixin and FiLM
        // sites read (all of them index rechannel_buf). With a
        // condition_dsp its per-sample output IS the condition for
        // every stack (reference passes `_condition_output` to each
        // LayerArray). Without one, the condition is the RAW model
        // input for every stack (reference `_process_condition` copies
        // `_condition_input` through; construction validated
        // condition_size == input_size == 1).
        match &self.condition_dsp {
            Some(_) => {
                let n = self.condition_buf.len();
                self.rechannel_buf[..n].copy_from_slice(&self.condition_buf);
            }
            None => self.rechannel_buf[0] = input,
        }

        // Skip accumulator for this stack. Its width follows the
        // head path: head1x1.out_channels when the stack's head1x1 is
        // active, else bottleneck (reference `_head_output_size`). All
        // layers of a stack share one head1x1 config (per-layer-array in
        // the reference), so the width is uniform within the stack.
        //
        // The stacks' head paths CHAIN (reference `LayerArray::Process`
        // with head inputs): stack 0 starts at zero, and every later
        // stack starts from the preceding stack's head-rechannel
        // output (`head_input` here, widths validated equal at
        // construction).
        let skip_ch = stack[0]
            .head1x1
            .as_ref()
            .map_or(bottleneck, |h| h.out_ch);
        if stack_idx > 0 {
            let (seed, _) = self.head_input.split_at(skip_ch);
            self.skip_accum[..skip_ch].copy_from_slice(seed);
        } else {
            self.skip_accum[..skip_ch].fill(0.0);
        }

        for layer_idx in 0..self.stacks[stack_idx].len() {
            self.layer_conv(stack_idx, layer_idx);
            self.layer_mixin(stack_idx, layer_idx);
            self.layer_activate(stack_idx, layer_idx);
            self.layer_skip(stack_idx, layer_idx);
            self.layer_residual(stack_idx, layer_idx);
        }

        self.stack_head_rechannel(stack_idx, skip_ch);
    }

    /// Layer stage 1: write the layer input into the dilation ring (with
    /// the conv_pre FiLM applied to the ring copy only) and run the
    /// dilated convolution, leaving the biased conv output in
    /// `conv_out[..mid_ch]`.
    fn layer_conv(&mut self, stack_idx: usize, layer_idx: usize) {
        let layer = &self.stacks[stack_idx][layer_idx];
        let ring = &mut self.ring_buffers[stack_idx][layer_idx];
        let ch = layer.channels;
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
    }

    /// Layer stage 2: input mixin — add the condition signal projected to
    /// mid_ch into `conv_out`.
    fn layer_mixin(&mut self, stack_idx: usize, layer_idx: usize) {
        let layer = &self.stacks[stack_idx][layer_idx];
        let mid_ch = layer.mid_ch;

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
    }

    /// Layer stage 3: activation / gating (with the activation_pre and
    /// activation_post FiLM sites). Leaves the activated z bottleneck-wide
    /// in `conv_out[..bottleneck]`.
    fn layer_activate(&mut self, stack_idx: usize, layer_idx: usize) {
        let layer = &self.stacks[stack_idx][layer_idx];
        let bottleneck = layer.bottleneck;
        let mid_ch = layer.mid_ch;

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
    }

    /// Layer stage 4: skip contribution — head1x1(activated z) when the
    /// stack's head1x1 is active (reference `_head1x1->process_(z)` on
    /// the activated top-bottleneck rows), else the activated z itself
    /// (A1 direct skip, bottleneck-wide).
    fn layer_skip(&mut self, stack_idx: usize, layer_idx: usize) {
        let layer = &self.stacks[stack_idx][layer_idx];
        let bottleneck = layer.bottleneck;

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
    }

    /// Layer stage 5: residual connection — layer1x1 maps z (bottleneck)
    /// back to channels; without a layer1x1, bottleneck == channels
    /// (enforced at construction) and z IS the residual. The raw
    /// layer input still sits in `self.activation` (the conv_pre
    /// FiLM, when active, modulated only the ring copy), so the
    /// residual reads it from there — bit-identical to the
    /// historical ring `read_current()`, whose newest frame was
    /// that same value.
    fn layer_residual(&mut self, stack_idx: usize, layer_idx: usize) {
        let layer = &self.stacks[stack_idx][layer_idx];
        let ch = layer.channels;

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
                // No layer1x1: the residual is the identity — the
                // next layer's input is the raw layer input alone
                // (`_output_next_layer = input` in Layer::Process),
                // which already sits in `self.activation`.
            }
        }
    }

    /// Head rechannel: project skip_accum (skip_ch wide) to head_size and
    /// leave the current stack's head-rechannel output in `head_input` —
    /// it seeds the next stack's skip accumulator, and after the last
    /// stack it IS the model output (reference `WaveNet::process` reads
    /// only `_layer_arrays.back().GetHeadOutputs()`).
    fn stack_head_rechannel(&mut self, stack_idx: usize, skip_ch: usize) {
        let hr = &self.head_rechannels[stack_idx];
        match &mut self.head_rings[stack_idx] {
            None => {
                // Kernel-1 path (A1 / head_kernel_size == 1,
                // memoryless). The reference applies NO activation here
                // (`_head_rechannel.Process(_head_inputs)` directly in
                // LayerArray::ProcessInner): the per-layer skip
                // contributions are already activated z.
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
        for c in 0..hr.out_ch {
            self.head_input[c] = self.head_buf_a[c] + hr.bias[c];
        }
    }

    /// Legacy head MLP: ping-pong through the dense layers and expose the
    /// result in `head_input` so every path reads the pre-head_scale
    /// output channels from the same place (out_channels == head_size on
    /// the MLP path). No-op when the model has no head MLP — the
    /// pre-head_scale output channels are the accumulated head_input
    /// as-is.
    fn run_head_mlp(&mut self) {
        if self.head_layers.is_empty() {
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

        let src = if use_a {
            &self.head_buf_a
        } else {
            &self.head_buf_b
        };
        self.head_input[..head_size].copy_from_slice(&src[..head_size]);
    }
}
