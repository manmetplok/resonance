//! Where a rendered signal goes once its fader ramp is known: the main
//! post-fader route (bus summing buffer or straight to master) and the
//! parallel aux-send taps from a track and from a bus.

use resonance_dsp::db_to_linear;

use crate::mixer::common::{sum_to_output, sum_to_stereo};
use crate::types::*;

use super::context::{BlockCtx, BusBufs, GainRamp, OutputTargets};

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
/// sample-correct regardless of bus ordering. `src` is the track's
/// pre-fader signal. `only_dest` restricts the taps to sends into that
/// one bus (a return-bus measurement); `None` applies every send.
pub(crate) fn apply_track_aux_sends(
    track_id: TrackId,
    gains: GainRamp,
    src: (&[f32], &[f32]),
    ctx: &BlockCtx<'_>,
    bus_bufs: &mut BusBufs,
    only_dest: Option<BusId>,
) {
    let frames = ctx.inputs.frames;
    let (gain_l, gain_r) = gains;
    for send in ctx.inputs.aux_sends {
        if !send.enabled || send.source != SendSource::Track(track_id) {
            continue;
        }
        if only_dest.is_some_and(|only| only != send.dest) {
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
        let (dst_l, dst_r) = &mut bus_bufs[dst_idx];
        sum_to_stereo(dst_l, dst_r, frames, src.0, src.1, send_gain_l, send_gain_r);
    }
}

/// Aux sends INTO bus `dst_idx` from the busses before it, tapped after
/// each source's own fader so post-fader reflects the source bus's level
/// (its pre-fader buffer is intact — the sum into master only reads it).
///
/// The bus pass runs busses level by level (realtime-multithreading.md
/// §4.5), so instead of each source pushing its sends as it finishes, each
/// destination pulls them just before it runs: every source is a
/// lower-indexed bus of an earlier level, and they are added in source
/// index order — exactly the order the one serial index-order loop added
/// them in, so the sum is bit-identical. `source(i)` is bus `i`'s
/// finished buffer and fader ramp when it reached the mix this block.
///
/// Only lower-indexed sources count: return busses are created after their
/// feeder busses, so the "returns after feeders" ordering holds, and a send
/// to an earlier-indexed bus (whose own output is already decided) has no
/// audible effect — it never had one in the serial loop either.
pub(crate) fn pull_bus_aux_sends<'b>(
    dst_idx: usize,
    dst: &mut (Vec<f32>, Vec<f32>),
    source: impl Fn(usize) -> Option<(&'b (Vec<f32>, Vec<f32>), GainRamp)>,
    ctx: &BlockCtx<'_>,
) {
    let frames = ctx.inputs.frames;
    for (src_idx, src_bus) in ctx.inputs.busses.values().enumerate().take(dst_idx) {
        let Some(((src_l, src_r), gains)) = source(src_idx) else {
            continue;
        };
        let (bus_gain_l, bus_gain_r) = gains;
        for send in ctx.inputs.aux_sends {
            if !send.enabled || send.source != SendSource::Bus(src_bus.id) {
                continue;
            }
            if ctx.inputs.busses.get_index_of(&send.dest) != Some(dst_idx) {
                continue;
            }
            let send_lin = db_to_linear(send.level_db);
            let (send_gain_l, send_gain_r) = if send.pre_fader {
                ((send_lin, send_lin), (send_lin, send_lin))
            } else {
                (
                    (bus_gain_l.0 * send_lin, bus_gain_l.1 * send_lin),
                    (bus_gain_r.0 * send_lin, bus_gain_r.1 * send_lin),
                )
            };
            sum_to_stereo(
                &mut dst.0,
                &mut dst.1,
                frames,
                src_l,
                src_r,
                send_gain_l,
                send_gain_r,
            );
        }
    }
}
