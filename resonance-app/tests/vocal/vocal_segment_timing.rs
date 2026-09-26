//! VIEW-34: the SVS segment must span exactly the time its notes occupy
//! on the timeline. Stacked notes (two on one tick) and notes shorter
//! than the 50 ms articulation floor used to be floored *up*, adding
//! time that doesn't exist — every later phoneme in the render unit then
//! sang late, drifting further behind the MIDI until the next silence.

use resonance_app::compose::expression::ExpressionCurves;
use resonance_app::compose::vocal_svs::{
    build_segment, resolve_clip_pronunciation, split_render_units, validate_for_voicebank,
};
use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::derive::LyricLine;
use resonance_music_theory::g2p::AssignedSyllable;
use resonance_music_theory::{VocalParams, VocalVoicebank};

const TPQ: u32 = TICKS_PER_QUARTER_NOTE as u32;
/// Leading + trailing `AP` pads the builder wraps every segment in.
const PADS_SEC: f64 = 0.6;

fn params() -> VocalParams {
    VocalParams {
        voicebank: VocalVoicebank::Lilia,
        ..VocalParams::default()
    }
}

fn assign(n: usize) -> Vec<AssignedSyllable> {
    let text = vec!["la"; n].join(" ");
    let draft = vec![LyricLine {
        n: 1,
        rhyme: 'A',
        syllables: n as u8,
        text,
        locked: false,
    }];
    let resolved = resolve_clip_pronunciation(&draft, &[], n, &Default::default(), &[], &[]);
    validate_for_voicebank(&resolved, VocalVoicebank::Lilia).expect("plain English resolves")
}

fn note(start_tick: u64, duration_ticks: u64) -> MidiNote {
    MidiNote {
        note: 64,
        velocity: 0.8,
        start_tick,
        duration_ticks,
    }
}

/// Seconds the notes really span: first onset to the last note's end.
fn span_sec(notes: &[MidiNote], bpm: f32) -> f64 {
    let spt = 60.0 / (bpm as f64 * TPQ as f64);
    let last = notes.last().unwrap();
    (last.start_tick + last.duration_ticks - notes[0].start_tick) as f64 * spt
}

fn segment_sec(notes: &[MidiNote], bpm: f32) -> f64 {
    let seg = build_segment(
        notes,
        &params(),
        &assign(notes.len()),
        &ExpressionCurves::default(),
        TPQ,
        bpm,
    );
    assert!(seg.ph_dur.iter().all(|&d| d > 0.0), "{:?}", seg.ph_dur);
    seg.ph_dur.iter().sum::<f64>() - PADS_SEC
}

#[test]
fn a_run_of_32nd_notes_at_160_bpm_adds_no_time() {
    // 1/32 at 160 BPM is ~47 ms, under the 50 ms floor.
    let bpm = 160.0;
    let len = TICKS_PER_QUARTER_NOTE / 8;
    let notes: Vec<MidiNote> = (0..16).map(|i| note(i * len, len)).collect();
    let (got, want) = (segment_sec(&notes, bpm), span_sec(&notes, bpm));
    assert!((got - want).abs() < 1e-6, "segment {got} s vs timeline {want} s");
}

#[test]
fn stacked_notes_add_no_time() {
    // A two-note "chord" drawn in the vocal roll, then a phrase.
    let bpm = 120.0;
    let q = TICKS_PER_QUARTER_NOTE;
    let notes = vec![note(0, q), note(0, q), note(q, q), note(2 * q, q), note(3 * q, q)];
    let (got, want) = (segment_sec(&notes, bpm), span_sec(&notes, bpm));
    assert!((got - want).abs() < 1e-6, "segment {got} s vs timeline {want} s");
}

#[test]
fn render_units_span_their_notes() {
    // Short notes, a genuine rest, more short notes: each unit's segment
    // must still match its slice of the timeline.
    let bpm = 160.0;
    let len = TICKS_PER_QUARTER_NOTE / 8;
    let mut notes: Vec<MidiNote> = (0..6).map(|i| note(i * len, len)).collect();
    let rest = 8 * TICKS_PER_QUARTER_NOTE;
    notes.extend((0..6).map(|i| note(rest + i * len, len)));
    notes.push(note(rest + 5 * len, len)); // stacked on the previous one
    let units = split_render_units(
        &notes,
        &params(),
        &assign(notes.len()),
        &ExpressionCurves::default(),
        TPQ,
        bpm,
    );
    assert_eq!(units.len(), 2);
    for u in &units {
        let slice = &notes[u.note_range.clone()];
        let got = u.segment.ph_dur.iter().sum::<f64>() - PADS_SEC;
        let want = span_sec(slice, bpm);
        assert!((got - want).abs() < 1e-6, "unit {:?}: {got} vs {want}", u.note_range);
    }
}
