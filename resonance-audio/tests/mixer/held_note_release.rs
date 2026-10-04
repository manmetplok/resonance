//! Editing a sounding note releases it (code review RT-05).
//!
//! Note-offs used to be stateless: the render pass emitted one only when a
//! note's end fell inside the block. A pad note that was shortened to
//! before the playhead, deleted, moved, had its clip deleted or trimmed,
//! or was pulled earlier by a tempo change while it sounded never got its
//! note-off, and hung until the next loop seam or Stop. The render pass
//! now remembers which keys the timeline holds down per instrument track
//! and releases any that no clip note covers at the playhead any more.

use crate::note_recorder;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use note_recorder::{note_recorder, Recorder};
use resonance_audio::test_support::MixAudioHarness;
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const INSTRUMENT: PluginInstanceId = 310;

/// One instrument track whose clip 1 plays key 60 from the top for
/// `quarters` quarter notes (120 bpm: 24 000 samples each), plus key 64
/// for the same span as a bystander that must keep sounding.
fn harness(quarters: u64) -> (MixAudioHarness, Recorder) {
    let mut track = Track::with_type(1, "Pad".into(), TrackType::Instrument);
    track.set_output(TrackOutput::Master);
    track.push_plugin(INSTRUMENT);
    let note = |key| MidiNote {
        note: key,
        velocity: 1.0,
        start_tick: 0,
        duration_ticks: quarters * TICKS_PER_QUARTER_NOTE,
    };
    let clip = MidiClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        duration_ticks: 64 * TICKS_PER_QUARTER_NOTE,
        notes: vec![note(60), note(64)],
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
    h.edit_plugins(|p| p.insert(INSTRUMENT, Arc::new(slot)));
    h.shared().playing.store(true, Ordering::Relaxed);
    (h, rec)
}

/// Play into the sustained notes, then run `edit` and render one block.
/// Returns the recorder's `calls` count from before that block.
fn sound_then_edit(
    h: &mut MixAudioHarness,
    rec: &Recorder,
    edit: impl FnOnce(&MixAudioHarness),
) -> usize {
    sound_for_then_edit(h, rec, 20, edit)
}

fn sound_for_then_edit(
    h: &mut MixAudioHarness,
    rec: &Recorder,
    blocks: usize,
    edit: impl FnOnce(&MixAudioHarness),
) -> usize {
    for _ in 0..blocks {
        h.render();
    }
    {
        let r = rec.lock();
        assert!(r.held[60] && r.held[64], "both keys sound before the edit");
    }
    edit(h);
    let calls = rec.lock().calls;
    h.render();
    calls
}

fn note_offs_since(rec: &Recorder, call: usize, key: u8) -> usize {
    rec.lock()
        .events
        .iter()
        .filter(|e| !e.on && e.key == key && e.call >= call)
        .count()
}

/// The fix's own verification: shorten the sounding note to end before
/// the playhead. The next block delivers its note-off — and only its.
#[test]
fn shortening_a_sounding_note_before_the_playhead_releases_it() {
    let (mut h, rec) = harness(16);
    let call = sound_then_edit(&mut h, &rec, |h| {
        h.edit_midi_clip(1, |c| c.notes[0].duration_ticks = 10).unwrap();
    });
    assert_eq!(note_offs_since(&rec, call, 60), 1, "key 60 released once");
    assert!(!rec.lock().held[60], "key 60 no longer sounds");
    assert!(rec.lock().held[64], "the untouched key keeps sounding");
    assert!(!rec.lock().panicked_in(call), "a release, not a panic");

    // And it is not sent again.
    for _ in 0..10 {
        h.render();
    }
    assert_eq!(note_offs_since(&rec, call, 60), 1);
}

#[test]
fn deleting_a_sounding_note_releases_it() {
    let (mut h, rec) = harness(16);
    sound_then_edit(&mut h, &rec, |h| {
        h.edit_midi_clip(1, |c| {
            c.notes.remove(0);
        })
        .unwrap();
    });
    assert!(!rec.lock().held[60]);
    assert!(rec.lock().held[64]);
}

#[test]
fn moving_a_sounding_note_away_releases_it() {
    let (mut h, rec) = harness(16);
    sound_then_edit(&mut h, &rec, |h| {
        h.edit_midi_clip(1, |c| c.notes[0].start_tick = 40 * TICKS_PER_QUARTER_NOTE)
            .unwrap();
    });
    assert!(!rec.lock().held[60]);
    assert!(rec.lock().held[64]);
}

#[test]
fn trimming_the_clip_under_a_sounding_note_releases_everything_it_held() {
    let (mut h, rec) = harness(16);
    sound_then_edit(&mut h, &rec, |h| {
        h.edit_midi_clip(1, |c| c.trim_end_ticks = c.duration_ticks - 10).unwrap();
    });
    assert!(!rec.lock().any_held());
}

#[test]
fn deleting_the_clip_under_a_sounding_note_releases_it() {
    let (mut h, rec) = harness(16);
    sound_then_edit(&mut h, &rec, |h| {
        h.shared_arc().edit_midi_clips(|v| v.clear());
    });
    assert!(!rec.lock().any_held());
}

#[test]
fn a_tempo_change_that_ends_the_note_before_the_playhead_releases_it() {
    // A one-quarter note: at 120 bpm it ends at 24 000 samples. Play to
    // 12 800 samples, then go to 240 bpm: the quarter now lasts 12 000
    // samples, already behind the playhead.
    let (mut h, rec) = harness(1);
    sound_for_then_edit(&mut h, &rec, 100, |h| {
        let mut map = TempoMap::default();
        map.bpm = 240.0;
        map.tempo_points = vec![TempoPoint { bar: 0, bpm: 240.0 }];
        map.rebuild_bar_table(SR);
        assert_eq!(map.tick_to_abs_sample(0, TICKS_PER_QUARTER_NOTE, SR), 12_000);
        h.set_tempo_map(map);
    });
    assert!(!rec.lock().any_held(), "both one-quarter notes are released");
}

/// Lengthening a sounding note is not a release.
#[test]
fn lengthening_a_sounding_note_keeps_it() {
    let (mut h, rec) = harness(16);
    let call = sound_then_edit(&mut h, &rec, |h| {
        h.edit_midi_clip(1, |c| c.notes[0].duration_ticks *= 2).unwrap();
    });
    assert_eq!(note_offs_since(&rec, call, 60), 0);
    assert!(rec.lock().held[60]);
}
