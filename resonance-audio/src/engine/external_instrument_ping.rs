//! Round-trip latency auto-detect ("ping") for external-instrument tracks
//! (doc #169, epic #39, todo #453).
//!
//! Measuring the round-trip means timing how long a note takes to make the
//! full loop: MIDI impulse out the track's hardware MIDI output → outboard
//! synth → audio return into the configured input → frames landing in our
//! capture ring. That end-to-end span is exactly the latency live monitoring
//! and the realtime bounce have to compensate for, so measuring through the
//! same capture path is what makes the number usable.
//!
//! The flow mirrors the realtime bounce ([`super::bounce_realtime`]): a
//! [`PendingLatencyPing`] is stashed in [`HandlerState`] and the engine
//! control loop's [`poll_pending_latency_ping`] hook drains the ring each
//! iteration, runs the (pure, unit-tested) onset detector over the
//! accumulated samples, and either reports the measured round-trip + applies
//! it, or — once the listen window elapses — reports a clean failure. Nothing
//! here runs on the audio callback; the capture stream pushes into a lock-free
//! ring and we drain it on the engine thread.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use ringbuf::traits::{Consumer, Split};

use crate::input_handle::InputHandle;
use crate::platform;
use crate::types::{AudioEvent, TrackId};

use super::thread::{HandlerCtx, HandlerState};

/// MIDI note number fired as the ping impulse (A4 = 69). A mid-range note is
/// the most likely to trigger a clearly-audible, sharp attack on a synth left
/// on whatever patch the user has loaded.
const PING_NOTE: u8 = 69;
/// Full-velocity note so the attack transient is as loud as the patch allows,
/// maximising the margin between the return and the input noise floor.
const PING_VELOCITY: u8 = 127;

/// How long to listen for the return before giving up, in milliseconds. A
/// generous ceiling: even a USB-MIDI + soft-synth + USB-audio chain rarely
/// exceeds a few hundred ms round-trip, but we'd rather wait than miss a slow
/// return. The poll loop still terminates promptly on a clean detection.
const LISTEN_WINDOW_MS: u64 = 750;

/// Lead-in window used to estimate the input noise floor, in milliseconds.
/// The detector measures the average level over the first `LEAD_IN_MS` of
/// capture (before the return can plausibly have arrived) and triggers on the
/// first sample that rises well above it. Kept short so a fast return isn't
/// swallowed by the lead-in, but long enough to average out a few buffers.
const LEAD_IN_MS: u64 = 8;

/// The detected onset must exceed `max(noise_floor * NOISE_FACTOR, MIN_ABS)`.
/// The multiplicative term rejects a noisy-but-steady input; the absolute
/// floor rejects a dead-silent input whose "noise floor" is ~0 (so any tiny
/// dither wouldn't false-trigger).
const NOISE_FACTOR: f32 = 8.0;
const MIN_ABS: f32 = 0.02;

/// Outcome of running the onset detector over a captured buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnsetOutcome {
    /// The impulse onset was found at this 0-based frame index into the
    /// captured stream (counted from the moment the impulse was sent).
    Found(usize),
    /// No sample rose above the trigger anywhere in the buffer. With a full
    /// listen window this means "no detectable return".
    NotFound,
    /// The buffer was too short to even estimate the noise floor — the caller
    /// keeps accumulating rather than declaring failure.
    NeedMore,
}

/// Estimate the input noise floor as the mean absolute sample value over the
/// first `lead_in` frames. Returns `None` when there aren't yet `lead_in`
/// frames to measure. Pure; unit-tested.
pub fn estimate_noise_floor(samples: &[f32], lead_in: usize) -> Option<f32> {
    if lead_in == 0 || samples.len() < lead_in {
        return None;
    }
    let sum: f32 = samples[..lead_in].iter().map(|s| s.abs()).sum();
    Some(sum / lead_in as f32)
}

