//! The block forward pass, split into per-stage methods: condition,
//! per-stack rechannel/seed, the per-layer stages (conv, mixin,
//! activation/gating, skip, residual), the per-stack head rechannel, and
//! the legacy head MLP.
//!
//! Every stage runs over a whole block of frames before the next starts,
//! so the convolutions are block matrix products ([`grouped_gemm_acc`])
//! instead of one matvec per sample; a layer's taps read contiguous
//! windows of its input [`History`](super::super::history::History).
//! Elementwise stages (FiLM, activations, gating) loop per frame and are
//! the reference math unchanged. Output is stream-equivalent to the old
//! sample-serial pass for any block split, up to f32 accumulation order.
//!
//! All stages are allocation-free.

use super::super::super::activations::Activation;
use super::super::super::gemm::{gemm_acc, grouped_gemm_acc, Frames};
use super::super::super::matvec;
use super::super::conv_layer::{LayerGating, WaveNetLayer};
use super::{WaveNetModel, MAX_BLOCK};

/// Add `n` frames of `bias` into a block whose rows are `stride` apart.
#[inline]
fn add_rows(buf: &mut [f32], stride: usize, bias: &[f32], n: usize) {
    for t in 0..n {
        for (v, b) in buf[t * stride..][..bias.len()].iter_mut().zip(bias) {
            *v += b;
        }
    }
}

/// `dst[t][..width] += src[t][..width]` over `n` frames.
#[inline]
fn add_block(dst: &mut [f32], ds: usize, src: &[f32], ss: usize, width: usize, n: usize) {
    for t in 0..n {
        for (d, s) in dst[t * ds..][..width].iter_mut().zip(&src[t * ss..][..width]) {
            *d += s;
        }
    }
}

/// Apply `act` to the first `width` channels of each of `n` frames spaced
/// `stride` apart. PReLU's slopes are per channel, so it only sees one
/// frame per call; every other activation is position-free and takes a
/// dense block in one call.
#[inline]
fn activate_rows(act: &Activation, buf: &mut [f32], stride: usize, width: usize, n: usize) {
    if stride == width && !matches!(act, Activation::PRelu { .. }) {
        act.apply(&mut buf[..n * width]);
    } else {
        for t in 0..n {
            act.apply(&mut buf[t * stride..][..width]);
        }
    }
}

impl WaveNetModel {
    /// One forward pass over `input` (at most [`MAX_BLOCK`] frames);
    /// leaves each frame's pre-`head_scale` output channels at the start
    /// of its `head_w`-wide row of `self.head`.
    pub(super) fn forward_block(&mut self, input: &[f32]) {
        let n = input.len();
        debug_assert!(n <= MAX_BLOCK);

        // condition_dsp (A2): the nested net transforms the raw input
        // into the condition before the stacks run (reference
        // `WaveNet::process`: `_process_condition` ahead of the layer
        // arrays). Without one the condition is the RAW model input for
        // every stack (construction validated condition_size ==
        // input_size == 1).
        match &mut self.condition_dsp {
            Some(cd) => cd.process_block_into(input, &mut self.cond),
            None => self.cond[..n].copy_from_slice(input),
        }

        // Seed the stream with the raw input on channel 0 (rechanneled by
        // the first stack).
        let in_w = self.in_channels;
        self.act[..n * in_w].fill(0.0);
        for (t, &x) in input.iter().enumerate() {
            self.act[t * in_w] = x;
        }

        self.head[..n * self.head_w].fill(0.0);

        let mut stream_w = in_w;
        for stack_idx in 0..self.stacks.len() {
            stream_w = self.process_stack(stack_idx, stream_w, n);
        }

        self.run_head_mlp(n);
    }

