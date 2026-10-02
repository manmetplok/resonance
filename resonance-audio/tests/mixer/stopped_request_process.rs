//! `clap_host.request_process()` while the transport is stopped.
//!
//! With the transport stopped and a track unmonitored, the host processes
//! an instrument only while it has live notes to play (code review
//! MIX-08). A plugin that has work to do in `process()` with no note in
//! sight — the drum plugin installs a kit picked while stopped there, and
//! frees the one it retired — asks for a `process()` through
//! `request_process`, which used to be a no-op: the kit never installed,
//! its load progress stuck below 1.0, and editor auditions were silent.

use crate::note_recorder;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use note_recorder::{note_recorder, request_process, Recorder};
use resonance_audio::test_support::MixAudioHarness;
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const TRACK: TrackId = 1;
const PLUGIN: PluginInstanceId = 300;
/// `limits::IDLE_HOLD_SECS` (3 s) of blocks: what one request buys.
const HOLD_BLOCKS: usize = 3 * SR as usize / BLOCK;

/// One track carrying the recorder as its only slot, transport stopped,
/// no input device, no monitoring.
fn stopped_harness(track_type: TrackType) -> (MixAudioHarness, Recorder) {
    let mut track = Track::with_type(TRACK, "Track".into(), track_type);
    track.set_output(TrackOutput::Master);
    track.push_plugin(PLUGIN);
    let h = MixAudioHarness::new(
        vec![track],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    let (slot, rec) = note_recorder(SR);
    h.edit_plugins(|p| p.insert(PLUGIN, Arc::new(slot)));
    h.shared()
        .master_volume_bits
        .store(1.0f32.to_bits(), Ordering::Relaxed);
    (h, rec)
}

fn render(h: &mut MixAudioHarness, blocks: usize) {
    for _ in 0..blocks {
        h.render();
    }
}

fn calls(rec: &Recorder) -> usize {
    rec.lock().calls
}

/// `request_process` is `[thread-safe]`: the drum plugin calls it from
/// its kit loader.
fn request_from_another_thread(rec: &Recorder) {
    let rec = Arc::clone(rec);
    std::thread::spawn(move || request_process(&rec))
        .join()
        .expect("requesting thread");
}

#[test]
fn a_request_while_stopped_runs_the_instrument_for_the_hold_then_stops() {
    let (mut h, rec) = stopped_harness(TrackType::Instrument);
    render(&mut h, 10);
    assert_eq!(
        calls(&rec),
        0,
        "an idle instrument is not processed while stopped"
    );

    request_from_another_thread(&rec);
    h.render();
    assert_eq!(calls(&rec), 1, "the request is served in the next block");

    render(&mut h, 2 * HOLD_BLOCKS);
    assert_eq!(
        calls(&rec),
        HOLD_BLOCKS,
        "one request buys IDLE_HOLD_SECS of blocks, and no more"
    );
}

#[test]
fn asking_again_extends_the_window() {
    let (mut h, rec) = stopped_harness(TrackType::Instrument);
    request_process(&rec);
    render(&mut h, 500);
    assert_eq!(calls(&rec), 500);

    request_process(&rec);
    render(&mut h, 2 * HOLD_BLOCKS);
    assert_eq!(
        calls(&rec),
        500 + HOLD_BLOCKS,
        "the second request re-arms the full hold"
    );
}

#[test]
fn a_request_on_an_inactive_instance_is_dropped() {
    let (mut h, rec) = stopped_harness(TrackType::Instrument);
    {
        let plugins = h.plugins();
        let mut inst = plugins.get(&PLUGIN).unwrap().lock();
        rec.lock().fail_activate = true;
        assert!(!inst.0.restart(), "reactivation fails");
        assert!(!inst.0.is_active());
    }

    request_process(&rec);
    render(&mut h, 20);
    assert_eq!(calls(&rec), 0, "an inactive instance is never processed");

    // Back up: the dropped request does not linger and fire late.
    {
        let plugins = h.plugins();
        let mut inst = plugins.get(&PLUGIN).unwrap().lock();
        rec.lock().fail_activate = false;
        assert!(inst.0.restart());
    }
    render(&mut h, 20);
    assert_eq!(calls(&rec), 0, "the request was consumed while inactive");
}

#[test]
fn a_contended_block_leaves_the_request_for_the_next_one() {
    let (mut h, rec) = stopped_harness(TrackType::Instrument);
    request_process(&rec);
    {
        // The engine thread holds the instance (a param flush, a state
        // load): the audio thread's `try_lock` fails.
        let plugins = h.plugins();
        let _held = plugins.get(&PLUGIN).unwrap().lock();
        render(&mut h, 5);
    }
    assert_eq!(calls(&rec), 0);

    h.render();
    assert_eq!(calls(&rec), 1, "the request survived the contended blocks");
    render(&mut h, 2 * HOLD_BLOCKS);
    assert_eq!(calls(&rec), HOLD_BLOCKS);
}

#[test]
fn any_process_satisfies_a_pending_request() {
    let (mut h, rec) = stopped_harness(TrackType::Instrument);
    request_process(&rec);
    {
        let plugins = h.plugins();
        let mut inst = plugins.get(&PLUGIN).unwrap().lock();
        let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
        inst.0.process(&mut l, &mut r, BLOCK);
    }
    assert_eq!(calls(&rec), 1);

    render(&mut h, 20);
    assert_eq!(calls(&rec), 1, "the request was already met");
}

/// An effect asks too — on an audio track, which the stopped pass used to
/// skip outright.
#[test]
fn an_effect_slot_on_an_audio_track_is_served() {
    let (mut h, rec) = stopped_harness(TrackType::Audio);
    render(&mut h, 10);
    assert_eq!(
        calls(&rec),
        0,
        "an idle audio track is not processed while stopped"
    );

    request_from_another_thread(&rec);
    render(&mut h, 2 * HOLD_BLOCKS);
    assert_eq!(calls(&rec), HOLD_BLOCKS);
}

/// A bypassed slot is skipped by the chain, so its window could never
/// count down: its request waits for the bypass to lift instead of
/// running the track every stopped block for good.
#[test]
fn a_bypassed_slot_keeps_its_request_until_unbypassed() {
    let (mut h, rec) = stopped_harness(TrackType::Audio);
    let slot = Arc::clone(h.plugins().get(&PLUGIN).unwrap());
    slot.bypass.set_bypassed_settled(true);

    request_process(&rec);
    render(&mut h, 20);
    assert_eq!(calls(&rec), 0);

    slot.bypass.set_bypassed_settled(false);
    render(&mut h, 2 * HOLD_BLOCKS);
    assert_eq!(calls(&rec), HOLD_BLOCKS, "served once the slot runs again");
}