/// Find the first sample, at or after `lead_in`, whose absolute value exceeds
/// the trigger derived from the noise floor over `[0, lead_in)`.
///
/// Returns:
/// - [`OnsetOutcome::NeedMore`] until at least `lead_in` frames are present,
/// - [`OnsetOutcome::Found`] at the first frame index that crosses the
///   trigger,
/// - [`OnsetOutcome::NotFound`] when the whole buffer stays below it.
///
/// The trigger is `max(noise_floor * NOISE_FACTOR, MIN_ABS)`: the floor-scaled
/// term ignores steady input hum, the absolute term stops a dead-silent input
/// from triggering on dither. Pure; unit-tested — this is the heart of the
/// measurement and the only part that needs to be exercised without hardware.
pub fn detect_impulse_onset(samples: &[f32], lead_in: usize) -> OnsetOutcome {
    let Some(floor) = estimate_noise_floor(samples, lead_in) else {
        return OnsetOutcome::NeedMore;
    };
    let trigger = (floor * NOISE_FACTOR).max(MIN_ABS);
    match samples
        .iter()
        .enumerate()
        .skip(lead_in)
        .find(|(_, s)| s.abs() >= trigger)
    {
        Some((idx, _)) => OnsetOutcome::Found(idx),
        None => OnsetOutcome::NotFound,
    }
}

/// Convert an onset frame index measured at the input device's sample rate to
/// the engine's sample rate, rounding to nearest. Pure; unit-tested. The
/// device may capture at a different rate than the engine runs at, so the raw
/// frame count has to be rescaled before it can be used as a sample offset in
/// the engine's plugin-delay-compensation domain.
pub fn onset_to_engine_samples(onset_input_frames: usize, input_sr: u32, engine_sr: u32) -> i64 {
    if input_sr == 0 {
        return onset_input_frames as i64;
    }
    ((onset_input_frames as f64) * (engine_sr as f64) / (input_sr as f64)).round() as i64
}

/// Convert an onset frame index measured at the input device's sample rate to
/// milliseconds. Rate-independent of the engine. Pure; unit-tested.
pub fn onset_to_ms(onset_input_frames: usize, input_sr: u32) -> f32 {
    if input_sr == 0 {
        return 0.0;
    }
    (onset_input_frames as f64 / input_sr as f64 * 1000.0) as f32
}

/// In-flight latency-ping run. Owns the capture stream + ring consumer for the
/// duration of the measurement so it never touches the recording session's
/// state. Survives across engine-thread iterations until a return is detected
/// or the listen window elapses.
pub(crate) struct PendingLatencyPing {
    pub track_id: TrackId,
    /// MIDI output channel + device the impulse was sent on. Kept so we can
    /// send the matching Note Off on teardown and report the device on an
    /// offline failure.
    pub channel: u8,
    pub midi_out_device: Option<String>,
    /// The track's manual latency offset at ping time — the floor the applied
    /// offset can't drop below ("manual offset is the floor; auto-detect is
    /// the convenience", doc #169).
    pub manual_floor: i64,
    /// Live capture stream. Dropping it closes the device; kept alive for the
    /// whole measurement. Never read directly.
    pub _input: InputHandle,
    /// Consumer half of the capture ring; drained on the engine thread.
    pub ring_consumer: ringbuf::HeapCons<f32>,
    /// Interleaved channel count of the capture stream.
    pub input_channels: u16,
    /// The capture device's sample rate (may differ from the engine's).
    pub input_sample_rate: u32,
    /// Mono samples accumulated from the configured return port since the
    /// impulse was sent. The onset index into this buffer is the round-trip.
    pub captured: Vec<f32>,
    /// 0-based starting channel of the track's audio return in the
    /// interleaved capture stream.
    pub return_port: u16,
    /// Frames after which we stop listening and report failure.
    pub deadline_frames: u64,
    /// Lead-in frames used to estimate the noise floor (input-rate).
    pub lead_in_frames: usize,
}

impl PendingLatencyPing {
    /// Drain whatever the capture callback has pushed into the ring since the
    /// last poll, deinterleaving the track's return channel into `captured`.
    /// Returns the number of frames appended.
    fn drain(&mut self) -> usize {
        let channels = self.input_channels.max(1) as usize;
        let port = (self.return_port as usize).min(channels - 1);
        let mut scratch = [0.0f32; 4096];
        // Pop whole frames only so channel alignment never rotates.
        let scratch_len = (scratch.len() / channels) * channels;
        let mut appended = 0usize;
        loop {
            let count = self.ring_consumer.pop_slice(&mut scratch[..scratch_len]);
            if count == 0 {
                break;
            }
            let frames = count / channels;
            for f in 0..frames {
                self.captured.push(scratch[f * channels + port]);
            }
            appended += frames;
        }
        appended
    }
}

/// Resolve the track's MIDI output (channel + device) and audio-return
/// (device + port) from the shared track table. Mirrors the resolution the
/// other external-instrument handlers do.
fn track_routing(ctx: &HandlerCtx, track_id: TrackId) -> (u8, Option<String>, Option<String>, u16) {
    let tracks = ctx.tracks.read();
    match tracks.get(&track_id) {
        Some(t) => (
            t.midi_output_channel.unwrap_or(0),
            t.midi_output_device.load_full().map(|n| (*n).clone()),
            t.input_device_name.load_full().map(|n| (*n).clone()),
            t.input_port(),
        ),
        None => (0, None, None, 0),
    }
}

