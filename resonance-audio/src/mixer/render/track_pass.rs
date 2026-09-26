//! The per-track phase of a render block: source (frozen cache, timeline
//! instrument or clips + monitor input), sidechain key capture, plugin
//! delay compensation, meters, post-fader routing and aux sends.
//!
//! Sub-tracks are skipped by the top-level walk — they carry no source of
//! their own and are driven entirely by their parent's port fan-out (see
//! [`super::sub_track`]) at the end of the parent's own iteration.

use resonance_common::AutomationTarget;

use crate::mixer::automation_apply::{apply_plugin_params, auto_gain_ramp, auto_muted};
use crate::mixer::common::ramped_stereo_peaks;
use crate::mixer::midi_events::collect_midi_events;
use crate::types::*;

use crate::mixer::take_comp::mix_track_comp;

use super::clips::{mix_track_clips_governed, recorded_monitor_gate};
use super::context::{key_consumed, run_fx_chain, BlockCtx, BlockScratch};
use super::frozen::fill_from_frozen_source;
use super::ports::process_multi_port;
use super::routing::{apply_track_aux_sends, route_post_fader};
use super::strategy::{RenderStrategy, TrackDisposition};
use super::sub_track::fan_out_to_sub_tracks;

/// What a track's source stage produced, for the stages that follow it.
struct TrackSource {
    has_audio: bool,
    /// How many output ports the instrument plugin filled on this block,
    /// so the fan-out knows how many `port_scratch` entries to route to
    /// sub-tracks. Zero unless a multi-output instrument ran.
    extra_ports_filled: usize,
}

/// Walk every top-level track: (clips + monitor input) -> plugins ->
/// volume -> master (or bus). Sub-tracks are driven from their parent's
/// fan-out inside [`render_one_track`], not from here.
pub(crate) fn render_track_pass(
    ctx: &BlockCtx<'_>,
    scratch: &mut BlockScratch<'_>,
    strategy: &mut RenderStrategy<'_>,
) {
    for track in ctx.inputs.tracks.values() {
        if track.sub_track_of.is_some() {
            continue;
        }
        render_one_track(track, ctx, scratch, strategy);
    }
}

fn render_one_track(
    track: &Track,
    ctx: &BlockCtx<'_>,
    scratch: &mut BlockScratch<'_>,
    strategy: &mut RenderStrategy<'_>,
) {
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
        Some(d) if d.discard_after_instrument && keys_from(track, ctx, scratch.sidechain) => {
            TrackDisposition::key_only()
        }
        Some(d) => d,
        None if strategy.renders(track.id) && keys_from(track, ctx, scratch.sidechain) => {
            TrackDisposition::key_only()
        }
        None => return,
    };
    let (gain_l, gain_r) = (disp.gain_l, disp.gain_r);

    // Zero per-track buffers
    scratch.track_buf_l[..frames].fill(0.0);
    scratch.track_buf_r[..frames].fill(0.0);

    let Some(TrackSource {
        mut has_audio,
        extra_ports_filled,
    }) = render_track_source(track, &disp, ctx, scratch, strategy)
    else {
        return;
    };

    // Capture this track post-FX and pre-fader for anything keying
    // off it. Costs a `copy_from_slice` only for tracks that are
    // actually routed somewhere as a key.
    let tap_source = SendSource::Track(track.id);
    if scratch.sidechain.is_tapped(tap_source) {
        scratch.sidechain.capture(
            tap_source,
            &scratch.track_buf_l[..frames],
            &scratch.track_buf_r[..frames],
            frames,
        );
    }

    // Silenced, rendered only for a key (code review MIX-05): its own
    // key is captured above; its taps may carry one too, so the fan-out
    // still runs (each tap then stops at its own capture). Nothing of it
    // reaches PDC, the fader or the mix.
    if disp.key_only {
        if extra_ports_filled > 1 {
            fan_out_to_sub_tracks(track, &disp, extra_ports_filled, ctx, scratch, strategy);
        }
        return;
    }

    // Present only as a key source: its audio has just been captured,
    // and it belongs to a different stem (ba doc #277). Everything
    // below — PDC, fader, aux sends, routing — would put it in this
    // one, so stop here.
    if strategy.is_key_only(track.id) {
        return;
    }

    // Plugin-delay compensation: delay the post-chain signal so
    // every track reaches master with the same total latency (see
    // `crate::latency`). Runs even when the track produced no audio
    // this block so delayed tails keep flushing.
    if ctx.inputs.latency_comp.apply(
        track.id,
        &mut scratch.track_buf_l[..frames],
        &mut scratch.track_buf_r[..frames],
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
        return;
    }

    // Compute post-fader peak levels for VU meters (live only).
    if strategy.is_live() {
        let (peak_l, peak_r) = ramped_stereo_peaks(
            scratch.track_buf_l,
            scratch.track_buf_r,
            frames,
            gain_l,
            gain_r,
        );
        track.update_peak_l(peak_l);
        track.update_peak_r(peak_r);
    }

    // Route post-fader audio: either directly to the interleaved output
    // or into the target bus's summing buffer. Freeze capture forces the
    // master route (`None`); see `route_post_fader`.
    let dest = (!strategy.force_master_route()).then(|| track.output());
    route_post_fader(
        dest,
        (scratch.track_buf_l, scratch.track_buf_r),
        (gain_l, gain_r),
        (&mut *scratch.data, &mut *scratch.bus_bufs),
        ctx,
    );
    if strategy.is_live() {
        track.set_last_gains(gain_l.1, gain_r.1);
    }

    apply_track_aux_sends(track.id, (gain_l, gain_r), ctx, scratch);

    if extra_ports_filled > 1 {
        fan_out_to_sub_tracks(track, &disp, extra_ports_filled, ctx, scratch, strategy);
    }
}

