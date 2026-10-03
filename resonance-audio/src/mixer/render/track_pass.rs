//! The per-track phase of a render block, split at the summing point
//! (realtime-multithreading.md §4.1):
//!
//! - **Jobs**, one per top-level track: source (frozen cache, timeline
//!   instrument or clips + monitor input), insert chain, sidechain key
//!   capture, plugin delay compensation, meters and the multi-output
//!   fan-out. A job writes only the [`TrackSlot`]s it owns and never a
//!   shared sum, so jobs are independent of each other.
//! - **The reduction**, serial, in track-map order: the post-fader route,
//!   the aux sends and the sub-track routes each job left in its slots —
//!   exactly the additions, in exactly the order, the single per-track
//!   loop used to make. The mix is therefore bit-identical to that loop
//!   whatever order the jobs ran in.
//!
//! Sub-tracks are skipped by the top-level walk — they carry no source of
//! their own and are driven entirely by their parent's port fan-out (see
//! [`super::sub_track`]) at the end of the parent's own job.

use std::time::Instant;

use resonance_common::AutomationTarget;

use crate::mixer::automation_apply::{apply_plugin_params, auto_gain_ramp, auto_muted};
use crate::mixer::common::ramped_stereo_peaks;
use crate::mixer::midi_events::collect_midi_events;
use crate::mixer::midi_stash::{MidiStash, StashEntry};
use crate::render_pool::{RunStats, WorkerBufs};
use crate::types::*;

use crate::mixer::take_comp::mix_track_comp;

use super::clips::{mix_track_clips_governed, recorded_monitor_gate};
use super::context::{key_consumed, run_fx_chain, BlockCtx, BlockScratch, BusBufs, JobScratch};
use super::frozen::fill_from_frozen_source;
use super::ports::process_multi_port;
use super::routing::{apply_track_aux_sends, route_post_fader};
use super::slots::{PassStats, SlotCells, SlotRoute, TrackSlot};
use super::strategy::{RenderStrategy, SendFilter, TrackDisposition};
use super::sub_track::fan_out_to_sub_tracks;

/// What a track's source stage produced, for the stages that follow it.
struct TrackSource {
    has_audio: bool,
    /// How many output ports the instrument plugin filled on this block,
    /// so the fan-out knows how many `port_scratch` entries to route to
    /// sub-tracks. Zero unless a multi-output instrument ran.
    extra_ports_filled: usize,
}

/// The job's own buffers: its slot's pre-fader signal, full length (as
/// the scratch track buffer they replace was), and its MIDI carry.
struct TrackBufs<'a> {
    l: &'a mut [f32],
    r: &'a mut [f32],
    carry: &'a mut StashEntry,
}

