//! A brand-new project must have a usable tempo-map bar table.
//!
//! File → New (`UiMessage::StartNewProject`) never runs the project-load
//! replay, and construction used to leave `TempoMap::default()` in place:
//! empty bar table, `table_sample_rate: 0`, under which `bar_to_sample`
//! maps EVERY bar to sample 0. The Compose track canvas gates its drawing
//! on the placed section's sample span (`section_end <= section_start`
//! in `view/compose/tracks/canvas.rs`), so on a fresh project every synth
//! lane was silently blank until the project was saved and reopened —
//! reopening replays, and the replay rebuilds the table. Tracks added
//! after that reopen showed up live, which is what pinned the bug to
//! initialization rather than to the (fully live) track list.
//!
//! These tests pin the two rebuild points the fix added: once at
//! construction (at the 44.1k placeholder rate) and again when the engine
//! reports the real device rate via `SampleRateDetected`.

use resonance_app::compose::{GenerateParams, SectionDefinitionState};
use resonance_app::state::InstrumentType;
use resonance_app::Resonance;
use resonance_audio::types::{AudioEvent, TrackType};
use resonance_music_theory::MotifSource;

/// Construction defaults: 120 BPM, 4/4, 44 100 Hz placeholder rate →
/// one bar is 44 100 * 60 / 120 * 4 = 88 200 samples.
const SAMPLES_PER_BAR_AT_44_1K: u64 = 88_200;

/// A minimal 4-bar section definition, as the "+ Section" flow creates.
fn section_def(id: u64) -> SectionDefinitionState {
    SectionDefinitionState {
        id,
        name: "Intro".to_string(),
        color: [0, 0, 0],
        length_bars: 4,
        chords: Vec::new(),
        scale: None,
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
    }
}

#[test]
fn fresh_app_bar_table_is_built_at_construction() {
    let (app, _task) = Resonance::new_for_test();

    assert_eq!(
        app.test_tempo_map().bar_to_sample(1),
        SAMPLES_PER_BAR_AT_44_1K,
        "a fresh app must map bars through a built bar table \
         (an empty TempoMap::default table maps every bar to 0)",
    );
}

#[test]
fn sample_rate_detected_rebuilds_bar_table() {
    let (mut app, _task) = Resonance::new_for_test();

    // The engine reports the real device rate after the stream opens.
    app.test_apply_engine_event(AudioEvent::SampleRateDetected {
        sample_rate: 96_000,
    });

    // 96 000 * 60 / 120 * 4 = 192 000 samples per bar.
    assert_eq!(
        app.test_tempo_map().bar_to_sample(1),
        192_000,
        "the bar table is denominated in samples and must be rebuilt \
         at the detected rate",
    );
}

/// The user-visible symptom: on a fresh project with a placed section and
/// a synth track, the Compose track canvas must have both a row to draw
/// and a non-degenerate section span — the two conditions its draw guard
/// checks before rendering any lane.
#[test]
fn fresh_project_compose_section_span_is_nondegenerate() {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);

    app.test_push_section_definition(section_def(1));
    app.test_place_section(1, 0);
    app.test_add_track(10, TrackType::Instrument);

    // Exactly the span `view/compose/tracks/mod.rs` hands the canvas for
    // a 4-bar section placed at bar 0.
    let section_start = app.test_tempo_map().bar_to_sample(0);
    let section_end = app.test_tempo_map().bar_to_sample(4);
    assert!(
        section_end > section_start,
        "degenerate section span ({section_start}..{section_end}): the canvas's \
         `section_end <= section_start` guard would draw no synth lane at all",
    );

    // And the row it should draw is there: the canvas's sorted_tracks()
    // predicate matches the freshly added synth track.
    let rows = app
        .test_registry()
        .tracks
        .iter()
        .filter(|t| {
            matches!(t.track_type, TrackType::Instrument)
                && t.sub_track.is_none()
                && t.instrument_type != InstrumentType::Drum
        })
        .count();
    assert_eq!(rows, 1, "the added synth track must be a Compose canvas row");
}
