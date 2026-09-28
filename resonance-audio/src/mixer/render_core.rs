//! Shared per-track render core used by both the live audio callback
//! (`track_block::render_timeline_block`) and the offline bounce
//! renderer (`engine::bounce::render::render_chunk`).
//!
//! The block structure — per-track clip/MIDI/plugin processing, the
//! multi-output instrument fan-out, sub-track routing, latency
//! compensation, and the per-bus pass — is identical on both paths.
//! What differs is captured by [`RenderStrategy`]:
//!
//! - **Live**: non-blocking `try_lock` on plugins (dropping out for one
//!   block on contention, with MIDI parked in the `MidiStash`),
//!   transport latching, monitor-input mixing, per-sample gain ramps
//!   from the last-gain atomics (with mute fade-out blocks and the
//!   silenced-instrument path that keeps NoteOffs flowing), and VU peak
//!   metering.
//! - **Bounce**: deterministic blocking locks (spin + back-off so the
//!   audio thread isn't starved), `in_filter` / `respect_mute_solo`
//!   gating, constant gains (a ramp with equal endpoints degenerates to
//!   the constant — bit-identical), and no meter or last-gain atomic
//!   writes, since a bounce may run concurrently with live playback.
//!
//! This module owns only the **order** the block's phases run in. Each
//! phase lives in [`super::render`]: the clip mix, the frozen-source
//! read, the multi-output port call, the strategy, the routing and
//! aux-send taps, the per-track pass and the per-bus pass. The block's
//! parameters travel as the two borrowed structs
//! [`BlockInputs`] (read-only project view) and [`BlockScratch`] (the
//! caller's pre-allocated buffers), so no phase needs a 24-argument
//! signature and none of them can allocate.

use super::render::bus_pass::render_bus_pass;
use super::render::context::BlockCtx;
use super::render::track_pass::render_track_pass;

pub(crate) use super::render::context::{BlockInputs, BlockScratch};
pub(crate) use super::render::strategy::{RenderStrategy, SendFilter};
pub use super::render::clips::{mix_track_clips, recorded_monitor_gate, CLIP_DECLICK_FRAMES};

/// Render one contiguous timeline block into the interleaved output:
/// walks every active track + bus, mixes audio clips, dispatches MIDI
/// events to instrument plugins, routes per-port multi-output
/// instruments through their sub-tracks, and sums into the output (or
/// per-bus summing buffer). Allocation-free.
///
/// `inputs.aux_sends` is the engine's current aux-send table (a lock-free
/// snapshot loaded once per block by the caller). For every enabled send
/// the source track/bus's signal is tapped — pre-fader (raw, send level
/// only) or post-fader (after the source's fader/pan ramp) — scaled by
/// the send level and summed into the destination return bus's summing
/// buffer, in addition to the source's normal output. Empty ⇒ the block
/// renders byte-for-byte as before, so projects without sends are
/// unaffected.
///
/// The caller is responsible for:
/// - Passing `scratch.data` sliced to exactly `frames * channels` samples
///   and cleared before the first call.
/// - Running any master FX / metronome / master-volume passes afterwards.
pub(crate) fn render_block(
    inputs: BlockInputs<'_>,
    scratch: &mut BlockScratch<'_>,
    strategy: &RenderStrategy<'_>,
) {
    let ctx = BlockCtx::new(inputs);
    let frames = ctx.inputs.frames;

    // Zero every active bus summing buffer at the start of the block so
    // tracks can accumulate into them.
    for (buf_l, buf_r) in scratch.bus_bufs.iter_mut().take(ctx.inputs.active_busses) {
        buf_l[..frames].fill(0.0);
        buf_r[..frames].fill(0.0);
    }

    // Per-track processing: (clips + monitor input) -> plugins -> volume
    // -> master, plus each multi-output instrument's sub-track fan-out.
    render_track_pass(&ctx, scratch, strategy);

    // Bus-stage equalization for master-direct signals: everything the
    // track pass summed straight into the output (`data` holds exactly
    // those contributions here — the bus pass below hasn't run yet) is
    // delayed by the shared dry line so it arrives together with
    // signals that traverse a bus chain (see `crate::latency`). No-op
    // when no bus carries latency.
    ctx.inputs.latency_comp.apply_dry(
        scratch.data,
        ctx.inputs.channels,
        frames,
        ctx.inputs.playhead,
    );

    // Per-bus processing: plugin chain, volume/pan, peaks, sum to master.
    render_bus_pass(&ctx, scratch, strategy);
}
