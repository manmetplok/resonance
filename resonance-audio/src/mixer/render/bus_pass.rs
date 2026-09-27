//! The per-bus phase of a render block: insert-FX chain over the
//! accumulated summing buffer, sidechain key capture, bus-stage delay
//! compensation, meters, the sum into master, and the bus's own aux
//! sends.

use resonance_common::AutomationTarget;

use crate::mixer::automation_apply::{auto_gain_ramp, auto_muted};
use crate::mixer::common::{ramped_stereo_peaks, sum_to_output};
use crate::types::*;

use super::context::{key_consumed, run_fx_chain, BlockCtx, BlockScratch};
use super::routing::apply_bus_aux_sends;
use super::strategy::RenderStrategy;

/// Run every active bus, in index order. Return busses are created after
/// their feeder busses, so this ordering is also the routing order: a
/// feeder's aux send lands in a higher-indexed return's buffer before
/// that return is processed.
pub(crate) fn render_bus_pass(
    ctx: &BlockCtx<'_>,
    scratch: &mut BlockScratch<'_>,
    strategy: &RenderStrategy<'_>,
) {
    for (bus_idx, bus) in ctx
        .inputs
        .busses
        .values()
        .enumerate()
        .take(ctx.inputs.active_busses)
    {
        render_one_bus(bus, bus_idx, ctx, scratch, strategy);
    }
}

fn render_one_bus(
    bus: &Bus,
    bus_idx: usize,
    ctx: &BlockCtx<'_>,
    scratch: &mut BlockScratch<'_>,
    strategy: &RenderStrategy<'_>,
) {
    let frames = ctx.inputs.frames;
    let bus_auto_gain = auto_gain_ramp(
        ctx.inputs.automation,
        AutomationTarget::BusGain(bus.id),
        AutomationTarget::BusPan(bus.id),
        bus.volume(),
        bus.pan(),
        ctx.evals.bus_start,
        ctx.evals.bus_end,
    );
    let bus_auto_mute = auto_muted(
        ctx.inputs.automation,
        AutomationTarget::BusMute(bus.id),
        ctx.evals.bus_start,
    );
    // A muted bus that keys something still runs its chain for the
    // capture, and stops there (code review MIX-05).
    let bus_tap = SendSource::Bus(bus.id);
    let ((bus_gain_l, bus_gain_r), key_only) =
        match strategy.bus_disposition(bus, bus_auto_gain, bus_auto_mute) {
            Some(gains) => (gains, false),
            None if key_consumed(ctx, scratch.sidechain, strategy, bus_tap) => {
                (((0.0, 0.0), (0.0, 0.0)), true)
            }
            None => return,
        };

    // Process the bus plugin chain in place over the accumulated buffer
    // (skipped once the bus's bypass fade has fully landed).
    {
        let (bus_buf_l, bus_buf_r) = &mut scratch.bus_bufs[bus_idx];
        let (bus_buf_l, bus_buf_r) = (bus_buf_l.as_mut_slice(), bus_buf_r.as_mut_slice());
        run_fx_chain(
            bus.plugin_ids.iter().copied(),
            bus.fx_bypass(),
            ctx,
            scratch.sidechain,
            (bus_buf_l, bus_buf_r),
            scratch.fx_dry,
            strategy,
        );
    }

    // Capture this bus post-FX and pre-fader for anything keying off
    // it — the bus half of the track tap. `SendSource::Bus` has been a
    // legal route target all along (`sidechain::from_bus`), so without
    // this a bus-sourced route resolved to a slot that was never written
    // and silently keyed off the plugin's own input.
    if scratch.sidechain.is_tapped(bus_tap) {
        let (bus_buf_l, bus_buf_r) = &scratch.bus_bufs[bus_idx];
        scratch.sidechain.capture(
            bus_tap,
            &bus_buf_l[..frames],
            &bus_buf_r[..frames],
            frames,
        );
    }

    // Captured, and not part of this stem (ba doc #277).
    if key_only || strategy.is_key_only_bus(bus.id) {
        return;
    }

    // Bus-stage equalization: pad this bus's chain up to the longest bus
    // chain, so every path through *any* bus (main output or aux send)
    // reaches master with the same bus-stage latency (see
    // `crate::latency`). Runs before peaks / fader / send taps; every
    // active bus is processed each block, so delayed tails keep flushing.
    {
        let (bus_buf_l, bus_buf_r) = &mut scratch.bus_bufs[bus_idx];
        ctx.inputs.latency_comp.apply_bus(
            bus.id,
            &mut bus_buf_l[..frames],
            &mut bus_buf_r[..frames],
            ctx.inputs.playhead,
        );
    }

    {
        let (bus_buf_l, bus_buf_r) = &scratch.bus_bufs[bus_idx];
        // Compute post-fader peaks (live only).
        if strategy.is_live() {
            let (bus_peak_l, bus_peak_r) =
                ramped_stereo_peaks(bus_buf_l, bus_buf_r, frames, bus_gain_l, bus_gain_r);
            bus.update_peak_l(bus_peak_l);
            bus.update_peak_r(bus_peak_r);
        }

        // Sum the bus output into master.
        sum_to_output(
            scratch.data,
            ctx.inputs.channels,
            frames,
            bus_buf_l,
            bus_buf_r,
            bus_gain_l,
            bus_gain_r,
        );
    }
    if strategy.is_live() {
        bus.set_last_gains(bus_gain_l.1, bus_gain_r.1);
        bus.mark_rendered_once();
    }

    apply_bus_aux_sends(bus.id, bus_idx, (bus_gain_l, bus_gain_r), ctx, scratch);
}