/// Dispatch glue for `AudioCommand::DetectExternalInstrumentLatency`. Validates
/// the track is an external instrument with both endpoints configured and the
/// transport stopped, opens the capture stream, fires the impulse, and stashes
/// a [`PendingLatencyPing`] for the poll hook. Any precondition miss emits an
/// `ExternalInstrumentLatencyDetectFailed` and changes nothing.
pub(crate) fn handle_detect_latency(ctx: &HandlerCtx, state: &mut HandlerState, track_id: TrackId) {
    // Not an external instrument → silent no-op, matching the other handlers'
    // missing-lookup convention.
    let Some(config) = state.external_instruments.get(&track_id).copied() else {
        return;
    };

    let fail = |reason: &str| {
        let _ = ctx
            .event_tx
            .send(AudioEvent::ExternalInstrumentLatencyDetectFailed {
                track_id,
                reason: reason.to_string(),
            });
    };

    // Only one ping at a time; and never while the transport is running (the
    // capture device is busy and the timeline would smear the impulse).
    if state.pending_latency_ping.is_some() {
        fail("A latency auto-detect is already in progress.");
        return;
    }
    if ctx.shared.playing.load(Ordering::Relaxed) {
        fail("Stop the transport before auto-detecting latency.");
        return;
    }
    if state.pending_bounce.is_some() || ctx.shared.recording.load(Ordering::Relaxed) {
        fail("Finish recording / bouncing before auto-detecting latency.");
        return;
    }

    let (channel, midi_out_device, return_input_device, return_port) = track_routing(ctx, track_id);

    if midi_out_device.is_none() {
        fail("Pick a MIDI output for this track before auto-detecting latency.");
        return;
    }
    let Some(return_device) = return_input_device.clone() else {
        fail("Pick an audio return input before auto-detecting latency.");
        return;
    };

    // The track's return port needs at least that many input channels.
    let desired_channels = return_port.saturating_add(1).max(1);

    // Dedicated capture ring — never the recording session's. Sized like a
    // record ring so a slow drain can't drop the return.
    let ring = ringbuf::HeapRb::<f32>::new(super::RECORDING_RING_SIZE);
    let (prod, cons) = ring.split();

    let (input, in_sr, in_ch) = match platform::build_input_stream(
        Some(return_device.as_str()),
        Arc::clone(ctx.shared),
        Some(prod),
        Arc::clone(ctx.monitor_prod),
        ctx.buf_frames,
        ctx.quantum,
        ctx.sample_rate,
        desired_channels,
    ) {
        Ok(triple) => triple,
        Err(e) => {
            fail(&format!(
                "Could not open the audio return '{return_device}': {e}"
            ));
            return;
        }
    };

    // Fire the impulse. `send_program_change`'s sibling `send_note_on`
    // returns nothing, so check reachability the same way the patch send does
    // — a missing live connection means the MIDI output is offline.
    let reached = state
        .midi_hw
        .midi_outputs
        .send_program_change(track_id, channel, None, None);
    // `send_program_change` with no bank/program reports reachability without
    // sending anything; only fire the actual note when the device is live.
    if !reached {
        // Dropping `input` here closes the just-opened capture stream.
        let _ = input;
        let _ = ctx
            .event_tx
            .send(AudioEvent::ExternalInstrumentMidiOutOffline {
                track_id,
                device: midi_out_device.clone(),
            });
        fail("The MIDI output is offline — can't send the ping.");
        return;
    }
    state
        .midi_hw
        .midi_outputs
        .send_note_on(track_id, channel, PING_NOTE, PING_VELOCITY);

    let deadline_frames = (in_sr as u64 * LISTEN_WINDOW_MS) / 1000;
    let lead_in_frames = ((in_sr as u64 * LEAD_IN_MS) / 1000).max(1) as usize;

    state.pending_latency_ping = Some(PendingLatencyPing {
        track_id,
        channel,
        midi_out_device,
        manual_floor: config.latency_offset_samples,
        _input: input,
        ring_consumer: cons,
        input_channels: in_ch,
        input_sample_rate: in_sr,
        captured: Vec::with_capacity((deadline_frames as usize).min(1 << 20)),
        return_port,
        deadline_frames,
        lead_in_frames,
    });
}