/// Whether a sidechain key is tapped from `track` or from one of its
/// sub-tracks by a consumer that reads it ([`key_consumed`]) — the taps
/// a silenced track must still render for. Only
/// consulted for silenced tracks, so the sub-track scan costs nothing on
/// the audible path.
fn keys_from(track: &Track, ctx: &BlockCtx<'_>, sidechain: &SidechainTaps) -> bool {
    if key_consumed(ctx, sidechain, SendSource::Track(track.id)) {
        return true;
    }
    ctx.inputs.tracks.values().any(|t| {
        matches!(t.sub_track_of, Some((parent, _)) if parent == track.id)
            && key_consumed(ctx, sidechain, SendSource::Track(t.id))
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
    scratch: &mut BlockScratch<'_>,
    strategy: &mut RenderStrategy<'_>,
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
            &mut scratch.track_buf_l[..frames],
            &mut scratch.track_buf_r[..frames],
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
        render_instrument_source(track, disp, ctx, scratch, strategy)
    } else {
        Some(TrackSource {
            has_audio: render_audio_source(track, ctx, scratch, strategy),
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
    scratch: &mut BlockScratch<'_>,
    strategy: &mut RenderStrategy<'_>,
) -> Option<TrackSource> {
    let frames = ctx.inputs.frames;
    let mut has_audio = false;
    let mut extra_ports_filled: usize = 0;

    collect_midi_events(
        ctx.inputs.midi_clips,
        track.id,
        ctx.inputs.playhead,
        frames,
        ctx.inputs.tempo_map,
        ctx.inputs.sample_rate,
        scratch.note_event_buf,
    );

    // The first plugin is the instrument (receives note events); the
    // remaining ones are effects (audio-only).
    let track_plugins = track.plugins();
    let mut plugin_iter = track_plugins.iter();
    if let Some(&instrument_id) = plugin_iter.next() {
        if let Some(mutex) = ctx.inputs.plugins.get(&instrument_id) {
            if let Some(mut inst) = strategy.lock_instrument(mutex, instrument_id) {
                apply_plugin_params(
                    &mut inst,
                    ctx.inputs.automation,
                    instrument_id,
                    ctx.evals.eval_start,
                );
                for event in scratch.note_event_buf.iter() {
                    if event.is_note_on {
                        inst.0
                            .queue_note_on(event.note, event.velocity, event.sample_offset);
                    } else {
                        inst.0.queue_note_off(event.note, event.sample_offset);
                    }
                }

                let port_count = inst.0.output_port_count().min(scratch.port_scratch.len());
                if port_count > 1 {
                    // Multi-output instrument: fan out into the per-port
                    // scratch pool, then copy port 0 back into the
                    // track's main buffer so the rest of the track chain
                    // (effects + fader + bus routing) runs unchanged.
                    process_multi_port(&mut inst, &mut *scratch.port_scratch, port_count, frames);
                    scratch.track_buf_l[..frames]
                        .copy_from_slice(&scratch.port_scratch[0].0[..frames]);
                    scratch.track_buf_r[..frames]
                        .copy_from_slice(&scratch.port_scratch[0].1[..frames]);
                    extra_ports_filled = port_count;
                } else {
                    // Single-output path (legacy plugins): use the thin
                    // wrapper that re-targets onto track_buf_l/r.
                    inst.0.process(
                        &mut scratch.track_buf_l[..frames],
                        &mut scratch.track_buf_r[..frames],
                        frames,
                    );
                }
                has_audio = true;
            } else {
                strategy.instrument_lock_failed(instrument_id, scratch.note_event_buf);
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
        scratch.track_buf_l[..frames].fill(0.0);
        scratch.track_buf_r[..frames].fill(0.0);
    } else if run_fx_chain(
        plugin_iter.copied(),
        track.fx_bypass(),
        ctx,
        scratch.sidechain,
        (&mut *scratch.track_buf_l, &mut *scratch.track_buf_r),
        scratch.fx_dry,
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
    scratch: &mut BlockScratch<'_>,
    strategy: &mut RenderStrategy<'_>,
) -> bool {
    let frames = ctx.inputs.frames;
    let mut has_audio = false;

    // Mix monitor input for all tracks with monitoring enabled (live path
    // only) — unless the Recorded playback source gates it because a
    // recorded take covers this block (doc #257); the take itself arrives
    // via the clip mix just below.
    if !recorded_monitor_gate(track, ctx.inputs.clips, ctx.inputs.playhead, frames)
        && strategy.mix_monitor(track, scratch.track_buf_l, scratch.track_buf_r, frames)
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
        scratch.track_buf_l,
        scratch.track_buf_r,
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
                scratch.track_buf_l,
                scratch.track_buf_r,
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
            scratch.sidechain,
            (&mut *scratch.track_buf_l, &mut *scratch.track_buf_r),
            scratch.fx_dry,
            strategy,
        )
    {
        has_audio = true;
    }

    has_audio
}
