//! Cycle-record MIDI passes roll on the audio thread's loop wrap, not on
//! the engine thread's view of the playhead (BUG-22).
//!
//! The audio callback counts every wrap (`SharedState::loop_wrap_count`)
//! and stamps each live note it forwards with the count and the playhead
//! of the block that picked it up (`ForwardedLiveMidi`). The engine thread
//! files a note into the pass its stamp names and places it from the
//! stamp's playhead; it used to roll the pass when the playhead read lower
//! than on its previous ~16 ms tick and place the note from the playhead
//! it read when it got to it.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use resonance_audio::test_support::{
    EngineHandlerHarness, ForwardedLiveMidi, LiveMidiEvent, LoopRange, MixAudioHarness,
};
use resonance_audio::types::*;
use resonance_common::{TakeContent, TakeNote};

const TRACK: TrackId = 3;
const LOOP_IN: u64 = 48_000;
/// Two bars at the harness's 120 BPM / 48 kHz.
const LOOP_LEN: u64 = 192_000;
/// One quarter at 120 BPM / 48 kHz.
const QUARTER: u64 = 24_000;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-midi-seam-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A cycle-record run over `LOOP_IN..LOOP_IN + LOOP_LEN` on one armed
/// instrument track, recording from `LOOP_IN`.
fn cycle_record(tag: &str) -> (EngineHandlerHarness, PathBuf) {
    let mut h = EngineHandlerHarness::new();
    let dir = tempdir(tag);
    h.set_project_dir(dir.clone());
    let track = Track::with_type(TRACK, "keys".into(), TrackType::Instrument);
    track.set_record_armed(true);
    h.push_track(track);
    h.set_loop_range(true, LOOP_IN, LOOP_IN + LOOP_LEN);
    h.set_loop_record_mode(true);
    h.set_playhead(LOOP_IN);
    let _input = h.record_with_fake_input(0, 2).expect("session opened");
    assert!(h.shared().recording.load(Ordering::SeqCst));
    h.drain_events();
    (h, dir)
}

/// A note event as the callback forwards it: picked up now, in the block
/// starting at `playhead`, under loop-wrap count `wrap`, having arrived
/// `ago` before the pickup.
fn picked_up(event: fn(Instant) -> LiveMidiEvent, wrap: u64, playhead: u64, ago: Duration) -> ForwardedLiveMidi {
    let now = Instant::now();
    ForwardedLiveMidi {
        event: event(now - ago),
        loop_wrap: wrap,
        playhead,
        picked_up: now,
    }
}

fn on(note: u8) -> impl Fn(Instant) -> LiveMidiEvent {
    move |arrival| LiveMidiEvent::InboundNoteOn {
        track_id: TRACK,
        note,
        velocity: 0.8,
        arrival,
    }
}

fn off(note: u8) -> impl Fn(Instant) -> LiveMidiEvent {
    move |arrival| LiveMidiEvent::InboundNoteOff {
        track_id: TRACK,
        note,
        arrival,
    }
}

fn stamped(event: LiveMidiEvent, wrap: u64, playhead: u64) -> ForwardedLiveMidi {
    ForwardedLiveMidi {
        event,
        loop_wrap: wrap,
        playhead,
        picked_up: Instant::now(),
    }
}

/// The MIDI takes captured, in order: each one's notes.
fn midi_takes(events: &[AudioEvent]) -> Vec<Vec<TakeNote>> {
    events
        .iter()
        .filter_map(|e| match e {
            AudioEvent::TakeCaptured {
                content: TakeContent::Midi { notes },
                ..
            } => Some(notes.clone()),
            _ => None,
        })
        .collect()
}

/// The notes of each take, by pitch.
fn pitches(takes: &[Vec<TakeNote>]) -> Vec<Vec<u8>> {
    takes
        .iter()
        .map(|t| t.iter().map(|n| n.note).collect())
        .collect()
}

/// A note played just after the wrap, and filed by the engine thread
/// before its next tick, belongs to the new pass. The tick used to roll
/// the old pass only when it next saw the playhead lower than before — by
/// then the new note had gone into the old pass's clip, and the old take
/// carried a note from the pass after it.
#[test]
fn a_note_played_after_the_wrap_never_lands_in_the_pass_before_it() {
    let (mut h, dir) = cycle_record("after-wrap");
    let wrap0 = h.shared().loop_wraps();

    // Pass 0: note 60 a quarter in, released a quarter later.
    h.forwarded_live_midi(stamped(on(60)(Instant::now()), wrap0, LOOP_IN + QUARTER));
    h.forwarded_live_midi(stamped(off(60)(Instant::now()), wrap0, LOOP_IN + 2 * QUARTER));
    h.set_playhead(LOOP_IN + LOOP_LEN - 1_000);
    h.recording_tick();
    assert!(midi_takes(&h.drain_events()).is_empty(), "no wrap yet");

    // The callback wraps the loop and picks up note 64 in the new pass;
    // the engine thread files it before its next tick.
    h.shared().note_loop_wrap();
    h.set_playhead(LOOP_IN + 2_000);
    h.forwarded_live_midi(stamped(on(64)(Instant::now()), wrap0 + 1, LOOP_IN + 1_000));
    h.forwarded_live_midi(stamped(off(64)(Instant::now()), wrap0 + 1, LOOP_IN + 1_500));
    h.recording_tick();
    let rolled = midi_takes(&h.drain_events());
    assert_eq!(pitches(&rolled), vec![vec![60]], "pass 0 holds only its own note");

    h.stop();
    let trailing = midi_takes(&h.drain_events());
    assert_eq!(pitches(&trailing), vec![vec![64]], "the new pass got the note played in it");
    let _ = std::fs::remove_dir_all(dir);
}