/// Every top-level track: prepare the slots, run each track's job (on the
/// render pool when the caller has one), then reduce the slots into the
/// bus buffers and the output in track order.
pub(crate) fn render_track_pass(
    ctx: &BlockCtx<'_>,
    scratch: &mut BlockScratch<'_>,
    strategy: &RenderStrategy<'_>,
) {
    let threads = scratch.pool.map_or(1, |pool| pool.effective_threads());
    let (slots, order) = scratch.slots.parts();
    let n = ctx.inputs.tracks.len().min(slots.len());
    let slots = &mut slots[..n];

    prepare_slots(
        ctx,
        slots,
        scratch.sidechain,
        scratch.stash.as_deref_mut(),
        strategy,
    );
    let caller_only = build_schedule(ctx, slots, order, threads > 1);

    let stats = {
        let order: &[u32] = order;
        let cells = SlotCells::new(slots);
        let sidechain: &SidechainTaps = scratch.sidechain;
        let job = |k: usize, bufs: &mut WorkerBufs<'_>| {
            let idx = order[k] as usize;
            let Some((_, track)) = ctx.inputs.tracks.get_index(idx) else {
                return;
            };
            let start = Instant::now();
            let mut js = JobScratch {
                port_scratch: &mut *bufs.port_scratch,
                note_event_buf: &mut *bufs.note_event_buf,
                sidechain,
                fx_dry: &mut *bufs.fx_dry,
            };
            run_track_job(idx, track, ctx, &cells, &mut js, strategy);
            let ns = start.elapsed().as_nanos().min(u32::MAX as u128) as u32;
            // SAFETY: slot `idx` is this job's own.
            let slot = unsafe { cells.get(idx) };
            slot.last_ns = ns;
            slot.cost_ns = if slot.cost_ns == 0 {
                ns
            } else {
                // EMA, 1/8 per block: follows a chain change within a few
                // dozen blocks without chasing one-block spikes.
                slot.cost_ns - slot.cost_ns / 8 + ns / 8
            };
        };
        let mut caller = WorkerBufs {
            port_scratch: &mut *scratch.port_scratch,
            note_event_buf: &mut *scratch.note_event_buf,
            fx_dry: &mut *scratch.fx_dry,
        };
        let wall_start = Instant::now();
        let run = match scratch.pool {
            Some(pool) => pool.run(&mut caller, caller_only, order.len(), &job),
            None => {
                for k in 0..order.len() {
                    job(k, &mut caller);
                }
                RunStats {
                    join_wait_ns: 0,
                    threads: 1,
                }
            }
        };
        let wall_ns = wall_start.elapsed().as_nanos() as u64;
        pass_stats(ctx, slots_of(&cells), order, wall_ns, run)
    };
    scratch.slots.record(stats);
    let (slots, _) = scratch.slots.parts();
    let slots = &mut slots[..n];

    reduce_track_pass(
        ctx,
        slots,
        &mut *scratch.data,
        &mut *scratch.bus_bufs,
        scratch.stash.as_deref_mut(),
        strategy,
    );
}

/// Re-borrow the whole slot range once every job is done.
fn slots_of<'s>(cells: &'s SlotCells<'_>) -> impl Fn(usize) -> &'s TrackSlot + 's {
    // SAFETY: called only after the pool's join, when no job holds a
    // slot any more.
    move |idx| unsafe { cells.get_ref(idx) }
}

/// Summarize the job phase for the load report.
fn pass_stats<'s>(
    ctx: &BlockCtx<'_>,
    slot: impl Fn(usize) -> &'s TrackSlot,
    order: &[u32],
    wall_ns: u64,
    run: RunStats,
) -> PassStats {
    let mut stats = PassStats {
        wall_ns,
        join_wait_ns: run.join_wait_ns,
        threads: run.threads,
        ..PassStats::default()
    };
    for &idx in order {
        let ns = slot(idx as usize).last_ns as u64;
        stats.jobs_ns += ns;
        if ns > stats.critical_ns {
            stats.critical_ns = ns;
            stats.critical_track = ctx.inputs.tracks.get_index(idx as usize).map(|(id, _)| *id);
        }
    }
    stats
}

/// Fill `order` with this block's jobs — one per top-level track — and
/// return how many of them lead the list as caller-only.
///
/// Caller-only jobs hold a serial-only plugin (realtime-multithreading.md
/// §4.6) and run on the rendering thread itself. With more than one
/// thread the rest are sorted longest-first by their cost EMA, so the
/// heaviest chain starts at once and the cheap ones fill in behind it
/// (longest-processing-time scheduling). Order never changes the output —
/// the reduction fixes it — only the makespan. Allocation-free: `order`
/// has capacity for every slot.
fn build_schedule(
    ctx: &BlockCtx<'_>,
    slots: &[TrackSlot],
    order: &mut Vec<u32>,
    parallel: bool,
) -> usize {
    let tracks = ctx.inputs.tracks;
    order.clear();
    for (idx, track) in tracks.values().enumerate().take(slots.len()) {
        if track.sub_track_of.is_none() {
            order.push(idx as u32);
        }
    }
    if !parallel {
        return 0;
    }
    let mut caller_only = 0;
    if crate::clap_host::any_serial_only_plugin() {
        for k in 0..order.len() {
            let (_, track) = tracks.get_index(order[k] as usize).expect("indexed above");
            if holds_serial_only_plugin(ctx, track) {
                order.swap(caller_only, k);
                caller_only += 1;
            }
        }
    }
    order[caller_only..]
        .sort_unstable_by_key(|&idx| std::cmp::Reverse(slots[idx as usize].cost_ns));
    caller_only
}

