//! The block's parameter structs.
//!
//! `render_block` used to take 24 positional arguments, which every phase
//! helper would have had to re-thread. They are split here by mutability
//! instead, which is also how the borrow checker wants them: [`BlockInputs`]
//! is the read-only view of the project for this block (the guards the
//! caller already holds, plus tempo / automation / latency / position), and
//! [`BlockScratch`] is the caller's pre-allocated, mutably borrowed buffer
//! set. Neither owns anything, so building one is free and no phase can
//! allocate through it.
//!
//! [`BlockCtx`] pairs the inputs with the [`EvalPositions`] derived from
//! them once per block, so a helper never recomputes the comp-delayed
//! automation frames.

use std::sync::Arc;

use indexmap::IndexMap;

use crate::bypass::{run_faded, BypassFade, FadeStage, FxDryScratch};
use crate::clap_host::{PluginMap, StereoBufMut};
use crate::engine::AutomationSnapshot;
use crate::latency::LatencyComp;
use crate::mixer::automation_apply::apply_plugin_params;
use crate::mixer::take_comp::CompRenderTable;
use crate::types::*;

use super::strategy::RenderStrategy;

/// Gain ramp endpoints for one block, per channel: `((l_from, l_to),
/// (r_from, r_to))`. A bounce collapses both pairs to `from == to`, which
/// the ramp helpers reduce to a plain multiply.
pub(crate) type GainRamp = ((f32, f32), (f32, f32));

/// The per-bus summing buffers, indexed by the bus's position in the
/// project's bus map.
pub(crate) type BusBufs = [(Vec<f32>, Vec<f32>)];

/// Where a post-fader signal can land: the interleaved master output and
/// the bus summing buffers, borrowed together because the destination is
/// only known per track.
pub(crate) type OutputTargets<'a> = (&'a mut [f32], &'a mut BusBufs);

/// Everything one render block reads. All borrowed, all shared: copying a
/// `BlockInputs` copies a handful of pointers, never any audio.
#[derive(Clone, Copy)]
pub(crate) struct BlockInputs<'a> {
    /// Channel count of the interleaved output buffer.
    pub(crate) channels: usize,
    pub(crate) tracks: &'a IndexMap<TrackId, Track>,
    pub(crate) busses: &'a IndexMap<BusId, Bus>,
    pub(crate) clips: &'a [AudioClip],
    pub(crate) midi_clips: &'a [Arc<MidiClip>],
    pub(crate) plugins: &'a PluginMap,
    pub(crate) tempo_map: &'a TempoMap,
    pub(crate) sample_rate: u32,
    pub(crate) any_solo: bool,
    /// Busses with a summing buffer this block; the tail of `busses` past
    /// this index is not rendered.
    pub(crate) active_busses: usize,
    /// The engine's aux-send table (a lock-free snapshot the caller
    /// loaded once for this block).
    pub(crate) aux_sends: &'a [AuxSend],
    pub(crate) sidechain_routes: &'a [SidechainRoute],
    /// The take-comp playback plan (epic #15, doc #165) — a lock-free
    /// snapshot the caller loaded once for this block. Names, per track,
    /// which recorded take is audible over which slice of a loop slot.
    /// Empty ⇒ no take groups exist and the block renders exactly as it
    /// did before, so projects without take lanes pay nothing. The live
    /// callback and the offline bounce load the *same* published table,
    /// which is what makes a comp bounce the way it plays.
    pub(crate) take_comp: &'a CompRenderTable,
    /// Timeline frame the block starts on.
    pub(crate) playhead: u64,
    pub(crate) frames: usize,
    pub(crate) latency_comp: &'a LatencyComp,
    pub(crate) automation: &'a AutomationSnapshot,
}

/// The caller's pre-allocated scratch, mutably borrowed for one block.
/// The fields are disjoint on purpose: a phase can hold the port scratch
/// and the bus buffers at once without any copying.
pub(crate) struct BlockScratch<'a> {
    /// Interleaved output, exactly `frames * channels` samples, cleared
    /// by the caller before the first block.
    pub(crate) data: &'a mut [f32],
    pub(crate) track_buf_l: &'a mut [f32],
    pub(crate) track_buf_r: &'a mut [f32],
    pub(crate) bus_bufs: &'a mut BusBufs,
    pub(crate) port_scratch: &'a mut [(Vec<f32>, Vec<f32>)],
    pub(crate) note_event_buf: &'a mut Vec<PendingNoteEvent>,
    pub(crate) sidechain: &'a mut SidechainTaps,
    /// Dry-signal staging for the bypass crossfades (`crate::bypass`).
    /// Pre-allocated by the scratch's owner so a bypass transition never
    /// allocates on the audio thread.
    pub(crate) fx_dry: &'a mut FxDryScratch,
}