    /// One stack (reference LayerArray): rechannel, skip-accumulator seed,
    /// the layer chain, and the head rechannel. `in_w` is the width of the
    /// incoming stream; returns the stack's channel count (the outgoing
    /// stream width).
    fn process_stack(&mut self, stack_idx: usize, in_w: usize, n: usize) -> usize {
        // Rechannel (always present; the reference constructs and
        // applies it unconditionally, a 1-to-1 rechannel included).
        let rc = &self.rechannels[stack_idx];
        debug_assert_eq!(rc.in_ch, in_w);
        self.act_next[..n * rc.out_ch].fill(0.0);
        gemm_acc(
            Frames { data: &self.act, stride: in_w },
            &rc.weight,
            rc.in_ch,
            rc.out_ch,
            &mut self.act_next,
            rc.out_ch,
            n,
        );
        std::mem::swap(&mut self.act, &mut self.act_next);

        // Skip accumulator for this stack. Its width follows the head
        // path: head1x1.out_channels when the stack's head1x1 is active,
        // else bottleneck (reference `_head_output_size`); uniform within
        // a stack.
        //
        // The stacks' head paths CHAIN (reference `LayerArray::Process`
        // with head inputs): stack 0 starts at zero, and every later stack
        // starts from the preceding stack's head-rechannel output (widths
        // validated equal at construction).
        let first = &self.stacks[stack_idx][0];
        let skip_ch = first.head1x1.as_ref().map_or(first.bottleneck, |h| h.out_ch);
        if stack_idx > 0 {
            for t in 0..n {
                self.skip[t * skip_ch..][..skip_ch]
                    .copy_from_slice(&self.head[t * self.head_w..][..skip_ch]);
            }
        } else {
            self.skip[..n * skip_ch].fill(0.0);
        }

        for layer_idx in 0..self.stacks[stack_idx].len() {
            self.layer_conv(stack_idx, layer_idx, n);
            self.layer_mixin(stack_idx, layer_idx, n);
            self.layer_activate(stack_idx, layer_idx, n);
            self.layer_skip(stack_idx, layer_idx, skip_ch, n);
            self.layer_residual(stack_idx, layer_idx, n);
        }

        self.stack_head_rechannel(stack_idx, skip_ch, n);
        self.rechannels[stack_idx].out_ch
    }

    /// Layer stage 1: append the layer input to its history (with the
    /// conv_pre FiLM applied to the history copy only) and run the dilated
    /// convolution, leaving the biased conv output in `z` (mid_ch wide).
    fn layer_conv(&mut self, stack_idx: usize, layer_idx: usize, n: usize) {
        let layer = &self.stacks[stack_idx][layer_idx];
        let hist = &mut self.histories[stack_idx][layer_idx];
        let ch = layer.channels;
        let mid = layer.mid_ch;
        let cw = self.cond_w;

        // With an active conv_pre FiLM the conv (and its tap history)
        // sees the FiLM-modulated input (reference
        // `_conv.Process(_conv_pre_film->GetOutput())`); the residual
        // path keeps the raw input in `self.act`.
        let slot = hist.begin_block(n);
        match &layer.conv_pre_film {
            Some(f) => {
                for t in 0..n {
                    f.modulate(
                        &self.cond[t * cw..],
                        &self.act[t * ch..],
                        &mut slot[t * ch..],
                        &mut self.film_ss_buf,
                    );
                }
            }
            None => slot.copy_from_slice(&self.act[..n * ch]),
        }

        // Dilated convolution (combined filter+gate): one block product
        // per tap, then the bias. Tap k reads the input
        // `(ks - 1 - k) * dilation` frames back (reference
        // `Conv1D::Process` offsets).
        self.z[..n * mid].fill(0.0);
        let ks = layer.kernel_size;
        for (tap, w) in layer.w_conv.iter().enumerate() {
            let delay = (ks - 1 - tap) * layer.dilation;
            grouped_gemm_acc(
                Frames { data: hist.window(delay, n), stride: ch },
                w,
                ch,
                mid,
                layer.groups_input,
                &mut self.z,
                mid,
                n,
            );
        }
        hist.end_block(n);
        add_rows(&mut self.z, mid, &layer.b_conv, n);

        // conv_post FiLM: modulate the conv output in place before the
        // mixin sum (reference `Process_` on `_conv.GetOutput()`).
        if let Some(f) = &layer.conv_post_film {
            for t in 0..n {
                f.modulate_in_place(
                    &self.cond[t * cw..],
                    &mut self.z[t * mid..][..mid],
                    &mut self.film_ss_buf,
                );
            }
        }
    }

    /// Layer stage 2: input mixin — add the condition signal projected to
    /// mid_ch into `z`.
    fn layer_mixin(&mut self, stack_idx: usize, layer_idx: usize, n: usize) {
        let layer = &self.stacks[stack_idx][layer_idx];
        let Some(w_mixin) = &layer.w_input_mixin else {
            return;
        };
        let mid = layer.mid_ch;
        let cw = self.cond_w;
        let cond_size = layer.condition_size;

        // An active input_mixin_pre FiLM modulates the condition fed to
        // the mixin ONLY (self-conditioned, reference
        // `Process(condition, condition)`); later sites still see the raw
        // condition.
        let src = match &layer.input_mixin_pre_film {
            Some(f) => {
                for t in 0..n {
                    f.modulate(
                        &self.cond[t * cw..],
                        &self.cond[t * cw..],
                        &mut self.film_in[t * cond_size..],
                        &mut self.film_ss_buf,
                    );
                }
                Frames { data: &self.film_in, stride: cond_size }
            }
            None => Frames { data: &self.cond, stride: cw },
        };

        // The mixin is computed on its own and then summed into z (the
        // reference order), with the input_mixin_post FiLM, when active,
        // modulating it in between.
        self.mixin[..n * mid].fill(0.0);
        grouped_gemm_acc(
            src,
            w_mixin,
            cond_size,
            mid,
            layer.groups_input_mixin,
            &mut self.mixin,
            mid,
            n,
        );
        if let Some(f) = &layer.input_mixin_post_film {
            for t in 0..n {
                f.modulate_in_place(
                    &self.cond[t * cw..],
                    &mut self.mixin[t * mid..][..mid],
                    &mut self.film_ss_buf,
                );
            }
        }
        add_block(&mut self.z, mid, &self.mixin, mid, mid, n);
    }

