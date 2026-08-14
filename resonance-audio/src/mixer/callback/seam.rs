//! Loop-seam detection and the arrangement render it stitches together.
//!
//! When the callback reaches or crosses `loop_out`, the buffer is rendered
//! as two sub-blocks — the pre-wrap portion from the current playhead,
//! then (after an all-notes-off on instrument plugins) the post-wrap
//! portion starting from `loop_in`. That gives sample-accurate cycle
//! playback: no silent gap, and no stray audio from past `loop_out`
//! bleeding across the seam.

use std::sync::atomic::Ordering;

use crate::engine::SharedState;
use crate::mixer::common::{panic_instrument_tracks, TransportSnap};
use crate::mixer::render_core::BlockInputs;
use crate::mixer::track_block::{render_timeline_block, LiveBlock};

use super::context::{CallbackScratch, MonitorRead};

/// A loop boundary inside this buffer: `head_frames` play from the current
/// playhead up to `loop_out`, then `tail_frames` play from `loop_in`.
#[derive(Clone, Copy)]
pub(super) struct Seam {
    pub(super) head_frames: usize,
    pub(super) tail_frames: usize,
    pub(super) loop_in: u64,
}

impl Seam {
    /// The metronome pass's view of the split: it maps output frames onto
    /// two timeline ranges the same way.
    pub(super) fn as_ranges(self) -> (usize, usize, u64) {
        (self.head_frames, self.tail_frames, self.loop_in)
    }
}

/// Detect a loop seam inside this buffer.
///
/// The `>=` on the end-of-block check is load-bearing: when the buffer
/// size divides the loop length exactly (common with small pro-audio
/// quanta like 128 frames), a strict `>` would miss the seam every time —
/// the block would end exactly on `loop_out` and the next block would
/// start past it, failing the `playhead < hi` test. With `>=`, that
/// aligned case renders the full block as `head` and sets `tail = 0`,
/// snapping the playhead back to `loop_in` for the next buffer.
pub(super) fn detect(shared: &SharedState, playhead: u64, frames: usize) -> Option<Seam> {
    if !shared.loop_enabled.load(Ordering::Relaxed) {
        return None;
    }
    let lo = shared.loop_in.load(Ordering::Relaxed);
    let hi = shared.loop_out.load(Ordering::Relaxed);
    if hi > lo && playhead < hi && playhead + frames as u64 >= hi {
        let head_frames = (hi - playhead) as usize;
        Some(Seam {
            head_frames,
            tail_frames: frames - head_frames,
            loop_in: lo,
        })
    } else {
        None
    }
}

/// Render the arrangement for this buffer — one sub-block, or two around
/// the seam — and return the playhead the next buffer starts on.
///
/// `inputs` describes the whole buffer; each sub-block is the same
/// [`BlockInputs`] with its own `playhead` / `frames` (it is `Copy`, so
/// that costs nothing). Monitor input is timeline-independent: it streams
/// linearly across the full callback, so each sub-block takes the next
/// slice of it rather than one keyed to its playhead.
pub(super) fn render_arrangement(
    inputs: BlockInputs<'_>,
    scratch: &mut CallbackScratch<'_>,
    transport: Option<TransportSnap>,
    monitor: MonitorRead,
    seam: Option<Seam>,
) -> u64 {
    let channels = inputs.channels;
    let stride = monitor.frame_stride;
    let Some(seam) = seam else {
        render_sub(
            inputs,
            scratch,
            0..inputs.frames * channels,
            0..monitor.frames * stride,
            monitor.frames,
            monitor.input_channels,
            transport,
        );
        return inputs.playhead + inputs.frames as u64;
    };

    // ---- Pre-wrap sub-block (plays to `loop_out`) -------------------------
    let head_monitor_frames = monitor.frames.min(seam.head_frames);
    render_sub(
        BlockInputs {
            frames: seam.head_frames,
            ..inputs
        },
        scratch,
        0..seam.head_frames * channels,
        0..head_monitor_frames * stride,
        head_monitor_frames,
        monitor.input_channels,
        transport,
    );

    // Flush instrument voices at the seam.
    panic_instrument_tracks(inputs.tracks, inputs.plugins, scratch.midi_stash);

    // ---- Post-wrap sub-block (plays from `loop_in`) -----------------------
    let tail_monitor_start = head_monitor_frames * stride;
    let tail_monitor_avail = monitor.frames.saturating_sub(head_monitor_frames);
    let tail_monitor_frames = tail_monitor_avail.min(seam.tail_frames);
    render_sub(
        BlockInputs {
            playhead: seam.loop_in,
            frames: seam.tail_frames,
            ..inputs
        },
        scratch,
        seam.head_frames * channels..(seam.head_frames + seam.tail_frames) * channels,
        tail_monitor_start..tail_monitor_start + tail_monitor_frames * stride,
        tail_monitor_frames,
        monitor.input_channels,
        transport,
    );

    seam.loop_in + seam.tail_frames as u64
}

/// Render one timeline sub-block over the `out` slice of the output, with
/// the `mon` slice of this callback's live input.
fn render_sub(
    inputs: BlockInputs<'_>,
    scratch: &mut CallbackScratch<'_>,
    out: std::ops::Range<usize>,
    mon: std::ops::Range<usize>,
    monitor_frames: usize,
    input_channels: usize,
    transport: Option<TransportSnap>,
) {
    let (mut block, midi_stash, monitor_temp) = scratch.split_block(out, mon);
    render_timeline_block(
        inputs,
        &mut block,
        midi_stash,
        LiveBlock {
            monitor_temp,
            monitor_frames,
            input_channels,
            transport_snap: transport,
        },
    );
}
