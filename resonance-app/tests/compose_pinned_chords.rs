//! Coverage for constraining Compose regeneration to the user's pinned
//! chord-track harmony (epic #33, doc #168, todo #445).
//!
//! Three layers:
//! * `overlay_pinned_chords` — the pure overlay that re-symbols a
//!   section's chords from pinned chord-track regions.
//! * overlay → `derive_notes` — that the overlaid chords actually change
//!   the generated MIDI ("pinned chords flow through to generated notes").
//! * `apply_chord_track_harmony` via `Resonance` — that lane regeneration
//!   anchors a section to its placement, overlays pinned regions, and
//!   adopts the chord track's key context.

use resonance_app::chord_track::{ChordRegion, ChordTrack, KeyChange};
use resonance_app::compose::generate::{derive_notes, overlay_pinned_chords};
use resonance_app::compose::{
    ChordState, DeriveKind, GenerateParams, SectionDefinitionState,
};
use resonance_app::{Resonance, STARTUP_TAB};
use resonance_audio::types::TICKS_PER_QUARTER_NOTE;
use resonance_music_theory::{
    BassParams, BassStyle, Chord, ChordQuality, Mode, MotifSource, PitchClass, Scale,
};

// 48 kHz @ 120 BPM → one beat is 24 000 samples.
const SAMPLE_RATE: u32 = 48_000;
const SAMPLES_PER_BEAT: f64 = 24_000.0;

fn maj(root: PitchClass) -> Chord {
    Chord::new(root, ChordQuality::Maj)
}

fn min(root: PitchClass) -> Chord {
    Chord::new(root, ChordQuality::Min)
}

fn chord_state(id: u64, start_beat: u32, duration_beats: u32, chord: Chord) -> ChordState {
    ChordState {
        id,
        start_beat,
        duration_beats,
        chord,
    }
}

fn pinned_region(id: u64, start: u64, end: u64, chord: Chord) -> ChordRegion {
    ChordRegion {
        id,
        chord,
        start_sample: start,
        end_sample: end,
        pinned: true,
    }
}

// ----------------- overlay_pinned_chords (pure) -----------------

#[test]
fn pinned_region_overrides_matching_chord_and_keeps_rhythm() {
    let section = vec![chord_state(10, 0, 4, min(PitchClass::A))];
    let mut track = ChordTrack::new();
    track.insert_region(pinned_region(1, 0, 100_000, maj(PitchClass::C)));

    let out = overlay_pinned_chords(&section, &track, 0, SAMPLES_PER_BEAT);

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].chord, maj(PitchClass::C), "symbol follows the pin");
    // Rhythm + identity untouched — only the chord symbol changes.
    assert_eq!(out[0].id, 10);
    assert_eq!(out[0].start_beat, 0);
    assert_eq!(out[0].duration_beats, 4);
}

#[test]
fn unpinned_region_does_not_override() {
    let section = vec![chord_state(10, 0, 4, min(PitchClass::A))];
    let mut track = ChordTrack::new();
    let mut region = pinned_region(1, 0, 100_000, maj(PitchClass::C));
    region.pinned = false; // only explicit pins constrain regeneration
    track.insert_region(region);

    let out = overlay_pinned_chords(&section, &track, 0, SAMPLES_PER_BEAT);
    assert_eq!(out[0].chord, min(PitchClass::A), "unpinned regions are ignored");
}

#[test]
fn chord_in_a_gap_keeps_its_generated_symbol() {
    let section = vec![chord_state(10, 0, 4, min(PitchClass::A))];
    let mut track = ChordTrack::new();
    // Pinned region starts after beat 0 (sample 0), so it doesn't cover it.
    track.insert_region(pinned_region(1, 200_000, 300_000, maj(PitchClass::C)));

    let out = overlay_pinned_chords(&section, &track, 0, SAMPLES_PER_BEAT);
    assert_eq!(out[0].chord, min(PitchClass::A));
}

#[test]
fn each_chord_resolves_against_the_region_under_its_own_beat() {
    // Chord A at beat 0 (sample 0); chord B at beat 4 (sample 96 000).
    let section = vec![
        chord_state(10, 0, 4, min(PitchClass::A)),
        chord_state(11, 4, 4, min(PitchClass::A)),
    ];
    let mut track = ChordTrack::new();
    track.insert_region(pinned_region(1, 0, 48_000, maj(PitchClass::C)));
    track.insert_region(pinned_region(2, 48_000, 200_000, maj(PitchClass::F)));

    let out = overlay_pinned_chords(&section, &track, 0, SAMPLES_PER_BEAT);
    assert_eq!(out[0].chord, maj(PitchClass::C), "beat 0 → first pin");
    assert_eq!(out[1].chord, maj(PitchClass::F), "beat 4 → second pin");
}

