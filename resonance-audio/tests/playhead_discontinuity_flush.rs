//! The audio thread flushes instrument voices whenever the transport
//! jumps under it (code review MIX-06).
//!
//! Voices used to be flushed only at the loop seam and by the engine
//! thread's `panic_all_instrument_plugins`, which `try_lock`s every
//! instrument and silently SKIPS one the audio thread is processing —
//! nothing was queued for later, whatever its comment said. The mixer
//! itself never noticed the playhead jump, so a seek while a sustained
//! note played could leave that note hanging forever. Likewise the A/B
//! reference branch: while it monitors the reference the mix is not
//! rendered, the timeline's NoteOffs in that stretch are never collected,
//! and switching back resumed with those voices still held.
//!
//! The callback now remembers where the next playing block should start
//! and flushes (all-notes-off, from the realtime side) on any mismatch —
//! a seek, a MIDI-clock relocate, a lock-contended or reference block, a
//! lost `commit_playhead` — and when the transport stops.

mod note_recorder;

use std::sync::atomic::Ordering;

use note_recorder::{note_recorder, Recorder};
use resonance_audio::__test_support::MixAudioHarness;
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const INSTRUMENT: PluginInstanceId = 300;

/// One instrument track playing a single key-60 note from the top of the
/// song for `quarters` quarter notes (120 bpm: 24 000 samples each).
fn harness(quarters: u64) -> (MixAudioHarness, Recorder) {
    let track = Track::with_type(1, "Synth".into(), TrackType::Instrument);
    track.set_output(TrackOutput::Master);
    track.push_plugin(INSTRUMENT);
    let clip = MidiClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        duration_ticks: 64 * TICKS_PER_QUARTER_NOTE,
        notes: vec![MidiNote {
            note: 60,
            velocity: 1.0,
            start_tick: 0,
            duration_ticks: quarters * TICKS_PER_QUARTER_NOTE,
        }],
        name: "pad".into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    };
    let h = MixAudioHarness::new(
        vec![track],
        Vec::new(),
        Vec::new(),
        vec![clip],
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    let (slot, rec) = note_recorder(SR);
    h.plugins().write().insert(INSTRUMENT, slot);
    h.shared().playing.store(true, Ordering::Relaxed);
    (h, rec)
}

/// Seek while playing: the pad started at 0 and ends at 8 s; the user
/// jumps to 20 s, well past its NoteOff. The next block must flush it.
#[test]
fn a_seek_while_playing_flushes_held_voices() {
    let (mut h, rec) = harness(16);
    for _ in 0..3 {
        h.render();
    }
    assert!(rec.lock().held[60], "the pad is sounding before the seek");

    let calls_before = rec.lock().calls;
    h.shared().playhead.store(20 * SR as u64, Ordering::Release);
    h.render();

    let rec = rec.lock();
    assert!(
        rec.panicked_in(calls_before),
        "the first block after the jump must deliver an all-notes-off"
    );
    assert!(!rec.any_held(), "no voice may survive the seek");
}

/// Continuous playback is not a discontinuity: no spurious flush, so a
/// sustained note is never cut.
#[test]
fn continuous_playback_never_flushes() {
    let (mut h, rec) = harness(16);
    for _ in 0..20 {
        h.render();
    }
    let rec = rec.lock();
    assert!(rec.held[60], "the pad still sounds");
    assert!(
        rec.events.iter().all(|e| e.on),
        "no NoteOff was sent during continuous playback: {:?}",
        rec.events.iter().filter(|e| !e.on).count()
    );
}

/// A lock-contended block advances the playhead without rendering, so
/// whatever NoteOff fell in it was never collected: the next rendered
/// block flushes.
#[test]
fn a_block_skipped_under_lock_contention_flushes_on_the_next_render() {
    let (mut h, rec) = harness(16);
    for _ in 0..3 {
        h.render();
    }
    h.render_lock_contended();
    let calls_before = rec.lock().calls;
    h.render();
    assert!(rec.lock().panicked_in(calls_before));
}

/// A/B reference monitoring while playing: the chord's NoteOff falls
/// while the reference is monitored, so the mix never collects it.
/// Switching back must flush before anything else is played.
#[test]
fn returning_from_the_reference_flushes_voices_whose_note_off_was_skipped() {
    // A one-quarter note: NoteOff at 24 000 samples (~188 blocks).
    let (mut h, rec) = harness(1);
    for _ in 0..3 {
        h.render();
    }
    assert!(rec.lock().held[60]);

    h.enable_reference(vec![0.1; SR as usize * 4]);
    for _ in 0..250 {
        h.render();
    }
    h.disable_reference();

    let calls_before = rec.lock().calls;
    h.render();
    let rec = rec.lock();
    assert!(
        rec.panicked_in(calls_before),
        "switching back to the mix must flush the voices first"
    );
    assert!(!rec.any_held(), "the skipped NoteOff must not leave the note hanging");
}

/// Stop while the audio thread holds the instrument: the engine's own
/// panic is a `try_lock` and may be skipped, so the stopped branch
/// flushes the playing run's voices itself — and, since an instrument
/// with queued events is now processed while stopped (MIX-08), the
/// NoteOffs actually reach it instead of waiting for the next Play.
#[test]
fn stopping_flushes_voices_from_the_audio_thread() {
    let (mut h, rec) = harness(16);
    for _ in 0..3 {
        h.render();
    }
    assert!(rec.lock().held[60]);

    h.shared().playing.store(false, Ordering::Relaxed);
    for _ in 0..2 {
        h.render();
    }
    assert!(
        !rec.lock().any_held(),
        "no voice may outlive the stop, even when the engine's panic was skipped"
    );
}
