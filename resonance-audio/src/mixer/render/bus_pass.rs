//! The per-bus phase of a render block: aux sends in, insert-FX chain over
//! the accumulated summing buffer, sidechain key capture, bus-stage delay
//! compensation and meters, then the sum into master.
//!
//! Busses run **level by level** (realtime-multithreading.md §4.5). A bus's
//! inputs are the track routes (all summed by the track pass) plus aux
//! sends from lower-indexed busses, so level 0 is every bus no bus sends
//! into, and level `k` is fed only by levels below `k`. The busses of one
//! level are independent jobs and run on the render pool; each writes only
//! its own buffer and record.
//!
//! Bit-identity with the old single index-order loop rests on two orders:
//!
//! - into a bus's buffer, sends arrive in source-index order: the bus
//!   *pulls* them when its job starts, from sources that all finished at an
//!   earlier level ([`pull_bus_aux_sends`]);
//! - into master, busses are summed in index order, by one serial
//!   reduction after the last level.

use resonance_common::AutomationTarget;

use crate::limits::MAX_BUSSES;
use crate::mixer::automation_apply::{auto_gain_ramp, auto_muted};
use crate::mixer::common::{ramped_stereo_peaks, sum_to_output};
use crate::render_pool::{RunStats, WorkerBufs};
use crate::types::*;

use super::context::{key_consumed, run_fx_chain, BlockCtx, BlockScratch, GainRamp};
use super::routing::pull_bus_aux_sends;
use super::slots::SlotCells;
use super::strategy::RenderStrategy;

/// Run every active bus, level by level, then sum them into master in
/// index order.
pub(crate) fn render_bus_pass(
    ctx: &BlockCtx<'_>,
    scratch: &mut BlockScratch<'_>,
    strategy: &RenderStrategy<'_>,
) {
    let n = ctx
        .inputs
        .active_busses
        .min(scratch.bus_bufs.len())
        .min(MAX_BUSSES);
    let busses = ctx.inputs.busses;
    let sidechain: &SidechainTaps = scratch.sidechain;

    // Decided before any bus job runs: they read other busses' live state
    // (last gains, bypass fades), which those busses' jobs advance.
    let mut keyed = [false; MAX_BUSSES];
    for (idx, bus) in busses.values().enumerate().take(n) {
        keyed[idx] = key_consumed(ctx, sidechain, strategy, SendSource::Bus(bus.id));
    }
    let mut levels = [0u8; MAX_BUSSES];
    let max_level = assign_levels(ctx, &mut levels[..n]);

    // Each bus's fader ramp, set by its job when it reaches the mix.
    let mut routes: [Option<GainRamp>; MAX_BUSSES] = [None; MAX_BUSSES];
    {
        let bufs = SlotCells::new(&mut scratch.bus_bufs[..n]);
        let route_cells = SlotCells::new(&mut routes[..n]);
        let mut caller = WorkerBufs {
            port_scratch: &mut *scratch.port_scratch,
            note_event_buf: &mut *scratch.note_event_buf,
            fx_dry: &mut *scratch.fx_dry,
        };
        let serial_only = crate::clap_host::any_serial_only_plugin();
        let mut order = [0u8; MAX_BUSSES];
        for level in 0..=max_level {
            // This level's busses; any holding a serial-only plugin first,
            // as caller-only jobs.
            let mut count = 0;
            let mut caller_only = 0;
            for (idx, bus) in busses.values().enumerate().take(n) {
                if levels[idx] != level {
                    continue;
                }
                order[count] = idx as u8;
                if serial_only && holds_serial_only_plugin(ctx, bus) {
                    order.swap(caller_only, count);
                    caller_only += 1;
                }
                count += 1;
            }
            let order = &order[..count];
            let job = |k: usize, wb: &mut WorkerBufs<'_>| {
                let idx = order[k] as usize;
                let Some((_, bus)) = busses.get_index(idx) else {
                    return;
                };
                render_one_bus(
                    BusJob {
                        bus,
                        idx,
                        keyed: keyed[idx],
                        bufs: &bufs,
                        routes: &route_cells,
                        sidechain,
                    },
                    ctx,
                    wb,
                    strategy,
                );
            };
            let _: RunStats = match scratch.pool {
                Some(pool) => pool.run(&mut caller, caller_only, count, &job),
                None => {
                    for k in 0..count {
                        job(k, &mut caller);
                    }
                    RunStats::default()
                }
            };
        }
    }

    // The ordered reduction: every bus that reached the mix, summed into
    // master in index order, as the serial loop did.
    for (idx, (buf_l, buf_r)) in scratch.bus_bufs.iter().enumerate().take(n) {
        let Some((gain_l, gain_r)) = routes[idx] else {
            continue;
        };
        sum_to_output(
            scratch.data,
            ctx.inputs.channels,
            ctx.inputs.frames,
            buf_l,
            buf_r,
            gain_l,
            gain_r,
        );
    }
}