#[test]
fn section_start_offset_shifts_the_lookup() {
    // Section anchored at sample 96 000 (= bar 1 @ 120 BPM, 4/4). Its
    // beat-0 chord must resolve against the region covering sample 96 000.
    let section = vec![chord_state(10, 0, 4, min(PitchClass::A))];
    let mut track = ChordTrack::new();
    track.insert_region(pinned_region(1, 0, 48_000, maj(PitchClass::C)));
    track.insert_region(pinned_region(2, 48_000, 200_000, maj(PitchClass::F)));

    let out = overlay_pinned_chords(&section, &track, 96_000, SAMPLES_PER_BEAT);
    assert_eq!(out[0].chord, maj(PitchClass::F));
}

// ----------------- overlay → derive_notes -----------------

/// Pitch sequence of a Root-hold bass derive (MidiNote has no `PartialEq`,
/// and pitch is what a chord change moves).
fn root_hold_bass_pitches(chords: &[ChordState]) -> Vec<u8> {
    let params = GenerateParams {
        bass: BassParams {
            style: BassStyle::RootHold,
            ..BassParams::default()
        },
        ..GenerateParams::default()
    };
    derive_notes(
        DeriveKind::Bass,
        chords,
        None,
        &params,
        &MotifSource::default(),
        TICKS_PER_QUARTER_NOTE as u32,
        0,
    )
    .iter()
    .map(|n| n.note)
    .collect()
}

#[test]
fn pinned_chord_flows_through_to_generated_notes() {
    let section = vec![chord_state(10, 0, 4, min(PitchClass::A))];
    let mut track = ChordTrack::new();
    track.insert_region(pinned_region(1, 0, 100_000, maj(PitchClass::C)));

    let overlaid = overlay_pinned_chords(&section, &track, 0, SAMPLES_PER_BEAT);

    let from_pinned = root_hold_bass_pitches(&overlaid);
    let from_original = root_hold_bass_pitches(&section);
    let direct_c = root_hold_bass_pitches(&[chord_state(10, 0, 4, maj(PitchClass::C))]);

    // Overlaying the pin makes the bass voice the pinned C, not the
    // section's own A-minor.
    assert_eq!(
        from_pinned, direct_c,
        "generated bass follows the pinned C-major root"
    );
    assert_ne!(
        from_pinned, from_original,
        "the pin must change the generated notes"
    );
}

// ----------------- apply_chord_track_harmony via Resonance -----------------

fn placed_section(app: &mut Resonance, chords: Vec<ChordState>, scale: Option<Scale>) -> u64 {
    let def = SectionDefinitionState {
        id: 1,
        name: "S".to_string(),
        color: [0, 0, 0],
        length_bars: 1,
        chords,
        scale,
        progression_seed: 0,
        generate_params: GenerateParams::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators: std::collections::HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: MotifSource::default(),
        arrangement: Vec::new(),
    };
    app.test_push_section_definition(def);
    // Placed at the song start so the section maps to absolute sample 0.
    let _placement: u64 = app.test_place_section(1, 0);
    1
}

fn new_app() -> Resonance {
    let _ = STARTUP_TAB.set(resonance_app::state::ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    app.test_set_sample_rate(SAMPLE_RATE);
    app
}

#[test]
fn regeneration_adopts_pinned_chords_and_key_context() {
    let mut app = new_app();
    let def_id = placed_section(
        &mut app,
        vec![chord_state(10, 0, 4, min(PitchClass::A))],
        Some(Scale::new(PitchClass::C, Mode::Major)),
    );

    {
        let track = app.test_chord_track_mut();
        track.insert_region(pinned_region(20, 0, 10_000_000, maj(PitchClass::C)));
        track.insert_key_change(KeyChange {
            id: 21,
            start_sample: 0,
            scale: Scale::new(PitchClass::G, Mode::Mixolydian),
        });
    }

    let (chords, scale) = app.test_section_harmony(def_id);
    assert_eq!(chords[0].chord, maj(PitchClass::C), "pin overrides the section chord");
    assert_eq!(chords[0].id, 10, "rhythm/identity preserved");
    assert_eq!(
        scale,
        Some(Scale::new(PitchClass::G, Mode::Mixolydian)),
        "scale comes from the chord track's key context"
    );
}

#[test]
fn regeneration_leaves_section_untouched_without_pins_or_keys() {
    let mut app = new_app();
    let def_id = placed_section(
        &mut app,
        vec![chord_state(10, 0, 4, min(PitchClass::A))],
        Some(Scale::new(PitchClass::C, Mode::Major)),
    );

    // A non-pinned region in the chord track must not constrain anything.
    {
        let track = app.test_chord_track_mut();
        track.insert_region(ChordRegion {
            id: 20,
            chord: maj(PitchClass::C),
            start_sample: 0,
            end_sample: 10_000_000,
            pinned: false,
        });
    }

    let (chords, scale) = app.test_section_harmony(def_id);
    assert_eq!(chords[0].chord, min(PitchClass::A), "no pins → section chord stays");
    assert_eq!(
        scale,
        Some(Scale::new(PitchClass::C, Mode::Major)),
        "no key context → section's own scale stays"
    );
}