/// Whether `track`'s chain, or one of its sub-tracks' chains, holds a
/// serial-only plugin.
fn holds_serial_only_plugin(ctx: &BlockCtx<'_>, track: &Track) -> bool {
    let serial = |t: &Track| {
        t.plugins()
            .iter()
            .any(|id| ctx.inputs.plugins.get(id).is_some_and(|slot| slot.serial_only))
    };
    serial(track)
        || ctx.inputs.tracks.values().any(|t| {
            matches!(t.sub_track_of, Some((parent, _)) if parent == track.id) && serial(t)
        })
}

/// The serial prologue before any job runs: every decision a job needs
/// that reads *another* track's live state, and the MIDI hand-off.
///
/// - `key_consumed` per track: whether a silenced track must still render
///   for a sidechain key depends on the consuming track's last gains and
///   bypass fades, which the consumer's own job advances. Deciding it here
///   makes it independent of job order.
/// - The stash lends each instrument track's parked MIDI to its slot's
///   carry, so the job delivers (or parks more) without sharing the
///   stash; [`reduce_track_pass`] takes back what is left.
fn prepare_slots(
    ctx: &BlockCtx<'_>,
    slots: &mut [TrackSlot],
    sidechain: &SidechainTaps,
    stash: Option<&mut MidiStash>,
    strategy: &RenderStrategy<'_>,
) {
    let tracks = ctx.inputs.tracks;
    for (slot, track) in slots.iter_mut().zip(tracks.values()) {
        slot.route = None;
        slot.fanned_out = false;
        slot.key_consumed = key_consumed(ctx, sidechain, strategy, SendSource::Track(track.id));
    }
    if let Some(stash) = stash {
        if stash.has_pending() {
            for (slot, track) in slots.iter_mut().zip(tracks.values()) {
                if track.sub_track_of.is_some() {
                    continue;
                }
                if let Some(&instrument_id) = track.plugins().first() {
                    stash.take(instrument_id, &mut slot.carry);
                }
            }
        }
    }
}

/// One track's job: render it into its own slot, then its fan-out into
/// its sub-tracks' slots. Touches no slot but those (see `SlotCells`).
fn run_track_job(
    idx: usize,
    track: &Track,
    ctx: &BlockCtx<'_>,
    cells: &SlotCells<'_>,
    js: &mut JobScratch<'_>,
    strategy: &RenderStrategy<'_>,
) {
    if let Some((disp, ports_filled)) = render_one_track(idx, track, ctx, cells, js, strategy) {
        // SAFETY: slot `idx` is this job's own track.
        unsafe { cells.get(idx) }.fanned_out = true;
        fan_out_to_sub_tracks(track, &disp, ports_filled, ctx, cells, js, strategy);
    }
}

/// The ordered reduction: for each top-level track, in map order, replay
/// the post-fader route and aux sends its job recorded, then its
/// sub-tracks' routes — the order the single per-track loop summed in.
/// Also returns each job's MIDI carry to the stash.
fn reduce_track_pass(
    ctx: &BlockCtx<'_>,
    slots: &mut [TrackSlot],
    data: &mut [f32],
    bus_bufs: &mut BusBufs,
    mut stash: Option<&mut MidiStash>,
    strategy: &RenderStrategy<'_>,
) {
    let tracks = ctx.inputs.tracks;
    let n = slots.len();
    for (idx, track) in tracks.values().enumerate().take(n) {
        if track.sub_track_of.is_some() {
            continue;
        }
        let slot = &mut slots[idx];
        // Depth measurement (warmth-width-depth.md §7.6): a return's
        // feeder reaches the mix only through its sends into that return;
        // a dry render taps no send.
        let filter = strategy.send_filter(track.id);
        let send_only = match filter {
            Some(SendFilter::OnlyInto(bus)) => Some(bus),
            _ => None,
        };
        if let Some(route) = slot.route.take() {
            let src = (slot.l.as_slice(), slot.r.as_slice());
            if send_only.is_none() {
                route_post_fader(
                    route.dest,
                    src,
                    route.gains,
                    (&mut *data, &mut *bus_bufs),
                    ctx,
                );
            }
            if filter != Some(SendFilter::Dry) {
                apply_track_aux_sends(track.id, route.gains, src, ctx, bus_bufs, send_only);
            }
        }
        if let Some(stash) = stash.as_deref_mut() {
            stash.restore(&mut slot.carry);
        }
        if !std::mem::take(&mut slot.fanned_out) || send_only.is_some() {
            // A send-only feeder's sub-tracks carry its other outputs,
            // which are not what the return is fed; drop their routes.
            if send_only.is_some() {
                for (sub_idx, sub_track) in tracks.values().enumerate().take(n) {
                    if matches!(sub_track.sub_track_of, Some((parent, _)) if parent == track.id) {
                        slots[sub_idx].route = None;
                    }
                }
            }
            continue;
        }
        for (sub_idx, sub_track) in tracks.values().enumerate().take(n) {
            if !matches!(sub_track.sub_track_of, Some((parent, _)) if parent == track.id) {
                continue;
            }
            let sub_slot = &mut slots[sub_idx];
            if let Some(route) = sub_slot.route.take() {
                route_post_fader(
                    route.dest,
                    (sub_slot.l.as_slice(), sub_slot.r.as_slice()),
                    route.gains,
                    (&mut *data, &mut *bus_bufs),
                    ctx,
                );
            }
        }
    }
}

