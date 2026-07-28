//! Slimmable packed-weight slice extraction (reference
//! `NAM/wavenet/slimmable.h`/`.cpp` in NeuralAmpModelerCore, MIT).
//!
//! A slimmable WaveNet packs ONE flat weight vector, laid out for the
//! largest allowed channel count of every layer array — identical to a
//! non-slimmable export of the full-size model. A normalized size in
//! [0.0, 1.0] maps to a channel count per array through that array's
//! `allowed_channels` list ([`channels_for_size`]); the
//! `slice_channels_uniform` method then walks the packed vector in the
//! reference `set_weights_` tensor order and keeps, for every tensor, the
//! leading rows/columns that survive at the reduced widths
//! ([`extract_slimmed_weights`]). The sliced sub-model's config comes from
//! [`derive_params_for_channels`]: `channels` takes the selected count,
//! `bottleneck` scales proportionally (`max(1, bottleneck * new / full)`,
//! or `= channels` when the layer1x1 is inactive), interior array
//! boundaries follow their neighbors (`input_size` of array i>0 = target of
//! array i-1; `head_size` of array i<last = target of array i+1), and
//! everything else (condition_size, head1x1.out_channels, kernel sizes,
//! dilations, activations) stays fixed.
//!
//! v1 (todo #1112) always selects the FULL size at load time; with that
//! selection the slice is the whole packed vector (the reference takes the
//! `is_full_size` fast path in `_create_wavenet_for_channels`), so the full
//! weights flow to construction bit-identically. The loader still runs the
//! full-size walk once as its layout/length verification: it consumes the
//! packed vector tensor-by-tensor (head_scale included) and rejects any
//! mismatch with a clear error. The extraction walk and the config
//! derivation are implemented and tested for smaller sizes too, so runtime
//! A2-Lite selection (follow-on todo) is a size-parameter change, not a
//! rewrite. Everything here runs at load time, off the audio thread.

use super::params::{GatingMode, LayerArrayParams};

/// The normalized size that selects every array's full channel count
/// (`SetSlimmableSize(1.0)` in the reference; the load-time default).
pub const FULL_SIZE: f64 = 1.0;

/// Map a normalized size in [0.0, 1.0] to one channel count per layer array
/// (reference `_get_channels_for_slimmable_size`): slimmable arrays pick
/// `allowed_channels[min(floor(size * len), len - 1)]`; non-slimmable
/// arrays keep their full channel count.
pub fn channels_for_size(arrays: &[LayerArrayParams], size: f64) -> Vec<usize> {
    arrays
        .iter()
        .map(|p| match &p.slimmable {
            Some(s) => ratio_to_channels(size, &s.allowed_channels),
            None => p.channels,
        })
        .collect()
}

/// Reference `ratio_to_channels`: `idx = min(floor(ratio * len), len - 1)`
/// (clamped at 0 for safety; the reference indexes unchecked).
fn ratio_to_channels(ratio: f64, allowed: &[usize]) -> usize {
    let idx = (ratio * allowed.len() as f64).floor().max(0.0) as usize;
    allowed[idx.min(allowed.len() - 1)]
}

/// True when every target equals its array's full channel count (reference
/// `is_full_size`) — the whole packed vector IS the slice then.
pub fn is_full_size(arrays: &[LayerArrayParams], target_channels: &[usize]) -> bool {
    arrays
        .iter()
        .zip(target_channels)
        .all(|(p, &t)| t == p.channels)
}

/// Sliced bottleneck for a target channel count (reference
/// `compute_slim_bottleneck`): without an active layer1x1 the bottleneck
/// must equal the channels; otherwise it scales proportionally, floored at 1.
fn slim_bottleneck(p: &LayerArrayParams, new_channels: usize) -> usize {
    if !p.layer1x1.active {
        new_channels
    } else {
        (p.bottleneck * new_channels / p.channels).max(1)
    }
}

/// Derive the sliced sub-model's layer-array config from the full config
/// (reference `modify_params_for_channels`). Only the channel-dependent
/// dims change; see the module docs for exactly which scale and which stay.
pub fn derive_params_for_channels(
    arrays: &[LayerArrayParams],
    target_channels: &[usize],
) -> Result<Vec<LayerArrayParams>, String> {
    check_targets(arrays, target_channels)?;
    let last = arrays.len() - 1;
    Ok(arrays
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let new_ch = target_channels[i];
            let mut d = p.clone();
            d.channels = new_ch;
            d.bottleneck = slim_bottleneck(p, new_ch);
            if i > 0 {
                d.input_size = target_channels[i - 1];
            }
            if i < last {
                d.head_size = target_channels[i + 1];
            }
            d
        })
        .collect())
}

