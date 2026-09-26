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

fn note_off(note: u8) -> NoteEvent {
    NoteEvent::NoteOff { note, timing: 0 }
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

/// Mono legato does not retrigger (FU-G2d): a note pressed while another is
/// held takes over the voice *without* restarting its envelopes. The voice
/// used to be stolen and re-`trigger`ed, so the amp envelope jumped from
/// its sustain level back into the attack — an audible swell on every
/// legato note. Also checked without glide: legato is a mono property, not
/// a portamento one.
#[test]
fn mono_legato_does_not_retrigger_the_envelope() {
    for glide in [false, true] {
        let params = WavetableParams::new();
        params.max_voices.set_value(1);
        params.glide_enabled.set_value(glide);
        params.glide_time.set_value(50.0);
        // Fast attack and decay into a low sustain: a retrigger would lift
        // the level from 0.2 back toward 1.0 within a few milliseconds.
        params.amp_env.attack.set_value(0.002);
        params.amp_env.decay.set_value(0.01);
        params.amp_env.sustain.set_value(0.2);
        params.amp_env.release.set_value(0.5);
        let mut engine = SynthEngine::new();
        engine.initialize(SR);

        render(&mut engine, &params, &[note_on(48)]);
        let mut sustain_peak = 0.0f32;
        for i in 0..40 {
            let p = peak(&render(&mut engine, &params, &[]));
            if i >= 30 {
                sustain_peak = sustain_peak.max(p);
            }
        }
        assert!(sustain_peak > 1e-3, "glide={glide}: rendered silence");

        // Legato G3 while C3 is still held; watch the next ~50 ms.
        let mut after = peak(&render(&mut engine, &params, &[note_on(55)]));
        for _ in 0..18 {
            after = after.max(peak(&render(&mut engine, &params, &[])));
        }
        assert!(
            after < sustain_peak * 1.5,
            "glide={glide}: legato note retriggered the envelope \
             (sustain peak {sustain_peak:.3}, after legato {after:.3})"
        );
        let voices: Vec<(u8, f32)> = engine.sounding_voices().collect();
        assert_eq!(voices.len(), 1, "glide={glide}: mono played {voices:?}");
        assert_eq!(voices[0].0, 55, "glide={glide}: legato note not taken over");

        // A note after every key was *released* still retriggers. (Only
        // releasing 55 would return the voice to the still-held 48 — see
        // `mono_release_returns_to_the_most_recent_held_key`.)
        render(&mut engine, &params, &[note_off(55), note_off(48)]);
        let mut fresh = peak(&render(&mut engine, &params, &[note_on(60)]));
        for _ in 0..4 {
            fresh = fresh.max(peak(&render(&mut engine, &params, &[])));
        }
        assert!(
            fresh > sustain_peak * 2.0,
            "glide={glide}: a detached note did not retrigger ({fresh:.3})"
        );
    }
}

/// Mono held-note stack (FU-G2d): releasing the sounding legato note while
/// earlier keys are still down returns the voice — legato, no retrigger —
/// to the most recent still-held key. Releasing a held key that is not
/// sounding changes nothing. The voice only releases when no key is held.
/// Checked with and without glide: with glide the return glides.
#[test]
fn mono_release_returns_to_the_most_recent_held_key() {
    for glide in [false, true] {
        let params = WavetableParams::new();
        params.max_voices.set_value(1);
        params.glide_enabled.set_value(glide);
        params.glide_time.set_value(50.0);
        params.amp_env.attack.set_value(0.002);
        params.amp_env.decay.set_value(0.01);
        params.amp_env.sustain.set_value(0.5);
        // Short release: a released voice is near-silent ~100 ms later.
        params.amp_env.release.set_value(0.02);
        let mut engine = SynthEngine::new();
        engine.initialize(SR);
        let blocks_100ms = (SR as usize / 10) / BLOCK;
        let settle = |engine: &mut SynthEngine| {
            let mut p = 0.0f32;
            for _ in 0..blocks_100ms {
                p = peak(&render(engine, &params, &[]));
            }
            p
        };

        render(&mut engine, &params, &[note_on(48)]);
        render(&mut engine, &params, &[note_on(52)]);
        render(&mut engine, &params, &[note_on(55)]);
        let held_level = settle(&mut engine);
        assert!(held_level > 1e-2, "glide={glide}: rendered silence");
        assert_eq!(engine.sounding_voices().map(|(n, _)| n).collect::<Vec<_>>(), vec![55]);

        // Releasing 52 (held, not sounding) leaves 55 sounding.
        render(&mut engine, &params, &[note_off(52)]);
        assert_eq!(
            engine.sounding_voices().map(|(n, _)| n).collect::<Vec<_>>(),
            vec![55],
            "glide={glide}: releasing a non-sounding held key moved the voice"
        );

        // Releasing 55 returns to 48 (the only key still down; 52 is up).
        let mut after = peak(&render(&mut engine, &params, &[note_off(55)]));
        let voices: Vec<(u8, f32)> = engine.sounding_voices().collect();
        assert_eq!(voices.len(), 1, "glide={glide}: mono played {voices:?}");
        assert_eq!(voices[0].0, 48, "glide={glide}: did not return to the held key");
        if glide {
            assert!(
                voices[0].1 > 48.0 && voices[0].1 < 55.0,
                "glide={glide}: return did not glide (pitch {})",
                voices[0].1
            );
        } else {
            assert_eq!(voices[0].1, 48.0, "glide={glide}: return did not jump");
        }
        for _ in 0..18 {
            after = after.max(peak(&render(&mut engine, &params, &[])));
        }
        // Still held at the sustain level: neither released nor retriggered.
        let returned_level = settle(&mut engine);
        assert!(
            returned_level > held_level * 0.5,
            "glide={glide}: the voice released instead of returning \
             ({held_level:.3} -> {returned_level:.3})"
        );
        assert!(
            after < held_level * 1.5,
            "glide={glide}: the return retriggered the envelope \
             ({held_level:.3} -> {after:.3})"
        );
        for _ in 0..(SR as usize / BLOCK) {
            render(&mut engine, &params, &[]);
        }
        let (_, settled) = engine.sounding_voices().next().expect("voice sounding");
        assert!((settled - 48.0).abs() < 0.01, "glide={glide}: pitch {settled}");

        // Releasing the last held key releases the voice.
        render(&mut engine, &params, &[note_off(48)]);
        let released = settle(&mut engine);
        assert!(
            released < held_level * 0.05,
            "glide={glide}: voice kept sounding with no key held ({released:.4})"
        );
    }
}

/// The held-note stack is fixed-size: holding more keys than it has slots
/// forgets the oldest ones, but the newest still return in order and the
/// voice still releases once every key is up.
#[test]
fn mono_held_note_stack_overflow_keeps_the_newest_keys() {
    let params = WavetableParams::new();
    params.max_voices.set_value(1);
    params.amp_env.release.set_value(0.02);
    let mut engine = SynthEngine::new();
    engine.initialize(SR);

    let keys: Vec<u8> = (40..80).collect();
    for &k in &keys {
        render(&mut engine, &params, &[note_on(k)]);
    }
    // Release from the top: each release returns to the next lower key.
    for w in keys.windows(2).rev().take(8) {
        render(&mut engine, &params, &[note_off(w[1])]);
        let notes: Vec<u8> = engine.sounding_voices().map(|(n, _)| n).collect();
        assert_eq!(notes, vec![w[0]], "release of {} did not return to {}", w[1], w[0]);
    }
    let events: Vec<NoteEvent> = keys.iter().map(|&k| note_off(k)).collect();
    render(&mut engine, &params, &events);
    for _ in 0..(SR as usize / BLOCK) {
        render(&mut engine, &params, &[]);
    }
    assert_eq!(engine.sounding_voices().count(), 0, "voice never released");
}