/// Give each of the first `levels.len()` busses its level: one more than
/// the highest level of any lower-indexed bus with an enabled aux send
/// into it. Every enabled send counts, whether or not its source reaches
/// the mix this block — a conservative order is still a correct one.
/// Returns the highest level.
fn assign_levels(ctx: &BlockCtx<'_>, levels: &mut [u8]) -> u8 {
    let n = levels.len();
    let busses = ctx.inputs.busses;
    let mut max_level = 0;
    for (src_idx, src) in busses.values().enumerate().take(n) {
        let level = levels[src_idx];
        max_level = max_level.max(level);
        for send in ctx.inputs.aux_sends {
            if !send.enabled || send.source != SendSource::Bus(src.id) {
                continue;
            }
            if let Some(dst_idx) = busses.get_index_of(&send.dest) {
                if dst_idx > src_idx && dst_idx < n {
                    levels[dst_idx] = levels[dst_idx].max(level + 1);
                }
            }
        }
    }
    max_level
}

/// Whether `bus`'s chain holds a serial-only plugin (realtime-
/// multithreading.md §4.6).
fn holds_serial_only_plugin(ctx: &BlockCtx<'_>, bus: &Bus) -> bool {
    bus.plugin_ids.iter().any(|id| {
        ctx.inputs
            .plugins
            .get(id)
            .is_some_and(|slot| slot.serial_only)
    })
}

/// One bus job's view: its bus, and the shared per-bus state it may touch.
struct BusJob<'j, 'b> {
    bus: &'j Bus,
    idx: usize,
    /// A sidechain key is tapped from this bus by a consumer that reads
    /// it — decided before the level ran.
    keyed: bool,
    bufs: &'j SlotCells<'b, (Vec<f32>, Vec<f32>)>,
    routes: &'j SlotCells<'b, Option<GainRamp>>,
    sidechain: &'j SidechainTaps,
}

fn render_one_bus(
    job: BusJob<'_, '_>,
    ctx: &BlockCtx<'_>,
    wb: &mut WorkerBufs<'_>,
    strategy: &RenderStrategy<'_>,
) {
    let BusJob {
        bus,
        idx: bus_idx,
        keyed,
        bufs,
        routes,
        sidechain,
    } = job;
    let frames = ctx.inputs.frames;

    // SAFETY: bus `bus_idx` is this job's own; the sources it pulls from
    // are lower-indexed busses of earlier levels, finished before this
    // level started and written by nobody while it runs.
    let buf = unsafe { bufs.get(bus_idx) };
    pull_bus_aux_sends(
        bus_idx,
        buf,
        |src| unsafe { routes.get_ref(src) }.map(|gains| (unsafe { bufs.get_ref(src) }, gains)),
        ctx,
    );

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
            None if keyed => (((0.0, 0.0), (0.0, 0.0)), true),
            None => return,
        };

    let (bus_buf_l, bus_buf_r) = (buf.0.as_mut_slice(), buf.1.as_mut_slice());

    // Process the bus plugin chain in place over the accumulated buffer
    // (skipped once the bus's bypass fade has fully landed).
    run_fx_chain(
        bus.plugin_ids.iter().copied(),
        bus.fx_bypass(),
        ctx,
        sidechain,
        (&mut *bus_buf_l, &mut *bus_buf_r),
        wb.fx_dry,
        strategy,
    );

    // Capture this bus post-FX and pre-fader for anything keying off
    // it — the bus half of the track tap. `SendSource::Bus` has been a
    // legal route target all along (`sidechain::from_bus`), so without
    // this a bus-sourced route resolved to a slot that was never written
    // and silently keyed off the plugin's own input.
    if sidechain.is_tapped(bus_tap) {
        // SAFETY: only this job renders — so captures — this bus.
        unsafe {
            sidechain.capture_shared(bus_tap, &bus_buf_l[..frames], &bus_buf_r[..frames], frames);
        }
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
    ctx.inputs.latency_comp.apply_bus(
        bus.id,
        &mut bus_buf_l[..frames],
        &mut bus_buf_r[..frames],
        ctx.inputs.playhead,
    );

    // Compute post-fader peaks (live only).
    if strategy.is_live() {
        let (bus_peak_l, bus_peak_r) =
            ramped_stereo_peaks(bus_buf_l, bus_buf_r, frames, bus_gain_l, bus_gain_r);
        bus.update_peak_l(bus_peak_l);
        bus.update_peak_r(bus_peak_r);
        bus.set_last_gains(bus_gain_l.1, bus_gain_r.1);
        bus.mark_rendered_once();
    }

    // Leave the sum into master to the ordered reduction, and the aux
    // sends to the busses they feed, which pull them.
    // SAFETY: this bus's own record.
    *unsafe { routes.get(bus_idx) } = Some((bus_gain_l, bus_gain_r));
}