/// Engine-loop hook: advance an in-flight latency ping. Drains the capture
/// ring, runs the onset detector, and on a hit computes + applies the
/// round-trip offset (and republishes PDC); on the listen-window deadline,
/// reports a clean failure. No-op when no ping is pending.
pub(crate) fn poll_pending_latency_ping(ctx: &HandlerCtx, state: &mut HandlerState) {
    if state.pending_latency_ping.is_none() {
        return;
    }
    // Borrow split: detect on the owned ping, then mutate engine state after
    // taking it, so we don't hold a `&mut state.pending_latency_ping` across
    // the `&mut state.external_instruments` / `&mut state.midi_hw` writes.
    let (outcome, frames, input_sr, deadline) = {
        let ping = state.pending_latency_ping.as_mut().unwrap();
        ping.drain();
        let outcome = detect_impulse_onset(&ping.captured, ping.lead_in_frames);
        (
            outcome,
            ping.captured.len() as u64,
            ping.input_sample_rate,
            ping.deadline_frames,
        )
    };

    match outcome {
        OnsetOutcome::Found(onset_input_frames) => {
            let ping = state.pending_latency_ping.take().unwrap();
            finish_ping_success(ctx, state, &ping, onset_input_frames);
        }
        OnsetOutcome::NeedMore | OnsetOutcome::NotFound => {
            // Keep listening until the window elapses; only then is
            // "NotFound" actually a failure. `NeedMore` simply means the
            // lead-in hasn't filled yet.
            if frames >= deadline {
                let ping = state.pending_latency_ping.take().unwrap();
                finish_ping_failure(ctx, state, &ping, input_sr);
            }
        }
    }
}

/// A detection landed: compute the round-trip, apply it as the track's offset
/// (raising — never lowering — past the manual floor), emit the measured
/// event, and republish the plugin-delay-compensation table. Also sends the
/// matching Note Off so the synth doesn't sustain the ping note.
fn finish_ping_success(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    ping: &PendingLatencyPing,
    onset_input_frames: usize,
) {
    send_ping_note_off(state, ping);

    let measured =
        onset_to_engine_samples(onset_input_frames, ping.input_sample_rate, ctx.sample_rate);
    let latency_ms = onset_to_ms(onset_input_frames, ping.input_sample_rate);

    // Manual offset is the floor: auto-detect only ever raises the applied
    // offset, never drops it below what the user dialled in.
    let applied = measured.max(ping.manual_floor);

    if let Some(config) = state.external_instruments.get_mut(&ping.track_id) {
        config.latency_offset_samples = applied;
        let config = *config;
        // Echo the config so the app mirror's offset matches the engine.
        let _ = ctx
            .event_tx
            .send(AudioEvent::ExternalInstrumentChanged { config });
    }

    let _ = ctx
        .event_tx
        .send(AudioEvent::ExternalInstrumentLatencyMeasured {
            track_id: ping.track_id,
            latency_samples: applied,
            latency_ms,
        });

    // The offset feeds `add_external_offsets`; republish PDC so the rest of
    // the mix is delayed to meet the (now-known) hardware return.
    super::plugins::refresh_latency_comp(ctx, &state.external_instruments);
}

/// The listen window elapsed with no detectable return. Report a clean
/// failure and send the Note Off so the ping note doesn't hang.
fn finish_ping_failure(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    ping: &PendingLatencyPing,
    input_sr: u32,
) {
    send_ping_note_off(state, ping);

    // Distinguish "no frames at all" (device opened but delivered nothing)
    // from "frames but no impulse" (cabling/level problem) for a clearer
    // message — the realtime bounce draws the same distinction.
    let reason = if input_sr == 0 {
        "The audio return delivered no audio — check the device.".to_string()
    } else {
        "No return detected within the listen window. Check that the synth's \
         audio output is wired to the picked return input and that its level \
         isn't silent."
            .to_string()
    };
    let _ = ctx
        .event_tx
        .send(AudioEvent::ExternalInstrumentLatencyDetectFailed {
            track_id: ping.track_id,
            reason,
        });
}

/// Send the matching Note Off for the ping impulse so a hardware synth doesn't
/// sustain the note after the measurement ends.
fn send_ping_note_off(state: &mut HandlerState, ping: &PendingLatencyPing) {
    state
        .midi_hw
        .midi_outputs
        .send_note_off(ping.track_id, ping.channel, PING_NOTE);
    let _ = &ping.midi_out_device; // reported only on the offline path
}
