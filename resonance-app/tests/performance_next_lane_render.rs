//! Render smoke-tests for the Performance next-chords look-ahead lane
//! (todo #309, design #151, arch doc #152).
//!
//! The view layer returns Iced `Element`s that can't be asserted on
//! structurally, so — like the centre-stage suite (#308) — these exercise
//! the happy path through `Resonance::view`: with a *placed progression*
//! rolling, the lane derives the next chords and builds the cards (symbol +
//! mini chord-box + bars-until) without panicking; with no progression it
//! falls back to the empty state. The on-screen pixels are locked by the
//! `iced_test` golden suite (owned by the e2e-tester); the correctness of the
//! emphasis / bars-until / fingerprint data is covered headlessly in
//! `performance_next_lane.rs`.

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

/// A minimal section definition carrying `chords`; the generator/motif fields
/// are defaulted (the Performance readout never reads them).
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

fn maj(root: PitchClass) -> Chord {
    Chord::new(root, ChordQuality::Maj)
}

/// Enter Performance mode with a four-chord, one-per-bar progression placed at
/// bar 0 — so the default playhead (sample 0) lands on the first chord and the
/// lane previews the next three.
fn perform_with_progression() -> Resonance {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);

    let chords = vec![
        chord_state(10, 0, 4, maj(PitchClass::C)),
        chord_state(11, 4, 4, maj(PitchClass::F)),
        chord_state(12, 8, 4, maj(PitchClass::G)),
        chord_state(13, 12, 4, Chord::new(PitchClass::A, ChordQuality::Min)),
    ];
    app.test_push_section_definition(def(100, 4, chords));
    app.test_place_section(100, 0);
    app.test_set_view_mode(ViewMode::Performance);
    app
}

#[test]
fn renders_lane_with_upcoming_chords() {
    let app = perform_with_progression();
    // The lane builds three look-ahead cards (F, G, Am) with mini diagrams.
    let _ = app.view();
}

#[test]
fn renders_lane_with_a_slash_chord_upcoming() {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    let chords = vec![
        chord_state(20, 0, 4, maj(PitchClass::C)),
        // F/A — an upcoming inversion exercises the bass-consistent mini box.
        chord_state(21, 4, 4, maj(PitchClass::F).with_bass(PitchClass::A)),
    ];
    app.test_push_section_definition(def(200, 4, chords));
    app.test_place_section(200, 0);
    app.test_set_view_mode(ViewMode::Performance);
    let _ = app.view();
}

#[test]
fn renders_empty_lane_with_no_progression() {
    let (mut app, _task) = Resonance::new();
    app.test_set_active_project(true);
    app.test_set_view_mode(ViewMode::Performance);
    // No placed sections: the lane shows the "no upcoming chords" empty state.
    let _ = app.view();
}
