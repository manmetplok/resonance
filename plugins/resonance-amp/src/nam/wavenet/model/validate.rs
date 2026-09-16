//! Load-time validation: config-shape checks (before any weight is read)
//! and matvec dimension checks on the built weight matrices.

use super::super::super::parse::WaveNetConfig;
use super::super::super::{validate_grouped_matvec_dims, validate_matvec_dims};
use super::super::conv_layer::{Conv1x1, WaveNetLayer};
use super::super::head::{DenseLayer, HeadRechannel};

/// Structural config validation (reference WaveNet / LayerArray ctor
/// checks): layer-list shapes, mono input, and the stack-chaining width
/// invariants. Runs before any weights are consumed.
pub(super) fn validate_config(config: &WaveNetConfig) -> Result<(), String> {
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

    // The engine feeds one scalar sample per process call; a
    // multi-channel reference model cannot be driven correctly.
    if config.input_size != 1 {
        return Err(format!(
            "WaveNet: only mono models are supported (input_size {})",
            config.input_size
        ));
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
    Ok(())
}

/// Cap on one dilated-conv (or head) state ring, in f32 slots (frame
/// capacity x channels; the power-of-two rounding in `RingBuffer::new` at
/// most doubles it). Unlike the width fields, kernel sizes and dilations
/// are not naturally bounded by the file's weight count — a ring stores
/// history, not weights — so this is a hard cap: real NAM models stay
/// under ~2^17 (receptive field ~2048 x at most a few dozen channels),
/// leaving this ~500x headroom while keeping a hostile dilation from
/// aborting the process on a multi-TiB allocation.
const MAX_RING_FLOATS: usize = 1 << 26;

/// Bounds validation against the file's own weight count, before any
/// config-derived allocation. Every width field sizes at least one
/// per-width tensor in the weight stream, so a width beyond the whole
/// weight array can never be satisfied; a hostile file claiming such a
/// width would otherwise wrap the weight-count products in release builds
/// or abort on an absurd allocation before the underflow check could
/// fire. Real models always pass: their widths are dwarfed by their
/// weight counts.
pub(super) fn validate_bounds(config: &WaveNetConfig, total_weights: usize) -> Result<(), String> {
    let check = |what: &str, v: usize| -> Result<(), String> {
        if v > total_weights {
            return Err(format!(
                "WaveNet config: {what} ({v}) exceeds the {total_weights} weights the file provides"
            ));
        }
        Ok(())
    };
    check("head_size", config.head_size)?;
    for (hi, &hidden) in config.head.iter().enumerate() {
        check(&format!("head layer {hi} size"), hidden)?;
    }
    for (si, s) in config.stacks.iter().enumerate() {
        check(&format!("stack {si} channels"), s.channels)?;
        check(&format!("stack {si} bottleneck"), s.bottleneck)?;
        check(&format!("stack {si} condition_size"), s.condition_size)?;
        check(&format!("stack {si} head_size"), s.head_size)?;
        check(&format!("stack {si} head_kernel_size"), s.head_kernel_size)?;
        if s.head1x1.active {
            check(&format!("stack {si} head1x1 out_channels"), s.head1x1.out_channels)?;
        }
        for (li, (&ks, &dilation)) in s.kernel_sizes.iter().zip(&s.dilations).enumerate() {
            if ks == 0 {
                return Err(format!(
                    "WaveNet stack {si} layer {li}: kernel size must be >= 1"
                ));
            }
            check(&format!("stack {si} layer {li} kernel size"), ks)?;
            check_ring(
                &format!("stack {si} layer {li}"),
                ks,
                dilation,
                s.channels,
            )?;
        }
        // The head rechannel keeps a skip-frame history ring too.
        let skip_ch = if s.head1x1.active {
            s.head1x1.out_channels
        } else {
            s.bottleneck
        };
        check_ring(
            &format!("stack {si} head rechannel"),
            s.head_kernel_size.max(1),
            s.head_dilation,
            skip_ch,
        )?;
    }
    Ok(())
}

/// Bound one state ring's allocation (`(ks - 1) * dilation + 2` frames of
/// `channels` floats) with checked arithmetic; see [`MAX_RING_FLOATS`].
fn check_ring(ctx: &str, ks: usize, dilation: usize, channels: usize) -> Result<(), String> {
    let floats = (ks - 1)
        .checked_mul(dilation)
        .and_then(|f| f.checked_add(2))
        .and_then(|f| f.checked_mul(channels));
    match floats {
        Some(f) if f <= MAX_RING_FLOATS => Ok(()),
        _ => Err(format!(
            "WaveNet {ctx}: kernel size ({ks}) x dilation ({dilation}) x channels ({channels}) exceeds the supported state-buffer size"
        )),
    }
}

/// The scratch buffers used as representative in/out slices for the
/// dimension checks below (still zeroed at validation time).
pub(super) struct ScratchSlices<'a> {
    pub(super) activation: &'a [f32],
    pub(super) conv_out: &'a [f32],
    pub(super) skip: &'a [f32],
    pub(super) head_buf: &'a [f32],
    pub(super) film_ss: &'a [f32],
}

/// Validate matvec dimensions for all weight matrices at load time, using
/// the (still zeroed) scratch buffers as representative in/out slices.
pub(super) fn validate_dims(
    rechannels: &[Conv1x1],
    stacks: &[Vec<WaveNetLayer>],
    head_rechannels: &[HeadRechannel],
    head_layers: &[DenseLayer],
    scratch: &ScratchSlices<'_>,
) -> Result<(), String> {
    let ScratchSlices {
        activation: scratch_activation,
        conv_out: scratch_conv_out,
        skip: scratch_skip,
        head_buf: scratch_head_buf,
        film_ss: scratch_film_ss,
    } = scratch;
    for (si, rc) in rechannels.iter().enumerate() {
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
    Ok(())
}
