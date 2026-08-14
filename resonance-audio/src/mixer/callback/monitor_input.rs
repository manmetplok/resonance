//! This callback's read of the live-input monitor ring.

use ringbuf::traits::{Consumer, Observer};
use std::sync::atomic::Ordering;

use crate::mixer::monitor::{monitor_catchup_skip, monitor_read_len};

use super::context::{CallbackInputs, CallbackScratch, MonitorRead};

/// Read monitor input into `scratch.monitor_temp` with a jitter margin,
/// skipping stale data so monitoring latency stays at ~1 buffer period.
///
/// The monitor stream carries raw interleaved multi-channel data (one
/// `input_channels` block per frame), so every sample count scales by the
/// current input channel count — and every skip and read is a whole number
/// of frames, because a sub-frame one would permanently rotate the channel
/// interleave.
pub(super) fn read_monitor_input(
    inputs: &CallbackInputs<'_>,
    scratch: &mut CallbackScratch<'_>,
    frames: usize,
) -> MonitorRead {
    let shared = inputs.shared;
    let input_channels = shared.input_channels.load(Ordering::Relaxed) as usize;
    let frame_stride = input_channels.max(1);
    let needed = frames * frame_stride;

    let available = scratch.monitor_cons.occupied_len();
    let catchup = monitor_catchup_skip(available, needed, inputs.quantum, frame_stride);
    if catchup > 0 {
        scratch.monitor_cons.skip(catchup);
    }
    // Native backend: drain the sticky post-startup quantum once the
    // backlog has been stably above `needed` (doc #260 finding #12) —
    // the margin skip above still bounds transients on every path.
    let extra = scratch.monitor_drain.excess_drain(
        scratch.monitor_cons.occupied_len(),
        needed,
        frame_stride,
    );
    if extra > 0 {
        scratch.monitor_cons.skip(extra);
    }
    let to_read = monitor_read_len(needed, scratch.monitor_cons.occupied_len(), frame_stride);
    let monitor_samples = scratch
        .monitor_cons
        .pop_slice(&mut scratch.monitor_temp[..to_read]);
    let monitor_frames = monitor_samples / frame_stride;

    // A short read while monitoring is a quantum of dropped live input
    // (the input stream's push for this cycle hadn't landed) — audible
    // as a click in the monitored signal even though every stream met
    // its graph deadline. Counted for the cycle-load report, and fed
    // back to the drain so a post-drain shortfall re-establishes the
    // standing margin for the rest of the session.
    if monitor_frames < frames && input_channels > 0 && shared.monitoring.load(Ordering::Relaxed) {
        shared
            .monitor_shortfall_cycles
            .fetch_add(1, Ordering::Relaxed);
        scratch.monitor_drain.note_shortfall();
    }

    MonitorRead {
        frames: monitor_frames,
        input_channels,
        frame_stride,
    }
}