/// The frames at which this block evaluates automation.
///
/// Automation is evaluated at the block's first frame and at the
/// next-block-start frame; the gain ramp sweeps between them. Because one
/// block's end frame equals the next block's start frame, consecutive
/// blocks chain into one continuous sweep.
///
/// Post-PDC parameters (fader / pan / mute, applied at sum time after the
/// delay lines) act on audio whose timeline position is one comp stage
/// older than the raw playhead: track/sub gains meet their audio
/// `track_stage()` late, bus gains a further `bus_stage` late. Evaluating
/// their automation at the comp-delayed position makes a drawn move land
/// on the audio it was drawn against (doc #260 finding #9). Pre-chain
/// parameters (plugin params, MIDI) keep the raw positions. Zero-latency
/// projects shift by 0 and stay bit-identical.
#[derive(Clone, Copy)]
pub(crate) struct EvalPositions {
    /// Raw block start — plugin params and the pre-chain stages.
    pub(crate) eval_start: u64,
    /// Track / sub-track fader, pan and mute window.
    pub(crate) gain_start: u64,
    pub(crate) gain_end: u64,
    /// Bus fader, pan and mute window.
    pub(crate) bus_start: u64,
    pub(crate) bus_end: u64,
}

impl EvalPositions {
    pub(crate) fn new(latency_comp: &LatencyComp, playhead: u64, frames: usize) -> Self {
        let eval_start = playhead;
        let eval_end = playhead + frames as u64;
        let track_gain_shift = latency_comp.track_stage();
        let bus_gain_shift = latency_comp.max_latency();
        Self {
            eval_start,
            gain_start: eval_start.saturating_sub(track_gain_shift),
            gain_end: eval_end.saturating_sub(track_gain_shift),
            bus_start: eval_start.saturating_sub(bus_gain_shift),
            bus_end: eval_end.saturating_sub(bus_gain_shift),
        }
    }
}

/// [`BlockInputs`] plus the evaluation positions derived from it. Built
/// once by `render_block` and passed to every phase.
pub(crate) struct BlockCtx<'a> {
    pub(crate) inputs: BlockInputs<'a>,
    pub(crate) evals: EvalPositions,
}

impl<'a> BlockCtx<'a> {
    pub(crate) fn new(inputs: BlockInputs<'a>) -> Self {
        let evals = EvalPositions::new(inputs.latency_comp, inputs.playhead, inputs.frames);
        Self { inputs, evals }
    }
}

/// Run an insert-FX chain in place over one stereo buffer pair: apply each
/// plugin's automation, resolve its sidechain key, and process. Shared by
/// the track, sub-track and bus passes — the only difference between them
/// is which buffer they hand over.
///
/// Bypass runs at two levels, both click-free (ba doc #275 finding X3):
///
/// - `chain` is the whole chain's bypass — the track's / sub-track's /
///   bus's `fx_bypass`. Settled-bypassed, the chain is skipped and the
///   buffer passes through untouched; mid-transition, the chain runs and
///   its output is crossfaded against the chain's own input, so a reverb
///   tail fades out over [`crate::bypass::BYPASS_FADE_MS`] instead of
///   being truncated.
/// - each slot carries its own [`crate::bypass::BypassFade`], applied the
///   same way over that one plugin's input. A slot whose plugin declares
///   a bypass parameter of its own is driven through that parameter
///   instead of being skipped, which keeps its latency (and therefore the
///   whole comp table) untouched across the toggle.
///
/// Whether a SILENCED `source` must still render for its sidechain key
/// (code review MIX-05): some enabled route taps it AND that route's
/// plugin is present and not bypassed AND the chain holding it runs this
/// block ([`consumer_chain_runs`]) — a bypassed (or removed) keyed plugin,
/// or one in a bypassed chain or on a silenced track / bus, does not read
/// its key, so a muted source feeding only such consumers renders nothing
/// (FU-M3c, FU-A5c). Audible sources capture whenever they are tapped;
/// this only gates the key-only render. A keyed plugin being un-bypassed
/// (or its track un-muted) gets its first key one block later, during its
/// fade-in. Allocation- and lock-free.
pub(crate) fn key_consumed(
    ctx: &BlockCtx<'_>,
    sidechain: &SidechainTaps,
    strategy: &RenderStrategy<'_>,
    source: SendSource,
) -> bool {
    sidechain.is_tapped(source)
        && ctx.inputs.sidechain_routes.iter().any(|r| {
            r.enabled
                && r.source == source
                && ctx
                    .inputs
                    .plugins
                    .get(&r.plugin)
                    .is_some_and(|slot| !slot.bypass.bypassed())
                && consumer_chain_runs(ctx, sidechain, strategy, r.plugin)
        })
}

