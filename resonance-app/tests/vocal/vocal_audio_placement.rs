//! Rendered vocal audio must land where the lane's notes were written
//! (ba doc #272 V-1).
//!
//! The SVS render is relative to the lane's first note — `render_cache`
//! subtracts `base_tick = notes[0].start_tick` — so the waveform starts
//! at that note with no leading silence. Placing the clip at the section
//! boundary therefore played the whole phrase early by however far into
//! the section it was written, silently, with relative timing inside the
//! phrase preserved. A lane whose first note sat at beat 96 landed 24
//! bars early.

use resonance_app::compose::vocal_svs::vocal_audio_start;
use resonance_audio::types::{TempoMap, TICKS_PER_QUARTER_NOTE};

const SR: u32 = 48_000;
const BPM: f32 = 120.0;
/// One beat at 120 BPM / 48 kHz.
const BEAT_SAMPLES: u64 = 24_000;

fn flat_tempo_map() -> TempoMap {
    let mut tm = TempoMap::default();
    tm.bpm = BPM;
    tm.numerator = 4;
    tm.denominator = 4;
    tm.rebuild_bar_table(SR);
    tm
}

fn beats(n: u64) -> u64 {
    n * TICKS_PER_QUARTER_NOTE
}

#[test]
fn a_lane_starting_on_the_downbeat_is_unmoved() {
    let tm = flat_tempo_map();
    let section_start = tm.bar_to_sample(16); // bar 17, 1-based
    assert_eq!(
        vocal_audio_start(&tm, section_start, 0, SR),
        section_start,
        "a first note at beat 0 must not shift the clip"
    );
}

#[test]
fn the_clip_advances_by_the_first_notes_offset() {
    let tm = flat_tempo_map();
    let section_start = tm.bar_to_sample(16);

    // The reporter's case: a single note at section beat 8 sounded at
    // the section start instead of two bars in.
    let placed = vocal_audio_start(&tm, section_start, beats(8), SR);
    assert_eq!(
        placed - section_start,
        8 * BEAT_SAMPLES,
        "a note at beat 8 places its audio 8 beats into the section"
    );
}

/// The pathological case from the field report: a Coda lane holding one
/// note at beat 112 rendered from the section start, smearing the phrase
/// across the whole section.
#[test]
fn a_far_offset_note_is_not_dragged_to_the_section_start() {
    let tm = flat_tempo_map();
    let section_start = tm.bar_to_sample(48);
    let placed = vocal_audio_start(&tm, section_start, beats(112), SR);
    assert_eq!(placed - section_start, 112 * BEAT_SAMPLES);
}

#[test]
fn the_offset_scales_with_the_note_position() {
    let tm = flat_tempo_map();
    let section_start = tm.bar_to_sample(8);
    let mut previous = section_start;
    for beat in [1u64, 2, 4, 8, 16, 32] {
        let placed = vocal_audio_start(&tm, section_start, beats(beat), SR);
        assert!(
            placed > previous,
            "beat {beat} must place later than the previous offset"
        );
        assert_eq!(placed - section_start, beat * BEAT_SAMPLES);
        previous = placed;
    }
}

/// The offset goes through the tempo map, so a tempo change before the
/// section still yields the right musical position rather than a flat
/// samples-per-tick guess.
#[test]
fn the_offset_follows_a_tempo_change() {
    let mut tm = TempoMap::default();
    tm.tempo_points = vec![
        resonance_audio::types::TempoPoint { bar: 0, bpm: 120.0 },
        resonance_audio::types::TempoPoint { bar: 8, bpm: 60.0 },
    ];
    tm.signature_points = vec![resonance_audio::types::SignaturePoint {
        bar: 0,
        numerator: 4,
        denominator: 4,
    }];
    tm.bpm = 120.0;
    tm.numerator = 4;
    tm.denominator = 4;
    tm.rebuild_bar_table(SR);

    // A section well after the tempo drop: one beat is 48000 samples.
    let section_start = tm.bar_to_sample(16);
    let one_beat = vocal_audio_start(&tm, section_start, beats(1), SR) - section_start;
    assert_eq!(
        one_beat, 48_000,
        "a beat at 60 BPM is 48000 samples, not the 24000 of the song's opening tempo"
    );
}
