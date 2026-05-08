//! Master-bus passes: insert FX chain over the post-bus-sum buffer,
//! then volume + hard clip + per-channel peak metering. Both run on
//! the audio thread once per callback (after the per-track / per-bus
//! work) and are intentionally allocation-free.

use std::sync::atomic::Ordering;

use indexmap::IndexMap;

use crate::clap_host::SyncClapInstance;
use crate::engine::SharedState;
use crate::types::*;

use super::common::latch_transport;

/// Run the master FX insert chain over the interleaved `data` buffer in
/// place. De-interleaves into the borrowed `scratch_l`/`scratch_r` pair
/// (the per-track mix buffers are free at this point in `mix_audio`),
/// processes each plugin in order, then re-interleaves back into `data`.
/// Silently no-ops when the chain is empty, the read lock is contended,
/// or a plugin's instance is momentarily locked by the control thread.
#[inline]
pub(super) fn apply_master_fx_chain(
    data: &mut [f32],
    channels: usize,
    master: &parking_lot::RwLock<MasterBus>,
    plugins_guard: &IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>,
    scratch_l: &mut [f32],
    scratch_r: &mut [f32],
    transport_snap: Option<(f64, u16, u16, bool, f64)>,
) {
    let Some(master_guard) = master.try_read() else {
        return;
    };
    if master_guard.plugin_ids.is_empty() {
        return;
    }
    let output_frames = data.len() / channels;
    let frames = output_frames.min(scratch_l.len()).min(scratch_r.len());
    if frames == 0 {
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
    for &plugin_id in &master_guard.plugin_ids {
        if let Some(mutex) = plugins_guard.get(&plugin_id) {
            if let Some(mut inst) = mutex.try_lock() {
                latch_transport(&mut inst, transport_snap);
                inst.0
                    .process(&mut scratch_l[..frames], &mut scratch_r[..frames], frames);
            }
        }
    }
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

/// Apply master volume, hard clip at [-1.0, 1.0], and update master peak level atomics.
#[inline]
pub(super) fn apply_master_volume_and_peaks(data: &mut [f32], channels: usize, shared: &SharedState) {
    let master_vol = f32::from_bits(shared.master_volume_bits.load(Ordering::Relaxed));
    let output_frames = data.len() / channels;
    let mut master_peak_l = 0.0f32;
    let mut master_peak_r = 0.0f32;
    // The per-frame `if channels >= 2` branch was preventing
    // auto-vectorisation; `channels` is loop-invariant so we hoist
    // the branch into two specialised loops. With buffer sizes of
    // ~1024 frames per callback this is a noticeable win on the
    // master pass because the optimiser can now SIMD the multiply +
    // clamp + abs.
    if channels >= 2 {
        for f in 0..output_frames {
            let idx = f * channels;
            data[idx] = (data[idx] * master_vol).clamp(-1.0, 1.0);
            data[idx + 1] = (data[idx + 1] * master_vol).clamp(-1.0, 1.0);
            master_peak_l = master_peak_l.max(data[idx].abs());
            master_peak_r = master_peak_r.max(data[idx + 1].abs());
        }
    } else {
        for f in 0..output_frames {
            let idx = f * channels;
            data[idx] = (data[idx] * master_vol).clamp(-1.0, 1.0);
            master_peak_l = master_peak_l.max(data[idx].abs());
        }
        master_peak_r = master_peak_l;
    }
    // SAFETY of fetch_max on bit-punned f32: IEEE 754 binary32 bit
    // ordering matches u32 ordering for non-negative values. This is
    // correct here because peak values are always >= 0 (.abs() is
    // applied before this point). Negative or NaN values would break
    // the ordering invariant.
    shared
        .master_peak_l_bits
        .fetch_max(master_peak_l.to_bits(), Ordering::Relaxed);
    shared
        .master_peak_r_bits
        .fetch_max(master_peak_r.to_bits(), Ordering::Relaxed);
}
