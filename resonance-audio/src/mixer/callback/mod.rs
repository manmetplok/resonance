//! The audio callback: [`mix_audio`] and the branches it delegates to
//! (ba todo #1255).
//!
//! `mix_audio` runs on the realtime audio thread, so everything under this
//! module is allocation-free and lock-free (the state locks are `try_read`
//! only, and a contended block drops out rather than waiting).
//!
//! The top-level function only sequences the block: pick up live MIDI,
//! clear the output, snapshot tempo / transport, read the monitor ring,
//! then pick exactly one branch. Each branch owns its own module:
//!
//! - [`reference`]: the A/B monitor replaces the whole output with the
//!   loaded reference and the callback ends there.
//! - [`count_in`]: the playhead is pinned and the count-in metronome plays
//!   over monitored input.
//! - [`stopped`]: no transport, but armed tracks still monitor.
//! - [`play`]: the arrangement render — including the lock-contended
//!   fallback that outputs silence and advances the playhead.
//! - [`master_pass`]: the whole-buffer tail of a playing block (master FX,
//!   metronome, master volume, mix meter).
//! - [`seam`]: loop-seam detection and the one-or-two sub-block stitch the
//!   playing branch renders through.
//! - [`monitor_input`]: this callback's read of the monitor ring.
//! - [`context`]: the [`CallbackInputs`] / [`CallbackScratch`] parameter
//!   structs, the per-block tempo/transport snapshot and the monitor read,
//!   shared by all of the above.

pub(crate) mod context;
mod count_in;
mod master_pass;
mod monitor_input;
mod play;
mod reference;
mod seam;
mod stopped;

use std::sync::atomic::Ordering;

use super::audition::mix_audition_overlay;
use super::live_midi::pickup_live_midi;
pub(crate) use context::{CallbackInputs, CallbackScratch, MixFn};
use context::BlockTiming;

/// Mix audio from all active clips into the output buffer.
///
/// Runs on the audio callback thread: allocation-free (it only writes into
/// the caller's pre-allocated [`CallbackScratch`]) and non-blocking.
pub(crate) fn mix_audio(inputs: CallbackInputs<'_>, scratch: &mut CallbackScratch<'_>) {
    resonance_common::flush_denormals();

    // Live hardware-MIDI pickup runs first — before any early-exit branch
    // (reference monitor, count-in, stopped) — so a live note reaches its
    // instrument within one quantum no matter which branch renders this
    // block (doc #260 finding #16).
    pickup_live_midi(
        inputs.live_midi_rx,
        inputs.live_midi_fwd,
        inputs.tracks,
        inputs.plugins,
        scratch.midi_stash,
        inputs.sample_rate,
        scratch.data.len() / inputs.channels.max(1),
    );

    // Zero the output buffer.
    scratch.data.fill(0.0);

    let frames = block_frames(&inputs, scratch.data.len());

    // The reference A/B monitor bypasses everything below it, including
    // the audition overlay.
    if reference::monitor_reference(&inputs, scratch, frames) {
        return;
    }

    // Snapshot tempo once per block. Holding the `ArcSwap` guard pins this
    // block's bar table for tempo-map-aware MIDI tick→sample conversion in
    // the rendering path; the engine thread publishes tempo changes
    // wait-free via `ArcSwap::store`.
    let playhead_now = inputs.shared.playhead.load(Ordering::Relaxed);
    let tempo_guard = inputs.tempo_map.load();
    let timing = BlockTiming::new(
        &tempo_guard,
        inputs.shared,
        playhead_now,
        inputs.sample_rate,
    );

    let monitor = monitor_input::read_monitor_input(&inputs, scratch, frames);

    if inputs.shared.count_in_active.load(Ordering::Relaxed) {
        count_in::render_count_in_block(&inputs, scratch, &timing, monitor, frames);
    } else if !inputs.shared.playing.load(Ordering::Relaxed) {
        stopped::render_stopped_block(&inputs, scratch, &timing, monitor);
    } else {
        play::render_playing_block(&inputs, scratch, &timing, monitor, frames);
    }

    // Audition preview overlay: summed in after the arrangement + master
    // pass, independent of transport, so a sample audition is audible
    // whether or not the project is rolling. Bypasses the master fader/FX
    // by design — it's a monitor-style preview, not part of the mix.
    mix_audition_overlay(scratch.data, inputs.channels, inputs.shared);
}

/// Frames this callback renders: what the backend asked for, clamped to
/// the scratch the engine pre-allocated.
///
/// If the backend hands us a buffer larger than our scratch can hold (only
/// possible under the `BufferSize::Default` fallback path), every
/// downstream calculation is clamped to what we can actually render.
/// Advancing by the raw frame count while only rendering `frames` would
/// race the playhead past the audio and silently miss loop seams. The
/// OS-side tail of the buffer stays at zero from the `fill(0.0)` above.
fn block_frames(inputs: &CallbackInputs<'_>, data_len: usize) -> usize {
    let raw_output_frames = data_len / inputs.channels;
    if raw_output_frames > inputs.buf_frames {
        log_oversize_buffer(raw_output_frames, inputs.buf_frames);
    }
    raw_output_frames.min(inputs.buf_frames)
}

/// One-shot warning when the backend requests a buffer larger than our
/// pre-allocated scratch. Latches via `AtomicBool` so the audio thread
/// doesn't flood stderr; subsequent oversize buffers are silently clamped
/// (audio plays slower than real-time, but does not desync).
fn log_oversize_buffer(requested: usize, scratch: usize) {
    use std::sync::atomic::AtomicBool;
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "audio: cpal requested buf={} frames but scratch is {} — clamping; audio will run slow",
            requested, scratch
        );
    }
}
