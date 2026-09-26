//! Compose bar↔sample conversion must go through the project tempo map
//! in BOTH directions (ba todo #1163, Bug 3 of the control-API report).
//!
//! The old `compose_samples_per_bar` truncated `(samples_per_beat *
//! numerator) as u64`, and callers multiplied that truncated scalar by a
//! bar index — so at a non-integral samples-per-bar tempo (108 BPM at
//! 48 kHz = 106666.67 samples/bar) generated clips drifted earlier and
//! earlier with bar position, and the recovery side (`rebuild_derived_clips`)
//! gated on `start_sample % samples_per_bar == 0`, which only ever matched
//! because both directions shared the same wrong math. Switching only
//! placement to `tempo_map.bar_to_sample` would then orphan every derived
//! lane on reload.
//!
//! These tests pin both halves:
//!   1. a clip generated for bar N lands at exactly `tempo_map.bar_to_sample(N)`;
//!   2. save + reload fully repopulates `derived_clips`.

use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::{AudioCommand, AudioEvent, TrackType};
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::methods::generate::{self as proto, GenerateResult, GenerateRole};
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::{KeyScale, Request, Response};
use crate::common::roundtrip;

// 108 BPM at 48 kHz in 4/4 has a non-integral samples-per-bar:
// 48000 * 60 / 108 * 4 = 106666.666…
const SR: u32 = 48_000;
const BPM: f32 = 108.0;

fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/compose-bar-sample.rprj"));
    app.test_set_sample_rate(SR);
    // Flat 108 BPM — non-integral samples-per-bar.
    app.test_set_flat_tempo(BPM);
    app
}

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

/// A section with a 4-chord Am-key progression, returning its id.
fn section_with_chords(app: &mut Resonance) -> SectionDefinitionId {
    let response = call(
        app,
        "section.create",
        &section_proto::CreateParams {
            name: "Verse".to_owned(),
            length_bars: 4,
            scale: None,
            place: true,
        },
    );
    let section_id = response
        .result::<section_proto::CreateResult>()
        .expect("section.create succeeds")
        .section_id;

    let mut params = harmony_proto::ApplyProgressionParams::for_section(section_id);
    params.key = Some(KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(
        ["i", "iv", "v", "i"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    );
    let response = call(app, "harmony.apply_progression", &params);
    response
        .result::<harmony_proto::ApplyProgressionResult>()
        .expect("progression applies");
    section_id
}

fn add_synth_track(app: &mut Resonance, id: u64) -> ProtoTrackId {
    app.test_add_track(id, TrackType::Instrument);
    ProtoTrackId(id)
}

/// Drop every placement, then place the section exactly once at `start_bar`,
/// so exactly one derived clip is produced and we know its target bar.
fn place_only_at(app: &mut Resonance, definition_id: u64, start_bar: u32) {
    app.test_clear_placements();
    app.test_place_section(definition_id, start_bar);
}

fn generate_part(app: &mut Resonance, section_id: SectionDefinitionId, track: ProtoTrackId) {
    let params = proto::PartParams {
        section_id,
        track_id: track,
        role: GenerateRole::Pad,
        chord_count: None,
        beats_per_chord: None,
        sevenths: None,
        seed: Some(42),
        options: None,
    };
    let response = call(app, "generate.part", &params);
    response
        .result::<GenerateResult>()
        .expect("generate.part succeeds");
}

/// Pull the single `LoadMidiClipDirect` a derive emitted out of the captured
/// command stream, returning `(clip_id, start_sample)`.
fn load_direct_start(
    rx: &resonance_audio::test_support::Receiver<AudioCommand>,
) -> (u64, u64) {
    let mut found = None;
    while let Ok(cmd) = rx.try_recv() {
        if let AudioCommand::LoadMidiClipDirect {
            clip_id,
            start_sample,
            ..
        } = cmd
        {
            assert!(
                found.is_none(),
                "expected exactly one LoadMidiClipDirect for one placement",
            );
            found = Some((u64::from(clip_id), start_sample));
        }
    }
    found.expect("a LoadMidiClipDirect was emitted")
}

// ---------------------------------------------------------------------------
// Test 1: placement lands on the exact tempo-map bar sample.
// ---------------------------------------------------------------------------

#[test]
fn generated_clip_lands_on_exact_tempo_map_bar_sample() {
    let mut app = app_with_project();
    let section_id = section_with_chords(&mut app);
    let track = add_synth_track(&mut app, 10);

    // Bars whose truncated-scalar placement drifted noticeably per the bug
    // report (73 was 48 samples early, 113 was 75 samples early).
    for bar in [0u32, 1, 4, 73, 113] {
        place_only_at(&mut app, u64::from(section_id), bar);

        let rx = app.test_capture_engine();
        generate_part(&mut app, section_id, track);
        let (_clip_id, start_sample) = load_direct_start(&rx);

        let expected = app.test_tempo_map().bar_to_sample(bar);
        assert_eq!(
            start_sample, expected,
            "bar {bar}: clip must sit at tempo_map.bar_to_sample = {expected}, got {start_sample}",
        );
    }

    // Nail the documented true positions so a regression to the truncated
    // scalar is caught outright. The bug report counts bars 1-based; this
    // API is 0-based, so its "bar 73" is bar_to_sample(72) = 7,680,000 (the
    // truncated scalar gave 106666*72 = 7,679,952) and its "bar 113" is
    // bar_to_sample(112) = 11,946,667 (truncated 11,946,592).
    assert_eq!(app.test_tempo_map().bar_to_sample(72), 7_680_000);
    assert_eq!(app.test_tempo_map().bar_to_sample(112), 11_946_667);
}

// ---------------------------------------------------------------------------
// Test 2: save/reload fully repopulates derived_clips.
// ---------------------------------------------------------------------------

#[test]
fn derived_clips_survive_save_and_reload_at_non_integral_tempo() {
    let mut app = app_with_project();
    let section_id = section_with_chords(&mut app);
    let track = add_synth_track(&mut app, 10);

    // Place at bar 73 — the non-integral offset that a `%` gate against a
    // truncated scalar would reject on reload (7_680_000 % 106_666 == 48).
    place_only_at(&mut app, u64::from(section_id), 73);

    // Generate, capturing the derived clip's id + start sample so we can
    // mirror the engine's `MidiClipCreated` echo (the live engine does this
    // async; a test app has no engine thread).
    let rx = app.test_capture_engine();
    generate_part(&mut app, section_id, track);
    let (clip_id, start_sample) = load_direct_start(&rx);

    assert_eq!(
        start_sample,
        app.test_tempo_map().bar_to_sample(73),
        "sanity: derived clip placed via the tempo map",
    );

    // The derived clip is already in the map at generation time.
    assert_eq!(app.test_derived_clip_count(10), 1, "one derived clip after generate");

    // Mirror the empty clip into app state so it is serialized on save.
    app.test_apply_engine_event(AudioEvent::MidiClipCreated {
        clip_id,
        track_id: u64::from(track),
        start_sample,
        duration_ticks: 4 * 4 * resonance_audio::types::TICKS_PER_QUARTER_NOTE,
        name: "derived".to_owned(),
        notes: Vec::new(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });

    // Serialize and reload as if opening the project from disk.
    let file = app.test_build_project_file();
    app.test_replay_loaded_project(file);

    // The recovery side must re-associate the loaded clip with bar 73's
    // placement — this is exactly what the coupled sample→bar fix protects.
    assert_eq!(
        app.test_derived_clip_count(10),
        1,
        "derived_clips must be fully repopulated after reload",
    );
}