/// Common target-vector sanity for the derivation and the extraction walk.
fn check_targets(arrays: &[LayerArrayParams], target_channels: &[usize]) -> Result<(), String> {
    if arrays.is_empty() {
        return Err("Slimmable WaveNet: config has no layer arrays".into());
    }
    if target_channels.len() != arrays.len() {
        return Err(format!(
            "Slimmable WaveNet: target channel counts ({}) must match the number of layer arrays ({})",
            target_channels.len(),
            arrays.len()
        ));
    }
    for (i, (p, &t)) in arrays.iter().zip(target_channels).enumerate() {
        if t < 1 || t > p.channels {
            return Err(format!(
                "Slimmable WaveNet: layer array {i}: target channel count ({t}) must be in 1..={}",
                p.channels
            ));
        }
    }
    Ok(())
}

/// Sequential cursor over the packed full-size weight vector.
struct Cursor<'a> {
    weights: &'a [f32],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn next(&mut self, ctx: &str) -> Result<f32, String> {
        let v = self.weights.get(self.pos).copied().ok_or_else(|| {
            format!(
                "Slimmable WaveNet: packed weights exhausted at {ctx} (position {}, total {})",
                self.pos,
                self.weights.len()
            )
        })?;
        self.pos += 1;
        Ok(v)
    }
}

/// Reference `extract_conv1x1` (groups = 1): the full tensor is row-major
/// `[full_out x full_in]` plus an optional `full_out` bias; keep the first
/// `slim_out` rows and first `slim_in` columns.
#[allow(clippy::too_many_arguments)]
fn extract_conv1x1(
    src: &mut Cursor,
    full_in: usize,
    full_out: usize,
    slim_in: usize,
    slim_out: usize,
    bias: bool,
    dst: &mut Vec<f32>,
    ctx: &str,
) -> Result<(), String> {
    for i in 0..full_out {
        for j in 0..full_in {
            let w = src.next(ctx)?;
            if i < slim_out && j < slim_in {
                dst.push(w);
            }
        }
    }
    if bias {
        for i in 0..full_out {
            let b = src.next(ctx)?;
            if i < slim_out {
                dst.push(b);
            }
        }
    }
    Ok(())
}

/// Reference `extract_conv1d` (groups = 1): flat `[out][in][tap]` weights
/// then an always-present `full_out` bias; keep the first `slim_out` output
/// and `slim_in` input channels (all taps of every kept pair).
#[allow(clippy::too_many_arguments)]
fn extract_conv1d(
    src: &mut Cursor,
    full_in: usize,
    full_out: usize,
    slim_in: usize,
    slim_out: usize,
    kernel_size: usize,
    dst: &mut Vec<f32>,
    ctx: &str,
) -> Result<(), String> {
    for i in 0..full_out {
        for j in 0..full_in {
            for _ in 0..kernel_size {
                let w = src.next(ctx)?;
                if i < slim_out && j < slim_in {
                    dst.push(w);
                }
            }
        }
    }
    for i in 0..full_out {
        let b = src.next(ctx)?;
        if i < slim_out {
            dst.push(b);
        }
    }
    Ok(())
}

/// Reference `copy_weights`: `n` weights pass through unchanged.
fn copy_weights(src: &mut Cursor, n: usize, dst: &mut Vec<f32>, ctx: &str) -> Result<(), String> {
    for _ in 0..n {
        let w = src.next(ctx)?;
        dst.push(w);
    }
    Ok(())
}

/// One FiLM insertion point: a biased 1x1 conv from the condition to
/// `(shift ? 2 : 1) * width` scale/shift channels. Unsliced widths copy
/// through with the grouped-aware compact count (matching the engine's
/// `read_film`); slicing follows the reference's dense `extract_conv1x1`
/// and, like the reference, is only defined for a single group.
#[allow(clippy::too_many_arguments)]
fn extract_film(
    src: &mut Cursor,
    cond: usize,
    full_width: usize,
    slim_width: usize,
    shift: bool,
    groups: usize,
    dst: &mut Vec<f32>,
    ctx: &str,
) -> Result<(), String> {
    let mult = if shift { 2 } else { 1 };
    let full_out = mult * full_width;
    if full_width == slim_width {
        return copy_weights(src, cond * full_out / groups.max(1) + full_out, dst, ctx);
    }
    if groups != 1 {
        return Err(format!(
            "Slimmable WaveNet: {ctx}: FiLM groups > 1 not supported"
        ));
    }
    extract_conv1x1(src, cond, full_out, cond, mult * slim_width, true, dst, ctx)
}