/// Render one top-level track into slot `idx`. Returns the disposition
/// and port count when its multi-output fan-out must run next.
fn render_one_track(
    idx: usize,
    track: &Track,
    ctx: &BlockCtx<'_>,
    cells: &SlotCells<'_>,
    js: &mut JobScratch<'_>,
    strategy: &RenderStrategy<'_>,
) -> Option<(TrackDisposition, usize)> {
    let frames = ctx.inputs.frames;
    let auto_gain = auto_gain_ramp(
        ctx.inputs.automation,
        AutomationTarget::TrackGain(track.id),
        AutomationTarget::TrackPan(track.id),
        track.volume(),
        track.pan(),
        ctx.evals.gain_start,
        ctx.evals.gain_end,
    );
    let auto_mute = auto_muted(
        ctx.inputs.automation,
        AutomationTarget::TrackMute(track.id),
        ctx.evals.gain_start,
    );
    // A silenced track still renders when a key is tapped from it (code
    // review MIX-05): the ghost kick keys the bass compressor while muted.
    let disp = match strategy.track_disposition(track, ctx.inputs.any_solo, auto_gain, auto_mute)
    {
        Some(d) if d.discard_after_instrument && keys_from(idx, track, ctx, cells) => {
            TrackDisposition::key_only()
        }
        Some(d) => d,
        None if strategy.renders(track.id) && keys_from(idx, track, ctx, cells) => {
            TrackDisposition::key_only()
        }
        None => return None,
    };
    let (gain_l, gain_r) = (disp.gain_l, disp.gain_r);

    // SAFETY: slot `idx` is this job's own track, and nothing else of it
    // is borrowed while this lives.
    let TrackSlot {
        l, r, carry, route, ..
    } = unsafe { cells.get(idx) };
    let (buf_l, buf_r) = (l.as_mut_slice(), r.as_mut_slice());

    // Zero per-track buffers
    buf_l[..frames].fill(0.0);
    buf_r[..frames].fill(0.0);

    let TrackSource {
        mut has_audio,
        extra_ports_filled,
    } = render_track_source(
        track,
        &disp,
        ctx,
        TrackBufs {
            l: &mut *buf_l,
            r: &mut *buf_r,
            carry,
        },
        js,
        strategy,
    )?;
    let fan_out = (extra_ports_filled > 1).then_some(extra_ports_filled);

    // Capture this track post-FX and pre-fader for anything keying
    // off it. Costs a `copy_from_slice` only for tracks that are
    // actually routed somewhere as a key.
    let tap_source = SendSource::Track(track.id);
    if js.sidechain.is_tapped(tap_source) {
        // SAFETY: only this job renders — so captures — this track.
        unsafe {
            js.sidechain
                .capture_shared(tap_source, &buf_l[..frames], &buf_r[..frames], frames);
        }
    }

    // Silenced, rendered only for a key (code review MIX-05): its own
    // key is captured above; its taps may carry one too, so the fan-out
    // still runs (each tap then stops at its own capture). Nothing of it
    // reaches PDC, the fader or the mix.
    if disp.key_only {
        return fan_out.map(|ports| (disp, ports));
    }

    // Present only as a key source: its audio has just been captured,
    // and it belongs to a different stem (ba doc #277). Everything
    // below — PDC, fader, aux sends, routing — would put it in this
    // one, so stop here.
    if strategy.is_key_only(track.id) {
        return None;
    }

    // Plugin-delay compensation: delay the post-chain signal so
    // every track reaches master with the same total latency (see
    // `crate::latency`). Runs even when the track produced no audio
    // this block so delayed tails keep flushing.
    if ctx.inputs.latency_comp.apply(
        track.id,
        &mut buf_l[..frames],
        &mut buf_r[..frames],
        ctx.inputs.playhead,
    ) {
        has_audio = true;
    }

    if !has_audio {
        // Nothing to ramp over: snap the remembered gain to the
        // target so changes made during silence don't ramp later.
        if strategy.is_live() {
            track.set_last_gains(gain_l.1, gain_r.1);
        }
        return None;
    }

    // Compute post-fader peak levels for VU meters (live only).
    if strategy.is_live() {
        let (peak_l, peak_r) = ramped_stereo_peaks(buf_l, buf_r, frames, gain_l, gain_r);
        track.update_peak_l(peak_l);
        track.update_peak_r(peak_r);
    }

    // Leave the post-fader route to the reduction: either directly to the
    // interleaved output or into the target bus's summing buffer, then
    // the aux sends. Freeze capture forces the master route (`None`); see
    // `route_post_fader`.
    *route = Some(SlotRoute {
        dest: (!strategy.force_master_route()).then(|| track.output()),
        gains: (gain_l, gain_r),
    });
    if strategy.is_live() {
        track.set_last_gains(gain_l.1, gain_r.1);
    }

    fan_out.map(|ports| (disp, ports))
}

