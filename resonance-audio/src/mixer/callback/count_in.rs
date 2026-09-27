//! Count-in branch: hold the playhead, skip track/clip rendering, and
//! emit metronome ticks from a count-in-local elapsed counter so the last
//! click lands exactly one beat before the punch-in line.
//!
//! `count_in_active` stays set across the brief window between
//! `count_in_remaining` hitting zero and the engine control thread opening
//! the recording stream, so the playhead stays pinned to the punch-in line
//! throughout.

use std::sync::atomic::Ordering;

use crate::cycle_load::{try_read_counted, StateMap};
use crate::mixer::click::render_count_in_clicks;
use crate::mixer::master::apply_master_volume_and_peaks;
use crate::mixer::monitor::mix_monitor_passthrough;

use super::context::{BlockTiming, CallbackInputs, CallbackScratch, MonitorRead};

pub(super) fn render_count_in_block(
    inputs: &CallbackInputs<'_>,
    scratch: &mut CallbackScratch<'_>,
    timing: &BlockTiming<'_>,
    monitor: MonitorRead,
    frames: usize,
) {
    let shared = inputs.shared;
    let count_in_remaining = shared.count_in_remaining.load(Ordering::Relaxed);
    let count_in_total = shared.count_in_total.load(Ordering::Relaxed);
    let elapsed_at_start = count_in_total.saturating_sub(count_in_remaining);
    let click_frames = (frames as u64).min(count_in_remaining) as usize;

    // Monitor pass-through so the performer can hear themselves warm up
    // during the count-in. Mirrors the playing=false monitor branch but is
    // best-effort on lock contention — dropping monitor audio for one
    // buffer is acceptable; losing the count-in tick is not.
    if monitor.frames > 0 && shared.monitoring.load(Ordering::Relaxed) {
        let misses = &shared.lock_misses;
        let graph = shared.graph.load();
        let plugins_guard = try_read_counted(inputs.plugins, StateMap::Plugins, misses);
        if let Some(plugins_guard) = plugins_guard {
            mix_monitor_passthrough(
                scratch.data,
                inputs.channels,
                &graph.tracks,
                &plugins_guard,
                scratch.monitor_temp,
                monitor.frames,
                monitor.input_channels,
                scratch.track_buf_l,
                scratch.track_buf_r,
                scratch.fx_dry,
                timing.transport,
                inputs.sample_rate,
            );
        }
    }

    // Metronome click synthesis using a count-in-local timeline. Beats are
    // indexed from the start of the count-in; with `count_in_total ==
    // precount_bars * numerator * spb`, the final click in the loop lands
    // at elapsed `(precount_bars * numerator - 1) * spb`, leaving exactly
    // one beat of silence before the punch-in line.
    render_count_in_clicks(
        scratch.data,
        inputs.channels,
        inputs.sample_rate,
        timing.map,
        elapsed_at_start,
        click_frames,
    );

    // Master volume + peaks so the count-in audio hits meters the same way
    // normal playback does.
    apply_master_volume_and_peaks(scratch.data, inputs.channels, shared, None);

    // Decrement the remaining-clicks counter. Once it hits zero the
    // metronome goes quiet, but `count_in_active` keeps the mixer in this
    // branch until the engine control thread has actually opened the
    // recording stream — that cross-thread handoff is what guarantees the
    // playhead doesn't start advancing until recording is armed.
    let new_remaining = count_in_remaining.saturating_sub(frames as u64);
    shared
        .count_in_remaining
        .store(new_remaining, Ordering::Relaxed);
}
