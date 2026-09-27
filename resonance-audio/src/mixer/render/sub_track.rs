//! The multi-output instrument fan-out: each extra output port of a
//! parent instrument, routed through the sub-track that owns it.
//!
//! Sub-track audio exists nowhere else — a sub-track has no clips and no
//! instrument of its own, only the port its parent filled — so this phase
//! runs at the end of the parent's own job, over the port scratch the
//! parent's `process_multi` just wrote. Each routed tap is copied into the
//! sub-track's own slot for the reduction to sum.

use resonance_common::AutomationTarget;

use crate::mixer::automation_apply::{auto_gain_ramp, auto_muted, auto_volume_ramp};
use crate::mixer::common::ramped_stereo_peaks;
use crate::types::*;

use super::context::{run_fx_chain, BlockCtx, JobScratch};
use super::slots::{SlotRoute, TrackSlot};
use super::strategy::{RenderStrategy, TrackDisposition};

/// One multi-output port routed through the sub-track that owns it.
struct SubTrackTap<'a> {
    sub_track: &'a Track,
    port_idx: usize,
    /// The parent track's fader (start, end) for this block, applied as a
    /// group trim over the tap (ba doc #275 P1.1).
    parent_volume: (f32, f32),
    /// Live: the parent is muted / solo-suppressed, so its taps fade out
    /// in the same block.
    parent_silenced: bool,
}

/// Sub-track fan-out: for every non-main plugin output port the
/// instrument filled, look up the matching sub-track (if any) and route
/// its scratch buffer through that sub-track's FX, fader, pan and bus.
pub(super) fn fan_out_to_sub_tracks(
    parent: &Track,
    disp: &TrackDisposition,
    ports_filled: usize,
    ctx: &BlockCtx<'_>,
    slots: &mut [TrackSlot],
    js: &mut JobScratch<'_>,
    strategy: &RenderStrategy<'_>,
) {
    // The parent's fader is the kit's group trim (ba doc #275 P1.1).
    // Evaluated once per block, outside the tap loop.
    let parent_volume = auto_volume_ramp(
        ctx.inputs.automation,
        AutomationTarget::TrackGain(parent.id),
        parent.volume(),
        ctx.evals.gain_start,
        ctx.evals.gain_end,
    );
    let n = slots.len();
    for (slot_idx, sub_track) in ctx.inputs.tracks.values().enumerate().take(n) {
        let Some((parent_id, port_idx)) = sub_track.sub_track_of else {
            continue;
        };
        if parent_id != parent.id {
            continue;
        }
        let port_idx = port_idx as usize;
        if port_idx == 0 || port_idx >= ports_filled {
            continue;
        }
        render_sub_track_tap(
            SubTrackTap {
                sub_track,
                port_idx,
                parent_volume,
                parent_silenced: disp.silenced,
            },
            ctx,
            &mut slots[slot_idx],
            js,
            strategy,
        );
    }
}

fn render_sub_track_tap(
    tap: SubTrackTap<'_>,
    ctx: &BlockCtx<'_>,
    slot: &mut TrackSlot,
    js: &mut JobScratch<'_>,
    strategy: &RenderStrategy<'_>,
) {
    let frames = ctx.inputs.frames;
    let SubTrackTap {
        sub_track,
        port_idx,
        parent_volume,
        parent_silenced,
    } = tap;

    let sub_auto_gain = auto_gain_ramp(
        ctx.inputs.automation,
        AutomationTarget::TrackGain(sub_track.id),
        AutomationTarget::TrackPan(sub_track.id),
        sub_track.volume(),
        sub_track.pan(),
        ctx.evals.gain_start,
        ctx.evals.gain_end,
    );
    let sub_auto_mute = auto_muted(
        ctx.inputs.automation,
        AutomationTarget::TrackMute(sub_track.id),
        ctx.evals.gain_start,
    );
    // A silenced tap that keys something still runs its chain for the
    // capture, and stops there (code review MIX-05).
    let sub_tap = SendSource::Track(sub_track.id);
    let ((sub_gain_l, sub_gain_r), key_only) = match strategy.sub_track_disposition(
        sub_track,
        parent_silenced,
        sub_auto_gain,
        sub_auto_mute,
        parent_volume,
    ) {
        Some(gains) => (gains, false),
        None if strategy.renders(sub_track.id) && slot.key_consumed => {
            (((0.0, 0.0), (0.0, 0.0)), true)
        }
        None => return,
    };

    // Run the sub-track's own effect chain in place on its port buffer,
    // before peak metering and bus/master routing. Sub-tracks never host
    // an instrument, so every entry in the plugin chain is treated as an
    // audio effect and is subject to the sub-track's own FX-bypass flag.
    {
        let sub_plugins = sub_track.plugins();
        let (pl, pr) = &mut js.port_scratch[port_idx];
        let (pl, pr) = (pl.as_mut_slice(), pr.as_mut_slice());
        run_fx_chain(
            sub_plugins.iter().copied(),
            sub_track.fx_bypass(),
            ctx,
            js.sidechain,
            (pl, pr),
            js.fx_dry,
            strategy,
        );
    }

    // A sub-track is a first-class key source: "duck the bass from the
    // kick" on a multi-output kit means keying off the kick TAP, which is
    // the only place that piece exists as its own signal. Captured
    // post-FX, pre-fader, exactly like the top-level tracks.
    if js.sidechain.is_tapped(sub_tap) {
        let (pl, pr) = &js.port_scratch[port_idx];
        js.sidechain
            .capture(sub_tap, &pl[..frames], &pr[..frames], frames);
    }

    // Captured, and not a member of this stem (ba doc #277). This is the
    // drum-tap case the field report hit: keying a compressor from the
    // kick TAP while measuring the ducked track, which is how the routing
    // gets verified.
    if key_only || strategy.is_key_only(sub_track.id) {
        return;
    }

    // Plugin-delay compensation for the sub-track's chain.
    {
        let (pl, pr) = &mut js.port_scratch[port_idx];
        ctx.inputs.latency_comp.apply(
            sub_track.id,
            &mut pl[..frames],
            &mut pr[..frames],
            ctx.inputs.playhead,
        );
    }

    let (pl, pr) = &js.port_scratch[port_idx];
    // Peak levels for sub-track VU meter (live only).
    if strategy.is_live() {
        let (sub_peak_l, sub_peak_r) = ramped_stereo_peaks(pl, pr, frames, sub_gain_l, sub_gain_r);
        sub_track.update_peak_l(sub_peak_l);
        sub_track.update_peak_r(sub_peak_r);
    }

    // Leave the post-fader route to the reduction, which sums it into the
    // sub-track's destination after the parent's own route and sends.
    // Freeze capture folds the fan-out into master so the parent's cache
    // carries the whole multi-output mix.
    slot.l[..frames].copy_from_slice(&pl[..frames]);
    slot.r[..frames].copy_from_slice(&pr[..frames]);
    slot.route = Some(SlotRoute {
        dest: (!strategy.force_master_route()).then(|| sub_track.output()),
        gains: (sub_gain_l, sub_gain_r),
    });
    if strategy.is_live() {
        sub_track.set_last_gains(sub_gain_l.1, sub_gain_r.1);
    }
}