/// Whether a sidechain key is tapped from `track` or from one of its
/// sub-tracks by a consumer that reads it ([`key_consumed`], decided per
/// slot by [`prepare_slots`]) — the taps a silenced track must still
/// render for. Only consulted for silenced tracks, so the sub-track scan
/// costs nothing on the audible path.
fn keys_from(idx: usize, track: &Track, ctx: &BlockCtx<'_>, cells: &SlotCells<'_>) -> bool {
    // SAFETY (both reads): slot `idx` and the slots of `track`'s
    // sub-tracks are this job's own; each borrow ends at once.
    if unsafe { cells.get(idx) }.key_consumed {
        return true;
    }
    ctx.inputs
        .tracks
        .values()
        .enumerate()
        .take(cells.len())
        .any(|(sub_idx, t)| {
            matches!(t.sub_track_of, Some((parent, _)) if parent == track.id)
                && unsafe { cells.get(sub_idx) }.key_consumed
        })
}

/// Fill the track buffers with the track's source signal: the frozen
/// cache if one is attached, else the timeline instrument, else clips +
/// live monitor input. `None` means "stop rendering this track for this
/// block" — the live mute fade has fully landed, so the instrument ran
/// (voice state stays consistent) but nothing downstream should.
fn render_track_source(
    track: &Track,
    disp: &TrackDisposition,
    ctx: &BlockCtx<'_>,
    bufs: TrackBufs<'_>,
    js: &mut JobScratch<'_>,
    strategy: &RenderStrategy<'_>,
) -> Option<TrackSource> {
    // Frozen playback substitution (doc #187, todo #573): when the
    // track carries an active frozen source, play its cached post-FX
    // samples in place of running the timeline synth + insert FX. The
    // cache is timeline-aligned and captured pre-fader (see
    // `freeze_raw`), so the post-source mixer stage — PDC, volume, pan,
    // mute / solo, routing and aux sends — still applies live. This path
    // is shared by the live callback and the offline bounce / stem
    // renderer, so a frozen track is transparently identical in playback
    // and in export, with no separate code path. (The instrument's
    // fan-out is skipped, so `extra_ports_filled` stays 0; a frozen
    // multi-output track's sub-mix is already baked into its single cache
    // file.)
    let frozen_source = track.frozen_source.load_full();
    if let Some(source) = frozen_source.as_deref() {
        let frames = ctx.inputs.frames;
        let has_audio = fill_from_frozen_source(
            source,
            ctx.inputs.sample_rate,
            ctx.inputs.playhead,
            frames,
            &mut bufs.l[..frames],
            &mut bufs.r[..frames],
        );
        return Some(TrackSource {
            has_audio,
            extra_ports_filled: 0,
        });
    }

    // The same predicate the capture side uses to decide whether to open
    // an audio recording buffer, so playback and capture cannot drift
    // apart about what a track's source is.
    if track.runs_internal_instrument() {
        render_instrument_source(track, disp, ctx, bufs, js, strategy)
    } else {
        Some(TrackSource {
            has_audio: render_audio_source(track, ctx, bufs, js, strategy),
            extra_ports_filled: 0,
        })
    }
}

