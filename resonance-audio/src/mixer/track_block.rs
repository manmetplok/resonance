//! Per-block timeline rendering for the live audio callback: a thin
//! wrapper over [`render_block`] with the [`RenderStrategy::Live`] policy
//! (non-blocking plugin locks with the MIDI stash fallback, transport
//! latching, monitor-input mixing, gain ramps, and VU peak metering).
//!
//! Called once per buffer in the no-seam path, twice (head + tail) when a
//! buffer crosses a loop boundary — see [`super::callback::seam`].
//! Allocation-free.

use super::common::TransportSnap;
use super::render_core::{render_block, BlockInputs, BlockScratch, RenderStrategy};

/// What the live strategy needs on top of the shared block parameters:
/// this sub-block's slice of the callback's monitor input, and the
/// transport event to latch onto every plugin.
pub(crate) struct LiveBlock<'a> {
    /// Interleaved live input for this sub-block. Monitor audio is
    /// timeline-independent — it streams linearly across the full callback
    /// — so this is the *next* slice of the callback's input, not one
    /// keyed to `inputs.playhead`.
    pub(crate) monitor_temp: &'a [f32],
    pub(crate) monitor_frames: usize,
    pub(crate) input_channels: usize,
    pub(crate) transport_snap: Option<TransportSnap>,
}

/// Render one contiguous timeline sub-block into a slice of the output.
/// Separated from the callback so that a buffer which crosses the loop
/// seam can be rendered as two sub-blocks (pre-wrap and post-wrap) with
/// different `playhead` values, giving sample-accurate cycle playback.
///
/// The caller is responsible for:
/// - Passing `scratch.data` sliced to exactly `inputs.frames *
///   inputs.channels` samples, and `live.monitor_temp` to the
///   corresponding portion of this callback's live input.
/// - Clearing the output buffer before the first call.
/// - Running the metronome and master-volume passes once over the full
///   callback buffer afterwards.
pub(crate) fn render_timeline_block(
    inputs: BlockInputs<'_>,
    scratch: &mut BlockScratch<'_>,
    live: LiveBlock<'_>,
) {
    let strategy = RenderStrategy::Live {
        transport_snap: live.transport_snap,
        monitor_temp: live.monitor_temp,
        monitor_frames: live.monitor_frames,
        input_channels: live.input_channels,
    };
    render_block(inputs, scratch, &strategy);
}
