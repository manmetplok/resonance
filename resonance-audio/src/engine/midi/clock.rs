//! MIDI clock master and slave. As master we emit 24 PPQN clock
//! pulses, plus Start / Stop / Continue / Song Position Pointer,
//! latched to the engine's tempo map. As slave we drive the engine's
//! transport from inbound clock messages and derive a smoothed BPM
//! that the GUI mirrors back into the tempo map.

use std::sync::atomic::Ordering;

use crate::midi_clock::MidiClockEvent;
use crate::types::*;

use super::super::thread::{HandlerCtx, HandlerState};

/// Convert an absolute sample position to an absolute MIDI clock tick
/// (24 PPQN). The internal tempo map's tick resolution is 480 PPQN
/// (`TICKS_PER_QUARTER_NOTE`), so dividing by 20 lands on the clock
/// resolution.
fn sample_to_clock_tick(map: &TempoMap, sample_pos: u64, sample_rate: u32) -> u64 {
    let abs_tick = map.sample_to_abs_tick(sample_pos, sample_rate);
    abs_tick / (TICKS_PER_QUARTER_NOTE / 24)
}

/// Convert an absolute sample position to a Song Position Pointer
/// value in MIDI beats (16th notes from song start).
fn sample_to_spp(map: &TempoMap, sample_pos: u64, sample_rate: u32) -> u16 {
    let abs_tick = map.sample_to_abs_tick(sample_pos, sample_rate);
    // 480 ticks per quarter ÷ 4 = 120 ticks per 16th note.
    let sixteenths = abs_tick / (TICKS_PER_QUARTER_NOTE / 4);
    sixteenths.min(0x3FFF) as u16
}

pub(crate) fn handle_set_midi_clock_output(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    device: Option<String>,
    enabled: bool,
) {
    if let Err(e) = state.midi_clock_sender.configure(device, enabled) {
        let _ = ctx.event_tx.send(AudioEvent::Error(e));
    }
}

pub(crate) fn handle_set_midi_clock_input(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    device: Option<String>,
    enabled: bool,
) {
    if let Err(e) = state.midi_clock_receiver.configure(device, enabled) {
        let _ = ctx.event_tx.send(AudioEvent::Error(e));
    }
    // Resetting the tempo tracker on every reconfigure avoids a stale
    // average being applied to a freshly opened device.
    state.midi_clock_tempo.reset();
    state.midi_clock_external_running = false;
    state.midi_clock_last_emitted_bpm = 0.0;
}

/// Catch the master clock up to the playhead. Always called from the
/// engine thread loop; bails cheaply when the master isn't enabled.
/// Driven both during playback (clock pulses follow the tempo map)
/// and while stopped (no pulses, but `last_clock_tick` stays in sync
/// with the playhead so a Continue resumes from the right spot).
pub(crate) fn poll_midi_clock_send(ctx: &HandlerCtx, state: &mut HandlerState) {
    if !state.midi_clock_sender.is_active() {
        return;
    }
    if !ctx.shared.playing.load(Ordering::Relaxed) {
        return;
    }
    let playhead = ctx.shared.playhead.load(Ordering::Relaxed);
    let clock_tick = sample_to_clock_tick(&ctx.tempo_map.load(), playhead, ctx.sample_rate);
    state.midi_clock_sender.poll_send_clock(clock_tick);
}

/// Emit a MIDI Start aligned to playhead 0, called from the transport
/// Play handler. Caller is responsible for deciding whether this is a
/// fresh start (playhead == 0) or a Continue (playhead > 0).
pub(crate) fn clock_send_start(state: &mut HandlerState) {
    state.midi_clock_sender.send_start();
}

