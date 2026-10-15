//! Stopping a live MIDI recording with keys still held (code review
//! FU-A2c).
//!
//! `close_open_recordings` finalises every still-open note at the Stop
//! playhead, but used to do it silently: the app's mirror had seen only
//! the zero-length `MidiNoteAdded`, so a held note stayed zero-length in
//! the app (and was saved that way) while the engine played it full
//! length. Each closed note is now echoed as a `MidiNoteResized`, and the
//! clip grows to hold it, exactly as a real NoteOff does.

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::types::{AudioEvent, Track, TrackType, TICKS_PER_QUARTER_NOTE};
use resonance_audio::test_support::LiveMidiEvent;

const TRACK: u64 = 3;
/// One quarter at the harness's default 120 BPM / 48 kHz.
const QUARTER: u64 = 24_000;

#[test]
fn a_note_held_at_stop_is_echoed_with_its_final_length() {
    let mut h = EngineHandlerHarness::new();
    let track = Track::with_type(TRACK, "keys".into(), TrackType::Instrument);
    track.set_record_armed(true);
    h.push_track(track);
    h.play();
    h.set_playhead(0);
    let arrival = std::time::Instant::now();
    h.live_midi_event(LiveMidiEvent::InboundNoteOn {
        track_id: TRACK,
        note: 60,
        velocity: 0.8,
        arrival,
    });
    let clip_id = h
        .drain_events()
        .into_iter()
        .find_map(|e| match e {
            AudioEvent::MidiNoteAdded { clip_id, .. } => Some(clip_id),
            _ => None,
        })
        .expect("the NoteOn lands in a recording clip");

    // Two quarters later the user presses Stop with the key still down.
    h.set_playhead(2 * QUARTER);
    h.stop();

    let resized: Vec<(u64, usize, u64)> = h
        .drain_events()
        .into_iter()
        .filter_map(|e| match e {
            AudioEvent::MidiNoteResized {
                clip_id,
                note_index,
                new_duration_ticks,
            } => Some((clip_id, note_index, new_duration_ticks)),
            _ => None,
        })
        .collect();
    assert_eq!(resized.len(), 1, "the held note is echoed: {resized:?}");
    let (id, index, ticks) = resized[0];
    assert_eq!((id, index), (clip_id, 0));
    let expected = 2 * TICKS_PER_QUARTER_NOTE;
    assert!(ticks.abs_diff(expected) <= 2, "final length {ticks}, want ~{expected}");
    assert_eq!(h.midi_notes(clip_id)[0].duration_ticks, ticks, "engine agrees");
}

/// `ClearAll` during a live MIDI capture, with no Stop first, must end
/// the capture (BUG-66). The per-track recording state used to survive:
/// the next project's first note on a track with the same id was routed
/// into the cleared clip — which no longer exists — and silently lost,
/// never opening a clip of its own. Masked in the app, which always sends
/// Stop before ClearAll.
#[test]
fn clear_all_without_stop_ends_a_live_midi_capture() {
    let armed_track = || {
        let track = Track::with_type(TRACK, "keys".into(), TrackType::Instrument);
        track.set_record_armed(true);
        track
    };
    let note_on = |h: &mut EngineHandlerHarness| {
        h.live_midi_event(LiveMidiEvent::InboundNoteOn {
            track_id: TRACK,
            note: 60,
            velocity: 0.8,
            arrival: std::time::Instant::now(),
        });
    };
    let added = |events: Vec<AudioEvent>| {
        events
            .into_iter()
            .find_map(|e| match e {
                AudioEvent::MidiNoteAdded { clip_id, .. } => Some(clip_id),
                _ => None,
            })
    };

    let mut h = EngineHandlerHarness::new();
    h.push_track(armed_track());
    h.play();
    note_on(&mut h);
    let first = added(h.drain_events()).expect("the first project records");

    h.clear_all();
    let fresh = EngineHandlerHarness::DEFAULT_CLIP_GRANT.end;
    h.grant_clip_ids(fresh..fresh + 1_000);
    h.push_track(armed_track());
    h.play();
    h.drain_events();
    note_on(&mut h);
    let second = added(h.drain_events()).expect("the next project records into a new clip");
    assert_ne!(second, first);
    assert_eq!(h.midi_clip_ids(), vec![second], "the note opened its own clip");
}