/// Instrument track: collect this block's MIDI, run the instrument (the
/// first entry in the plugin chain, single- or multi-output), then its
/// insert FX.
fn render_instrument_source(
    track: &Track,
    disp: &TrackDisposition,
    ctx: &BlockCtx<'_>,
    bufs: TrackBufs<'_>,
    js: &mut JobScratch<'_>,
    strategy: &RenderStrategy<'_>,
) -> Option<TrackSource> {
    let frames = ctx.inputs.frames;
    let TrackBufs {
        l: track_buf_l,
        r: track_buf_r,
        carry,
    } = bufs;
    let mut has_audio = false;
    let mut extra_ports_filled: usize = 0;

    collect_midi_events(
        ctx.inputs.midi_clips,
        track.id,
        ctx.inputs.playhead,
        frames,
        ctx.inputs.tempo_map,
        ctx.inputs.sample_rate,
        js.note_event_buf,
    );

    // The first plugin is the instrument (receives note events); the
    // remaining ones are effects (audio-only).
    let track_plugins = track.plugins();
    let mut plugin_iter = track_plugins.iter();
    if let Some(&instrument_id) = plugin_iter.next() {
        if let Some(mutex) = ctx.inputs.plugins.get(&instrument_id) {
            if let Some(mut inst) = strategy.lock_instrument(mutex) {
                // Replay events parked during earlier lock contention
                // before this block's (a no-op offline, where the carry
                // is always empty).
                carry.deliver(instrument_id, &mut *inst);
                apply_plugin_params(
                    &mut inst,
                    ctx.inputs.automation,
                    instrument_id,
                    ctx.evals.eval_start,
                    frames,
                );
                for event in js.note_event_buf.iter() {
                    if event.is_note_on {
                        inst.0
                            .queue_note_on(event.note, event.velocity, event.sample_offset);
                    } else {
                        inst.0.queue_note_off(event.note, event.sample_offset);
                    }
                }

                let port_count = inst.0.output_port_count().min(js.port_scratch.len());
                if port_count > 1 {
                    // Multi-output instrument: fan out into the per-port
                    // scratch pool, then copy port 0 back into the
                    // track's main buffer so the rest of the track chain
                    // (effects + fader + bus routing) runs unchanged.
                    process_multi_port(&mut inst, &mut *js.port_scratch, port_count, frames);
                    track_buf_l[..frames].copy_from_slice(&js.port_scratch[0].0[..frames]);
                    track_buf_r[..frames].copy_from_slice(&js.port_scratch[0].1[..frames]);
                    extra_ports_filled = port_count;
                } else {
                    // Single-output path (legacy plugins): use the thin
                    // wrapper that re-targets onto track_buf_l/r.
                    inst.0.process(
                        &mut track_buf_l[..frames],
                        &mut track_buf_r[..frames],
                        frames,
                    );
                }
                has_audio = true;
            } else if strategy.is_live() {
                // The UI thread holds the plugin lock (param drag /
                // autosave / reload): park this block's events so they
                // replay on the next successful lock instead of dropping
                // them. The one-block audio dropout is accepted for now
                // (future work: crossfade). A carry the prologue filled
                // for a *different* instrument (the chain was swapped
                // between the two reads) can't take them; they drop.
                if carry.instance().is_none_or(|id| id == instrument_id) {
                    carry.stash(instrument_id, js.note_event_buf);
                }
            }
        }
    }

    // Silenced track (live): the instrument ran (voice state stays
    // consistent) but its output is discarded — once the mute ramp has
    // finished fading the previous gain to zero.
    if disp.discard_after_instrument {
        return None;
    }

    if disp.discard_own_output {
        // Fan-out driver only (ba todo #1242): this track is in the
        // stem's filter so its instrument would RUN and fill the port
        // scratch, not because its own main output belongs here. Drop
        // port 0 — and with it the parent chain, fader, aux sends and
        // routing that would otherwise carry it into the stem — while
        // still returning the port count, which is the whole reason this
        // track is rendering.
        track_buf_l[..frames].fill(0.0);
        track_buf_r[..frames].fill(0.0);
    } else if run_fx_chain(
        plugin_iter.copied(),
        track.fx_bypass(),
        ctx,
        js.sidechain,
        (&mut *track_buf_l, &mut *track_buf_r),
        js.fx_dry,
        strategy,
    ) {
        // A ducker on a synth track is the most common sidechain there
        // is, so the key resolution here is the same as on an audio
        // track; routing one used to store the route and then key off
        // the track's own input (ba doc #275 P0).
        has_audio = true;
    }

    Some(TrackSource {
        has_audio,
        extra_ports_filled,
    })
}