    /// Layer stage 3: activation / gating (with the activation_pre and
    /// activation_post FiLM sites). Leaves the activated z in the first
    /// `bottleneck` columns of each `mid_ch`-wide frame of `z`.
    fn layer_activate(&mut self, stack_idx: usize, layer_idx: usize, n: usize) {
        let layer = &self.stacks[stack_idx][layer_idx];
        let bn = layer.bottleneck;
        let mid = layer.mid_ch;
        let cw = self.cond_w;

        // activation_pre FiLM: modulate z = conv + mixin (full conv
        // width, incl. the secondary half when gated/blended) before the
        // activation (reference `Process_(_z)`).
        if let Some(f) = &layer.activation_pre_film {
            for t in 0..n {
                f.modulate_in_place(
                    &self.cond[t * cw..],
                    &mut self.z[t * mid..][..mid],
                    &mut self.film_ss_buf,
                );
            }
        }

        // Activation / gating (dispatch resolved at construction). For
        // gated/blended layers the secondary half sits in columns
        // bn..2*bn of each frame and must not be used past this point
        // (reference NAM/gating_activations.h).
        match &layer.gating {
            LayerGating::None => activate_rows(&layer.activation, &mut self.z, mid, bn, n),
            LayerGating::Gated(secondary) => {
                for t in 0..n {
                    let (z, g) = self.z[t * mid..][..mid].split_at_mut(bn);
                    layer.activation.apply(z);
                    secondary.apply(g);
                    for (zc, gc) in z.iter_mut().zip(g.iter()) {
                        *zc *= gc;
                    }
                }
            }
            LayerGating::Blended(secondary) => {
                // Reference BlendingActivation: alpha = blend(g);
                // out = alpha * primary(z) + (1 - alpha) * z_pre, with
                // z_pre the pre-activation primary half.
                for t in 0..n {
                    let (z, g) = self.z[t * mid..][..mid].split_at_mut(bn);
                    let pre = &mut self.pre_act_buf[..bn];
                    pre.copy_from_slice(z);
                    layer.activation.apply(z);
                    secondary.apply(g);
                    for c in 0..bn {
                        let alpha = g[c];
                        z[c] = alpha * z[c] + (1.0 - alpha) * pre[c];
                    }
                }
            }
        }

        // activation_post FiLM: modulate the activated z (the primary
        // bottleneck-wide half in all gating modes).
        if let Some(f) = &layer.activation_post_film {
            for t in 0..n {
                f.modulate_in_place(
                    &self.cond[t * cw..],
                    &mut self.z[t * mid..][..bn],
                    &mut self.film_ss_buf,
                );
            }
        }
    }

    /// Layer stage 4: skip contribution — head1x1(activated z) when the
    /// stack's head1x1 is active (reference `_head1x1->process_(z)` on the
    /// activated top-bottleneck rows), else the activated z itself (A1
    /// direct skip, bottleneck-wide).
    fn layer_skip(&mut self, stack_idx: usize, layer_idx: usize, skip_ch: usize, n: usize) {
        let layer = &self.stacks[stack_idx][layer_idx];
        let bn = layer.bottleneck;
        let mid = layer.mid_ch;
        let cw = self.cond_w;
        let z = Frames { data: &self.z, stride: mid };

        match &layer.head1x1 {
            Some(h) => {
                // head1x1(z) + bias on its own, the head1x1_post FiLM when
                // active, then into the skip accumulator (reference
                // `Process_` on `_head1x1->GetOutput()`).
                let out = h.out_ch;
                self.conv_tmp[..n * out].fill(0.0);
                grouped_gemm_acc(z, &h.weight, h.in_ch, out, h.groups, &mut self.conv_tmp, out, n);
                add_rows(&mut self.conv_tmp, out, &h.bias, n);
                if let Some(f) = &layer.head1x1_post_film {
                    for t in 0..n {
                        f.modulate_in_place(
                            &self.cond[t * cw..],
                            &mut self.conv_tmp[t * out..][..out],
                            &mut self.film_ss_buf,
                        );
                    }
                }
                add_block(&mut self.skip, skip_ch, &self.conv_tmp, out, out, n);
            }
            None => add_block(&mut self.skip, skip_ch, &self.z, mid, bn, n),
        }
    }