/// A note the callback picked up just before the wrap but the engine
/// thread files only after it is placed where it was played — near the
/// loop end of the old pass — not rewound from the engine's post-wrap
/// playhead to the loop start.
#[test]
fn a_note_picked_up_before_the_wrap_is_placed_where_it_was_played() {
    let (mut h, dir) = cycle_record("before-wrap");
    let wrap0 = h.shared().loop_wraps();
    let press = LOOP_IN + LOOP_LEN - QUARTER;

    // The audio thread has already wrapped by the time the engine looks.
    h.shared().note_loop_wrap();
    h.set_playhead(LOOP_IN + 300);
    h.forwarded_live_midi(picked_up(|a| on(67)(a), wrap0, press, Duration::ZERO));
    h.forwarded_live_midi(picked_up(|a| off(67)(a), wrap0, press + QUARTER / 2, Duration::ZERO));

    let created: Vec<SamplePos> = h
        .drain_events()
        .into_iter()
        .filter_map(|e| match e {
            AudioEvent::MidiClipCreated { start_sample, .. } => Some(start_sample),
            _ => None,
        })
        .collect();
    assert_eq!(created, vec![press], "the pass-0 clip opens where the key went down");

    h.recording_tick();
    let rolled = midi_takes(&h.drain_events());
    assert_eq!(pitches(&rolled), vec![vec![67]], "the wrap rolls the note with its own pass");
    let expected = TICKS_PER_QUARTER_NOTE / 2;
    let got = rolled[0][0].duration_ticks;
    assert!(got.abs_diff(expected) <= 2, "held an eighth: {got} ticks, want ~{expected}");
    h.stop();
    let _ = std::fs::remove_dir_all(dir);
}

/// A note of a later pass whose press time, rewound from its pickup, falls
/// just before `loop_in` (pressed a hair before the wrap, picked up after
/// it) is held at the loop start of its pass rather than opening a clip
/// outside the loop.
#[test]
fn a_later_pass_note_rewound_past_loop_in_is_held_at_loop_in() {
    let (mut h, dir) = cycle_record("clamp");
    let wrap0 = h.shared().loop_wraps();
    h.shared().note_loop_wrap();
    h.set_playhead(LOOP_IN + 128);
    h.forwarded_live_midi(picked_up(|a| on(72)(a), wrap0 + 1, LOOP_IN + 128, Duration::from_millis(10)));
    let created: Vec<SamplePos> = h
        .drain_events()
        .into_iter()
        .filter_map(|e| match e {
            AudioEvent::MidiClipCreated { start_sample, .. } => Some(start_sample),
            _ => None,
        })
        .collect();
    assert_eq!(created, vec![LOOP_IN]);
    h.stop();
    let _ = std::fs::remove_dir_all(dir);
}

/// Several wraps with no note between them roll the open pass once: the
/// empty passes make no takes, and the note after them still lands in a
/// pass of its own.
#[test]
fn wraps_with_no_notes_between_them_make_no_takes() {
    let (mut h, dir) = cycle_record("empty");
    let wrap0 = h.shared().loop_wraps();
    h.forwarded_live_midi(stamped(on(60)(Instant::now()), wrap0, LOOP_IN + QUARTER));
    for _ in 0..3 {
        h.shared().note_loop_wrap();
    }
    h.recording_tick();
    assert_eq!(pitches(&midi_takes(&h.drain_events())), vec![vec![60]]);
    h.recording_tick();
    assert!(midi_takes(&h.drain_events()).is_empty());
    h.forwarded_live_midi(stamped(on(62)(Instant::now()), wrap0 + 3, LOOP_IN + QUARTER));
    h.stop();
    assert_eq!(pitches(&midi_takes(&h.drain_events())), vec![vec![62]]);
    let _ = std::fs::remove_dir_all(dir);
}

/// The callback counts a wrap at the loop seam, after its live-MIDI
/// pickup: a note picked up in the block that wraps carries the count of
/// the pass that block ends, and the playhead the block started on.
#[test]
fn the_callback_counts_the_wrap_after_stamping_the_notes_it_picked_up() {
    const FRAMES: usize = 128;
    let mut h = MixAudioHarness::on_shared(
        std::sync::Arc::new(resonance_audio::test_support::SharedState::default()),
        FRAMES,
        2,
        48_000,
    );
    let shared = h.shared_arc();
    let lo = 10 * FRAMES as u64;
    let hi = lo + 4 * FRAMES as u64;
    shared.set_loop_range(LoopRange::new(true, lo, hi));
    // The block that ends exactly on `loop_out`.
    shared.playhead.store(hi - FRAMES as u64, Ordering::SeqCst);
    shared.playing.store(true, Ordering::SeqCst);
    let before = shared.loop_wraps();

    h.send_live_midi(on(60)(Instant::now()));
    h.render();
    let forwarded = h.take_forwarded_midi();
    assert_eq!(forwarded.len(), 1);
    assert_eq!(forwarded[0].loop_wrap, before, "stamped with the pass the block ends");
    assert_eq!(forwarded[0].playhead, hi - FRAMES as u64);
    assert_eq!(shared.loop_wraps(), before + 1, "the seam is counted");
    assert_eq!(shared.playhead.load(Ordering::SeqCst), lo);

    h.send_live_midi(on(62)(Instant::now()));
    h.render();
    let forwarded = h.take_forwarded_midi();
    assert_eq!(forwarded[0].loop_wrap, before + 1);
    assert_eq!(shared.loop_wraps(), before + 1, "no seam in this block");
}
