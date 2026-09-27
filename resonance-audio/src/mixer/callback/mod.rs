//! The audio callback: [`mix_audio`] and the branches it delegates to
//! (ba todo #1255).
//!
//! `mix_audio` runs on the realtime audio thread, so everything under this
//! module is allocation-free and lock-free: the render graph (every
//! project map since ARCH-02 B-5) is one wait-free load, so no block
//! ever waits on, or drops out for, a state lock.
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
//! - [`play`]: the arrangement render from the published render graph
//!   (a wait-free load: a playing block always renders).
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
    resonance_dsp::flush_denormals();

    // The one "offline render in progress" gate (code review MIX-02 /
    // ENG-05). Every offline renderer (export, stems, bounce in place,
    // freeze, measurement) drives the SAME live plugin instances from a
    // worker thread; while one holds the gate this callback must not
    // touch a plugin: no live-MIDI delivery, no monitor pass through the
    // armed tracks' chains, no arrangement render. It outputs silence and
    // holds the transport (Play / Record refuse engine-side while the
    // gate is up, so this is the backstop for a render that started an
    // instant before a Play landed, or an external MIDI-clock master).
    // One acquire load, never a lock, never a wait.
    let offline_render = inputs.shared.offline_render_active();

    // Live hardware-MIDI pickup runs first — before any early-exit branch
    // (reference monitor, count-in, stopped) — so a live note reaches its
    // instrument within one quantum no matter which branch renders this
    // block (doc #260 finding #16).
    if offline_render {
        forward_live_midi_unplayed(&inputs);
    } else {
        pickup_live_midi(
            inputs.live_midi_rx,
            inputs.live_midi_fwd,
            inputs.shared,
            scratch.midi_stash,
            inputs.sample_rate,
            scratch.data.len() / inputs.channels.max(1),
        );
    }

    // Zero the output buffer.
    scratch.data.fill(0.0);

    let frames = block_frames(&inputs, scratch.data.len());

    // The playhead is observed exactly once per block. Every branch renders
    // from this value and publishes its advance against it
    // (`common::commit_playhead`), so a Seek / Stop / MIDI-clock reposition
    // the control thread stores mid-block is never overwritten — and the
    // timing snapshot, the render and the publish can't disagree about
    // where the block started (code review MIX-01).
    let playhead_now = inputs.shared.playhead.load(Ordering::Acquire);

    // The reference A/B monitor bypasses everything below it, including
    // the audition overlay.
    if reference::monitor_reference(&inputs, scratch, playhead_now, frames) {
        return;
    }

    // Snapshot tempo once per block. Holding the `ArcSwap` guard pins this
    // block's bar table for tempo-map-aware MIDI tick→sample conversion in
    // the rendering path; the engine thread publishes tempo changes
    // wait-free via `ArcSwap::store`.
    let tempo_guard = inputs.tempo_map.load();
    let timing = BlockTiming::new(
        &tempo_guard,
        inputs.shared,
        playhead_now,
        inputs.sample_rate,
    );

    // The monitor ring is drained even while gated, so monitoring resumes
    // at its standing latency (not a render's worth of backlog) once the
    // offline render lets go.
    let monitor = monitor_input::read_monitor_input(&inputs, scratch, frames);

    if offline_render {
        // Silence out, transport held, no plugin touched (see above). The
        // audition overlay below reads no plugin either, so a sample
        // preview stays audible during an export.
    } else if inputs.shared.count_in_active.load(Ordering::Relaxed) {
        count_in::render_count_in_block(&inputs, scratch, &timing, monitor, frames);
    } else if !inputs.shared.playing.load(Ordering::Relaxed) {
        stopped::render_stopped_block(&inputs, scratch, &timing, monitor, frames);
    } else {
        play::render_playing_block(&inputs, scratch, &timing, monitor, playhead_now, frames);
    }

    // Audition preview overlay: summed in after the arrangement + master
    // pass, independent of transport, so a sample audition is audible
    // whether or not the project is rolling. Bypasses the master fader/FX
    // by design — it's a monitor-style preview, not part of the mix.
    mix_audition_overlay(scratch.data, inputs.channels, inputs.shared);
}

/// While an offline render owns the plugin instances, live hardware MIDI
/// is still handed to the engine thread (recording / MIDI-thru
/// bookkeeping) but not queued into any instrument: the render would
/// otherwise pick the notes up as part of its own `process()` calls, and
/// leaving them in the channel would replay them, stale, when the render
/// ends. Non-blocking; a full forward channel drops the bookkeeping only,
/// exactly as `pickup_live_midi` does.
fn forward_live_midi_unplayed(inputs: &CallbackInputs<'_>) {
    for ev in inputs.live_midi_rx.try_iter() {
        let _ = inputs.live_midi_fwd.try_send(ev);
    }
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
        // Latched into `SharedState`; the engine loop logs it once. The
        // audio thread never formats or writes to stderr (ARCH-05 A5-2).
        inputs
            .shared
            .oversize_buffer
            .record(raw_output_frames, inputs.buf_frames);
    }
    raw_output_frames.min(inputs.buf_frames)
}
