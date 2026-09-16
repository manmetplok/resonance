//! Where a rendered signal goes once its fader ramp is known: the main
//! post-fader route (bus summing buffer or straight to master) and the
//! parallel aux-send taps from a track and from a bus.

use resonance_dsp::db_to_linear;

use crate::mixer::common::{sum_to_output, sum_to_stereo};
use crate::types::*;

use super::context::{BlockCtx, BlockScratch, BusBufs, GainRamp, OutputTargets};

/// Sum a post-fader stereo signal into its destination: the target bus's
/// summing buffer, or the interleaved master output.
///
/// `dest` is `None` when the caller forces the master route — freeze
/// capture, which must not run the signal through the track's bus (bus FX
/// would otherwise bake into the cache and double on playback). A
/// `TrackOutput::Bus` whose bus no longer exists (removed mid-block) or
/// which is past `active_busses` also falls back to master, so the track
/// is never silenced by a routing race.
pub(crate) fn route_post_fader(
    dest: Option<TrackOutput>,
    src: (&[f32], &[f32]),
    gains: GainRamp,
    out: OutputTargets<'_>,
    ctx: &BlockCtx<'_>,
) {
    let frames = ctx.inputs.frames;
    let (src_l, src_r) = src;
    let (gain_l, gain_r) = gains;
    let (data, bus_bufs) = out;

    let routed_to_bus = match dest {
        Some(TrackOutput::Bus(bus_id)) => ctx
            .inputs
            .busses
            .get_index_of(&bus_id)
            .filter(|idx| *idx < ctx.inputs.active_busses)
            .map(|idx| {
                let (bl, br) = &mut bus_bufs[idx];
                sum_to_stereo(bl, br, frames, src_l, src_r, gain_l, gain_r);
            })
            .is_some(),
        Some(TrackOutput::Master) | None => false,
    };
    if !routed_to_bus {
        sum_to_output(
            data,
            ctx.inputs.channels,
            frames,
            src_l,
            src_r,
            gain_l,
            gain_r,
        );
    }
}

/// Aux sends sourced from a track: tap the track buffers into each
/// destination return bus, on top of the main output routed above.
/// Post-fader follows the fader/pan/mute ramp `gains` (which ramps to
/// zero on a muted track); pre-fader takes the raw post-plugin signal
/// with the send level only. The destination's summing buffer is always
/// filled before the bus pass runs it, so a track→return send is
/// sample-correct regardless of bus ordering.
pub(crate) fn apply_track_aux_sends(
    track_id: TrackId,
    gains: GainRamp,
    ctx: &BlockCtx<'_>,
    scratch: &mut BlockScratch<'_>,
) {
    let frames = ctx.inputs.frames;
    let (gain_l, gain_r) = gains;
    for send in ctx.inputs.aux_sends {
        if !send.enabled || send.source != SendSource::Track(track_id) {
            continue;
        }
        let Some(dst_idx) = ctx
            .inputs
            .busses
            .get_index_of(&send.dest)
            .filter(|idx| *idx < ctx.inputs.active_busses)
        else {
            continue;
        };
        let send_lin = db_to_linear(send.level_db);
        let (send_gain_l, send_gain_r) = if send.pre_fader {
            ((send_lin, send_lin), (send_lin, send_lin))
        } else {
            (
                (gain_l.0 * send_lin, gain_l.1 * send_lin),
                (gain_r.0 * send_lin, gain_r.1 * send_lin),
            )
        };
        let (dst_l, dst_r) = &mut scratch.bus_bufs[dst_idx];
        sum_to_stereo(
            dst_l,
            dst_r,
            frames,
            scratch.track_buf_l,
            scratch.track_buf_r,
            send_gain_l,
            send_gain_r,
        );
    }
}

/// Aux sends sourced from a bus, tapped after its own fader so post-fader
/// reflects the bus level (the pre-fader buffer is still intact —
/// `sum_to_output` only read it). The tapped signal lands in the
/// destination's summing buffer, which is only re-read if that bus is
/// processed later in this pass: return busses are created after their
/// feeder busses, so their index is higher and the "returns after feeders"
/// ordering holds. A send to an earlier-indexed bus (already flushed this
/// block) is skipped by the natural ordering — its signal would otherwise
/// be summed into a buffer that's already gone to master.
pub(crate) fn apply_bus_aux_sends(
    bus_id: BusId,
    bus_idx: usize,
    gains: GainRamp,
    ctx: &BlockCtx<'_>,
    scratch: &mut BlockScratch<'_>,
) {
    let frames = ctx.inputs.frames;
    let (bus_gain_l, bus_gain_r) = gains;
    for send in ctx.inputs.aux_sends {
        if !send.enabled || send.source != SendSource::Bus(bus_id) {
            continue;
        }
        let Some(dst_idx) = ctx
            .inputs
            .busses
            .get_index_of(&send.dest)
            .filter(|idx| *idx < ctx.inputs.active_busses && *idx != bus_idx)
        else {
            continue;
        };
        let send_lin = db_to_linear(send.level_db);
        let (send_gain_l, send_gain_r) = if send.pre_fader {
            ((send_lin, send_lin), (send_lin, send_lin))
        } else {
            (
                (bus_gain_l.0 * send_lin, bus_gain_l.1 * send_lin),
                (bus_gain_r.0 * send_lin, bus_gain_r.1 * send_lin),
            )
        };
        sum_bus_to_bus(
            scratch.bus_bufs,
            bus_idx,
            dst_idx,
            frames,
            send_gain_l,
            send_gain_r,
        );
    }
}

/// Sum bus `src_idx`'s summing buffer into bus `dst_idx`'s, scaled by the
/// (possibly ramped) `gain_l`/`gain_r`. The two indices are required to
/// differ — a bus can never aux-send to itself (cyclic-route validation
/// rejects it) — so a disjoint `split_at_mut` lets both buffers be
/// borrowed at once without allocating a temporary.
#[inline]
fn sum_bus_to_bus(
    bus_bufs: &mut BusBufs,
    src_idx: usize,
    dst_idx: usize,
    frames: usize,
    gain_l: (f32, f32),
    gain_r: (f32, f32),
) {
    if src_idx == dst_idx {
        return;
    }
    let (src, dst) = if src_idx < dst_idx {
        let (left, right) = bus_bufs.split_at_mut(dst_idx);
        (&left[src_idx], &mut right[0])
    } else {
        let (left, right) = bus_bufs.split_at_mut(src_idx);
        (&right[0], &mut left[dst_idx])
    };
    sum_to_stereo(&mut dst.0, &mut dst.1, frames, &src.0, &src.1, gain_l, gain_r);
}
