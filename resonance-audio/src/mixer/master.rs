//! Master-bus passes: insert FX chain over the post-bus-sum buffer,
//! then volume + hard clip + per-channel peak metering. Both run on
//! the audio thread once per callback (after the per-track / per-bus
//! work) and are intentionally allocation-free.

use std::sync::atomic::Ordering;


use crate::bypass::{run_faded, BypassFade, FadeStage, FxDryScratch};
use crate::clap_host::{PluginMap, StereoBufMut};
use crate::engine::SharedState;
use crate::types::*;

use super::common::{latch_transport, TransportSnap};

/// Run the master FX insert chain over the interleaved `data` buffer in
/// place. De-interleaves into the borrowed `scratch_l`/`scratch_r` pair
/// (the per-track mix buffers are free at this point in `mix_audio`),
/// processes each plugin in order, then re-interleaves back into `data`.
/// Silently no-ops when the chain is empty, the read lock is contended,
/// or a plugin's instance is momentarily locked by the control thread.
///
/// `chain` is the master's own bypass and each slot carries its own, both
/// crossfaded exactly as in the per-track chains (`crate::bypass`) — so
/// A/B-ing a mastering chain mid-playback fades rather than switches. The
/// whole pass, de-interleave included, is skipped once the master bypass
/// has settled.
#[inline]
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_master_fx_chain(
    data: &mut [f32],
    channels: usize,
    master: &parking_lot::RwLock<MasterBus>,
    plugins_guard: &PluginMap,
    scratch_l: &mut [f32],
    scratch_r: &mut [f32],
    fx_dry: &mut FxDryScratch,
    transport_snap: Option<TransportSnap>,
    sidechain_routes: &[SidechainRoute],
    sidechain: &SidechainTaps,
    chain: &BypassFade,
    sample_rate: u32,
) {
    let output_frames = data.len() / channels;
    let frames = output_frames.min(scratch_l.len()).min(scratch_r.len());
    if frames == 0 {
        return;
    }
    // The master pass only ever runs live; an offline export drives its
    // own copy of this chain in `bounce::render`.
    let chain_stage = chain.stage(sample_rate, frames, true);
    if chain_stage == FadeStage::Dry {
        return;
    }
    let Some(master_guard) = master.try_read() else {
        return;
    };
    if master_guard.plugin_ids.is_empty() {
        return;
    }
    // De-interleave into scratch pair. Mono output shares L across R so
    // plugins see a proper stereo input.
    if channels >= 2 {
        for f in 0..frames {
            let idx = f * channels;
            scratch_l[f] = data[idx];
            scratch_r[f] = data[idx + 1];
        }
    } else {
        for f in 0..frames {
            let s = data[f * channels];
            scratch_l[f] = s;
            scratch_r[f] = s;
        }
    }
    let (chain_dry, slot_dry) = fx_dry.split();
    run_faded(
        chain_stage,
        frames,
        (&mut *scratch_l, &mut *scratch_r),
        chain_dry,
        |buf_l, buf_r| {
            let mut ran = false;
            for &plugin_id in &master_guard.plugin_ids {
                let Some(slot) = plugins_guard.get(&plugin_id) else {
                    continue;
                };
                let slot_stage = slot.stage(sample_rate, frames, true);
                if slot_stage == FadeStage::Dry {
                    continue;
                }
                let Some(mut inst) = slot.try_lock() else {
                    continue;
                };
                latch_transport(&mut inst, transport_snap);
                slot.sync_own_bypass(&mut inst.0);
                // A master-bus ducker keyed off the kick is the classic
                // "pumping mix" move, so the master chain honours key
                // routes like every other chain (ba doc #275 P0).
                let key = sidechain.key_for(sidechain_routes, plugin_id);
                ran |= run_faded(
                    slot_stage,
                    frames,
                    (&mut *buf_l, &mut *buf_r),
                    (&mut *slot_dry.0, &mut *slot_dry.1),
                    |l, r| {
                        let mut outs = [StereoBufMut {
                            left: &mut l[..frames],
                            right: &mut r[..frames],
                        }];
                        inst.0.process_multi_with_key(&mut outs, key, frames);
                        true
                    },
                );
            }
            ran
        },
    );
    // Interleave back into data.
    if channels >= 2 {
        for f in 0..frames {
            let idx = f * channels;
            data[idx] = scratch_l[f];
            data[idx + 1] = scratch_r[f];
        }
    } else {
        for f in 0..frames {
            data[f * channels] = 0.5 * (scratch_l[f] + scratch_r[f]);
        }
    }
}