/// Extract the weight slice for `target_channels` (one count per layer
/// array) from the packed full-size vector, walking it in the reference
/// `set_weights_` tensor order (reference `extract_slimmed_weights` in
/// `NAM/wavenet/slimmable.cpp`). The whole vector must be consumed exactly
/// — the packed layout IS the full-size model layout — so a wrong-length
/// vector is rejected with a clear error, and the full-size walk doubles as
/// the load-time layout/length verification.
///
/// Deliberate deviation from the reference: the reference validates its
/// slicing prerequisites (head kernel 1, all groups 1) per array
/// unconditionally, but it also never runs the walk at full size (the
/// `is_full_size` fast path skips it). Here the guards apply only to
/// tensors that actually slice, and unsliced tensors copy through with the
/// engine's grouped-aware compact counts — so a full-size walk accepts
/// every file the engine can construct (windowed heads, grouped convs)
/// while actual slicing keeps the reference's restrictions with clear
/// errors.
pub fn extract_slimmed_weights(
    arrays: &[LayerArrayParams],
    full_weights: &[f32],
    target_channels: &[usize],
) -> Result<Vec<f32>, String> {
    check_targets(arrays, target_channels)?;

    let mut slim = Vec::new();
    let mut src = Cursor {
        weights: full_weights,
        pos: 0,
    };
    let num_arrays = arrays.len();

    for (arr, p) in arrays.iter().enumerate() {
        let full_ch = p.channels;
        let full_bn = p.bottleneck;
        let slim_ch = target_channels[arr];
        let slim_bn = slim_bottleneck(p, slim_ch);
        let cond = p.condition_size;

        // Input size: the first array keeps the original; later arrays get
        // the previous array's target channels.
        let slim_input_size = if arr == 0 {
            p.input_size
        } else {
            target_channels[arr - 1]
        };
        // Head size: intermediate arrays must match the next array's
        // channels; the last keeps the original.
        let slim_head_size = if arr < num_arrays - 1 {
            target_channels[arr + 1]
        } else {
            p.head_size
        };

        let full_head_out = if p.head1x1.active {
            p.head1x1.out_channels
        } else {
            full_bn
        };
        let slim_head_out = if p.head1x1.active {
            p.head1x1.out_channels
        } else {
            slim_bn
        };

        let ctx = |what: &str| format!("layer array {arr} {what}");

        // ---- rechannel: Conv1x1(input_size -> channels, no bias) ----
        extract_conv1x1(
            &mut src,
            p.input_size,
            full_ch,
            slim_input_size,
            slim_ch,
            false,
            &mut slim,
            &ctx("rechannel"),
        )?;

        // ---- per layer ----
        for l in 0..p.dilations.len() {
            let kernel_size = p.kernel_sizes[l];
            let gated = p.gating_modes[l] != GatingMode::None;
            let full_bg = if gated { 2 * full_bn } else { full_bn };
            let slim_bg = if gated { 2 * slim_bn } else { slim_bn };

            // conv: Conv1D(channels -> B_g, K, bias = true)
            let conv_ctx = ctx(&format!("layer {l} conv"));
            if slim_ch == full_ch && slim_bg == full_bg {
                copy_weights(
                    &mut src,
                    full_bg * full_ch * kernel_size / p.groups_input + full_bg,
                    &mut slim,
                    &conv_ctx,
                )?;
            } else {
                if p.groups_input != 1 {
                    return Err(format!(
                        "Slimmable WaveNet: layer array {arr}: groups_input > 1 not supported"
                    ));
                }
                extract_conv1d(
                    &mut src, full_ch, full_bg, slim_ch, slim_bg, kernel_size, &mut slim, &conv_ctx,
                )?;
            }

            // input_mixin: Conv1x1(condition_size -> B_g, no bias)
            let mixin_ctx = ctx(&format!("layer {l} input_mixin"));
            if slim_bg == full_bg {
                copy_weights(
                    &mut src,
                    full_bg * cond / p.groups_input_mixin,
                    &mut slim,
                    &mixin_ctx,
                )?;
            } else {
                if p.groups_input_mixin != 1 {
                    return Err(format!(
                        "Slimmable WaveNet: layer array {arr}: groups_input_mixin > 1 not supported"
                    ));
                }
                extract_conv1x1(&mut src, cond, full_bg, cond, slim_bg, false, &mut slim, &mixin_ctx)?;
            }

            // layer1x1 (optional): Conv1x1(B -> C, bias = true)
            if p.layer1x1.active {
                let l1_ctx = ctx(&format!("layer {l} layer1x1"));
                if slim_ch == full_ch && slim_bn == full_bn {
                    copy_weights(
                        &mut src,
                        full_ch * full_bn / p.layer1x1.groups + full_ch,
                        &mut slim,
                        &l1_ctx,
                    )?;
                } else {
                    if p.layer1x1.groups != 1 {
                        return Err(format!(
                            "Slimmable WaveNet: layer array {arr}: layer1x1 groups > 1 not supported"
                        ));
                    }
                    extract_conv1x1(
                        &mut src, full_bn, full_ch, slim_bn, slim_ch, true, &mut slim, &l1_ctx,
                    )?;
                }
            }

            // head1x1 (optional): Conv1x1(B -> head1x1_out, bias = true);
            // its out_channels are channel-independent.
            if p.head1x1.active {
                let h1_ctx = ctx(&format!("layer {l} head1x1"));
                let h1_out = p.head1x1.out_channels;
                if slim_bn == full_bn {
                    copy_weights(
                        &mut src,
                        h1_out * full_bn / p.head1x1.groups + h1_out,
                        &mut slim,
                        &h1_ctx,
                    )?;
                } else {
                    if p.head1x1.groups != 1 {
                        return Err(format!(
                            "Slimmable WaveNet: layer array {arr}: head1x1 groups > 1 not supported"
                        ));
                    }
                    extract_conv1x1(
                        &mut src, full_bn, h1_out, slim_bn, h1_out, true, &mut slim, &h1_ctx,
                    )?;
                }
            }

            // ---- FiLM objects (8, in set_weights_ order; widths per the
            // reference Layer ctor) ----
            let f = &p.conv_pre_film;
            if f.active {
                extract_film(
                    &mut src, cond, full_ch, slim_ch, f.shift, f.groups, &mut slim,
                    &ctx(&format!("layer {l} conv_pre_film")),
                )?;
            }
            let f = &p.conv_post_film;
            if f.active {
                extract_film(
                    &mut src, cond, full_bg, slim_bg, f.shift, f.groups, &mut slim,
                    &ctx(&format!("layer {l} conv_post_film")),
                )?;
            }
            // input_mixin_pre_film is condition-sized: never slices.
            let f = &p.input_mixin_pre_film;
            if f.active {
                extract_film(
                    &mut src, cond, cond, cond, f.shift, f.groups, &mut slim,
                    &ctx(&format!("layer {l} input_mixin_pre_film")),
                )?;
            }
            let f = &p.input_mixin_post_film;
            if f.active {
                extract_film(
                    &mut src, cond, full_bg, slim_bg, f.shift, f.groups, &mut slim,
                    &ctx(&format!("layer {l} input_mixin_post_film")),
                )?;
            }
            let f = &p.activation_pre_film;
            if f.active {
                extract_film(
                    &mut src, cond, full_bg, slim_bg, f.shift, f.groups, &mut slim,
                    &ctx(&format!("layer {l} activation_pre_film")),
                )?;
            }
            let f = &p.activation_post_film;
            if f.active {
                extract_film(
                    &mut src, cond, full_bn, slim_bn, f.shift, f.groups, &mut slim,
                    &ctx(&format!("layer {l} activation_post_film")),
                )?;
            }
            let f = &p.layer1x1_post_film;
            if f.active && p.layer1x1.active {
                extract_film(
                    &mut src, cond, full_ch, slim_ch, f.shift, f.groups, &mut slim,
                    &ctx(&format!("layer {l} layer1x1_post_film")),
                )?;
            }
            // head1x1_post_film modulates head1x1.out_channels: never slices.
            let f = &p.head1x1_post_film;
            if f.active && p.head1x1.active {
                let width = p.head1x1.out_channels;
                extract_film(
                    &mut src, cond, width, width, f.shift, f.groups, &mut slim,
                    &ctx(&format!("layer {l} head1x1_post_film")),
                )?;
            }
        }

        // ---- head_rechannel: causal conv (kernel head_kernel_size,
        // usually 1) from the head output to head_size, bias = head_bias ----
        let hr_ctx = ctx("head_rechannel");
        if slim_head_size == p.head_size && slim_head_out == full_head_out {
            let bias = if p.head_bias { p.head_size } else { 0 };
            copy_weights(
                &mut src,
                p.head_size * full_head_out * p.head_kernel_size + bias,
                &mut slim,
                &hr_ctx,
            )?;
        } else {
            if p.head_kernel_size != 1 {
                return Err(format!(
                    "Slimmable WaveNet: layer array {arr}: head rechannel kernel_size must be 1 (slimming with head kernel_size > 1 is not implemented)"
                ));
            }
            extract_conv1x1(
                &mut src,
                full_head_out,
                p.head_size,
                slim_head_out,
                slim_head_size,
                p.head_bias,
                &mut slim,
                &hr_ctx,
            )?;
        }
    }

    // head_scale: one float, copied as-is.
    copy_weights(&mut src, 1, &mut slim, "head_scale")?;

    let leftover = full_weights.len() - src.pos;
    if leftover > 0 {
        return Err(format!(
            "Slimmable WaveNet: {leftover} unused weights after the slice extraction walk (packed vector doesn't match the full-size layout)"
        ));
    }
    Ok(slim)
}
