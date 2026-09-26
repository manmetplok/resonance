//! Voice allocation: the `max_voices` ceiling and legato glide in mono.
//!
//! Past the ceiling, `find_free_voice`'s stealing steps used to search all
//! 32 slots, idle ones included. An idle slot keeps a stale (usually zero)
//! age, so "steal the oldest" picked an *idle* slot, the new note sounded on
//! top of the held ones, and the ceiling was never enforced. In mono that
//! also broke legato portamento: the new note landed on a fresh idle voice,
//! which never glides.

use resonance_plugin::{EventIterator, NoteEvent};
use resonance_wavetable::dsp::engine::SynthEngine;
use resonance_wavetable::params::WavetableParams;

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;

fn render(engine: &mut SynthEngine, params: &WavetableParams, events: &[NoteEvent]) -> Vec<f32> {
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    let mut iter = EventIterator::new(events);
    engine.render_block(&mut left, &mut right, BLOCK, params, &mut iter, None);
    left.extend_from_slice(&right);
    left
}

fn note_on(note: u8) -> NoteEvent {
    NoteEvent::NoteOn {
        note,
        velocity: 0.8,
        timing: 0,
    }
}

fn peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

/// Held notes beyond `max_voices` steal instead of adding voices.
#[test]
fn held_notes_never_exceed_max_voices() {
    for max_voices in [1, 2, 4] {
        let params = WavetableParams::new();
        params.max_voices.set_value(max_voices);
        let mut engine = SynthEngine::new();
        engine.initialize(SR);

        let mut loudest = 0.0f32;
        for (i, note) in [48u8, 52, 55, 59, 62, 65, 69, 72, 76].into_iter().enumerate() {
            loudest = loudest.max(peak(&render(&mut engine, &params, &[note_on(note)])));
            let sounding = engine.sounding_voices().count();
            assert!(
                sounding <= max_voices as usize,
                "max_voices={max_voices}: {sounding} voices sounding after {} held notes",
                i + 1
            );
            // The newest note is always among the sounding ones.
            assert!(
                engine.sounding_voices().any(|(n, _)| n == note),
                "max_voices={max_voices}: newest note {note} is not sounding"
            );
        }
        assert!(loudest > 1e-3, "max_voices={max_voices}: rendered silence");
    }
}

/// Mono (max_voices = 1) + glide, played legato: the second note takes over
/// the held voice and glides from the first pitch to the second.
#[test]
fn mono_legato_glides_on_the_held_voice() {
    let params = WavetableParams::new();
    params.max_voices.set_value(1);
    params.glide_enabled.set_value(true);
    params.glide_time.set_value(100.0);
    let mut engine = SynthEngine::new();
    engine.initialize(SR);

    // Hold C3 long enough to settle on its pitch.
    let mut loudest = peak(&render(&mut engine, &params, &[note_on(48)]));
    for _ in 0..8 {
        loudest = loudest.max(peak(&render(&mut engine, &params, &[])));
    }
    // Press G3 while C3 is still held.
    loudest = loudest.max(peak(&render(&mut engine, &params, &[note_on(55)])));

    let voices: Vec<(u8, f32)> = engine.sounding_voices().collect();
    assert_eq!(voices.len(), 1, "mono played two voices: {voices:?}");
    let (note, pitch) = voices[0];
    assert_eq!(note, 55);
    assert!(
        pitch > 48.0 && pitch < 55.0,
        "legato note did not glide: pitch {pitch} one block after the note-on"
    );

    // And it keeps moving toward the target.
    for _ in 0..(SR as usize / BLOCK) {
        loudest = loudest.max(peak(&render(&mut engine, &params, &[])));
    }
    let (_, settled) = engine.sounding_voices().next().expect("voice still sounding");
    assert!((settled - 55.0).abs() < 0.01, "glide did not arrive: {settled}");
    assert!(loudest > 1e-3, "rendered silence");
}

/// Re-pressing a note that is already sounding at the ceiling reuses that
/// voice rather than stealing a different one.
#[test]
fn repeated_note_reuses_its_voice() {
    let params = WavetableParams::new();
    params.max_voices.set_value(4);
    let mut engine = SynthEngine::new();
    engine.initialize(SR);

    let mut loudest = 0.0f32;
    for note in [48u8, 52, 55, 59] {
        loudest = loudest.max(peak(&render(&mut engine, &params, &[note_on(note)])));
    }
    loudest = loudest.max(peak(&render(&mut engine, &params, &[note_on(52)])));
    let mut notes: Vec<u8> = engine.sounding_voices().map(|(n, _)| n).collect();
    notes.sort_unstable();
    assert_eq!(notes, vec![48, 52, 55, 59]);
    assert!(loudest > 1e-3, "rendered silence");
}
