//! Stopped branch: no transport, but armed tracks still monitor.

use std::sync::atomic::Ordering;

use crate::mixer::common::panic_instrument_tracks;
use crate::mixer::master::apply_master_volume_and_peaks;
use crate::mixer::monitor::mix_monitor_passthrough;

use super::context::{BlockTiming, CallbackInputs, CallbackScratch, MonitorRead};

/// Even when stopped, output monitored audio for armed tracks. Nothing
/// else runs: no timeline render, no master FX, no metronome, and the
/// playhead stays where it is.
pub(super) fn render_stopped_block(
    inputs: &CallbackInputs<'_>,
    scratch: &mut CallbackScratch<'_>,
    timing: &BlockTiming<'_>,
    monitor: MonitorRead,
) {
    let shared = inputs.shared;
    let flush = scratch.continuity.was_rolling();
    let monitor_on = monitor.frames > 0 && shared.monitoring.load(Ordering::Relaxed);
    if !flush && !monitor_on {
        return;
    }
    let (Some(tracks_guard), Some(plugins_guard)) =
        (inputs.tracks.try_read(), inputs.plugins.try_read())
    else {
        // Contended: a pending stop flush stays armed for the next block.
        return;
    };
    // The transport just stopped (code review MIX-06). The engine's own
    // Stop panic `try_lock`s each instrument and skips one the audio
    // thread was holding; this one is issued from the audio thread, whose
    // MIDI stash parks it on contention instead of losing it.
    if flush {
        panic_instrument_tracks(&tracks_guard, &plugins_guard, scratch.midi_stash);
        scratch.continuity.stopped();
    }
    if !monitor_on {
        return;
    }
    let any_monitor = mix_monitor_passthrough(
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
    if any_monitor {
        // Apply master volume and compute master peak levels.
        apply_master_volume_and_peaks(scratch.data, inputs.channels, shared, None);
    }
}