/// Replace non-finite samples (NaN, ±Inf) with 0.0 before hard-clipping.
/// Keeps a misbehaving plugin's output from poisoning cpal and the
/// bit-punned peak meters.
#[inline(always)]
fn sanitize_sample(v: f32) -> f32 {
    if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 }
}

/// Apply master volume, hard clip at [-1.0, 1.0], and update master peak level atomics.
/// Volume is ramped per sample from the previous block's value
/// (`master_last_volume_bits`) so fader drags don't zipper. When
/// `auto_volume` is `Some` a master-gain automation lane overrides the
/// static fader for this block (the static `master_volume_bits` atomic is
/// left untouched, so the user's fader value is preserved); the ramp from
/// `master_last_volume_bits` still applies, keeping the sweep click-free.
#[inline]
pub(super) fn apply_master_volume_and_peaks(
    data: &mut [f32],
    channels: usize,
    shared: &SharedState,
    auto_volume: Option<f32>,
) {
    let master_vol = auto_volume
        .unwrap_or_else(|| f32::from_bits(shared.master_volume_bits.load(Ordering::Relaxed)));
    let last_vol = f32::from_bits(shared.master_last_volume_bits.load(Ordering::Relaxed));
    let output_frames = data.len() / channels;
    if output_frames == 0 {
        return;
    }
    let vol_step = (master_vol - last_vol) / output_frames as f32;
    let mut master_peak_l = 0.0f32;
    let mut master_peak_r = 0.0f32;
    // The per-frame `if channels >= 2` branch was preventing
    // auto-vectorisation; `channels` is loop-invariant so we hoist
    // the branch into two specialised loops. With buffer sizes of
    // ~1024 frames per callback this is a noticeable win on the
    // master pass because the optimiser can now SIMD the multiply +
    // clamp + abs.
    //
    // NaN/Inf guard: a misbehaving plugin upstream can emit a
    // non-finite sample. `f32::clamp(NaN)` returns NaN; without this
    // guard the value would be written straight to cpal (audible pop
    // / silenced output) and would also corrupt the peak-meter
    // bit-punning invariant that requires non-negative non-NaN.
    if channels >= 2 {
        for f in 0..output_frames {
            let idx = f * channels;
            let vol = last_vol + vol_step * (f + 1) as f32;
            let l = sanitize_sample(data[idx] * vol);
            let r = sanitize_sample(data[idx + 1] * vol);
            data[idx] = l;
            data[idx + 1] = r;
            master_peak_l = master_peak_l.max(l.abs());
            master_peak_r = master_peak_r.max(r.abs());
        }
    } else {
        for f in 0..output_frames {
            let idx = f * channels;
            let vol = last_vol + vol_step * (f + 1) as f32;
            let s = sanitize_sample(data[idx] * vol);
            data[idx] = s;
            master_peak_l = master_peak_l.max(s.abs());
        }
        master_peak_r = master_peak_l;
    }
    shared
        .master_last_volume_bits
        .store(master_vol.to_bits(), Ordering::Relaxed);
    // SAFETY of fetch_max on bit-punned f32: IEEE 754 binary32 bit
    // ordering matches u32 ordering for non-negative values. This is
    // correct here because peak values are always >= 0 (.abs() is
    // applied before this point). Negative or NaN values would break
    // the ordering invariant.
    //
    // `AcqRel` synchronises with the engine-thread `swap` reader in
    // `handle_poll_peaks` so the reader observes the max published by
    // this audio-callback rather than racing the in-progress block.
    shared
        .master_peak_l_bits
        .fetch_max(master_peak_l.to_bits(), Ordering::AcqRel);
    shared
        .master_peak_r_bits
        .fetch_max(master_peak_r.to_bits(), Ordering::AcqRel);
}
