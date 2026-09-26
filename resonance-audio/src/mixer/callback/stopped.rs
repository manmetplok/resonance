//! Stopped branch: no transport, but armed tracks still monitor and
//! instruments still play live notes.

use std::sync::atomic::Ordering;

use crate::cycle_load::{try_read_counted, StateMap};
use crate::mixer::common::panic_instrument_tracks;
use crate::mixer::master::apply_master_volume_and_peaks;
use crate::mixer::monitor::{mix_idle_instruments, mix_monitor_passthrough};

use super::context::{BlockTiming, CallbackInputs, CallbackScratch, MonitorRead};

/// Even when stopped, output monitored audio for armed tracks, and play
/// the live notes (piano-roll preview, MIDI controller) instruments were
/// handed. Nothing else runs: no timeline render, no master FX, no
/// metronome, and the playhead stays where it is.
pub(super) fn render_stopped_block(
    inputs: &CallbackInputs<'_>,
    scratch: &mut CallbackScratch<'_>,
    timing: &BlockTiming<'_>,
    monitor: MonitorRead,
    frames: usize,
) {
    let shared = inputs.shared;
    let flush = scratch.continuity.was_rolling();
    let monitor_on = monitor.frames > 0 && shared.monitoring.load(Ordering::Relaxed);
    let misses = &shared.lock_misses;
    let tracks_guard = try_read_counted(inputs.tracks, StateMap::Tracks, misses);
    let plugins_guard = try_read_counted(inputs.plugins, StateMap::Plugins, misses);
    let (Some(tracks_guard), Some(plugins_guard)) = (tracks_guard, plugins_guard) else {
        // Contended: a pending stop flush stays armed for the next block.
        return;
    };
    // The transport just stopped (code review MIX-06). The engine's own
    // Stop panic `try_lock`s each instrument and skips one the audio
    // thread was holding; this one is issued from the audio thread, whose
    // MIDI stash parks it on contention instead of losing it.
    if flush {
        panic_instrument_tracks(&tracks_guard, &plugins_guard, scratch.midi_stash, false);
        scratch.continuity.stopped();
    }
    let any_monitor = monitor_on
        && mix_monitor_passthrough(
            scratch.data,
            inputs.channels,
            &tracks_guard,
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
    // Instruments with live notes pending or releasing, independent of
    // monitoring and of whether an input device exists (code review
    // MIX-08).
    let any_instrument = mix_idle_instruments(
        scratch.data,
        inputs.channels,
        frames,
        &tracks_guard,
        &plugins_guard,
        scratch.midi_stash,
        scratch.track_buf_l,
        scratch.track_buf_r,
        scratch.fx_dry,
        timing.transport,
        inputs.sample_rate,
        monitor_on,
    );
    if any_monitor || any_instrument {
        // Apply master volume and compute master peak levels.
        apply_master_volume_and_peaks(scratch.data, inputs.channels, shared, None);
    }
}
