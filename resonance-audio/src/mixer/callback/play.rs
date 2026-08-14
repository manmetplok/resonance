//! Playing branch: take the state locks, snapshot the wait-free tables,
//! render the arrangement (stitched across a loop seam when one falls
//! inside this buffer) and hand the buffer to the master passes.

use std::sync::atomic::Ordering;

use crate::mixer::common::advance_playhead_silent;
use crate::mixer::render_core::BlockInputs;
use crate::types::any_top_level_solo;

use super::context::{BlockTiming, CallbackInputs, CallbackScratch, MonitorRead};
use super::master_pass::{run_master_passes, MasterTail};
use super::seam;

pub(super) fn render_playing_block(
    inputs: &CallbackInputs<'_>,
    scratch: &mut CallbackScratch<'_>,
    timing: &BlockTiming<'_>,
    monitor: MonitorRead,
    frames: usize,
) {
    let shared = inputs.shared;
    let playhead = shared.playhead.load(Ordering::Relaxed);

    let (
        Some(tracks_guard),
        Some(busses_guard),
        Some(clips_guard),
        Some(midi_clips_guard),
        Some(plugins_guard),
    ) = (
        inputs.tracks.try_read(),
        inputs.busses.try_read(),
        inputs.clips.try_read(),
        inputs.midi_clips.try_read(),
        inputs.plugins.try_read(),
    )
    else {
        // Lock contended -- advance playhead to avoid desync, output
        // silence this buffer.
        shared.render_skip_cycles.fetch_add(1, Ordering::Relaxed);
        let new_playhead = advance_playhead_silent(shared, playhead, frames as u64);
        shared.playhead.store(new_playhead, Ordering::Relaxed);
        return;
    };

    let active_busses = busses_guard.len().min(scratch.bus_bufs.len());

    // Snapshot the plugin-delay-compensation table once per buffer.
    // Wait-free load; the engine thread publishes a new table whenever the
    // track/bus/plugin topology changes.
    let comp_guard = inputs.latency_comp.load();

    // Snapshot the aux-send table once per buffer (wait-free; the engine
    // thread republishes it on every send add/remove/clear).
    let aux_guard = shared.aux_sends.load();

    // Sidechain routes for this block, plus the bank swap that makes every
    // key read the PREVIOUS block's capture (see `types::sidechain` for why
    // the key is deliberately one block old).
    let sidechain_guard = shared.sidechain_routes.load();
    scratch.sidechain.begin_block(&sidechain_guard);

    // Snapshot the parameter-automation lanes once per buffer (wait-free,
    // published by the engine thread on lane edits). Held across both seam
    // sub-blocks and the master pass so the whole buffer agrees.
    let auto_guard = inputs.automation.load();

    let block = BlockInputs {
        channels: inputs.channels,
        tracks: &tracks_guard,
        busses: &busses_guard,
        clips: &clips_guard,
        midi_clips: &midi_clips_guard,
        plugins: &plugins_guard,
        tempo_map: timing.map,
        sample_rate: inputs.sample_rate,
        any_solo: any_top_level_solo(tracks_guard.values()),
        active_busses,
        aux_sends: &aux_guard,
        sidechain_routes: &sidechain_guard,
        playhead,
        frames,
        latency_comp: &comp_guard,
        automation: &auto_guard,
    };

    let split = seam::detect(shared, playhead, frames);
    let new_playhead = seam::render_arrangement(block, scratch, timing.transport, monitor, split);

    run_master_passes(
        inputs,
        scratch,
        timing,
        MasterTail {
            plugins_guard,
            sidechain_routes: &sidechain_guard,
            automation: &auto_guard,
            max_latency: comp_guard.max_latency(),
            playhead,
            frames,
            seam: split,
        },
    );

    shared.playhead.store(new_playhead, Ordering::Relaxed);
}
