//! The mix stage of the stopped and count-in branches (code review RT-14).
//!
//! While the transport is stopped (or counting in) the callback renders no
//! timeline, but monitored inputs and live-played instruments still sound.
//! They used to be summed straight into the output, skipping their bus,
//! their aux sends and the master chain: a keyboard played while stopped
//! sounded different from while rolling (no reverb send, no bus
//! compressor, no master chain), and stopping cut every bus and master
//! reverb tail with a step.
//!
//! Now the per-track passes ([`mix_monitor_passthrough`] /
//! [`mix_idle_instruments`]) route each track through
//! [`MixTargets`] — its output bus or master, plus its aux sends — and
//! this stage then runs the arrangement's own bus pass and the master
//! chain over the result: the same mixer as while rolling, minus the
//! timeline.
//!
//! **Cost.** The stage runs only while something is audible, and for
//! [`IDLE_HOLD_SECS`] after the last audible block — including the stop
//! itself, so the busses' and master's tails ring out — then the stopped
//! branch is back to running no bus or master plugin at all. The final
//! held block fades to zero, so a tail that outlasts the hold ends without
//! a step. Busses run without delay compensation (see
//! `TransportContinuity::idle_comp`).
//!
//! [`mix_monitor_passthrough`]: crate::mixer::monitor::mix_monitor_passthrough
//! [`mix_idle_instruments`]: crate::mixer::monitor::mix_idle_instruments
//! [`MixTargets`]: crate::mixer::monitor::MixTargets

use crate::engine::RenderGraph;
use crate::limits::IDLE_HOLD_SECS;
use crate::mixer::master::apply_master_fx_chain;
use crate::mixer::render::bus_pass::render_bus_pass;
use crate::mixer::render::context::{BlockCtx, BlockInputs, BlockScratch};
use crate::mixer::render_core::RenderStrategy;
use crate::mixer::take_comp::CompRenderTable;
use crate::types::*;

use super::context::{BlockTiming, CallbackInputs, CallbackScratch};

/// Busses with a summing buffer this block.
pub(super) fn active_busses(graph: &RenderGraph, scratch: &CallbackScratch<'_>) -> usize {
    graph.busses.len().min(scratch.bus_bufs.len())
}

/// Before the track passes: zero the active busses' summing buffers, and
/// swap the sidechain banks as a playing block does, so a keyed bus or
/// master plugin reads this run's captures and never a key frozen from
/// the last playing block.
pub(super) fn begin(
    scratch: &mut CallbackScratch<'_>,
    graph: &RenderGraph,
    routes: &[SidechainRoute],
    frames: usize,
) {
    let active = active_busses(graph, scratch);
    for (l, r) in scratch.bus_bufs.iter_mut().take(active) {
        l[..frames].fill(0.0);
        r[..frames].fill(0.0);
    }
    scratch.sidechain.begin_block(routes);
}

/// What the stage reads, loaded once by the branch.
pub(super) struct IdleMixInputs<'a> {
    pub(super) graph: &'a RenderGraph,
    pub(super) aux_sends: &'a [AuxSend],
    pub(super) sidechain_routes: &'a [SidechainRoute],
    pub(super) take_comp: &'a CompRenderTable,
    pub(super) playhead: u64,
    pub(super) frames: usize,
}

/// After the track passes: run the busses and the master chain if anything
/// was `audible` this block or the tail hold is still running. Returns
/// whether the stage ran (the branch then applies the master volume).
pub(super) fn finish(
    inputs: &CallbackInputs<'_>,
    scratch: &mut CallbackScratch<'_>,
    timing: &BlockTiming<'_>,
    stage: IdleMixInputs<'_>,
    audible: bool,
) -> bool {
    let sample_rate = inputs.sample_rate;
    if audible {
        scratch.continuity.tail_hold = IDLE_HOLD_SECS as usize * sample_rate as usize;
    }
    let hold = scratch.continuity.tail_hold;
    if hold == 0 {
        return false;
    }
    let frames = stage.frames;
    let channels = inputs.channels;
    let graph = stage.graph;
    let active = active_busses(graph, scratch);

    if active > 0 {
        let automation = inputs.automation.load();
        let block = BlockInputs {
            channels,
            tracks: &graph.tracks,
            busses: &graph.busses,
            clips: &[],
            midi_clips: &[],
            plugins: &graph.plugins,
            tempo_map: timing.map,
            sample_rate,
            any_solo: snapshot_top_level_solo(graph.tracks.values().map(|t| &**t)),
            active_busses: active,
            aux_sends: stage.aux_sends,
            sidechain_routes: stage.sidechain_routes,
            take_comp: stage.take_comp,
            playhead: stage.playhead,
            frames,
            latency_comp: &scratch.continuity.idle_comp,
            automation: &automation,
        };
        let strategy = RenderStrategy::Live {
            transport_snap: timing.transport,
            monitor_temp: &[],
            monitor_frames: 0,
            input_channels: 0,
        };
        let mut block_scratch = BlockScratch {
            data: &mut scratch.data[..frames * channels],
            bus_bufs: &mut *scratch.bus_bufs,
            slots: scratch.track_slots.current(),
            stash: Some(&mut *scratch.midi_stash),
            port_scratch: &mut *scratch.port_scratch,
            note_event_buf: &mut *scratch.note_event_buf,
            sidechain: &mut *scratch.sidechain,
            fx_dry: &mut *scratch.fx_dry,
            pool: Some(scratch.pool),
        };
        render_bus_pass(&BlockCtx::new(block), &mut block_scratch, &strategy);
    }

    apply_master_fx_chain(
        scratch.data,
        channels,
        &graph.master,
        &graph.plugins,
        scratch.track_buf_l,
        scratch.track_buf_r,
        scratch.fx_dry,
        timing.transport,
        stage.sidechain_routes,
        scratch.sidechain,
        &inputs.shared.master_fx_bypass,
        sample_rate,
    );

    if !audible {
        // Only tails are playing. The hold's last block fades them to
        // zero, so one that outlasts the hold stops without a step.
        if hold <= frames {
            let inv = 1.0 / frames.max(1) as f32;
            for (f, frame) in scratch.data[..frames * channels]
                .chunks_mut(channels)
                .enumerate()
            {
                let g = 1.0 - (f + 1) as f32 * inv;
                for s in frame {
                    *s *= g;
                }
            }
        }
        scratch.continuity.tail_hold = hold.saturating_sub(frames);
    }
    true
}