/// Whether the chain holding the keyed `plugin` runs this block, so it
/// would actually read a key (FU-A5c). False only when that is certain:
/// the owning track's / bus's whole FX chain is bypassed (an instrument
/// track's instrument slot excepted — chain bypass never skips it), or
/// the owner is silenced and would skip its chain — the same
/// `track_disposition` / `bus_disposition` call its own pass makes.
///
/// Conservative everywhere else, since a wrong `false` drops a key
/// someone hears: a silenced owner that is itself a key source may render
/// key-only and run its chain after all (not followed, so two muted
/// tracks keying each other can't recurse), a sub-track's silence depends
/// on its parent's, and master's chain has no mute — all count as
/// running.
fn consumer_chain_runs(
    ctx: &BlockCtx<'_>,
    sidechain: &SidechainTaps,
    strategy: &RenderStrategy<'_>,
    plugin: PluginInstanceId,
) -> bool {
    use crate::mixer::automation_apply::auto_muted;
    use resonance_common::AutomationTarget;

    let tracks = ctx.inputs.tracks;
    if let Some(track) = tracks.values().find(|t| t.plugins().contains(&plugin)) {
        let is_instrument = track.track_type == TrackType::Instrument
            && track.plugins().first() == Some(&plugin);
        if !is_instrument && track.fx_bypass().bypassed() {
            return false;
        }
        let keys_itself = |id: TrackId| sidechain.is_tapped(SendSource::Track(id));
        if track.sub_track_of.is_some()
            || keys_itself(track.id)
            || tracks.values().any(|t| {
                matches!(t.sub_track_of, Some((p, _)) if p == track.id) && keys_itself(t.id)
            })
        {
            return true;
        }
        let auto_mute = auto_muted(
            ctx.inputs.automation,
            AutomationTarget::TrackMute(track.id),
            ctx.evals.gain_start,
        );
        return strategy
            .track_disposition(track, ctx.inputs.any_solo, None, auto_mute)
            .is_some_and(|d| is_instrument || !d.discard_after_instrument);
    }
    if let Some(bus) = ctx.inputs.busses.values().find(|b| b.plugin_ids.contains(&plugin)) {
        if bus.fx_bypass().bypassed() {
            return false;
        }
        if sidechain.is_tapped(SendSource::Bus(bus.id)) {
            return true;
        }
        let auto_mute = auto_muted(
            ctx.inputs.automation,
            AutomationTarget::BusMute(bus.id),
            ctx.evals.bus_start,
        );
        return strategy.bus_disposition(bus, None, auto_mute).is_some();
    }
    true
}

/// Returns whether any plugin actually ran; a plugin whose instance is
/// missing, or (live) whose lock is contended, is skipped for this block.
pub(crate) fn run_fx_chain(
    ids: impl Iterator<Item = PluginInstanceId>,
    chain: &BypassFade,
    ctx: &BlockCtx<'_>,
    sidechain: &SidechainTaps,
    bufs: (&mut [f32], &mut [f32]),
    dry: &mut FxDryScratch,
    strategy: &mut RenderStrategy<'_>,
) -> bool {
    let frames = ctx.inputs.frames;
    let sample_rate = ctx.inputs.sample_rate;
    let live = strategy.is_live();
    let chain_stage = chain.stage(sample_rate, frames, live);
    let (chain_dry, slot_dry) = dry.split();
    run_faded(chain_stage, frames, bufs, chain_dry, |buf_l, buf_r| {
        let mut ran = false;
        for plugin_id in ids {
            let Some(slot) = ctx.inputs.plugins.get(&plugin_id) else {
                continue;
            };
            let slot_stage = slot.stage(sample_rate, frames, live);
            if slot_stage == FadeStage::Dry {
                continue;
            }
            let Some(mut inst) = strategy.lock_fx(slot) else {
                continue;
            };
            apply_plugin_params(
                &mut inst,
                ctx.inputs.automation,
                plugin_id,
                ctx.evals.eval_start,
            );
            slot.sync_own_bypass(&mut inst.0);
            // An external key, when this instance is routed one and
            // actually declares a key port. The taps are borrowed
            // immutably here and mutably at the capture phases, so the two
            // never overlap.
            let key = sidechain.key_for(ctx.inputs.sidechain_routes, plugin_id);
            ran |= run_faded(
                slot_stage,
                frames,
                (&mut *buf_l, &mut *buf_r),
                (&mut *slot_dry.0, &mut *slot_dry.1),
                |l, r| {
                    let mut outs = [StereoBufMut {
                        left: &mut l[..frames],
                        right: &mut r[..frames],
                    }];
                    inst.0.process_multi_with_key(&mut outs, key, frames);
                    true
                },
            );
        }
        ran
    })
}
