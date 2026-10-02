//! Count-in branch: hold the playhead, skip track/clip rendering, and
//! emit metronome ticks from a count-in-local elapsed counter so the last
//! click lands exactly one beat before the punch-in line.
//!
//! A record count-in starts its take here, on the audio thread, at the
//! exact frame the count-in ends (code review RT-08). The engine thread
//! opened the recording session when the count-in started and armed
//! `SharedState::count_in_record_arm`; the block whose frames finish the
//! count-in moves the playhead on by the frames left after the count-in
//! ended, sets `recording` and leaves this branch, so the next block is
//! already a playing block on the punch-in timeline. Nothing waits for the
//! engine thread's tick or an input-stream rebuild.
//!
//! A count-in with nothing to record (no armed session) hands over the old
//! way: `count_in_active` stays set across the brief window between
//! `count_in_remaining` hitting zero and the engine control thread clearing
//! it, so the playhead stays pinned to the punch-in line throughout.

use std::sync::atomic::Ordering;

use crate::engine::count_in_arm;
use crate::mixer::click::render_count_in_clicks;
use crate::mixer::common::commit_playhead;
use crate::mixer::master::apply_master_volume_and_peaks;
use crate::mixer::monitor::mix_monitor_passthrough;

use super::context::{BlockTiming, CallbackInputs, CallbackScratch, MonitorRead};

pub(super) fn render_count_in_block(
    inputs: &CallbackInputs<'_>,
    scratch: &mut CallbackScratch<'_>,
    timing: &BlockTiming<'_>,
    monitor: MonitorRead,
    playhead: u64,
    frames: usize,
) {
    let shared = inputs.shared;
    let count_in_remaining = shared.count_in_remaining.load(Ordering::Relaxed);
    let count_in_total = shared.count_in_total.load(Ordering::Relaxed);
    let elapsed_at_start = count_in_total.saturating_sub(count_in_remaining);
    let click_frames = (frames as u64).min(count_in_remaining) as usize;

    // Monitor pass-through so the performer can hear themselves warm up
    // during the count-in. Mirrors the playing=false monitor branch but is
    // read entirely from the render graph (tracks and plugins, ARCH-02
    // B-3/B-4), a load that cannot fail, so it never drops a buffer.
    if monitor.frames > 0 && shared.monitoring.load(Ordering::Relaxed) {
        let graph = shared.graph.load();
        mix_monitor_passthrough(
            scratch.data,
            inputs.channels,
            &graph.tracks,
            &graph.plugins,
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
    // metronome goes quiet; a record count-in flips to recording right
    // below, a count-in with nothing to record waits for the engine thread
    // to clear `count_in_active`.
    let new_remaining = count_in_remaining.saturating_sub(frames as u64);
    shared
        .count_in_remaining
        .store(new_remaining, Ordering::Relaxed);

    // Record count-in: the count-in ended inside this block, so the take
    // starts at that exact frame. The `click_frames..frames` tail is
    // timeline already — silent here, but the playhead moves over it, so
    // the next block starts exactly where the performer's downbeat fell.
    if new_remaining == 0
        && shared
            .count_in_record_arm
            .compare_exchange(
                count_in_arm::ARMED,
                count_in_arm::FIRING,
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .is_ok()
    {
        let past_downbeat = (frames - click_frames) as u64;
        // A Seek that landed under this block wins, as in every branch.
        commit_playhead(shared, playhead, playhead + past_downbeat);
        shared.recording.store(true, Ordering::SeqCst);
        shared.count_in_active.store(false, Ordering::Release);
        shared
            .count_in_record_arm
            .store(count_in_arm::FIRED, Ordering::Release);
    }
}