/// Audio track: live monitor input + clip mix + insert FX.
///
/// External-instrument tracks land here too (doc #169): their synth is
/// outboard, so there is no instrument plugin to run and their audio
/// arrives on the return input — live through the monitor mix, or as a
/// recorded take through the clip mix. Every plugin on such a track is an
/// insert effect.
fn render_audio_source(
    track: &Track,
    ctx: &BlockCtx<'_>,
    bufs: TrackBufs<'_>,
    js: &mut JobScratch<'_>,
    strategy: &RenderStrategy<'_>,
) -> bool {
    let frames = ctx.inputs.frames;
    let (track_buf_l, track_buf_r) = (bufs.l, bufs.r);
    let mut has_audio = false;

    // Mix monitor input for all tracks with monitoring enabled (live path
    // only) — unless the Recorded playback source gates it because a
    // recorded take covers this block (doc #257); the take itself arrives
    // via the clip mix just below.
    if !recorded_monitor_gate(track, ctx.inputs.clips, ctx.inputs.playhead, frames)
        && strategy.mix_monitor(track, track_buf_l, track_buf_r, frames)
    {
        has_audio = true;
    }

    // Accumulate all clips for this track into de-interleaved track
    // buffers, applying each clip's fade-in/out envelope, clip gain, and
    // the automatic same-track crossfade. Recorded take clips under comp
    // control are skipped here and rendered by the comp pass below, so the
    // raw overlapping passes never play on top of the comp.
    if mix_track_clips_governed(
        ctx.inputs.clips,
        track.id,
        ctx.inputs.playhead,
        frames,
        track_buf_l,
        track_buf_r,
        ctx.inputs.take_comp,
    ) {
        has_audio = true;
    }

    // Take-comp playback (epic #15, doc #165): switch the source take clip
    // per comp segment with an equal-power crossfade at each seam. Reached
    // by the live callback and the offline bounce through the same
    // `render_block`, off the same published table, so a comped or
    // active-take selection bounces exactly as it plays.
    if !ctx.inputs.take_comp.is_empty() {
        if let Some(track_comp) = ctx.inputs.take_comp.track_comp(track.id) {
            if mix_track_comp(
                track_comp,
                ctx.inputs.clips,
                ctx.inputs.playhead,
                frames,
                track_buf_l,
                track_buf_r,
            ) {
                has_audio = true;
            }
        }
    }

    // Process through the plugin chain (skipped once the chain's bypass
    // fade has fully landed on "bypassed").
    let track_plugins = track.plugins();
    if !track_plugins.is_empty()
        && run_fx_chain(
            track_plugins.iter().copied(),
            track.fx_bypass(),
            ctx,
            js.sidechain,
            (&mut *track_buf_l, &mut *track_buf_r),
            js.fx_dry,
            strategy,
        )
    {
        has_audio = true;
    }

    has_audio
}
