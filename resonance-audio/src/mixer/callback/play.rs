//! Playing branch: take the state locks, snapshot the wait-free tables,
//! render the arrangement (stitched across a loop seam when one falls
//! inside this buffer) and hand the buffer to the master passes.

use std::sync::atomic::Ordering;

use crate::cycle_load::{try_read_counted, StateMap};
use crate::mixer::common::{advance_playhead_silent, commit_playhead, panic_instrument_tracks};
use crate::mixer::render_core::BlockInputs;
use crate::types::any_top_level_solo;

use super::context::{BlockTiming, CallbackInputs, CallbackScratch, MonitorRead};
use super::master_pass::{run_master_passes, MasterTail};
use super::seam;

/// `playhead` is the callback's single observation of the transport for
/// this block (`mix_audio` loads it once); both publishes below are
/// conditional on it still being current, so a reposition from the control
/// thread that lands mid-block wins over this block's advance.
pub(super) fn render_playing_block(
    inputs: &CallbackInputs<'_>,
    scratch: &mut CallbackScratch<'_>,
    timing: &BlockTiming<'_>,
    monitor: MonitorRead,
    playhead: u64,
    frames: usize,
) {
    let shared = inputs.shared;

    // Every map is tried, so each one that misses is attributed
    // (`SharedState::lock_misses`); the render decision stays
    // all-or-nothing.
    let misses = &shared.lock_misses;
    let tracks_guard = try_read_counted(inputs.tracks, StateMap::Tracks, misses);
    let busses_guard = try_read_counted(inputs.busses, StateMap::Busses, misses);
    let clips_guard = try_read_counted(inputs.clips, StateMap::Clips, misses);
    let midi_clips_guard = try_read_counted(inputs.midi_clips, StateMap::MidiClips, misses);
    let plugins_guard = try_read_counted(inputs.plugins, StateMap::Plugins, misses);
    let (
        Some(tracks_guard),
        Some(busses_guard),
        Some(clips_guard),
        Some(midi_clips_guard),
        Some(plugins_guard),
    ) = (
        tracks_guard,
        busses_guard,
        clips_guard,
        midi_clips_guard,
        plugins_guard,
    )
    else {
        // Lock contended -- advance playhead to avoid desync, output
        // silence this buffer.
        shared.render_skip_cycles.fetch_add(1, Ordering::Relaxed);
        let new_playhead = advance_playhead_silent(shared, playhead, frames as u64);
        commit_playhead(shared, playhead, new_playhead);
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

    // Playhead discontinuity (code review MIX-06): a seek / relocate, or
    // a block that advanced without rendering (lock contention, the A/B
    // reference), means NoteOffs were never collected for whatever is
    // held. Flush every instrument from here — the audio thread owns the
    // MIDI stash, so a contended instrument gets the panic parked rather
    // than skipped — and drop captured keys, which belong to the old
    // position.
    if scratch.continuity.jumped(playhead) {
        panic_instrument_tracks(&tracks_guard, &plugins_guard, scratch.midi_stash, false);
        scratch.sidechain.clear();
    }

    // Snapshot the parameter-automation lanes once per buffer (wait-free,
    // published by the engine thread on lane edits). Held across both seam
    // sub-blocks and the master pass so the whole buffer agrees.
    let auto_guard = inputs.automation.load();

    // Snapshot the take-comp playback table once per buffer (wait-free; the
    // engine thread republishes it on every capture / comp edit /
    // active-take change). Empty when no take groups exist, so projects
    // without comps pay nothing.
    let take_comp_guard = shared.take_comp.load();

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
        take_comp: &take_comp_guard,
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

    // A lost commit means the control thread repositioned the transport
    // while this block rendered; the next block starts from its position
    // and, through `continuity`, flushes the voices first (MIX-06).
    let committed = commit_playhead(shared, playhead, new_playhead);
    scratch.continuity.rendered(new_playhead, committed);
}
