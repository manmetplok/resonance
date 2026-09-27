//! Instruments play live notes while the transport is stopped (code
//! review MIX-08).
//!
//! With the transport stopped the callback used to process only tracks
//! that were input-monitored AND had a delivering input stream. Live
//! hardware MIDI and piano-roll preview notes were still queued into the
//! instrument, but its `process()` never ran: the notes were silent, piled
//! up to the 256-event queue cap — past which note-offs were dropped too —
//! and the whole backlog fired at once on the next Play, with any note
//! whose note-off was dropped left hanging.

use crate::note_recorder;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use note_recorder::{note_recorder, Recorder};
use resonance_audio::test_support::{LiveMidiEvent, MixAudioHarness};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const TRACK: TrackId = 1;
const INSTRUMENT: PluginInstanceId = 300;

/// One instrument track, transport stopped, no input device and no
/// monitoring — the piano-roll / controller preview situation.
fn stopped_harness() -> (MixAudioHarness, Recorder) {
    let mut track = Track::with_type(TRACK, "Synth".into(), TrackType::Instrument);
    track.set_output(TrackOutput::Master);
    track.push_plugin(INSTRUMENT);
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
    h.edit_plugins(|p| p.insert(INSTRUMENT, Arc::new(slot)));
    h.shared()
        .master_volume_bits
        .store(1.0f32.to_bits(), Ordering::Relaxed);
    (h, rec)
}

fn note_on(h: &MixAudioHarness, note: u8) {
    h.send_live_midi(LiveMidiEvent::InboundNoteOn {
        track_id: TRACK,
        note,
        velocity: 1.0,
        arrival: Instant::now(),
    });
}

fn note_off(h: &MixAudioHarness, note: u8) {
    h.send_live_midi(LiveMidiEvent::InboundNoteOff {
        track_id: TRACK,
        note,
        arrival: Instant::now(),
    });
}

#[test]
fn a_live_note_while_stopped_is_heard_within_one_block() {
    let (mut h, rec) = stopped_harness();
    note_on(&h, 60);
    let out = h.render().to_vec();

    let rec = rec.lock();
    assert!(
        rec.events.iter().any(|e| e.on && e.key == 60),
        "the instrument must receive the note in the block it was played"
    );
    assert!(
        out.iter().any(|&s| s != 0.0),
        "and the note must be heard with the transport stopped"
    );
}

/// Many preview notes while stopped: each pair is delivered as it is
/// played, so nothing accumulates for Play to burst, and no voice hangs.
#[test]
fn preview_notes_while_stopped_never_pile_up_for_play() {
    let (mut h, rec) = stopped_harness();
    for i in 0..300u32 {
        let key = (i % 100) as u8;
        note_on(&h, key);
        h.render();
        note_off(&h, key);
        h.render();
    }
    assert!(!rec.lock().any_held(), "every preview note was released");
    {
        let plugins = h.plugins();
        let inst = plugins.get(&INSTRUMENT).unwrap().lock();
        assert!(
            inst.0.__pending_notes_for_test().is_empty(),
            "nothing may be left queued for the next Play"
        );
    }

    // Play: the first block carries no stale backlog.
    let calls = rec.lock().calls;
    h.shared().playing.store(true, Ordering::Relaxed);
    h.render();
    let rec = rec.lock();
    assert!(
        rec.events.iter().all(|e| e.call < calls),
        "Play must not fire a backlog of preview notes"
    );
}

/// Bounded cost: an instrument that never received a live note is not
/// processed while stopped, and one that did stops being processed once
/// its release window has run out.
#[test]
fn an_idle_instrument_is_not_processed_while_stopped() {
    let (mut h, rec) = stopped_harness();
    for _ in 0..10 {
        h.render();
    }
    assert_eq!(rec.lock().calls, 0, "an untouched instrument costs nothing");

    note_on(&h, 60);
    h.render();
    note_off(&h, 60);
    // Well past the release window (`limits::IDLE_HOLD_SECS`, 3 s).
    let window_blocks = 4 * SR as usize / BLOCK;
    for _ in 0..window_blocks {
        h.render();
    }
    let settled = rec.lock().calls;
    for _ in 0..50 {
        h.render();
    }
    assert_eq!(rec.lock().calls, settled, "processing stops once the tail has run out");
    assert!(settled > 1, "but the note's release was processed");
}

/// The queue cap drops a note-on before it ever drops a note-off: a lost
/// note-on is a missed note, a lost note-off a note that never ends.
#[test]
fn a_full_queue_evicts_a_note_on_rather_than_drop_a_note_off() {
    let (h, _rec) = stopped_harness();
    let plugins = h.plugins();
    let mut inst = plugins.get(&INSTRUMENT).unwrap().lock();
    for i in 0..1_000u32 {
        inst.0.queue_note_on((i % 128) as u8, 1.0, 0);
    }
    inst.0.queue_note_off(5, 7);

    let pending = inst.0.__pending_notes_for_test();
    assert!(
        pending.iter().any(|&(on, key, _, off)| !on && key == 5 && off == 7),
        "the note-off must be queued even at the cap"
    );
    assert!(pending.len() <= 256, "without growing the queue past its cap");
}
