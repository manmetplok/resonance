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

use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::clap_host::{StereoBufMut, SyncClapInstance};
use crate::engine::AutomationSnapshot;
use crate::latency::LatencyComp;
use crate::mixer::automation_apply::apply_plugin_params;
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
    pub(crate) midi_clips: &'a [MidiClip],
    pub(crate) plugins: &'a IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>,
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
/// Returns whether any plugin actually ran; a plugin whose instance is
/// missing, or (live) whose lock is contended, is skipped for this block.
pub(crate) fn run_fx_chain(
    ids: impl Iterator<Item = PluginInstanceId>,
    ctx: &BlockCtx<'_>,
    sidechain: &SidechainTaps,
    bufs: (&mut [f32], &mut [f32]),
    strategy: &mut RenderStrategy<'_>,
) -> bool {
    let frames = ctx.inputs.frames;
    let (buf_l, buf_r) = bufs;
    let mut ran = false;
    for plugin_id in ids {
        let Some(mutex) = ctx.inputs.plugins.get(&plugin_id) else {
            continue;
        };
        let Some(mut inst) = strategy.lock_fx(mutex) else {
            continue;
        };
        apply_plugin_params(
            &mut inst,
            ctx.inputs.automation,
            plugin_id,
            ctx.evals.eval_start,
        );
        // An external key, when this instance is routed one and actually
        // declares a key port. The taps are borrowed immutably here and
        // mutably at the capture phases, so the two never overlap.
        let key = sidechain.key_for(ctx.inputs.sidechain_routes, plugin_id);
        let mut outs = [StereoBufMut {
            left: &mut buf_l[..frames],
            right: &mut buf_r[..frames],
        }];
        inst.0.process_multi_with_key(&mut outs, key, frames);
        ran = true;
    }
    ran
}