pub(crate) fn clock_send_continue(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    sample_pos: u64,
) {
    if !state.midi_clock_sender.is_active() {
        return;
    }
    let tm = ctx.tempo_map.load();
    let clock_tick = sample_to_clock_tick(&tm, sample_pos, ctx.sample_rate);
    let spp = sample_to_spp(&tm, sample_pos, ctx.sample_rate);
    // Send SPP first so the receiver knows where to resume, then
    // Continue. This matches the convention most external gear
    // expects (Reason, Ableton, MPC, etc.).
    state.midi_clock_sender.send_song_position(spp, clock_tick);
    state.midi_clock_sender.send_continue(clock_tick);
}

pub(crate) fn clock_send_stop(state: &mut HandlerState) {
    state.midi_clock_sender.send_stop();
}

pub(crate) fn clock_send_song_position(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    sample_pos: u64,
) {
    if !state.midi_clock_sender.is_active() {
        return;
    }
    let tm = ctx.tempo_map.load();
    let clock_tick = sample_to_clock_tick(&tm, sample_pos, ctx.sample_rate);
    let spp = sample_to_spp(&tm, sample_pos, ctx.sample_rate);
    state.midi_clock_sender.send_song_position(spp, clock_tick);
}

/// Apply one drained inbound clock message to the transport and the
/// tempo tracker. Driving the transport from the engine thread keeps
/// the locking story simple: the same paths the GUI uses to start
/// playback are invoked here, so all the same invariants hold.
pub(crate) fn handle_midi_clock_event(
    ctx: &HandlerCtx,
    state: &mut HandlerState,
    event: MidiClockEvent,
) {
    if !state.midi_clock_receiver.is_active() {
        return;
    }
    match event {
        MidiClockEvent::Start { .. } => {
            state.midi_clock_tempo.reset();
            state.midi_clock_external_running = true;
            ctx.shared.playhead.store(0, Ordering::SeqCst);
            ctx.shared.playing.store(true, Ordering::SeqCst);
            let _ = ctx.event_tx.send(AudioEvent::PlayheadMoved(0));
            let _ = ctx.event_tx.send(AudioEvent::MidiClockStarted);
        }
        MidiClockEvent::Continue { .. } => {
            state.midi_clock_tempo.reset();
            state.midi_clock_external_running = true;
            ctx.shared.playing.store(true, Ordering::SeqCst);
            let _ = ctx.event_tx.send(AudioEvent::MidiClockContinued);
        }
        MidiClockEvent::Stop => {
            state.midi_clock_external_running = false;
            ctx.shared.playing.store(false, Ordering::SeqCst);
            // Mirror the standard Stop handler's all-notes-off so a
            // hardware synth doesn't sustain when the master halts.
            super::super::transport::handle_pause(ctx, state);
            let _ = ctx.event_tx.send(AudioEvent::MidiClockStopped);
        }
        MidiClockEvent::Clock { arrival } => {
            // Only react to BPM derivation while the external master
            // is in run state. Many devices send free-running clock
            // even when stopped; we don't want that smear into our
            // tempo while we're stationary.
            if !state.midi_clock_external_running {
                return;
            }
            if let Some(bpm) = state.midi_clock_tempo.observe(arrival) {
                if (bpm - state.midi_clock_last_emitted_bpm).abs() > 0.1 {
                    state.midi_clock_last_emitted_bpm = bpm;
                    super::super::rcu_tempo(ctx.tempo_map, |tm| tm.bpm = bpm);
                    let _ = ctx
                        .event_tx
                        .send(AudioEvent::MidiClockTempoDetected { bpm });
                }
            }
        }
        MidiClockEvent::SongPosition { sixteenths } => {
            // Convert 16th notes to absolute samples through the tempo
            // map and seek there. SPP only legally arrives while
            // stopped, but we accept it during playback too because
            // some hardware sequencers do exactly that.
            let abs_tick = (sixteenths as u64) * (TICKS_PER_QUARTER_NOTE / 4);
            let sample = ctx
                .tempo_map
                .load()
                .tick_to_abs_sample(0, abs_tick, ctx.sample_rate);
            ctx.shared.playhead.store(sample, Ordering::SeqCst);
            let _ = ctx.event_tx.send(AudioEvent::PlayheadMoved(sample));
        }
    }
}