    /// Layer stage 5: residual connection — layer1x1 maps z (bottleneck)
    /// back to channels and adds it to the layer input in `self.act`;
    /// without a layer1x1 (bottleneck == channels, enforced at
    /// construction) the residual is the identity: the next layer's input
    /// is the raw layer input alone (`_output_next_layer = input` in
    /// Layer::Process), which already sits in `self.act`.
    fn layer_residual(&mut self, stack_idx: usize, layer_idx: usize, n: usize) {
        let layer: &WaveNetLayer = &self.stacks[stack_idx][layer_idx];
        let Some(l1x1) = &layer.layer1x1 else {
            return;
        };
        let ch = layer.channels;
        let cw = self.cond_w;
        let z = Frames { data: &self.z, stride: layer.mid_ch };

        // layer1x1_post FiLM: reference-exact, applied ONLY in the
        // BLENDED gating branch of `Layer::Process` (the NONE and GATED
        // branches run the layer1x1 without it, even though its weights
        // were consumed).
        let post_film = match layer.gating {
            LayerGating::Blended(_) => layer.layer1x1_post_film.as_ref(),
            _ => None,
        };
        let out = l1x1.out_ch;
        self.conv_tmp[..n * out].fill(0.0);
        grouped_gemm_acc(
            z,
            &l1x1.weight,
            l1x1.in_ch,
            out,
            l1x1.groups,
            &mut self.conv_tmp,
            out,
            n,
        );
        add_rows(&mut self.conv_tmp, out, &l1x1.bias, n);
        if let Some(f) = post_film {
            for t in 0..n {
                f.modulate_in_place(
                    &self.cond[t * cw..],
                    &mut self.conv_tmp[t * out..][..out],
                    &mut self.film_ss_buf,
                );
            }
        }
        add_block(&mut self.act, ch, &self.conv_tmp, out, ch, n);
    }

    /// Head rechannel: project the skip accumulator to the stack's head
    /// width and leave it in `head` — it seeds the next stack's skip
    /// accumulator, and after the last stack it IS the model output
    /// (reference `WaveNet::process` reads only
    /// `_layer_arrays.back().GetHeadOutputs()`). The reference applies NO
    /// activation here (`_head_rechannel.Process(_head_inputs)` directly
    /// in LayerArray::ProcessInner): the per-layer skip contributions are
    /// already activated z.
    fn stack_head_rechannel(&mut self, stack_idx: usize, skip_ch: usize, n: usize) {
        let hr = &self.head_rechannels[stack_idx];
        let hw = self.head_w;
        // The previous stack's head output in the same columns was
        // already consumed by this stack's skip seed; columns past this
        // stack's width keep their value, as the per-sample pass did.
        for t in 0..n {
            self.head[t * hw..][..hr.out_ch].fill(0.0);
        }
        match &mut self.head_histories[stack_idx] {
            // Kernel-1 path (A1 / head_kernel_size == 1, memoryless).
            None => gemm_acc(
                Frames { data: &self.skip, stride: skip_ch },
                &hr.taps[0],
                hr.in_ch,
                hr.out_ch,
                &mut self.head,
                hw,
                n,
            ),
            // Windowed path (head_kernel_size > 1): causal dilated
            // convolution over the raw accumulated skip frames; tap k
            // reads the frame (ks - 1 - k) * dilation samples back, so
            // taps[ks - 1] multiplies the current frame.
            Some(hist) => {
                hist.begin_block(n).copy_from_slice(&self.skip[..n * skip_ch]);
                let ks = hr.kernel_size();
                for (tap, w) in hr.taps.iter().enumerate() {
                    let delay = (ks - 1 - tap) * hr.dilation;
                    gemm_acc(
                        Frames { data: hist.window(delay, n), stride: skip_ch },
                        w,
                        hr.in_ch,
                        hr.out_ch,
                        &mut self.head,
                        hw,
                        n,
                    );
                }
                hist.end_block(n);
            }
        }
        add_rows(&mut self.head, hw, &hr.bias, n);
    }

    /// Legacy head MLP, per frame: ping-pong through the dense layers and
    /// write the result back over the frame's first `head_size` head
    /// columns, so every path reads the pre-head_scale output channels
    /// from the same place. No-op when the model has no head MLP.
    fn run_head_mlp(&mut self, n: usize) {
        if self.head_layers.is_empty() {
            return;
        }
        let head_size = self.head_size;
        for t in 0..n {
            let row = &mut self.head[t * self.head_w..][..head_size];
            self.head_buf_a[..head_size].copy_from_slice(row);
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
                    self.head_activation
                        .apply(&mut dst[..head_layer.out_features]);
                }
                current_size = head_layer.out_features;
                use_a = !use_a;
            }

            let src = if use_a { &self.head_buf_a } else { &self.head_buf_b };
            row.copy_from_slice(&src[..head_size]);
        }
    }
}
