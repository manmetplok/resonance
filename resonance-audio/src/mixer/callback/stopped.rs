//! Stopped branch: no transport, but armed tracks still monitor and
//! instruments still play live notes — through their busses, aux sends
//! and the master chain, as while rolling (code review RT-14).

use std::sync::atomic::Ordering;

use crate::mixer::common::panic_instrument_tracks;
use crate::mixer::master::apply_master_volume_and_peaks;
use crate::mixer::monitor::{mix_idle_instruments, mix_monitor_passthrough, MixTargets};

use super::context::{BlockTiming, CallbackInputs, CallbackScratch, MonitorRead};
use super::idle_mix::{self, IdleMixInputs};

/// Even when stopped, output monitored audio for armed tracks, and play
/// the live notes (piano-roll preview, MIDI controller) instruments were
/// handed. Each goes where its track goes — bus or master, plus aux
/// sends — and the busses and master chain run over the result while
/// anything sounds, and for a few seconds after so their tails (and the
/// tails of the run that just stopped) ring out ([`idle_mix`]). No
/// timeline render, no metronome, and the playhead stays where it is.
pub(super) fn render_stopped_block(
    inputs: &CallbackInputs<'_>,
    scratch: &mut CallbackScratch<'_>,
    timing: &BlockTiming<'_>,
    monitor: MonitorRead,
    playhead: u64,
    frames: usize,
) {
    let shared = inputs.shared;
    let flush = scratch.continuity.was_rolling();
    let monitor_on = monitor.frames > 0 && shared.monitoring.load(Ordering::Relaxed);
    // Tracks, busses and plugins come from the render graph (ARCH-02
    // A2-6/A2-7): a load that cannot miss, so a pending stop flush always
    // runs here.
    let graph = shared.graph.load();
    let tracks_guard = &*graph.tracks;
    let plugins_guard = &*graph.plugins;
    // The transport just stopped (code review MIX-06). The engine's own
    // Stop panic `try_lock`s each instrument and skips one the audio
    // thread was holding; this one is issued from the audio thread, whose
    // MIDI stash parks it on contention instead of losing it.
    if flush {
        panic_instrument_tracks(tracks_guard, plugins_guard, scratch.midi_stash, false);
        scratch.continuity.stopped();
        // Let the busses and the master chain ring out what the run left
        // in them, rather than cutting it with a step (RT-14).
        scratch.continuity.tail_hold =
            crate::limits::IDLE_HOLD_SECS as usize * inputs.sample_rate as usize;
    }

    let aux_guard = shared.aux_sends.load();
    let routes_guard = shared.sidechain_routes.load();
    let take_comp_guard = shared.take_comp.load();
    idle_mix::begin(scratch, &graph, &routes_guard, frames);
    let active = idle_mix::active_busses(&graph, scratch);

    let mut out = MixTargets {
        data: &mut *scratch.data,
        channels: inputs.channels,
        bus_bufs: &mut scratch.bus_bufs[..active],
        busses: &graph.busses,
        aux_sends: &aux_guard,
    };
    let any_monitor = monitor_on
        && mix_monitor_passthrough(
            &mut out,
            tracks_guard,
            plugins_guard,
            scratch.monitor_temp,
            monitor.frames,
            monitor.input_channels,
            scratch.track_buf_l,
            scratch.track_buf_r,
            scratch.fx_dry,
            timing.transport,
            inputs.sample_rate,
        );
    // Instruments with live notes pending or releasing, and chains with a
    // plugin that asked for `process()` (`clap_host.request_process`),
    // independent of monitoring and of whether an input device exists
    // (code review MIX-08).
    let any_instrument = mix_idle_instruments(
        &mut out,
        frames,
        tracks_guard,
        plugins_guard,
        scratch.midi_stash,
        scratch.track_buf_l,
        scratch.track_buf_r,
        scratch.fx_dry,
        timing.transport,
        inputs.sample_rate,
        monitor_on,
    );
    let audible = any_monitor || any_instrument;
    let staged = idle_mix::finish(
        inputs,
        scratch,
        timing,
        IdleMixInputs {
            graph: &graph,
            aux_sends: &aux_guard,
            sidechain_routes: &routes_guard,
            take_comp: &take_comp_guard,
            playhead,
            frames,
        },
        audible,
    );
    if audible || staged {
        // Apply master volume and compute master peak levels.
        apply_master_volume_and_peaks(scratch.data, inputs.channels, shared, None);
    }
}
