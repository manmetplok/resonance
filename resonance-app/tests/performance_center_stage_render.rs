//! Render smoke-tests for the Performance centre-stage hero (todo #308).
//!
//! The view layer returns Iced `Element`s that can't be asserted on
//! structurally, so — like the scaffold suite (#307) — these exercise the
//! happy path: with a *placed progression* under the playhead the centre
//! stage builds the three-column hero (huge chord symbol + chord-tone chips
//! + Canvas fingering diagram) without panicking, across plain triads, a
//! seventh chord, and a slash/inverted chord (the bass-consistent voicing
//! path). The on-screen pixels are locked by the `iced_test` golden suite
//! (owned by the e2e-tester); the correctness of the underlying voicing /
//! layout data is covered headlessly in `performance_center_stage.rs`.

use std::collections::HashMap;

use resonance_app::compose::{ChordState, GenerateParams, SectionDefinitionState};
use resonance_app::state::ViewMode;
use resonance_app::Resonance;
use resonance_music_theory::{Chord, ChordQuality, MotifSource, PitchClass};

fn chord_state(id: u64, start_beat: u32, duration_beats: u32, chord: Chord) -> ChordState {
    ChordState {
        id,
        start_beat,
        duration_beats,
        chord,
    }
}

/// A minimal section definition carrying `chords`; the generator/motif
/// fields are defaulted (the Performance readout never reads them).
fn def(id: u64, length_bars: u32, chords: Vec<ChordState>) -> SectionDefinitionState {
    SectionDefinitionState {
        id,
        name: format!("S{id}"),
        color: [0, 0, 0],
        length_bars,
        chords,
        scale: None,
        progression_seed: 0,
        generate_params: GenerateParams::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators: HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: MotifSource::default(),
        arrangement: Vec::new(),
    }
}

/// Enter Performance mode with a single 4-bar section placed at bar 0 whose
/// first chord is `first` — so the default playhead (sample 0) lands on it.
fn perform_with_first_chord(first: Chord) -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);

    let chords = vec![
        chord_state(10, 0, 4, first),
        chord_state(11, 4, 4, Chord::new(PitchClass::G, ChordQuality::Maj)),
    ];
    app.test_push_section_definition(def(100, 4, chords));
    app.test_place_section(100, 0);

    app.test_set_view_mode(ViewMode::Performance);
    app
}

#[test]
fn renders_hero_for_a_plain_triad() {
    let app = perform_with_first_chord(Chord::new(PitchClass::C, ChordQuality::Maj));
    let _ = app.view();
}

#[test]
fn renders_hero_for_a_seventh_chord() {
    let app = perform_with_first_chord(Chord::new(PitchClass::D, ChordQuality::Min7));
    let _ = app.view();
}

#[test]
fn renders_hero_for_a_slash_chord() {
    // F/A — first inversion. Exercises the bass-consistent voicing + the
    // slash/bass tinting in the symbol.
    let f_over_a =
        Chord::new(PitchClass::F, ChordQuality::Maj).with_bass(PitchClass::A);
    let app = perform_with_first_chord(f_over_a);
    let _ = app.view();
}

#[test]
fn renders_hero_while_recording() {
    let mut app = perform_with_first_chord(Chord::new(PitchClass::A, ChordQuality::Min));
    app.test_set_transport_playing(true);
    app.test_set_transport_recording(true);
    let _ = app.view();
}
