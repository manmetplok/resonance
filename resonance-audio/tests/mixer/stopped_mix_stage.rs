//! The stopped and count-in branches run the mixer, not a shortcut
//! (code review RT-14).
//!
//! A monitored input (or a live-played instrument) used to be summed
//! straight into the output while the transport was stopped or counting
//! in: no bus, no aux send, no master chain — it sounded different from
//! the same input while rolling — and stopping cut every bus and master
//! reverb tail with a step. Now the stopped / count-in passes route each
//! track to its bus and sends and run the bus pass and the master chain,
//! while anything sounds and for `IDLE_HOLD_SECS` after (so a stop rings
//! out), and nothing at all once idle.

use crate::note_recorder;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use note_recorder::{note_recorder, Recorder};
use resonance_audio::test_support::MixAudioHarness;
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const IN_CH: usize = 2;
const BUS: BusId = 5;
const RETURN: BusId = 6;
const BUS_FX: PluginInstanceId = 330;
const INPUT: f32 = 0.4;
/// `limits::IDLE_HOLD_SECS` in blocks, rounded up.
const HOLD_BLOCKS: usize = (3 * SR as usize).div_ceil(BLOCK);

/// One monitored, armed track on input channel 0, routed to `output`.
fn harness(output: TrackOutput, busses: Vec<Bus>, sends: Vec<AuxSend>) -> MixAudioHarness {
    let mut t = Track::new(1, "mic".into());
    t.set_output(output);
    t.set_monitor_enabled(true);
    t.set_record_armed(true);
    t.set_input_port(0);
    t.set_mono(true);
    let h = MixAudioHarness::new(
        vec![t],
        busses,
        Vec::new(),
        Vec::new(),
        sends,
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    h.shared()
        .input_channels
        .store(IN_CH as u16, Ordering::Relaxed);
    h.shared()
        .master_volume_bits
        .store(1.0f32.to_bits(), Ordering::Relaxed);
    h.shared().monitoring.store(true, Ordering::Relaxed);
    h
}

fn bus(id: BusId, volume: f32) -> Bus {
    let b = Bus::new(id, format!("bus{id}"));
    b.set_volume(volume);
    b
}

/// Render a few stopped blocks with monitor input; the last block's left
/// channel, once the gain ramps have settled.
fn settled_monitor_level(h: &mut MixAudioHarness) -> f32 {
    let mut last = 0.0;
    for _ in 0..4 {
        h.push_monitor(&vec![INPUT; BLOCK * IN_CH]);
        last = h.render()[BLOCK * 2 - 2];
    }
    last
}

#[test]
fn stopped_monitoring_goes_through_the_tracks_bus() {
    let mut h = harness(TrackOutput::Bus(BUS), vec![bus(BUS, 0.5)], Vec::new());
    let got = settled_monitor_level(&mut h);
    assert!(
        (got - INPUT * 0.5).abs() < 1e-5,
        "the bus fader applies while stopped: {got}, expected {}",
        INPUT * 0.5
    );
}

#[test]
fn stopped_monitoring_feeds_its_aux_sends() {
    let send = AuxSend {
        id: 1,
        source: SendSource::Track(1),
        dest: RETURN,
        level_db: 0.0,
        pre_fader: true,
        enabled: true,
    };
    let mut h = harness(TrackOutput::Master, vec![bus(RETURN, 1.0)], vec![send]);
    let got = settled_monitor_level(&mut h);
    assert!(
        (got - 2.0 * INPUT).abs() < 1e-5,
        "dry + return while stopped: {got}, expected {}",
        2.0 * INPUT
    );
}

#[test]
fn count_in_monitoring_goes_through_the_tracks_bus() {
    let mut h = harness(TrackOutput::Bus(BUS), vec![bus(BUS, 0.5)], Vec::new());
    // A quarter of a beat into a long count-in (120 bpm: 24 000-frame
    // beats), so these blocks fall between two clicks.
    const REMAINING: u64 = 1_000_000;
    h.shared()
        .count_in_total
        .store(REMAINING + 6_000, Ordering::Relaxed);
    h.shared()
        .count_in_remaining
        .store(REMAINING, Ordering::Relaxed);
    h.shared().count_in_active.store(true, Ordering::Relaxed);
    let mut last = 0.0;
    for _ in 0..4 {
        h.push_monitor(&vec![INPUT; BLOCK * IN_CH]);
        last = h.render()[BLOCK * 2 - 2];
    }
    assert!(
        (last - INPUT * 0.5).abs() < 1e-5,
        "the bus fader applies during the count-in: {last}"
    );
}

/// A bus whose chain is the call-counting recorder.
fn bus_with_counter() -> (MixAudioHarness, Recorder) {
    let b = bus(BUS, 1.0);
    let h = harness(TrackOutput::Bus(BUS), vec![b], Vec::new());
    h.edit_bus(BUS, |b| b.plugin_ids.push(BUS_FX)).unwrap();
    let (slot, rec) = note_recorder(SR);
    h.edit_plugins(|p| p.insert(BUS_FX, Arc::new(slot)));
    // Nothing monitored: only the transport drives the bus.
    h.shared().monitoring.store(false, Ordering::Relaxed);
    (h, rec)
}

#[test]
fn an_idle_stopped_transport_runs_no_bus_plugin() {
    let (mut h, rec) = bus_with_counter();
    for _ in 0..20 {
        h.render();
    }
    assert_eq!(rec.lock().calls, 0, "nothing sounds, nothing runs");
}

#[test]
fn stopping_lets_the_busses_ring_out_then_goes_idle() {
    let (mut h, rec) = bus_with_counter();
    h.shared().playing.store(true, Ordering::Relaxed);
    for _ in 0..10 {
        h.render();
    }
    let rolling = rec.lock().calls;
    assert!(rolling >= 10, "the bus runs while rolling");

    h.shared().playing.store(false, Ordering::Relaxed);
    for _ in 0..100 {
        h.render();
    }
    assert_eq!(
        rec.lock().calls,
        rolling + 100,
        "after the stop the bus keeps processing, so its tail rings out"
    );
    for _ in 0..HOLD_BLOCKS + 10 {
        h.render();
    }
    let held = rec.lock().calls - rolling;
    assert!(
        (HOLD_BLOCKS - 1..=HOLD_BLOCKS + 1).contains(&held),
        "the tail hold lasts IDLE_HOLD_SECS ({HOLD_BLOCKS} blocks), ran {held}"
    );
    let idle = rec.lock().calls;
    for _ in 0..10 {
        h.render();
    }
    assert_eq!(rec.lock().calls, idle, "then the stopped branch is idle again");
}
